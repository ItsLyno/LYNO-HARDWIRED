//! Nexus Mods in the launcher: the player's account, version tracking of
//! mods from Nexus, and downloads from nxm links (the launcher can take over
//! "Mod Manager Download" from MO2 or Vortex).
//!
//! Downloads run one at a time in a queue: Nexus serves free accounts one
//! file at a time anyway, and installs must not overlap in `modlist.txt`.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lyno_core::download::Downloader;
use lyno_core::mo2::Instance;
use lyno_core::mod_install::{self, BuildMod, Context, Outcome, Progress};
use lyno_core::nexus::{NexusApi, RateLimit, User};
use lyno_core::nexus_sso;
use lyno_core::nxm::{self, HandlerStatus, NxmLink};
use lyno_core::tracking::{self, Cache, Status, Tracked};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

use crate::commands::{err, profile, running_processes, CmdResult};
use crate::secrets::{self, Secret};
use crate::AppState;

/// Slug of the launcher's application registered with Nexus, for SSO. Set at
/// build time (`LYNO_NEXUS_APP`) once Nexus approves the application; until
/// then players log in with a personal API key.
const SSO_APP: Option<&str> = option_env!("LYNO_NEXUS_APP");

#[derive(Default)]
pub struct NexusState {
    /// The account behind the saved key, once validated in this session.
    account: Mutex<Option<User>>,
    check: Mutex<Option<Arc<AtomicBool>>>,
    sso: Mutex<Option<Arc<AtomicBool>>>,
    jobs: Mutex<Vec<Job>>,
    worker: AtomicBool,
    next_id: AtomicU64,
    rate_limit: Mutex<Option<RateLimit>>,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn api() -> CmdResult<NexusApi> {
    secrets::get(Secret::NexusKey).map(|k| NexusApi::new(&k)).ok_or_else(|| "Войдите в Nexus Mods на вкладке «Nexus»".into())
}

/// Nexus refusals in the player's words.
fn nexus_error(e: &lyno_core::Error) -> String {
    match e {
        lyno_core::Error::Nexus { status: 401, .. } => "Ключ Nexus больше не действует: войдите заново на вкладке «Nexus»".into(),
        lyno_core::Error::Nexus { status: 403, message } => format!(
            "Nexus не дал скачать файл{}. Без Premium скачивание идёт только по кнопке «Mod Manager Download» на сайте.",
            if message.is_empty() { String::new() } else { format!(" ({message})") }
        ),
        lyno_core::Error::Nexus { status: 410, .. } => {
            "Ссылка на скачивание устарела: нажмите «Mod Manager Download» на сайте ещё раз".into()
        }
        lyno_core::Error::Nexus { status: 429, .. } => {
            "Исчерпан лимит запросов к Nexus API. Попробуйте через час.".into()
        }
        lyno_core::Error::Nexus { status, message } => format!("Nexus ответил ошибкой {status}: {message}"),
        lyno_core::Error::Download(m) => format!("Не удалось связаться с Nexus: {m}"),
        e => e.to_string(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusStatus {
    key_saved: bool,
    account: Option<User>,
    /// The saved key could not be checked (offline) or no longer works.
    account_error: Option<String>,
    /// "Log in with Nexus" works: the launcher has an application slug.
    sso: bool,
    handler: HandlerStatus,
}

fn handler() -> HandlerStatus {
    std::env::current_exe().map(|exe| nxm::handler_status(&exe)).unwrap_or_default()
}

/// The account behind the saved key; validates it once per session.
#[tauri::command]
pub async fn nexus_status(app: AppHandle) -> CmdResult<NexusStatus> {
    let state = app.state::<AppState>();
    let key = secrets::get(Secret::NexusKey);
    let mut account = state.nexus.account.lock().unwrap().clone();
    let mut account_error = None;
    if let (Some(key), None) = (&key, &account) {
        let key = key.clone();
        match tauri::async_runtime::spawn_blocking(move || NexusApi::new(&key).validate()).await.map_err(err)? {
            Ok(user) => {
                *state.nexus.account.lock().unwrap() = Some(user.clone());
                account = Some(user);
            }
            Err(e) => {
                log::warn!("nexus key: {e}");
                account_error = Some(nexus_error(&e));
            }
        }
    }
    Ok(NexusStatus { key_saved: key.is_some(), account, account_error, sso: SSO_APP.is_some(), handler: handler() })
}

/// Checks a personal API key and saves it in Credential Manager.
#[tauri::command]
pub async fn nexus_set_key(app: AppHandle, key: String) -> CmdResult<NexusStatus> {
    let key = key.trim().to_owned();
    if key.is_empty() {
        return Err("Вставьте ключ API".into());
    }
    let check = key.clone();
    let user = tauri::async_runtime::spawn_blocking(move || NexusApi::new(&check).validate())
        .await
        .map_err(err)?
        .map_err(|e| match e {
            lyno_core::Error::Nexus { status: 401, .. } => "Nexus не принял ключ: проверьте, что он скопирован целиком".into(),
            e => nexus_error(&e),
        })?;
    secrets::set(Secret::NexusKey, &key).map_err(|e| format!("Не удалось сохранить в диспетчер учётных данных Windows: {e}"))?;
    log::info!("nexus: logged in as {} (premium {})", user.name, user.premium);
    *app.state::<AppState>().nexus.account.lock().unwrap() = Some(user);
    nexus_status(app).await
}

#[tauri::command]
pub fn nexus_logout(state: TauriState<'_, AppState>) -> CmdResult<()> {
    *state.nexus.account.lock().unwrap() = None;
    secrets::delete(Secret::NexusKey)
}

/// "Log in with Nexus": opens the approval page in the browser and waits for
/// the key. `nexus_sso_cancel` stops waiting.
#[tauri::command]
pub async fn nexus_sso_login(app: AppHandle) -> CmdResult<NexusStatus> {
    let slug = SSO_APP.ok_or("Вход через Nexus пока недоступен: вставьте ключ API")?;
    let cancel = Arc::new(AtomicBool::new(false));
    *app.state::<AppState>().nexus.sso.lock().unwrap() = Some(cancel.clone());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let open = |url: &str| {
            tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| lyno_core::Error::Download(e.to_string()))
        };
        nexus_sso::login(nexus_sso::SSO_URL, slug, &open, &|| cancel.load(Ordering::Relaxed), nexus_sso::LOGIN_TIMEOUT)
    })
    .await
    .map_err(err)?;
    *app.state::<AppState>().nexus.sso.lock().unwrap() = None;
    match result {
        Ok(key) => nexus_set_key(app, key).await,
        Err(lyno_core::Error::Cancelled) => nexus_status(app).await,
        Err(e) => {
            log::warn!("nexus sso: {e}");
            Err(format!("Не удалось войти через Nexus: {e}"))
        }
    }
}

#[tauri::command]
pub fn nexus_sso_cancel(state: TauriState<'_, AppState>) {
    if let Some(flag) = state.nexus.sso.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Makes the launcher the program that opens nxm links.
#[tauri::command]
pub fn nxm_register() -> CmdResult<HandlerStatus> {
    let exe = std::env::current_exe().map_err(err)?;
    nxm::register(&exe).map_err(|e| format!("Не удалось зарегистрировать обработчик ссылок nxm://: {e}"))?;
    log::info!("nxm links now open {}", exe.display());
    Ok(handler())
}

/// Gives nxm links back to the previous handler (MO2, Vortex).
#[tauri::command]
pub fn nxm_unregister() -> CmdResult<HandlerStatus> {
    nxm::unregister().map_err(|e| format!("Не удалось вернуть ссылки nxm:// прежней программе: {e}"))?;
    log::info!("nxm links handed back");
    Ok(handler())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModUpdateRow {
    folder: String,
    game: String,
    mod_id: u64,
    version: Option<String>,
    personal: bool,
    /// Installed by the build: a player gets its updates with the build.
    managed: bool,
    /// The launcher may update it from Nexus: the player's own mod, or any in author mode.
    can_update: bool,
    status: Status,
    /// Files tab, scrolled to the new file when known.
    page_url: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesView {
    mods: Vec<ModUpdateRow>,
    /// Mods without a Nexus page in `meta.ini`.
    untracked: Vec<String>,
    /// Unix seconds of the oldest check; `None` when some mod was never checked.
    checked_at: Option<u64>,
    rate_limit: Option<RateLimit>,
}

fn view(state: &AppState, tracked: Vec<Tracked>, untracked: Vec<String>, cache: &Cache) -> UpdatesView {
    let author = state.settings.lock().unwrap().author_mode;
    let mods = tracked
        .iter()
        .map(|t| {
            let status = tracking::status(t, cache.get(&t.game, t.mod_id));
            let new_file = match &status {
                Status::Update { file: Some(f), .. } => Some(f.file_id),
                _ => None,
            };
            ModUpdateRow {
                folder: t.folder.clone(),
                game: t.game.clone(),
                mod_id: t.mod_id,
                version: t.version.clone(),
                personal: t.personal,
                managed: t.managed,
                can_update: t.personal || author,
                page_url: nxm::file_page(&t.game, t.mod_id, new_file),
                status,
            }
        })
        .collect();
    UpdatesView {
        checked_at: cache.oldest(&tracked),
        mods,
        untracked,
        rate_limit: *state.nexus.rate_limit.lock().unwrap(),
    }
}

fn instance(state: &AppState) -> CmdResult<(Instance, String)> {
    let inst = Instance::new(&state.settings.lock().unwrap().instance_dir);
    let profile = profile(&inst);
    if !inst.modlist_path(&profile).is_file() {
        return Err("Сборка не установлена".into());
    }
    Ok((inst, profile))
}

/// What the last check knew, without asking Nexus.
#[tauri::command]
pub fn nexus_updates(state: TauriState<'_, AppState>) -> CmdResult<UpdatesView> {
    let (inst, profile) = instance(&state)?;
    let (tracked, untracked) = tracking::tracked_mods(&inst, &profile).map_err(err)?;
    Ok(view(&state, tracked, untracked, &Cache::load(&Cache::path(&inst))))
}

#[derive(Clone, Serialize)]
struct CheckProgress {
    done: usize,
    total: usize,
}

/// Asks Nexus about mods whose files changed since the last check (`force`:
/// about every mod). Progress arrives as `nexus-check` events; `None` when cancelled.
#[tauri::command]
pub async fn nexus_check(app: AppHandle, force: bool) -> CmdResult<Option<UpdatesView>> {
    let state = app.state::<AppState>();
    let api = api()?;
    let (inst, profile) = instance(&state)?;
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.nexus.check.lock().unwrap();
        if slot.is_some() {
            return Err("Проверка уже идёт".into());
        }
        *slot = Some(cancel.clone());
    }
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let (tracked, untracked) = tracking::tracked_mods(&inst, &profile)?;
        let mut cache = Cache::load(&Cache::path(&inst));
        let result = tracking::check(&api, &mut cache, &tracked, now(), force, &|| cancel.load(Ordering::Relaxed), &mut |done, total| {
            let _ = handle.emit("nexus-check", CheckProgress { done, total });
        });
        let limit = api.rate_limit();
        result.map(|()| (tracked, untracked, cache, limit))
    })
    .await
    .map_err(err)?;
    *state.nexus.check.lock().unwrap() = None;
    match result {
        Ok((tracked, untracked, cache, limit)) => {
            *state.nexus.rate_limit.lock().unwrap() = Some(limit);
            let view = view(&state, tracked, untracked, &cache);
            let updates = view.mods.iter().filter(|m| matches!(m.status, Status::Update { .. })).count();
            log::info!("nexus check: {} mod(s), {updates} update(s), rate limit {limit:?}", view.mods.len());
            Ok(Some(view))
        }
        Err(lyno_core::Error::Cancelled) => Ok(None),
        Err(e) => {
            log::warn!("nexus check: {e}");
            Err(nexus_error(&e))
        }
    }
}

#[tauri::command]
pub fn nexus_check_cancel(state: TauriState<'_, AppState>) {
    if let Some(flag) = state.nexus.check.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum JobState {
    Queued,
    Downloading { done: u64, total: u64 },
    #[serde(rename_all = "camelCase")]
    Retry { attempt: u32, delay_secs: u64, error: String },
    /// Waiting for a build update or an integrity check to finish.
    Waiting,
    Installing,
    Done { outcome: Outcome },
    Failed { error: String },
    Cancelled,
}

impl JobState {
    fn is_final(&self) -> bool {
        matches!(self, JobState::Done { .. } | JobState::Failed { .. } | JobState::Cancelled)
    }
}

/// One file to download and install, as the UI shows it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    id: u64,
    game: String,
    mod_id: u64,
    file_id: u64,
    /// Mod page title once known.
    title: Option<String>,
    file_title: Option<String>,
    version: Option<String>,
    /// The installed mod folder this file replaces.
    replaces: Option<String>,
    state: JobState,
    #[serde(skip)]
    link: Option<NxmLink>,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
}

fn update_job(app: &AppHandle, id: u64, f: impl FnOnce(&mut Job)) {
    let state = app.state::<AppState>();
    let snapshot = {
        let mut jobs = state.nexus.jobs.lock().unwrap();
        let Some(job) = jobs.iter_mut().find(|j| j.id == id) else { return };
        f(job);
        job.clone()
    };
    let _ = app.emit("nexus-job", snapshot);
}

fn enqueue(app: &AppHandle, link: NxmLink) -> u64 {
    let state = app.state::<AppState>();
    let id = state.nexus.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let job = Job {
        id,
        game: link.game.clone(),
        mod_id: link.mod_id,
        file_id: link.file_id,
        title: None,
        file_title: None,
        version: None,
        replaces: None,
        state: JobState::Queued,
        link: Some(link),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    state.nexus.jobs.lock().unwrap().push(job.clone());
    let _ = app.emit("nexus-job", job);
    if !state.nexus.worker.swap(true, Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || work(&app));
    }
    id
}

/// An nxm link from the command line: the launcher was started for it, or a
/// second launcher passed it on (see `tauri_plugin_single_instance`).
pub fn receive(app: &AppHandle, url: &str) {
    log::info!("nxm link received: {}", url.split('?').next().unwrap_or(url));
    let _ = app.emit("nexus-link", ());
    match NxmLink::parse(url) {
        Ok(link) => {
            enqueue(app, link);
        }
        Err(e) => {
            log::warn!("{e}");
            let error = if NxmLink::is_collection(url) {
                "Коллекции Nexus лаунчер не устанавливает: установите их через Vortex или MO2".to_owned()
            } else {
                format!("Непонятная ссылка: {url}")
            };
            let id = enqueue_failed(app, error);
            log::warn!("nxm job {id} rejected");
        }
    }
}

fn enqueue_failed(app: &AppHandle, error: String) -> u64 {
    let state = app.state::<AppState>();
    let id = state.nexus.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let job = Job {
        id,
        game: String::new(),
        mod_id: 0,
        file_id: 0,
        title: None,
        file_title: None,
        version: None,
        replaces: None,
        state: JobState::Failed { error },
        link: None,
        cancel: Arc::new(AtomicBool::new(false)),
    };
    state.nexus.jobs.lock().unwrap().push(job.clone());
    let _ = app.emit("nexus-job", job);
    id
}

/// Downloads queued jobs until none is left.
fn work(app: &AppHandle) {
    let state = app.state::<AppState>();
    loop {
        let next = {
            let jobs = state.nexus.jobs.lock().unwrap();
            jobs.iter().find(|j| matches!(j.state, JobState::Queued)).cloned()
        };
        let Some(job) = next else {
            state.nexus.worker.store(false, Ordering::SeqCst);
            // A job queued between the look and the flag would wait forever.
            let queued = state.nexus.jobs.lock().unwrap().iter().any(|j| matches!(j.state, JobState::Queued));
            if queued && !state.nexus.worker.swap(true, Ordering::SeqCst) {
                continue;
            }
            return;
        };
        let result = run_job(app, &job);
        let final_state = match result {
            Ok(Ok(outcome)) => JobState::Done { outcome },
            Ok(Err(BuildMod(folder))) => JobState::Failed {
                error: format!("«{folder}» входит в сборку: он обновляется вместе со сборкой, а не с Nexus"),
            },
            Err(lyno_core::Error::Cancelled) => JobState::Cancelled,
            Err(e) => {
                log::error!("nexus download {}/{}: {e}", job.mod_id, job.file_id);
                JobState::Failed { error: nexus_error(&e) }
            }
        };
        if let JobState::Done { outcome } = &final_state {
            log::info!("nexus download {}/{}: {outcome:?}", job.mod_id, job.file_id);
            let _ = app.emit("nexus-changed", ());
        }
        update_job(app, job.id, |j| j.state = final_state);
    }
}

fn busy(state: &AppState) -> bool {
    state.update.lock().unwrap().is_some() || state.verify.lock().unwrap().is_some() || state.author_job.lock().unwrap().is_some()
}

fn run_job(app: &AppHandle, job: &Job) -> lyno_core::Result<Result<Outcome, BuildMod>> {
    let state = app.state::<AppState>();
    let link = job.link.clone().ok_or(lyno_core::Error::Cancelled)?;
    let key = secrets::get(Secret::NexusKey).ok_or_else(|| lyno_core::Error::Nexus {
        status: 401,
        message: String::new(),
    })?;
    let api = NexusApi::new(&key);
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    if !inst.is_installed() {
        return Err(lyno_core::Error::Manifest("Сначала установите сборку".into()));
    }
    let cancel = job.cancel.clone();
    let cancelled = || cancel.load(Ordering::Relaxed);
    // Installs write into the instance an update or a check is reading.
    if busy(&state) {
        update_job(app, job.id, |j| j.state = JobState::Waiting);
        while busy(&state) {
            if cancelled() {
                return Err(lyno_core::Error::Cancelled);
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }
    let (game, mo2) = running_processes();
    let ctx = Context { inst: &inst, profile: &profile(&inst), author: settings.author_mode, mo2_running: game || mo2, now: now() };
    let mut last_emit = std::time::Instant::now() - Duration::from_secs(1);
    let id = job.id;
    let result = mod_install::fetch_and_install(&api, &Downloader::new(), &ctx, &link, &cancelled, &mut |p| match p {
        Progress::Resolved { mod_name, file_title, version, size, replaces } => update_job(app, id, |j| {
            j.title = Some(mod_name);
            j.file_title = Some(file_title);
            j.version = version;
            j.replaces = replaces;
            j.state = JobState::Downloading { done: 0, total: size };
        }),
        Progress::Bytes { done, total } => {
            if last_emit.elapsed() >= Duration::from_millis(150) || done == total {
                last_emit = std::time::Instant::now();
                update_job(app, id, |j| j.state = JobState::Downloading { done, total });
            }
        }
        Progress::Retry { attempt, delay_secs, error } => {
            update_job(app, id, |j| j.state = JobState::Retry { attempt, delay_secs, error })
        }
        Progress::Installing => update_job(app, id, |j| j.state = JobState::Installing),
    });
    *state.nexus.rate_limit.lock().unwrap() = Some(api.rate_limit());
    result.map(|r| r.map(|(_, outcome)| outcome))
}

#[tauri::command]
pub fn nexus_jobs(state: TauriState<'_, AppState>) -> Vec<Job> {
    state.nexus.jobs.lock().unwrap().clone()
}

#[tauri::command]
pub fn nexus_cancel_job(app: AppHandle, id: u64) {
    let state = app.state::<AppState>();
    let queued = {
        let jobs = state.nexus.jobs.lock().unwrap();
        let Some(job) = jobs.iter().find(|j| j.id == id) else { return };
        job.cancel.store(true, Ordering::Relaxed);
        matches!(job.state, JobState::Queued)
    };
    // A running job notices the flag; a queued one never starts.
    if queued {
        update_job(&app, id, |j| j.state = JobState::Cancelled);
    }
}

/// Forgets finished jobs.
#[tauri::command]
pub fn nexus_clear_jobs(state: TauriState<'_, AppState>) {
    state.nexus.jobs.lock().unwrap().retain(|j| !j.state.is_final());
}

/// Downloads an update for a Premium account: the API gives the link without
/// a visit to the site. Free accounts get the files page instead (UI).
#[tauri::command]
pub fn nexus_download(app: AppHandle, game: String, mod_id: u64, file_id: u64) -> CmdResult<u64> {
    let premium = app.state::<AppState>().nexus.account.lock().unwrap().as_ref().is_some_and(|a| a.premium);
    if !premium {
        return Err("Без Premium Nexus отдаёт файлы только по кнопке «Mod Manager Download» на сайте".into());
    }
    Ok(enqueue(&app, NxmLink { game, mod_id, file_id, key: None, expires: None }))
}

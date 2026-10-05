//! Nexus Mods in the launcher: the player's account, version tracking of
//! mods from Nexus, downloads from nxm links (the launcher can take over
//! "Mod Manager Download" from MO2 or Vortex), and installs of archives from
//! MO2's downloads or the player's disk.
//!
//! Jobs run one at a time in a queue: Nexus serves free accounts one file at
//! a time anyway, and installs must not overlap in `modlist.txt`.
//!
//! What MO2 would ask, a job asks too and parks, the queue moving on: a
//! FOMOD installer (`Choosing`; the wizard asks `nexus_fomod_eval` for every
//! click, the rules live in `lyno_core::fomod`), an archive no rule
//! recognizes (`ChoosingRoot`), or MO2 / the game running (`WaitingMo2`: MO2
//! rewrites `modlist.txt` on exit, so the install waits for it to close and
//! then runs by itself). The answer puts the job back in the queue.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lyno_core::archive::Root;
use lyno_core::download::Downloader;
use lyno_core::fomod::{Evaluated, Outline, Selection};
use lyno_core::meta::ModMeta;
use lyno_core::mo2::Instance;
use lyno_core::mod_install::{self, BuildMod, Context, Download, DownloadItem, Fomod, Outcome, Progress, Target};
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

/// How often parked installs look whether MO2 and the game are closed.
const MO2_POLL: Duration = Duration::from_secs(2);

#[derive(Default)]
pub struct NexusState {
    /// The account behind the saved key, once validated in this session.
    account: Mutex<Option<User>>,
    check: Mutex<Option<Arc<AtomicBool>>>,
    sso: Mutex<Option<Arc<AtomicBool>>>,
    jobs: Mutex<Vec<Job>>,
    worker: AtomicBool,
    /// A thread waits for MO2 to close for `WaitingMo2` jobs.
    watcher: AtomicBool,
    next_id: AtomicU64,
    rate_limit: Mutex<Option<RateLimit>>,
    /// Archives of parked jobs, by job id.
    parked: Mutex<HashMap<u64, Arc<Parked>>>,
}

/// An archive waiting for the player or for MO2 to close.
struct Parked {
    archive: PathBuf,
    download: Download,
    target: Target,
    fomod: Option<Fomod>,
}

/// Stores the latest allowance and tells the footer.
fn record_limit(app: &AppHandle, limit: RateLimit) {
    if limit == RateLimit::default() {
        return;
    }
    *app.state::<AppState>().nexus.rate_limit.lock().unwrap() = Some(limit);
    let _ = app.emit("nexus-limits", limit);
}

/// Requests left on the account, from the last answer of Nexus in this session.
#[tauri::command]
pub fn nexus_limits(state: TauriState<'_, AppState>) -> Option<RateLimit> {
    *state.nexus.rate_limit.lock().unwrap()
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
        let checked = tauri::async_runtime::spawn_blocking(move || {
            let api = NexusApi::new(&key);
            (api.validate(), api.rate_limit())
        })
        .await
        .map_err(err)?;
        record_limit(&app, checked.1);
        match checked.0 {
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
            ModUpdateRow {
                folder: t.folder.clone(),
                game: t.game.clone(),
                mod_id: t.mod_id,
                version: t.version.clone(),
                personal: t.personal,
                managed: t.managed,
                can_update: t.personal || author,
                page_url: nxm::file_page(&t.game, t.mod_id),
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
        return Err("Mod Organizer 2 не установлен".into());
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
            record_limit(&app, limit);
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
    /// MO2 or the game is running: the install follows once they close.
    WaitingMo2,
    Installing,
    /// A FOMOD installer waits for the player's choice.
    Choosing,
    /// No rule recognizes the archive: the player picks the mod's folder.
    ChoosingRoot,
    Done { outcome: Outcome },
    Failed { error: String },
    Cancelled,
}

impl JobState {
    fn is_final(&self) -> bool {
        matches!(self, JobState::Done { .. } | JobState::Failed { .. } | JobState::Cancelled)
    }

    /// Parked: nothing runs, the job waits for the player or for MO2.
    fn is_parked(&self) -> bool {
        matches!(self, JobState::WaitingMo2 | JobState::Choosing | JobState::ChoosingRoot)
    }
}

/// What a queued job does when the worker takes it.
#[derive(Clone)]
enum Work {
    /// Download the file of an nxm link into MO2's downloads.
    Fetch(NxmLink),
    /// Install an archive from MO2's downloads or the player's disk.
    Install { archive: PathBuf, after: Option<String> },
    /// Install the parked archive as it is (it waited for MO2 to close).
    Resume,
    /// Install the parked FOMOD archive with this choice.
    Fomod(Selection),
    /// Install this folder of the parked archive as the mod.
    Root(String),
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Nexus,
    /// An archive from MO2's downloads or the player's disk.
    File,
}

/// One file to download and install, as the UI shows it.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    id: u64,
    source: Source,
    game: String,
    mod_id: u64,
    file_id: u64,
    /// Mod page title once known; the archive's name for a file.
    title: Option<String>,
    file_title: Option<String>,
    version: Option<String>,
    /// The installed mod folder this file replaces.
    replaces: Option<String>,
    state: JobState,
    #[serde(skip)]
    work: Option<Work>,
    #[serde(skip)]
    cancel: Arc<AtomicBool>,
}

impl Job {
    fn new(id: u64, source: Source, work: Option<Work>, state: JobState) -> Self {
        Job {
            id,
            source,
            game: String::new(),
            mod_id: 0,
            file_id: 0,
            title: None,
            file_title: None,
            version: None,
            replaces: None,
            state,
            work,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn describe(&mut self, dl: &Download) {
        self.game = dl.game.clone();
        self.mod_id = dl.mod_id;
        self.file_id = dl.file_id;
        self.title = Some(dl.mod_name.clone()).filter(|t| !t.is_empty()).or_else(|| Some(dl.file_name.clone()));
        self.file_title = Some(dl.file_title.clone()).filter(|t| !t.is_empty());
        self.version = dl.version.clone();
    }
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

fn push_job(app: &AppHandle, f: impl FnOnce(u64) -> Job) -> u64 {
    let state = app.state::<AppState>();
    let id = state.nexus.next_id.fetch_add(1, Ordering::Relaxed) + 1;
    let job = f(id);
    let queued = matches!(job.state, JobState::Queued);
    state.nexus.jobs.lock().unwrap().push(job.clone());
    let _ = app.emit("nexus-job", job);
    if queued {
        start_worker(app);
    }
    id
}

fn enqueue(app: &AppHandle, link: NxmLink) -> u64 {
    push_job(app, |id| {
        let mut job = Job::new(id, Source::Nexus, None, JobState::Queued);
        (job.game, job.mod_id, job.file_id) = (link.game.clone(), link.mod_id, link.file_id);
        job.work = Some(Work::Fetch(link));
        job
    })
}

fn start_worker(app: &AppHandle) {
    if !app.state::<AppState>().nexus.worker.swap(true, Ordering::SeqCst) {
        let app = app.clone();
        std::thread::spawn(move || work(&app));
    }
}

/// Puts `WaitingMo2` jobs back in the queue once MO2 and the game are closed.
fn start_watcher(app: &AppHandle) {
    if app.state::<AppState>().nexus.watcher.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let state = app.state::<AppState>();
        loop {
            std::thread::sleep(MO2_POLL);
            let waiting: Vec<u64> =
                state.nexus.jobs.lock().unwrap().iter().filter(|j| matches!(j.state, JobState::WaitingMo2)).map(|j| j.id).collect();
            if waiting.is_empty() {
                state.nexus.watcher.store(false, Ordering::SeqCst);
                // A job parked between the look and the flag would wait forever.
                let again = state.nexus.jobs.lock().unwrap().iter().any(|j| matches!(j.state, JobState::WaitingMo2));
                if again && !state.nexus.watcher.swap(true, Ordering::SeqCst) {
                    continue;
                }
                return;
            }
            if running_processes() == (false, false) {
                log::info!("MO2 and the game are closed: {} waiting install(s) resume", waiting.len());
                for id in waiting {
                    update_job(&app, id, |j| j.state = JobState::Queued);
                }
                start_worker(&app);
            }
        }
    });
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
            let id = push_job(app, |id| Job::new(id, Source::Nexus, None, JobState::Failed { error }));
            log::warn!("nxm job {id} rejected");
        }
    }
}

/// Where a job is after a run.
enum Next {
    Done(Outcome),
    Parked(JobState),
}

/// Runs queued jobs until none is left.
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
        let new_state = match run_job(app, &job) {
            Ok(Ok(Next::Parked(parked))) => parked,
            Ok(Ok(Next::Done(outcome))) => {
                log::info!("job {} ({:?}): {outcome:?}", job.id, job.title);
                let _ = app.emit("nexus-changed", ());
                JobState::Done { outcome }
            }
            Ok(Err(BuildMod(folder))) => JobState::Failed {
                error: format!("«{folder}» входит в сборку: он обновляется вместе со сборкой, а не с Nexus"),
            },
            Err(lyno_core::Error::Cancelled) => JobState::Cancelled,
            Err(e) => {
                log::error!("job {} ({:?}): {e}", job.id, job.title);
                JobState::Failed { error: job_error(&e) }
            }
        };
        if !new_state.is_parked() {
            state.nexus.parked.lock().unwrap().remove(&job.id);
        }
        if matches!(new_state, JobState::WaitingMo2) {
            start_watcher(app);
        }
        update_job(app, job.id, |j| j.state = new_state);
    }
}

fn job_error(e: &lyno_core::Error) -> String {
    match e {
        lyno_core::Error::Fomod(m) => format!("Не удалось установить: {m}"),
        lyno_core::Error::Parse { message, .. } => format!("Не удалось прочитать архив: {message}"),
        e => nexus_error(e),
    }
}

pub(crate) fn busy(state: &AppState) -> bool {
    state.update.lock().unwrap().is_some() || state.verify.lock().unwrap().is_some() || state.author_job.lock().unwrap().is_some()
}

fn run_job(app: &AppHandle, job: &Job) -> lyno_core::Result<Result<Next, BuildMod>> {
    let state = app.state::<AppState>();
    let work = job.work.clone().ok_or(lyno_core::Error::Cancelled)?;
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    if !inst.is_installed() {
        return Err(lyno_core::Error::Manifest("Сначала установите Mod Organizer 2".into()));
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
    let mo2_running = game || mo2;
    let profile = profile(&inst);
    let id = job.id;

    let parked = || app.state::<AppState>().nexus.parked.lock().unwrap().get(&id).cloned().ok_or(lyno_core::Error::Cancelled);
    let outcome = match work {
        Work::Fetch(link) => {
            let key = secrets::get(Secret::NexusKey).ok_or_else(|| lyno_core::Error::Nexus { status: 401, message: String::new() })?;
            let api = NexusApi::new(&key);
            let ctx = Context { inst: &inst, profile: &profile, author: settings.author_mode, now: now() };
            let mut last_emit = std::time::Instant::now() - Duration::from_secs(1);
            let result = mod_install::fetch(&api, &Downloader::new(), &ctx, &link, &cancelled, &mut |p| match p {
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
            });
            record_limit(app, api.rate_limit());
            match result? {
                Ok((_, outcome)) => return Ok(Ok(Next::Done(outcome))),
                Err(build_mod) => return Ok(Err(build_mod)),
            }
        }
        Work::Install { archive, after } => {
            let download = mod_install::download_for(&archive);
            update_job(app, id, |j| j.describe(&download));
            let target = match mod_install::target_for(&inst, &profile, &download, settings.author_mode, after)? {
                Ok(t) => t,
                Err(build_mod) => return Ok(Err(build_mod)),
            };
            if let Target::Replace(folder) = &target {
                update_job(app, id, |j| j.replaces = Some(folder.clone()));
            }
            if mo2_running {
                let parked = Parked { archive, download, target, fomod: None };
                app.state::<AppState>().nexus.parked.lock().unwrap().insert(id, Arc::new(parked));
                update_job(app, id, |j| j.work = Some(Work::Resume));
                return Ok(Ok(Next::Parked(JobState::WaitingMo2)));
            }
            update_job(app, id, |j| j.state = JobState::Installing);
            let outcome = mod_install::install(&inst, &profile, &archive, &download, &target)?;
            return Ok(Ok(park(app, id, &inst, download, outcome)));
        }
        // The rest install a parked archive; MO2 open again means waiting again.
        _ if mo2_running => return Ok(Ok(Next::Parked(JobState::WaitingMo2))),
        Work::Resume => {
            let p = parked()?;
            update_job(app, id, |j| j.state = JobState::Installing);
            let outcome = mod_install::install(&inst, &profile, &p.archive, &p.download, &p.target)?;
            return Ok(Ok(park(app, id, &inst, p.download.clone(), outcome)));
        }
        Work::Fomod(choice) => {
            let p = parked()?;
            let fomod = p.fomod.as_ref().ok_or(lyno_core::Error::Cancelled)?;
            update_job(app, id, |j| j.state = JobState::Installing);
            mod_install::install_fomod(&inst, &profile, fomod, &p.download, &p.target, &choice)?
        }
        Work::Root(root) => {
            let p = parked()?;
            update_job(app, id, |j| j.state = JobState::Installing);
            mod_install::install_root(&inst, &profile, &p.archive, &p.download, &p.target, &root)?
        }
    };
    Ok(Ok(Next::Done(outcome)))
}

/// Parks a job whose install stopped halfway; anything else is done.
fn park(app: &AppHandle, id: u64, inst: &Instance, download: Download, outcome: Outcome) -> Next {
    let (archive, target, state, work) = match &outcome {
        Outcome::Deferred { archive, target } => (archive, target, JobState::WaitingMo2, Work::Resume),
        Outcome::Manual { archive, target } => (archive, target, JobState::ChoosingRoot, Work::Resume),
        Outcome::Fomod { archive, target } => match mod_install::open_fomod(inst, archive, target) {
            Ok(fomod) => {
                let parked = Parked { archive: archive.clone(), download, target: target.clone(), fomod: Some(fomod) };
                app.state::<AppState>().nexus.parked.lock().unwrap().insert(id, Arc::new(parked));
                return Next::Parked(JobState::Choosing);
            }
            Err(e) => {
                log::warn!("FOMOD {}: {e}", archive.display());
                (archive, target, JobState::ChoosingRoot, Work::Resume)
            }
        },
        _ => return Next::Done(outcome),
    };
    let parked = Parked { archive: archive.clone(), download, target: target.clone(), fomod: None };
    app.state::<AppState>().nexus.parked.lock().unwrap().insert(id, Arc::new(parked));
    update_job(app, id, |j| j.work = Some(work));
    Next::Parked(state)
}

#[tauri::command]
pub fn nexus_jobs(state: TauriState<'_, AppState>) -> Vec<Job> {
    state.nexus.jobs.lock().unwrap().clone()
}

#[tauri::command]
pub fn nexus_cancel_job(app: AppHandle, id: u64) {
    let state = app.state::<AppState>();
    let idle = {
        let jobs = state.nexus.jobs.lock().unwrap();
        let Some(job) = jobs.iter().find(|j| j.id == id) else { return };
        job.cancel.store(true, Ordering::Relaxed);
        matches!(job.state, JobState::Queued) || job.state.is_parked()
    };
    // A running job notices the flag; a queued one never starts. A parked
    // archive stays in MO2's downloads, where it can still be installed.
    if idle {
        state.nexus.parked.lock().unwrap().remove(&id);
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

/// The newest archives of MO2's downloads, for the downloads list.
#[tauri::command]
pub async fn downloads_recent(app: AppHandle) -> CmdResult<Vec<DownloadItem>> {
    let (inst, _) = instance(&app.state::<AppState>())?;
    tauri::async_runtime::spawn_blocking(move || mod_install::recent_downloads(&inst, 40)).await.map_err(err)?.map_err(err)
}

/// An archive of the downloads list (a file name in MO2's downloads) or a
/// path dropped from Explorer.
fn archive_path(inst: &Instance, file: &str) -> CmdResult<PathBuf> {
    let path = PathBuf::from(file);
    let archive = match path.is_absolute() {
        true => path,
        false if file.contains(['/', '\\']) => return Err(format!("Непонятный путь: {file}")),
        false => inst.downloads_dir().join(file),
    };
    if !archive.is_file() {
        return Err(format!("Файл не найден: {}", archive.display()));
    }
    Ok(archive)
}

/// What installing an archive would replace, asked before the install: a new
/// version of an installed mod goes over its folder, which the player confirms.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveTarget {
    mod_name: String,
    version: Option<String>,
    /// The installed mod folder the archive goes over.
    replaces: Option<String>,
    installed_version: Option<String>,
}

#[tauri::command]
pub async fn archive_target(app: AppHandle, file: String) -> CmdResult<ArchiveTarget> {
    let state = app.state::<AppState>();
    let (inst, profile) = instance(&state)?;
    let author = state.settings.lock().unwrap().author_mode;
    tauri::async_runtime::spawn_blocking(move || {
        let archive = archive_path(&inst, &file)?;
        let download = mod_install::download_for(&archive);
        // A player's build mod: the install itself says why it can't go there.
        let replaces = match mod_install::target_for(&inst, &profile, &download, author, None).map_err(err)? {
            Ok(Target::Replace(folder)) => Some(folder),
            _ => None,
        };
        let installed_version = replaces
            .as_ref()
            .and_then(|f| ModMeta::load(&inst.mods_dir().join(f).join("meta.ini")).ok())
            .and_then(|m| m.version);
        Ok(ArchiveTarget {
            mod_name: download.mod_name,
            version: download.version.as_deref().map(lyno_core::meta::display_version),
            replaces,
            installed_version,
        })
    })
    .await
    .map_err(err)?
}

/// Installs an archive: a file name in MO2's downloads (the downloads list)
/// or a path dropped from Explorer. `after`: the list entry it was dropped
/// below (`None`: the end of the player's section).
#[tauri::command]
pub fn install_archive(app: AppHandle, file: String, after: Option<String>) -> CmdResult<u64> {
    let (inst, _) = instance(&app.state::<AppState>())?;
    let archive = archive_path(&inst, &file)?;
    log::info!("install {} (after {after:?})", archive.display());
    Ok(push_job(&app, |id| {
        let mut job = Job::new(id, Source::File, Some(Work::Install { archive: archive.clone(), after }), JobState::Queued);
        job.title = archive.file_name().map(|n| n.to_string_lossy().into_owned());
        job
    }))
}

fn parked(app: &AppHandle, id: u64) -> CmdResult<Arc<Parked>> {
    app.state::<AppState>().nexus.parked.lock().unwrap().get(&id).cloned().ok_or_else(|| "Установка уже закрыта: начните её заново".into())
}

fn requeue(app: &AppHandle, id: u64, work: Work) {
    update_job(app, id, |j| {
        j.work = Some(work);
        j.state = JobState::Queued;
    });
    start_worker(app);
}

/// Folders of a `ChoosingRoot` archive, to pick the mod from.
#[tauri::command]
pub async fn install_roots(app: AppHandle, id: u64) -> CmdResult<Vec<Root>> {
    let p = parked(&app, id)?;
    tauri::async_runtime::spawn_blocking(move || mod_install::roots(&p.archive)).await.map_err(err)?.map_err(|e| job_error(&e))
}

/// Installs the folder `root` of a `ChoosingRoot` archive as the mod.
#[tauri::command]
pub fn install_set_root(app: AppHandle, id: u64, root: String) -> CmdResult<()> {
    parked(&app, id)?;
    requeue(&app, id, Work::Root(root));
    Ok(())
}

/// The FOMOD wizard of a job: the installer, its images and the first state.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FomodWizard {
    outline: Outline,
    /// `data:` URLs by installer path.
    images: HashMap<String, String>,
    /// The choice of the installed version this file replaces.
    previous: Option<Selection>,
    state: Evaluated,
}

fn fomod_of(app: &AppHandle, id: u64) -> CmdResult<(Arc<Parked>, Instance, String)> {
    let p = parked(app, id)?;
    if p.fomod.is_none() {
        return Err("У этого архива нет установщика FOMOD".into());
    }
    let (inst, profile) = instance(&app.state::<AppState>())?;
    Ok((p, inst, profile))
}

#[tauri::command]
pub async fn nexus_fomod(app: AppHandle, id: u64) -> CmdResult<FomodWizard> {
    let (p, inst, profile) = fomod_of(&app, id)?;
    tauri::async_runtime::spawn_blocking(move || {
        let fomod = p.fomod.as_ref().expect("checked");
        // Images of a solid 7z need its stream read up to them: worth a log line, not a failure.
        let images = fomod.images().unwrap_or_else(|e| {
            log::warn!("FOMOD images: {e}");
            HashMap::new()
        });
        let selection = fomod.previous.clone().unwrap_or_default();
        let state = fomod.installer.evaluate(&selection, &mod_install::file_states(&inst, &profile));
        FomodWizard { outline: fomod.installer.outline(), images, previous: fomod.previous.clone(), state }
    })
    .await
    .map_err(err)
}

/// The wizard after a click: visible steps, plugin types and the selection
/// with the installer's rules applied.
#[tauri::command]
pub fn nexus_fomod_eval(app: AppHandle, id: u64, selection: Selection) -> CmdResult<Evaluated> {
    let (p, inst, profile) = fomod_of(&app, id)?;
    Ok(p.fomod.as_ref().expect("checked").installer.evaluate(&selection, &mod_install::file_states(&inst, &profile)))
}

/// Puts the job back in the queue to install `selection`; with MO2 open it
/// waits for MO2 to close.
#[tauri::command]
pub fn nexus_fomod_install(app: AppHandle, id: u64, selection: Selection) -> CmdResult<()> {
    let (p, inst, profile) = fomod_of(&app, id)?;
    let installer = &p.fomod.as_ref().expect("checked").installer;
    let ev = installer.evaluate(&selection, &mod_install::file_states(&inst, &profile));
    if let Some(step) = ev.valid.iter().position(|v| !v) {
        return Err(format!("Шаг «{}»: выберите варианты", installer.steps[step].name));
    }
    requeue(&app, id, Work::Fomod(selection));
    Ok(())
}

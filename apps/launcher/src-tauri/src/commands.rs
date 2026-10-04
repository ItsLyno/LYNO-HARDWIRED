use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use lyno_core::download::Downloader;
use lyno_core::game::{self, GameInstall};
use lyno_core::install::{self, Installer};
use lyno_core::manifest::{ChangelogEntry, Manifest, ModEntry};
use lyno_core::mo2::{self, Instance};
use lyno_core::modlist::{EntryState, ModList};
use lyno_core::plan;
use lyno_core::report;
use lyno_core::state::State;
use lyno_core::verify;
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};
use tauri_plugin_updater::UpdaterExt;

use crate::settings::Settings;
use crate::AppState;

const GAME_PROCESS: &str = "Cyberpunk2077.exe";
const MO2_PROCESS: &str = "ModOrganizer.exe";
const DEFAULT_PROFILE: &str = "LYNO";

/// Errors cross the IPC boundary as plain strings for the UI to show.
type CmdResult<T> = Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Manifest of the installed build, saved after every successful update.
fn installed_manifest_path(inst: &Instance) -> PathBuf {
    inst.root().join(".lyno").join("manifest.json")
}

fn installed_manifest(inst: &Instance) -> Option<Manifest> {
    let text = std::fs::read_to_string(installed_manifest_path(inst)).ok()?;
    Manifest::from_json(&text).ok()
}

fn profile(inst: &Instance) -> String {
    installed_manifest(inst).map_or_else(|| DEFAULT_PROFILE.to_owned(), |m| m.profile)
}

#[tauri::command]
pub fn get_settings(state: TauriState<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(state: TauriState<'_, AppState>, settings: Settings) -> CmdResult<()> {
    if let Some(dir) = &settings.game_dir {
        if !game::is_game_dir(dir) {
            return Err(format!("В папке нет bin\\x64\\Cyberpunk2077.exe: {}", dir.display()));
        }
        let inst = Instance::new(&settings.instance_dir);
        if inst.ini_path().is_file() {
            inst.set_game_path(dir).map_err(err)?;
        }
    }
    settings.save(&state.settings_path).map_err(err)?;
    log::info!("settings saved: instance {:?}, game {:?}, manifest {}", settings.instance_dir, settings.game_dir, settings.manifest_url);
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

#[tauri::command]
pub fn detect_games() -> Vec<GameInstall> {
    game::detect()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    mo2_installed: bool,
    game_dir: Option<PathBuf>,
    game_found: bool,
    installed_version: Option<String>,
    mods_total: usize,
    mods_enabled: usize,
    game_running: bool,
    mo2_running: bool,
    updating: bool,
}

#[tauri::command]
pub fn get_status(state: TauriState<'_, AppState>) -> Status {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let list = ModList::load(&inst.modlist_path(&profile(&inst))).ok();
    let installed = State::load(&install::state_path(&inst)).unwrap_or_default();
    let (game_running, mo2_running) = running_processes();
    Status {
        mo2_installed: inst.is_installed(),
        game_found: settings.game_dir.as_deref().is_some_and(game::is_game_dir),
        game_dir: settings.game_dir,
        installed_version: installed.build_version,
        mods_total: list.as_ref().map_or(0, |l| l.mods().count()),
        mods_enabled: list.as_ref().map_or(0, |l| l.mods().filter(|e| e.state == EntryState::Enabled).count()),
        game_running,
        mo2_running,
        updating: state.update.lock().unwrap().is_some(),
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    name: String,
    latest_version: String,
    installed_version: Option<String>,
    game_version: String,
    changelog: Vec<ChangelogEntry>,
    mods: Vec<ModRow>,
    up_to_date: bool,
    changes: usize,
    /// Of `changes`: damaged mods and base package to download again.
    repairs: usize,
    download_size: u64,
    /// Of `download_size`: already in the download cache from an interrupted update.
    downloaded: u64,
    /// False when GitHub was unreachable and the installed manifest is shown.
    online: bool,
    /// The finished update to the installed build; none after a first install.
    last_update: Option<LastUpdateInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastUpdateInfo {
    from: String,
    to: String,
    /// Folder names of mods the update removed.
    removed: Vec<String>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModRow {
    #[serde(rename_all = "camelCase")]
    Separator { title: String, color: Option<String> },
    #[serde(rename_all = "camelCase")]
    Mod {
        id: String,
        name: String,
        title: Option<String>,
        version: Option<String>,
        author: Option<String>,
        nexus_url: Option<String>,
        /// Effective state: an installed optional mod shows the player's choice.
        enabled: bool,
        /// The player may switch it on or off (see `set_mod_enabled`).
        optional: bool,
        size: u64,
        /// Installed but a different version than the latest build.
        outdated: bool,
        installed: bool,
        /// `added` / `updated` by the last update (`last_update`).
        recent: Option<&'static str>,
        /// Marked for repair by an integrity check; the next update downloads it again.
        damaged: bool,
    },
}

/// Fetches the latest manifest from GitHub and compares it with what is installed.
/// Falls back to the installed manifest when offline.
#[tauri::command]
pub async fn fetch_build(app: AppHandle) -> CmdResult<BuildInfo> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let url = settings.manifest_url.clone();
    let remote = tauri::async_runtime::spawn_blocking(move || {
        let text = Downloader::new().get_text(&url)?;
        Manifest::from_json(&text)
    })
    .await
    .map_err(err)?;

    let inst = Instance::new(&settings.instance_dir);
    if let Err(e) = &remote {
        log::warn!("manifest {}: {e}", settings.manifest_url);
    }
    let (manifest, online) = match remote {
        Ok(m) => (m, true),
        Err(e) => match installed_manifest(&inst) {
            Some(m) => (m, false),
            None if matches!(e, lyno_core::Error::Download(_)) => return Err(format!("Не удалось загрузить манифест сборки: {e}")),
            None => return Err(err(e)),
        },
    };
    *state.manifest.lock().unwrap() = online.then(|| manifest.clone());

    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    let current = ModList::load(&inst.modlist_path(&manifest.profile)).unwrap_or_default();
    let plan = plan::plan(&manifest, &installed, &current);
    let downloaded = install::cached_bytes(&inst, &manifest, &plan);
    log::info!(
        "build {} (installed {:?}, online {online}): {} action(s), {} bytes to download, {downloaded} of them cached",
        manifest.build_version,
        installed.build_version,
        plan.actions.len(),
        plan.download_size
    );

    // A repair-only run over an old state.json starts a record from a version to itself.
    let last_update = installed
        .last_update
        .as_ref()
        .filter(|u| installed.build_version.as_deref() == Some(u.to.as_str()) && u.from.as_deref() != Some(u.to.as_str()));
    let recent = |id: &String| {
        let u = last_update.filter(|u| u.from.is_some())?;
        if u.added.contains(id) {
            Some("added")
        } else {
            u.updated.contains(id).then_some("updated")
        }
    };

    let mods = manifest
        .mods
        .iter()
        .map(|e| match e {
            ModEntry::Separator { title, color } => ModRow::Separator { title: title.clone(), color: color.clone() },
            ModEntry::Mod(m) => {
                let have = installed.mods.get(&m.id);
                ModRow::Mod {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    title: m.title.clone(),
                    version: m.version.clone(),
                    author: m.author.clone(),
                    nexus_url: m.nexus.as_ref().map(|n| n.url()),
                    enabled: plan::is_enabled(m, &installed, &current),
                    optional: m.optional,
                    size: m.package.size,
                    outdated: have.is_some_and(|h| h.hash != m.package.hash),
                    installed: have.is_some(),
                    recent: recent(&m.id),
                    damaged: have.is_some_and(|h| h.damaged),
                }
            }
        })
        .collect();

    Ok(BuildInfo {
        name: manifest.name.clone(),
        latest_version: manifest.build_version.clone(),
        installed_version: installed.build_version.clone(),
        game_version: manifest.game_version.clone(),
        changelog: manifest.changelog.clone(),
        mods,
        up_to_date: plan.is_up_to_date(),
        changes: plan.actions.len(),
        repairs: plan
            .actions
            .iter()
            .filter(|a| match a {
                plan::Action::Repair { .. } => true,
                plan::Action::Base => installed.base_damaged && installed.base_hash.as_deref() == Some(manifest.base.hash.as_str()),
                _ => false,
            })
            .count(),
        download_size: plan.download_size,
        downloaded,
        online,
        last_update: last_update.and_then(|u| {
            Some(LastUpdateInfo { from: u.from.clone()?, to: u.to.clone(), removed: u.removed.clone() })
        }),
    })
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Finished {
    ok: bool,
    error: Option<String>,
}

/// Installs or updates the build in the background. Progress arrives as
/// `update-progress` events, the outcome as `update-finished`.
#[tauri::command]
pub fn start_update(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let manifest = state
        .manifest
        .lock()
        .unwrap()
        .clone()
        .ok_or("Сначала нужно загрузить манифест сборки")?;
    let settings = state.settings.lock().unwrap().clone();
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2 перед обновлением".into());
    }

    if state.verify.lock().unwrap().is_some() {
        return Err("Дождитесь окончания проверки файлов".into());
    }

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.update.lock().unwrap();
        if slot.is_some() {
            return Err("Обновление уже идёт".into());
        }
        *slot = Some(cancel.clone());
    }

    std::thread::spawn(move || {
        log::info!("update to build {} started", manifest.build_version);
        let result = run_update(&app, &manifest, &settings, &cancel);
        *app.state::<AppState>().update.lock().unwrap() = None;
        let finished = match result {
            Ok(()) => {
                log::info!("update to build {} finished", manifest.build_version);
                Finished { ok: true, error: None }
            }
            Err(lyno_core::Error::Cancelled) => {
                log::info!("update cancelled");
                Finished { ok: false, error: None }
            }
            Err(e) => {
                log::error!("update failed: {e}");
                let error = match e {
                    lyno_core::Error::Download(_) => format!(
                        "Не удалось скачать сборку: {e}. Скачанное сохранено — следующее обновление продолжит с того же места."
                    ),
                    e => e.to_string(),
                };
                Finished { ok: false, error: Some(error) }
            }
        };
        let _ = app.emit("update-finished", finished);
    });
    Ok(())
}

fn run_update(app: &AppHandle, manifest: &Manifest, settings: &Settings, cancel: &AtomicBool) -> lyno_core::Result<()> {
    let inst = Instance::new(&settings.instance_dir);
    let state = State::load(&install::state_path(&inst))?;
    let current = ModList::load(&inst.modlist_path(&manifest.profile)).unwrap_or_default();
    let plan = plan::plan(manifest, &state, &current);

    let downloader = Downloader::new();
    let installer = Installer { inst: &inst, manifest, downloader: &downloader, cancel };
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
    installer.apply(&plan, &mut |event| {
        match &event {
            install::Event::Step { index, total, label } => log::info!("[{index}/{total}] {label}"),
            install::Event::Retry { attempt, delay_secs, error } => {
                log::warn!("{error}; attempt {attempt} in {delay_secs} s")
            }
            _ => {}
        }
        // Byte events arrive per chunk; throttle them for the UI.
        let is_bytes = matches!(event, install::Event::Bytes { .. });
        if !is_bytes || last_emit.elapsed().as_millis() >= 100 {
            last_emit = std::time::Instant::now();
            let _ = app.emit("update-progress", event);
        }
    })?;

    if let Some(dir) = &settings.game_dir {
        inst.set_game_path(dir)?;
    }
    let json = serde_json::to_string_pretty(manifest)?;
    let path = installed_manifest_path(&inst);
    std::fs::write(&path, json).map_err(|e| lyno_core::Error::Io { path, source: e })
}

#[tauri::command]
pub fn cancel_update(state: TauriState<'_, AppState>) {
    if let Some(flag) = state.update.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Reads every file of the installed build and reports damaged mods. Progress
/// arrives as `verify-progress` events (same shape as `update-progress`);
/// `None` when cancelled.
#[tauri::command]
pub async fn verify_build(app: AppHandle) -> CmdResult<Option<verify::Report>> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    if installed.build_version.is_none() {
        return Err("Сборка не установлена".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        if state.update.lock().unwrap().is_some() {
            return Err("Дождитесь окончания обновления сборки".into());
        }
        let mut slot = state.verify.lock().unwrap();
        if slot.is_some() {
            return Err("Проверка уже идёт".into());
        }
        *slot = Some(cancel.clone());
    }

    log::info!("integrity check of build {:?} started", installed.build_version);
    let handle = app.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
        verify::verify(&inst, &installed, &cancel, &mut |event| {
            let is_bytes = matches!(event, install::Event::Bytes { .. });
            if !is_bytes || last_emit.elapsed().as_millis() >= 100 {
                last_emit = std::time::Instant::now();
                let _ = handle.emit("verify-progress", event);
            }
        })
    })
    .await;
    *state.verify.lock().unwrap() = None;
    match result.map_err(err)? {
        Ok(report) => {
            log::info!(
                "integrity check: {} mod(s) checked, damaged: {:?}, changed settings: {:?}",
                report.checked,
                report.damaged,
                report.customized
            );
            Ok(Some(report))
        }
        Err(lyno_core::Error::Cancelled) => {
            log::info!("integrity check cancelled");
            Ok(None)
        }
        Err(e) => {
            log::error!("integrity check failed: {e}");
            Err(format!("Не удалось проверить файлы: {e}"))
        }
    }
}

#[tauri::command]
pub fn cancel_verify(state: TauriState<'_, AppState>) {
    if let Some(flag) = state.verify.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

/// Marks the selected mods (manifest ids) and, with `base`, MO2 itself for
/// download and starts an update that reinstalls them, keeping the player's
/// settings files unless `reset_settings`. A newer build, if there is one,
/// is installed along the way: repair works with the latest manifest.
#[tauri::command]
pub fn start_repair(app: AppHandle, ids: Vec<String>, base: bool, reset_settings: bool) -> CmdResult<()> {
    let state = app.state::<AppState>();
    if state.manifest.lock().unwrap().is_none() {
        return Err("Нет связи с GitHub: файлы для починки негде скачать".into());
    }
    if state.update.lock().unwrap().is_some() {
        return Err("Обновление уже идёт".into());
    }
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2 перед починкой".into());
    }
    let settings = state.settings.lock().unwrap().clone();
    let path = install::state_path(&Instance::new(&settings.instance_dir));
    let mut installed = State::load(&path).map_err(err)?;
    verify::mark(&mut installed, &ids, base, reset_settings);
    installed.save(&path).map_err(err)?;
    log::info!("repair: mods {ids:?}, base {base}, reset settings {reset_settings}");
    start_update(app)
}

/// Switches an optional build mod on or off in the player's `modlist.txt`.
/// MO2 keeps the list in memory and writes it back on exit, so it must be closed.
#[tauri::command]
pub fn set_mod_enabled(state: TauriState<'_, AppState>, id: String, enabled: bool) -> CmdResult<()> {
    if state.update.lock().unwrap().is_some() {
        return Err("Дождитесь окончания обновления сборки".into());
    }
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2, чтобы включать и выключать моды".into());
    }
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    // The manifest the player sees: the next update applies it and keeps the choice.
    let manifest = state
        .manifest
        .lock()
        .unwrap()
        .clone()
        .or_else(|| installed_manifest(&inst))
        .ok_or("Сборка не установлена")?;
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    let path = inst.modlist_path(&profile(&inst));
    let mut list = ModList::load(&path).map_err(err)?;
    plan::set_enabled(&manifest, &installed, &mut list, &id, enabled).map_err(|e| {
        log::warn!("set_mod_enabled {id}: {e}");
        "Этот мод нельзя переключить: обновите сборку".to_owned()
    })?;
    list.save(&path).map_err(err)?;
    log::info!("mod {id} {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

#[tauri::command]
pub fn open_mod_folder(state: TauriState<'_, AppState>, id: String) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    let folder = installed.mods.get(&id).ok_or("Мод не установлен")?;
    open_dir(&inst.mods_dir().join(&folder.folder))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Folder {
    Instance,
    Game,
    Saves,
    Logs,
}

#[tauri::command]
pub fn open_folder(app: AppHandle, folder: Folder) -> CmdResult<()> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let path = match folder {
        Folder::Instance => settings.instance_dir,
        Folder::Game => settings.game_dir.ok_or("Папка игры не указана")?,
        // Without MO2's profile-local saves (the build keeps them off) the game
        // writes to the usual place, shared with the unmodded game.
        Folder::Saves => app.path().home_dir().map_err(err)?.join("Saved Games").join("CD Projekt Red").join("Cyberpunk 2077"),
        Folder::Logs => app.path().app_log_dir().map_err(err)?,
    };
    open_dir(&path)
}

fn open_dir(path: &std::path::Path) -> CmdResult<()> {
    if !path.is_dir() {
        return Err(format!("Папка не найдена: {}", path.display()));
    }
    tauri_plugin_opener::open_path(path, None::<&str>).map_err(|e| {
        log::error!("open {}: {e}", path.display());
        format!("Не удалось открыть папку: {e}")
    })
}

#[tauri::command]
pub fn launch_game(state: TauriState<'_, AppState>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    if running_processes().0 {
        return Err("Cyberpunk 2077 уже запущен".into());
    }
    let Some(game_dir) = settings.game_dir.as_deref().filter(|d| game::is_game_dir(d)) else {
        return Err("Не найдена папка Cyberpunk 2077. Укажите её в настройках.".into());
    };
    let inst = Instance::new(&settings.instance_dir);
    let manifest = installed_manifest(&inst);
    let redmod = manifest.as_ref().is_some_and(|m| m.redmod);
    if redmod && !game::has_redmod(game_dir) {
        return Err("В сборке есть REDmod-моды, а REDmod не установлен. \
                    Установите бесплатное DLC REDmod в Steam, GOG или Epic."
            .into());
    }
    let profile = manifest.map_or_else(|| DEFAULT_PROFILE.to_owned(), |m| m.profile);
    log::info!("launching the game (profile {profile}, REDmod {redmod})");
    spawn_mo2(&inst, &mo2::run_args(&profile, mo2::game_executable(redmod)))
}

#[tauri::command]
pub fn open_mo2(state: TauriState<'_, AppState>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    spawn_mo2(&inst, &mo2::open_args(&profile(&inst)))
}

fn spawn_mo2(inst: &Instance, args: &[String]) -> CmdResult<()> {
    if !inst.is_installed() {
        return Err("Сборка не установлена".into());
    }
    Command::new(inst.exe())
        .args(args)
        .current_dir(inst.root())
        .spawn()
        .map(drop)
        .map_err(|e| {
            log::error!("{}: {e}", inst.exe().display());
            format!("Не удалось запустить MO2: {e}")
        })
}

fn running_processes() -> (bool, bool) {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let is_running = |name: &str| {
        sys.processes()
            .values()
            .any(|p| p.name().to_string_lossy().eq_ignore_ascii_case(name))
    };
    (is_running(GAME_PROCESS), is_running(MO2_PROCESS))
}

/// Packs logs and install state into a zip on the desktop for the player to
/// send to the build author, and shows it in Explorer.
#[tauri::command]
pub async fn export_report(app: AppHandle) -> CmdResult<PathBuf> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    let latest = state.manifest.lock().unwrap().as_ref().map(|m| (m.build_version.clone(), m.game_version.clone()));
    let now = time::OffsetDateTime::now_utc();
    let stamp = now
        .format(time::macros::format_description!("[year]-[month]-[day]_[hour]-[minute]-[second]"))
        .map_err(err)?;
    let dir = app.path().desktop_dir().or_else(|_| app.path().download_dir()).map_err(err)?;
    let dest = dir.join(format!("LYNO-report-{stamp}.zip"));
    let launcher_logs: Vec<(String, PathBuf)> = app
        .path()
        .app_log_dir()
        .ok()
        .and_then(|d| std::fs::read_dir(d).ok())
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| (format!("launcher/{}", e.file_name().to_string_lossy()), e.path()))
        .collect();

    let inst = Instance::new(&settings.instance_dir);
    let (game_running, mo2_running) = running_processes();
    let game_dir = settings.game_dir.as_deref();
    let installed = State::load(&install::state_path(&inst)).unwrap_or_default();
    let yes_no = |b: bool| if b { "yes" } else { "no" };
    let summary = format!(
        "LYNO//HARDWIRED {version}\n\
         Created: {stamp} UTC\n\
         OS: {os}\n\
         Instance: {instance} (MO2 installed: {mo2})\n\
         Game: {game} (found: {found}, REDmod: {redmod})\n\
         Manifest URL: {url}\n\
         Installed build: {installed}\n\
         Latest build: {latest}\n\
         Running: game {game_running}, MO2 {mo2_running}\n",
        version = app.package_info().version,
        os = System::long_os_version().unwrap_or_default(),
        instance = settings.instance_dir.display(),
        mo2 = yes_no(inst.is_installed()),
        game = game_dir.map_or_else(|| "not set".into(), |d| d.display().to_string()),
        found = yes_no(game_dir.is_some_and(game::is_game_dir)),
        redmod = yes_no(game_dir.is_some_and(game::has_redmod)),
        url = settings.manifest_url,
        installed = installed.build_version.as_deref().unwrap_or("none"),
        latest = latest.map_or_else(|| "not fetched".into(), |(v, g)| format!("{v} (game {g})")),
        game_running = yes_no(game_running),
        mo2_running = yes_no(mo2_running),
    );
    log::info!("writing report {}", dest.display());

    let profile = profile(&inst);
    let game_dir = settings.game_dir.clone();
    let path = dest.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let input = report::ReportInput {
            summary,
            inst: &inst,
            profile: &profile,
            game_dir: game_dir.as_deref(),
            extra: launcher_logs,
        };
        report::write_report(&path, &input)
    })
    .await
    .map_err(err)?
    .map_err(|e| format!("Не удалось создать отчёт: {e}"))?;
    let _ = tauri_plugin_opener::reveal_item_in_dir(&dest);
    Ok(dest)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LauncherUpdate {
    version: String,
    current_version: String,
    notes: Option<String>,
}

/// Updates are signed; without the author's public key in `tauri.conf.json`
/// no update could be verified, so the launcher doesn't offer any.
fn updater_configured(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|c| c["pubkey"].as_str())
        .is_some_and(|k| !k.trim().is_empty())
}

#[tauri::command]
pub async fn check_launcher_update(app: AppHandle) -> CmdResult<Option<LauncherUpdate>> {
    if !updater_configured(&app) {
        return Ok(None);
    }
    let update = app.updater().map_err(err)?.check().await.map_err(|e| {
        log::warn!("launcher update check: {e}");
        format!("Не удалось проверить обновления лаунчера: {e}")
    })?;
    let info = update.as_ref().map(|u| LauncherUpdate {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
    });
    if let Some(u) = &info {
        log::info!("launcher {} available (running {})", u.version, u.current_version);
    }
    *app.state::<AppState>().launcher_update.lock().unwrap() = update;
    Ok(info)
}

/// Downloads the signed installer and runs it. On Windows the installer
/// closes the launcher itself and starts the new version when done.
#[tauri::command]
pub async fn install_launcher_update(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    if state.update.lock().unwrap().is_some() {
        return Err("Дождитесь окончания обновления сборки".into());
    }
    let update = state.launcher_update.lock().unwrap().clone().ok_or("Обновление лаунчера не найдено")?;
    log::info!("installing launcher {}", update.version);
    update.download_and_install(|_, _| {}, || {}).await.map_err(|e| {
        log::error!("launcher update: {e}");
        format!("Не удалось обновить лаунчер: {e}")
    })?;
    app.restart()
}

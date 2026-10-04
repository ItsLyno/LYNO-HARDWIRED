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
use lyno_core::state::State;
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

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
    download_size: u64,
    /// False when GitHub was unreachable and the installed manifest is shown.
    online: bool,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModRow {
    #[serde(rename_all = "camelCase")]
    Separator { title: String },
    #[serde(rename_all = "camelCase")]
    Mod {
        id: String,
        name: String,
        title: Option<String>,
        version: Option<String>,
        author: Option<String>,
        nexus_url: Option<String>,
        enabled: bool,
        size: u64,
        /// Installed but a different version than the latest build.
        outdated: bool,
        installed: bool,
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

    let mods = manifest
        .mods
        .iter()
        .map(|e| match e {
            ModEntry::Separator { title } => ModRow::Separator { title: title.clone() },
            ModEntry::Mod(m) => {
                let have = installed.mods.get(&m.id);
                ModRow::Mod {
                    id: m.id.clone(),
                    name: m.name.clone(),
                    title: m.title.clone(),
                    version: m.version.clone(),
                    author: m.author.clone(),
                    nexus_url: m.nexus.as_ref().map(|n| n.url()),
                    enabled: m.enabled,
                    size: m.package.size,
                    outdated: have.is_some_and(|h| h.hash != m.package.hash),
                    installed: have.is_some(),
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
        download_size: plan.download_size,
        online,
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

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.update.lock().unwrap();
        if slot.is_some() {
            return Err("Обновление уже идёт".into());
        }
        *slot = Some(cancel.clone());
    }

    std::thread::spawn(move || {
        let result = run_update(&app, &manifest, &settings, &cancel);
        *app.state::<AppState>().update.lock().unwrap() = None;
        let finished = match result {
            Ok(()) => Finished { ok: true, error: None },
            Err(lyno_core::Error::Cancelled) => Finished { ok: false, error: None },
            Err(e) => Finished { ok: false, error: Some(e.to_string()) },
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
        .map_err(|e| format!("Не удалось запустить MO2: {e}"))
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

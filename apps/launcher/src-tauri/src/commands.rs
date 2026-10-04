use std::process::Command;

use lyno_core::mo2::{self, Instance};
use lyno_core::modlist::ModList;
use serde::Serialize;
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::State;

use crate::settings::Settings;
use crate::AppState;

const GAME_PROCESS: &str = "Cyberpunk2077.exe";
const MO2_PROCESS: &str = "ModOrganizer.exe";

/// Errors cross the IPC boundary as plain strings for the UI to show.
type CmdResult<T> = Result<T, String>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    mo2_installed: bool,
    portable: bool,
    profile_exists: bool,
    mods_total: usize,
    mods_enabled: usize,
    game_running: bool,
    mo2_running: bool,
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(state: State<'_, AppState>, settings: Settings) -> CmdResult<()> {
    settings.save(&state.settings_path).map_err(|e| e.to_string())?;
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

#[tauri::command]
pub fn get_status(state: State<'_, AppState>) -> Status {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let list = ModList::load(&inst.modlist_path(&settings.profile)).ok();
    let (game_running, mo2_running) = running_processes();
    Status {
        mo2_installed: inst.is_installed(),
        portable: inst.is_portable(),
        profile_exists: list.is_some(),
        mods_total: list.as_ref().map_or(0, |l| l.mods().count()),
        mods_enabled: list.as_ref().map_or(0, |l| {
            l.mods().filter(|e| e.state == lyno_core::modlist::EntryState::Enabled).count()
        }),
        game_running,
        mo2_running,
    }
}

#[tauri::command]
pub fn launch_game(state: State<'_, AppState>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    if running_processes().0 {
        return Err("Cyberpunk 2077 уже запущен".into());
    }
    spawn_mo2(&settings, &mo2::run_args(&settings.profile, mo2::GAME_EXECUTABLE))
}

#[tauri::command]
pub fn open_mo2(state: State<'_, AppState>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    spawn_mo2(&settings, &mo2::open_args(&settings.profile))
}

fn spawn_mo2(settings: &Settings, args: &[String]) -> CmdResult<()> {
    let inst = Instance::new(&settings.instance_dir);
    if !inst.is_installed() {
        return Err(format!("MO2 не найден: {}", inst.exe().display()));
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

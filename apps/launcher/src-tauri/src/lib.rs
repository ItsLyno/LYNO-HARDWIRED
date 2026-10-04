mod commands;
mod settings;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use lyno_core::manifest::Manifest;
use tauri::Manager;

pub struct AppState {
    settings: Mutex<settings::Settings>,
    settings_path: std::path::PathBuf,
    /// Last manifest fetched from GitHub.
    manifest: Mutex<Option<Manifest>>,
    /// Cancel flag of the running update, if any.
    update: Mutex<Option<Arc<AtomicBool>>>,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_local_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let settings = settings::Settings::load_or_default(&settings_path, &data_dir);
            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_path,
                manifest: Mutex::new(None),
                update: Mutex::new(None),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::detect_games,
            commands::get_status,
            commands::fetch_build,
            commands::start_update,
            commands::cancel_update,
            commands::launch_game,
            commands::open_mo2,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LYNO//HARDWIRED");
}

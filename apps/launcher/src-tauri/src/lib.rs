mod commands;
mod settings;

use std::sync::Mutex;

use tauri::Manager;

pub struct AppState {
    settings: Mutex<settings::Settings>,
    settings_path: std::path::PathBuf,
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_local_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let settings = settings::Settings::load_or_default(&settings_path, &data_dir);
            app.manage(AppState { settings: Mutex::new(settings), settings_path });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_settings,
            commands::save_settings,
            commands::get_status,
            commands::launch_game,
            commands::open_mo2,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LYNO//HARDWIRED");
}

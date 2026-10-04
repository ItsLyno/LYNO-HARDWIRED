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
    /// Launcher release found by the last update check.
    launcher_update: Mutex<Option<tauri_plugin_updater::Update>>,
}

/// One rotated file is kept so a report still covers the session before
/// the one that hit the size limit.
fn log_plugin<R: tauri::Runtime>() -> tauri::plugin::TauriPlugin<R> {
    use tauri_plugin_log::{RotationStrategy, Target, TargetKind};
    tauri_plugin_log::Builder::new()
        .targets([Target::new(TargetKind::LogDir { file_name: Some(LOG_FILE.into()) }), Target::new(TargetKind::Stdout)])
        .level(log::LevelFilter::Info)
        .max_file_size(1024 * 1024)
        .rotation_strategy(RotationStrategy::KeepSome(1))
        .build()
}

const LOG_FILE: &str = "launcher";

pub fn run() {
    tauri::Builder::default()
        .plugin(log_plugin())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            log::info!("LYNO//HARDWIRED {} started", app.package_info().version);
            let config_dir = app.path().app_config_dir()?;
            let data_dir = app.path().app_local_data_dir()?;
            let settings_path = config_dir.join("settings.json");
            let settings = settings::Settings::load_or_default(&settings_path, &data_dir);
            app.manage(AppState {
                settings: Mutex::new(settings),
                settings_path,
                manifest: Mutex::new(None),
                update: Mutex::new(None),
                launcher_update: Mutex::new(None),
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
            commands::export_report,
            commands::check_launcher_update,
            commands::install_launcher_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LYNO//HARDWIRED");
}

mod author;
mod commands;
mod nexus;
mod secrets;
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
    /// Cancel flag of the running integrity check, if any.
    verify: Mutex<Option<Arc<AtomicBool>>>,
    /// Cancel flag of the running author build or publish, if any.
    author_job: Mutex<Option<Arc<AtomicBool>>>,
    /// Build of this session waiting to be published.
    built: Mutex<Option<author::BuiltRelease>>,
    /// Launcher release found by the last update check.
    launcher_update: Mutex<Option<tauri_plugin_updater::Update>>,
    nexus: nexus::NexusState,
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

/// The nxm link among command-line arguments: Windows starts the launcher
/// with the link as its argument (see `lyno_core::nxm::register`).
fn nxm_arg(args: &[String]) -> Option<&String> {
    args.iter().skip(1).find(|a| a.to_ascii_lowercase().starts_with("nxm://"))
}

pub fn run() {
    tauri::Builder::default()
        // First: a second launcher (started for an nxm link) hands its arguments
        // over to this one and exits.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
            if let Some(url) = nxm_arg(&args) {
                nexus::receive(app, url);
            }
        }))
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
                verify: Mutex::new(None),
                author_job: Mutex::new(None),
                built: Mutex::new(None),
                launcher_update: Mutex::new(None),
                nexus: nexus::NexusState::default(),
            });
            let args: Vec<String> = std::env::args().collect();
            if let Some(url) = nxm_arg(&args) {
                nexus::receive(app.handle(), url);
            }
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
            commands::verify_build,
            commands::cancel_verify,
            commands::start_repair,
            commands::set_mod_enabled,
            author::author_changes,
            author::author_adopt,
            author::author_secrets,
            author::author_enable,
            author::author_set_secret,
            author::author_built,
            author::author_build,
            author::author_publish,
            author::author_cancel,
            commands::open_mod_folder,
            commands::open_folder,
            commands::launch_game,
            commands::open_mo2,
            commands::export_report,
            commands::check_launcher_update,
            commands::install_launcher_update,
            nexus::nexus_status,
            nexus::nexus_set_key,
            nexus::nexus_logout,
            nexus::nexus_sso_login,
            nexus::nexus_sso_cancel,
            nexus::nxm_register,
            nexus::nxm_unregister,
            nexus::nexus_updates,
            nexus::nexus_check,
            nexus::nexus_check_cancel,
            nexus::nexus_jobs,
            nexus::nexus_cancel_job,
            nexus::nexus_clear_jobs,
            nexus::nexus_download,
            nexus::nexus_fomod,
            nexus::nexus_fomod_eval,
            nexus::nexus_fomod_install,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LYNO//HARDWIRED");
}

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use lyno_core::diagnose;
use lyno_core::download::{Downloader, PartEvent};
use lyno_core::game::{self, GameInstall};
use lyno_core::history;
use lyno_core::install::{self, Installer};
use lyno_core::manifest::{ChangelogEntry, Manifest, ModEntry};
use lyno_core::mo2::{self, Instance};
use lyno_core::mod_install;
use lyno_core::modlist::{Entry, EntryState, ModList};
use lyno_core::plan;
use lyno_core::report;
use lyno_core::state::State;
use lyno_core::verify;
use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter, Manager, State as TauriState};
use tauri_plugin_updater::UpdaterExt;

use crate::settings::{self, InstanceEntry, Settings};
use crate::AppState;

pub(crate) const GAME_PROCESS: &str = "Cyberpunk2077.exe";
pub(crate) const MO2_PROCESS: &str = "ModOrganizer.exe";
const DEFAULT_PROFILE: &str = "LYNO";

/// Errors cross the IPC boundary as plain strings for the UI to show.
pub(crate) type CmdResult<T> = Result<T, String>;

pub(crate) fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Remembers the profile's list before the launcher changes it (`history`). Only logged when it
/// fails: the change the player asked for matters more than its undo.
pub(crate) fn snapshot(inst: &Instance, profile: &str, label: &str, merge: bool) {
    if let Err(e) = history::record(inst, profile, label, merge) {
        log::warn!("modlist snapshot of {profile:?}: {e}");
    }
}


pub(crate) fn installed_manifest_path(inst: &Instance) -> PathBuf {
    install::installed_manifest_path(inst)
}

pub(crate) fn installed_manifest(inst: &Instance) -> Option<Manifest> {
    let text = std::fs::read_to_string(installed_manifest_path(inst)).ok()?;
    Manifest::from_json(&text).ok()
}

/// The build's profile; for the player's own MO2 the one it opened last.
pub(crate) fn profile(inst: &Instance) -> String {
    installed_manifest(inst)
        .map(|m| m.profile)
        .or_else(|| inst.selected_profile())
        .unwrap_or_else(|| DEFAULT_PROFILE.to_owned())
}

#[tauri::command]
pub fn get_settings(state: TauriState<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
pub fn save_settings(state: TauriState<'_, AppState>, mut settings: Settings) -> CmdResult<()> {
    // Instances change only through the `instance_*` commands.
    {
        let current = state.settings.lock().unwrap();
        settings.instance_dir = current.instance_dir.clone();
        settings.instances = current.instances.clone();
    }
    // Turning author mode on goes through `author_enable`, which checks the token.
    if settings.author_mode && !state.settings.lock().unwrap().author_mode {
        return Err("Режим автора включается по токену GitHub: «Я автор сборки» внизу настроек".into());
    }
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
    /// The patch of the player's `Cyberpunk2077.exe`.
    game_patch: Option<String>,
    /// The installed build was made for another patch.
    game_mismatch: Option<game::GameMismatch>,
}

/// `get_status` runs every few seconds; the exe is read again only when it changes (a game update).
fn exe_version(game_dir: &Path) -> Option<game::ExeVersion> {
    type Key = (PathBuf, u64, Option<std::time::SystemTime>);
    static CACHE: std::sync::Mutex<Option<(Key, Option<game::ExeVersion>)>> = std::sync::Mutex::new(None);
    let meta = std::fs::metadata(game::exe_path(game_dir)).ok()?;
    let key = (game_dir.to_owned(), meta.len(), meta.modified().ok());
    let mut cache = CACHE.lock().unwrap();
    if let Some((_, v)) = cache.as_ref().filter(|(k, _)| *k == key) {
        return *v;
    }
    let version = game::exe_version(game_dir);
    log::info!("game exe {}: {version:?}", game::exe_path(game_dir).display());
    *cache = Some((key, version));
    version
}

#[tauri::command]
pub fn get_status(state: TauriState<'_, AppState>) -> Status {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let list = ModList::load(&inst.modlist_path(&profile(&inst))).ok();
    let installed = State::load(&install::state_path(&inst)).unwrap_or_default();
    let (game_running, mo2_running) = running_processes();
    if game_running {
        // Started from MO2 itself: the logs it writes at start are a few seconds older than this poll.
        let mut launch = state.last_launch.lock().unwrap();
        launch.get_or_insert_with(|| std::time::SystemTime::now() - std::time::Duration::from_secs(15));
    }
    let exe = settings.game_dir.as_deref().filter(|d| game::is_game_dir(d)).and_then(exe_version);
    let built_for = install::has_build(&inst).then(|| installed_manifest(&inst)).flatten();
    let game_mismatch = exe.as_ref().zip(built_for).and_then(|(exe, m)| game::check(&m.game_version, m.game_exe.as_deref(), exe));
    Status {
        game_patch: exe.map(|v| v.patch()),
        game_mismatch,
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
    /// The player's copies of mods the update brought into the build, switched off (still on disk).
    own_copies: Vec<OwnCopyInfo>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnCopyInfo {
    /// The build mod.
    id: String,
    title: String,
    /// The player may switch the build's off and keep theirs.
    optional: bool,
    /// The player's folder.
    folder: String,
    renamed_from: Option<String>,
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
        /// The player may switch it on or off (see `set_mod_enabled`); false for core mods.
        optional: bool,
        description: Option<String>,
        requires: Vec<String>,
        size: u64,
        /// Installed but a different version than the latest build.
        outdated: bool,
        installed: bool,
        /// `added` / `updated` by the last update (`last_update`).
        recent: Option<&'static str>,
        /// Marked for repair by an integrity check; the next update downloads it again.
        damaged: bool,
        /// In the installed build (`state.json`): the author can take back their edits of it
        /// (`author_rollback`). False for a mod of the next release.
        published: bool,
        /// An optional mod the player removed (`remove_build_mod`): updates leave it out.
        removed: bool,
    },
}

/// The build section as it is in MO2: the player or the author removes, renames and updates
/// build mods there, and the manifest only learns of it with the next release. Mods the
/// manifest adds and the player doesn't have yet go after the entry they follow in the manifest.
/// A player's own mods there are in [`user_mods`]; the author's are the next release.
fn build_rows(
    inst: &Instance,
    manifest: &Manifest,
    installed: &State,
    current: &ModList,
    author: bool,
    spec_row: impl Fn(&lyno_core::manifest::ModSpec) -> ModRow,
) -> Vec<ModRow> {
    let metas = inst.scan_mods().unwrap_or_default();
    let specs: std::collections::HashMap<&str, &lyno_core::manifest::ModSpec> =
        manifest.mod_specs().map(|m| (m.id.as_str(), m)).collect();
    let mut core_section = false;
    let mut rows: Vec<ModRow> = lyno_core::publish::split_user_section(current)
        .0
        .iter()
        .filter(|e| e.state != EntryState::Unmanaged && (author || e.is_separator() || !plan::is_players(manifest, installed, e)))
        .map(|e| {
            let meta = metas.get(&e.name).cloned().unwrap_or_default();
            if let Some(title) = e.separator_title() {
                core_section = meta.lyno_core;
                return ModRow::Separator { title: title.to_owned(), color: meta.color };
            }
            let id = lyno_core::publish::mod_id(&e.name, &meta);
            let spec = specs.get(id.as_str());
            let mut row = match spec {
                Some(m) => spec_row(m),
                None => {
                    let game = meta.game_name.clone().unwrap_or_else(|| "cyberpunk2077".into()).to_lowercase();
                    ModRow::Mod {
                        id: id.clone(),
                        name: e.name.clone(),
                        title: None,
                        version: None,
                        author: None,
                        nexus_url: meta.mod_id.map(|n| format!("https://www.nexusmods.com/{game}/mods/{n}")),
                        enabled: false,
                        optional: true,
                        description: None,
                        requires: vec![],
                        size: 0,
                        outdated: false,
                        installed: true,
                        recent: None,
                        damaged: false,
                        removed: false,
                        published: installed.mods.contains_key(&id),
                    }
                }
            };
            if author {
                if let ModRow::Mod { optional, title, author, description, requires, .. } = &mut row {
                    *optional = meta.lyno_optional || !(meta.lyno_core || core_section);
                    if meta.lyno_title.is_some() { title.clone_from(&meta.lyno_title); }
                    if meta.lyno_author.is_some() { author.clone_from(&meta.lyno_author); }
                    description.clone_from(&meta.lyno_description);
                    requires.clone_from(&meta.lyno_requires);
                }
            }
            if let ModRow::Mod { name, version, enabled, .. } = &mut row {
                *name = e.name.clone();
                *enabled = e.state == EntryState::Enabled;
                // What is installed: updated in MO2 or from Nexus ahead of the published manifest.
                if meta.version.is_some() {
                    version.clone_from(&meta.version);
                }
            }
            row
        })
        .collect();

    let key = |r: &ModRow| match r {
        ModRow::Separator { title, .. } => format!("s:{title}"),
        ModRow::Mod { id, .. } => format!("m:{id}"),
    };
    let mut after: Option<String> = None;
    for e in &manifest.mods {
        let k = match e {
            ModEntry::Separator { title, .. } => format!("s:{title}"),
            ModEntry::Mod(m) => format!("m:{}", m.id),
        };
        if let ModEntry::Mod(m) = e {
            // A removed mod the player has their own version of shows as theirs (`user_mods`).
            let own = plan::is_removed(m, installed) && current.get(&m.name).is_some();
            if !installed.mods.contains_key(&m.id) && !own && !rows.iter().any(|r| key(r) == k) {
                let at = after.as_ref().and_then(|a| rows.iter().position(|r| key(r) == *a)).map_or(0, |i| i + 1);
                rows.insert(at, spec_row(m));
            }
        }
        if rows.iter().any(|r| key(r) == k) {
            after = Some(k);
        }
    }
    rows
}

/// Fetches the latest manifest from GitHub and compares it with what is installed.
/// Falls back to the installed manifest when offline. `None` for the player's own MO2.
#[tauri::command]
pub async fn fetch_build(app: AppHandle) -> CmdResult<Option<BuildInfo>> {
    let state = app.state::<AppState>();
    let settings = state.settings.lock().unwrap().clone();
    if !settings.build() {
        *state.manifest.lock().unwrap() = None;
        return Ok(None);
    }
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

    let current = ModList::load(&inst.modlist_path(&manifest.profile)).unwrap_or_default();
    // A build mod the author deleted in MO2 is a change for the next release, not the player's choice.
    let installed = match settings.author_mode {
        true => State::load(&install::state_path(&inst)),
        false => install::load_state(&inst, &manifest, &current),
    }
    .map_err(err)?;
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

    let spec_row = |m: &lyno_core::manifest::ModSpec| {
        let have = installed.mods.get(&m.id);
        ModRow::Mod {
            id: m.id.clone(),
            name: m.name.clone(),
            title: m.title.clone(),
            // Builds published before `meta::display_version` carry MO2's padded `1.35.0`.
            version: m.version.as_deref().map(lyno_core::meta::display_version),
            author: m.author.clone(),
            nexus_url: m.nexus.as_ref().map(|n| n.url()),
            enabled: plan::is_enabled(m, &installed, &current),
            optional: m.optional,
            description: m.description.clone(),
            requires: m.requires.clone(),
            size: m.package.size,
            outdated: have.is_some_and(|h| h.hash != m.package.hash),
            installed: have.is_some(),
            recent: recent(&m.id),
            damaged: have.is_some_and(|h| h.damaged),
            removed: plan::is_removed(m, &installed),
            published: have.is_some(),
        }
    };
    let mods = match installed.build_version {
        // First install: nothing in MO2 yet, the list is the manifest's.
        None => manifest
            .mods
            .iter()
            .map(|e| match e {
                ModEntry::Separator { title, color } => ModRow::Separator { title: title.clone(), color: color.clone() },
                ModEntry::Mod(m) => spec_row(m),
            })
            .collect(),
        Some(_) => build_rows(&inst, &manifest, &installed, &current, settings.author_mode, spec_row),
    };

    Ok(Some(BuildInfo {
        name: manifest.name.clone(),
        latest_version: manifest.build_version.clone(),
        installed_version: installed.build_version.clone(),
        game_version: manifest.game_version.clone(),
        changelog: manifest.changelog.clone(),
        mods,
        // A metadata-only release still needs to be adopted and stamped locally.
        up_to_date: plan.is_up_to_date() && installed.build_version.as_deref() == Some(manifest.build_version.as_str()),
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
            let own_copies = u
                .own_copies
                .iter()
                .filter(|c| inst.mods_dir().join(&c.folder).is_dir())
                .filter_map(|c| {
                    let spec = manifest.mod_specs().find(|m| m.id == c.id)?;
                    Some(OwnCopyInfo {
                        id: c.id.clone(),
                        title: spec.title.clone().unwrap_or_else(|| spec.name.clone()),
                        optional: spec.optional,
                        folder: c.folder.clone(),
                        renamed_from: c.renamed_from.clone(),
                    })
                })
                .collect();
            Some(LastUpdateInfo { from: u.from.clone()?, to: u.to.clone(), removed: u.removed.clone(), own_copies })
        }),
    }))
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
    if !settings.build() {
        return Err(NOT_BUILD.into());
    }
    if settings.author_mode {
        return Err(AUTHOR_MODE_NO_UPDATE.into());
    }
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
    let current = ModList::load(&inst.modlist_path(&manifest.profile)).unwrap_or_default();
    let state = install::load_state(&inst, manifest, &current)?;
    let label = match &state.build_version {
        Some(v) if *v == manifest.build_version => "Перед восстановлением сборки".to_owned(),
        Some(_) => format!("Перед обновлением сборки до {}", manifest.build_version),
        None => "Перед установкой сборки".to_owned(),
    };
    snapshot(&inst, &manifest.profile, &label, false);
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
        if state.author_job.lock().unwrap().is_some() {
            return Err(AUTHOR_JOB_RUNNING.into());
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
    let settings = state.settings.lock().unwrap().clone();
    if settings.author_mode {
        return Err(AUTHOR_MODE_NO_UPDATE.into());
    }
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2 перед починкой".into());
    }
    let path = install::state_path(&Instance::new(&settings.instance_dir));
    let mut installed = State::load(&path).map_err(err)?;
    verify::mark(&mut installed, &ids, base, reset_settings);
    installed.save(&path).map_err(err)?;
    log::info!("repair: mods {ids:?}, base {base}, reset settings {reset_settings}");
    start_update(app)
}

/// The build never goes over the player's own MO2: it brings its own MO2 and config.
const NOT_BUILD: &str = "Выбран ваш Mod Organizer 2, а сборка ставится в свою папку: выберите её в настройках";

/// The game and MO2 write into the instance being packed.
const AUTHOR_JOB_RUNNING: &str = "Идёт сборка или публикация выпуска: дождитесь окончания";

/// Both would undo the author's work: an update takes mods it doesn't know
/// for the player's, a repair downloads edited mods again.
const AUTHOR_MODE_NO_UPDATE: &str =
    "В режиме автора обновление и починка выключены: они откатили бы ваши правки модов. \
     Выключите режим автора в настройках, если нужно поставить опубликованную версию.";

/// Switches a non-core build mod on or off in the player's `modlist.txt`.
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
        e.to_string()
    })?;
    let folder = installed.mods.get(&id).map_or(id.as_str(), |m| m.folder.as_str());
    snapshot(&inst, &profile(&inst), &switch_label(folder, enabled), true);
    list.save(&path).map_err(err)?;
    log::info!("mod {id} {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

/// Deletes an optional build mod for good: updates no longer bring it back. MO2 would write
/// the list back on exit, so it must be closed.
#[tauri::command]
pub fn remove_build_mod(state: TauriState<'_, AppState>, id: String) -> CmdResult<()> {
    let inst = editable(&state)?;
    let manifest = state.manifest.lock().unwrap().clone().or_else(|| installed_manifest(&inst)).ok_or("Сборка не установлена")?;
    install::remove_mod(&inst, &manifest, &id).map_err(|e| {
        log::error!("remove build mod {id}: {e}");
        match e {
            lyno_core::Error::Manifest(message) => message,
            e => format!("Не удалось удалить мод: {e}"),
        }
    })?;
    log::info!("removed build mod {id}");
    Ok(())
}

/// Brings a removed build mod back with the next update.
#[tauri::command]
pub fn restore_build_mod(state: TauriState<'_, AppState>, id: String) -> CmdResult<()> {
    let inst = editable(&state)?;
    install::restore_mod(&inst, &id).map_err(err)?;
    log::info!("restored build mod {id}");
    Ok(())
}

/// A row of the player's own mods, in MO2's order.
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum UserRow {
    Separator { title: String, color: Option<String> },
    #[serde(rename_all = "camelCase")]
    Mod {
        name: String,
        enabled: bool,
        version: Option<String>,
        nexus_url: Option<String>,
        /// Among the build's: under this build entry (a mod's folder or a separator's), after
        /// the player's mods already there; `""`: above every build entry.
        /// `None`: under `LYNO USER MODS`.
        after: Option<String>,
        /// The player's version of this optional build mod (manifest id): `restore_build_mod`
        /// brings the build's back over it.
        build_id: Option<String>,
    },
}

/// The player's own mods: not in the manifest, so the build's list doesn't show them.
/// Their section, and their mods still among the build's: their version of a build mod, or one put
/// there in MO2 (`plan::target_modlist` moves it to the section with the next update).
#[tauri::command]
pub fn user_mods(state: TauriState<'_, AppState>) -> CmdResult<Vec<UserRow>> {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let path = inst.modlist_path(&profile(&inst));
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let list = ModList::load(&path).map_err(err)?;
    let (build, user) = match install::has_build(&inst) {
        true => {
            let (build, user) = lyno_core::publish::split_user_section(&list);
            (build, user.get(1..).unwrap_or_default())
        }
        false => (&[][..], &list.entries[..]),
    };
    // The author's mods in the build section are the next release: the build's list shows them.
    let manifest = match settings.author_mode {
        true => None,
        false => state.manifest.lock().unwrap().clone().or_else(|| installed_manifest(&inst)),
    };
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    let row = |e: &Entry, after: Option<String>| {
        let meta = inst.mods_dir().join(&e.name).join("meta.ini");
        let meta = lyno_core::meta::ModMeta::load(&meta).unwrap_or_default();
        match e.separator_title() {
            Some(title) => UserRow::Separator { title: title.to_owned(), color: meta.color },
            None => {
                let game = meta.game_name.clone().unwrap_or_else(|| "cyberpunk2077".into()).to_lowercase();
                UserRow::Mod {
                    name: e.name.clone(),
                    enabled: e.state == EntryState::Enabled,
                    version: meta.version,
                    nexus_url: meta.mod_id.map(|id| format!("https://www.nexusmods.com/{game}/mods/{id}")),
                    after,
                    build_id: manifest
                        .as_ref()
                        .and_then(|m| m.mod_specs().find(|m| m.name == e.name && plan::is_removed(m, &installed)))
                        .map(|m| m.id.clone()),
                }
            }
        }
    };
    let mut rows = Vec::new();
    if let Some(manifest) = manifest.as_ref().filter(|_| !build.is_empty()) {
        let mut anchor = String::new();
        for e in build.iter().filter(|e| e.state != EntryState::Unmanaged) {
            if !e.is_separator() && plan::is_players(manifest, &installed, e) {
                rows.push(row(e, Some(anchor.clone())));
            } else {
                anchor = e.name.clone();
            }
        }
    }
    rows.extend(user.iter().filter(|e| e.state != EntryState::Unmanaged).map(|e| row(e, None)));
    Ok(rows)
}

/// A folder as the list shows it: a separator by its title.
fn shown(folder: &str) -> String {
    let e = Entry::enabled(folder);
    e.separator_title().unwrap_or(folder).to_owned()
}

fn switch_label(folder: &str, enabled: bool) -> String {
    format!("Перед {} «{folder}»", if enabled { "включением" } else { "выключением" })
}

/// Switches a mod of the player's own section on or off.
#[tauri::command]
pub fn set_user_mod_enabled(state: TauriState<'_, AppState>, folder: String, enabled: bool) -> CmdResult<()> {
    if state.update.lock().unwrap().is_some() {
        return Err("Дождитесь окончания обновления сборки".into());
    }
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2, чтобы включать и выключать моды".into());
    }
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let path = inst.modlist_path(&profile(&inst));
    let mut list = ModList::load(&path).map_err(err)?;
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    // The author's list is MO2's: a mod of the next release has no manifest id to switch it by.
    let entry = list
        .entries
        .iter_mut()
        .find(|e| e.name == folder && !e.is_separator() && e.state != EntryState::Unmanaged && (settings.author_mode || !installed.is_managed_folder(&e.name)))
        .ok_or("Это не ваш мод: моды сборки включаются в её списке")?;
    entry.state = if enabled { EntryState::Enabled } else { EntryState::Disabled };
    snapshot(&inst, &profile(&inst), &switch_label(&folder, enabled), true);
    list.save(&path).map_err(err)?;
    log::info!("user mod {folder:?} {}", if enabled { "enabled" } else { "disabled" });
    Ok(())
}

#[tauri::command]
pub fn open_user_mod_folder(state: TauriState<'_, AppState>, folder: String) -> CmdResult<()> {
    if folder.contains(['/', '\\']) || folder == ".." {
        return Err("Непонятная папка".into());
    }
    let inst = Instance::new(&state.settings.lock().unwrap().instance_dir);
    open_dir(&inst.mods_dir().join(folder))
}

/// A mod the player may delete or rename: their own, or any in author mode.
/// A build mod of a player is the build's to change: it would come back with the next update.
fn own_mod(state: &AppState, folder: &str) -> CmdResult<Instance> {
    let inst = editable(state)?;
    let author = state.settings.lock().unwrap().author_mode;
    if folder.is_empty() || folder.contains(['/', '\\']) || folder == ".." || !inst.mods_dir().join(folder).is_dir() {
        return Err("Мод не найден".into());
    }
    if folder == Entry::separator(plan::USER_SEPARATOR).name {
        return Err("Этот разделитель отделяет ваши моды от сборки: его менять нельзя".into());
    }
    if !author && State::load(&install::state_path(&inst)).map_err(err)?.is_managed_folder(folder) {
        return Err("Это мод сборки: его можно только выключить".into());
    }
    // Build separators aren't in the state: the list tells them apart.
    if !author && Entry::enabled(folder).is_separator() {
        let list = ModList::load(&inst.modlist_path(&profile(&inst))).map_err(err)?;
        if !mod_install::is_users(&list, install::has_build(&inst), folder) {
            return Err("Это разделитель сборки: его можно поменять только в её списке".into());
        }
    }
    Ok(inst)
}

/// The instance, once nothing else writes `modlist.txt`: MO2 keeps the list in
/// memory and writes it back on exit, an update rewrites it.
fn editable(state: &AppState) -> CmdResult<Instance> {
    if state.update.lock().unwrap().is_some() {
        return Err("Дождитесь окончания обновления сборки".into());
    }
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2, чтобы менять моды".into());
    }
    Ok(Instance::new(&state.settings.lock().unwrap().instance_dir))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    id: u64,
    label: String,
    more: u32,
    build: Option<String>,
    diff: history::Diff,
    /// Why it can't come back.
    blocked: Option<String>,
}

/// A list from before a build update holds the old build's section: going back to it would
/// undo the update in the list only. The author's list is theirs, the player's own MO2 too.
fn snapshot_blocked(build_instance: bool, snapshot: &history::Snapshot, installed: &State) -> Option<String> {
    let other = build_instance && snapshot.build != installed.build_version;
    other.then(|| match &snapshot.build {
        Some(v) => format!("Список сборки {v}: после обновления сборки его не вернуть"),
        None => "Список до установки сборки: его не вернуть".to_owned(),
    })
}

/// The profile's list snapshots, newest first, with what going back would change.
#[tauri::command]
pub fn modlist_history(state: TauriState<'_, AppState>) -> CmdResult<Vec<HistoryRow>> {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let profile = profile(&inst);
    let current = ModList::load(&inst.modlist_path(&profile)).unwrap_or_default();
    let installed = State::load(&install::state_path(&inst)).unwrap_or_default();
    let build = install::has_build(&inst) && !settings.author_mode;
    Ok(history::list(&inst, &profile)
        .into_iter()
        .map(|s| HistoryRow {
            blocked: snapshot_blocked(build, &s, &installed),
            diff: history::diff(&inst, &s, &current),
            id: s.id,
            label: s.label,
            more: s.more,
            build: s.build,
        })
        .collect())
}

/// Puts a snapshot's order and on/off back; the list before that becomes a snapshot itself.
#[tauri::command]
pub fn modlist_restore(state: TauriState<'_, AppState>, id: u64) -> CmdResult<()> {
    let inst = editable(&state)?;
    let author = state.settings.lock().unwrap().author_mode;
    let profile = profile(&inst);
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    let build = install::has_build(&inst) && !author;
    let snap = history::list(&inst, &profile).into_iter().find(|s| s.id == id).ok_or("Этого снимка уже нет")?;
    if let Some(why) = snapshot_blocked(build, &snap, &installed) {
        return Err(why);
    }
    let manifest = build.then(|| installed_manifest(&inst)).flatten();
    if build && manifest.is_none() {
        return Err("Сборка не установлена".into());
    }
    let list = history::restored(&inst, &profile, id, manifest.as_ref().map(|m| (m, &installed))).map_err(err)?;
    snapshot(&inst, &profile, "Перед возвратом списка", false);
    list.save(&inst.modlist_path(&profile)).map_err(err)?;
    log::info!("modlist of {profile:?} restored to snapshot {id} ({:?})", snap.label);
    Ok(())
}

#[tauri::command]
pub fn delete_mod(state: TauriState<'_, AppState>, folder: String) -> CmdResult<()> {
    let inst = own_mod(&state, &folder)?;
    snapshot(&inst, &profile(&inst), &format!("Перед удалением «{}»", shown(&folder)), true);
    lyno_core::mod_install::remove(&inst, &folder).map_err(|e| {
        log::error!("delete mod {folder:?}: {e}");
        format!("Не удалось удалить мод: {e}")
    })?;
    log::info!("deleted mod {folder:?}");
    Ok(())
}

/// Renames a mod, or a separator by its title (`name`).
#[tauri::command]
pub fn rename_mod(state: TauriState<'_, AppState>, folder: String, name: String) -> CmdResult<()> {
    let inst = own_mod(&state, &folder)?;
    let title = name.trim();
    let separator = Entry::enabled(&folder).is_separator();
    let name = if separator { separator_folder(title)? } else { title.to_owned() };
    if let Some(why) = mod_install::invalid_name(title) {
        return Err(why.into());
    }
    // Windows names ignore case: a change of case only is the same folder.
    if inst.mods_dir().join(&name).exists() && !folder.eq_ignore_ascii_case(&name) {
        return Err(if separator { format!("Разделитель «{title}» уже есть") } else { format!("Мод «{name}» уже есть") });
    }
    mod_install::rename(&inst, &folder, &name).map_err(|e| {
        log::error!("rename mod {folder:?}: {e}");
        format!("Не удалось переименовать мод: {e}")
    })?;
    log::info!("renamed mod {folder:?} to {name:?}");
    Ok(())
}

/// MO2's folder of a separator the player names; never the one of their section.
fn separator_folder(title: &str) -> CmdResult<String> {
    if let Some(why) = mod_install::invalid_name(title) {
        return Err(why.into());
    }
    if title.eq_ignore_ascii_case(plan::USER_SEPARATOR) {
        return Err("Это название занято лаунчером".into());
    }
    Ok(Entry::separator(title).name)
}

/// Drags a mod or separator of the player below `after` (`None`: the end), in the current
/// profile only: the order is a profile's, as in MO2. Within their section, over the build; the author's anywhere.
#[tauri::command]
pub fn move_user_mod(state: TauriState<'_, AppState>, folder: String, after: Option<String>) -> CmdResult<()> {
    let inst = editable(&state)?;
    let path = inst.modlist_path(&profile(&inst));
    let mut list = ModList::load(&path).map_err(err)?;
    let installed = State::load(&install::state_path(&inst)).map_err(err)?;
    let moved = match state.settings.lock().unwrap().author_mode {
        true => mod_install::move_any(&mut list, &folder, after.as_deref()),
        false => mod_install::move_entry(&mut list, install::has_build(&inst), |f| installed.is_managed_folder(f), &folder, after.as_deref()),
    };
    if !moved {
        return Err("Перетаскивать можно только ваши моды: порядок сборки задаёт её автор".into());
    }
    snapshot(&inst, &profile(&inst), &format!("Перед перемещением «{}»", shown(&folder)), !folder.ends_with("_separator"));
    list.save(&path).map_err(err)?;
    log::info!("moved {folder:?} below {after:?}");
    Ok(())
}

/// A new separator of the player below `after` (`None`: the end of their section).
#[tauri::command]
pub fn add_separator(state: TauriState<'_, AppState>, title: String, after: Option<String>) -> CmdResult<()> {
    let inst = editable(&state)?;
    let title = title.trim();
    let folder = separator_folder(title)?;
    if inst.mods_dir().join(&folder).exists() {
        return Err(format!("Разделитель «{title}» уже есть"));
    }
    mod_install::add_separator(&inst, &profile(&inst), title, after.as_deref()).map_err(|e| {
        log::error!("add separator {title:?}: {e}");
        format!("Не удалось добавить разделитель: {e}")
    })?;
    log::info!("added separator {title:?}");
    Ok(())
}

/// `color`: `#rrggbb`, `None` takes it away. MO2 reads it from the separator's `meta.ini`.
#[tauri::command]
pub fn set_separator_color(state: TauriState<'_, AppState>, folder: String, color: Option<String>) -> CmdResult<()> {
    if !Entry::enabled(&folder).is_separator() {
        return Err("Это не разделитель".into());
    }
    let inst = own_mod(&state, &folder)?;
    lyno_core::meta::save_color(&inst.mods_dir().join(&folder).join("meta.ini"), color.as_deref()).map_err(err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverwriteInfo {
    files: usize,
    size: u64,
}

/// What the game and its tools wrote into MO2's `overwrite/`.
#[tauri::command]
pub fn overwrite_info(state: TauriState<'_, AppState>) -> CmdResult<OverwriteInfo> {
    let inst = Instance::new(&state.settings.lock().unwrap().instance_dir);
    let files = mod_install::overwrite_files(&inst).map_err(err)?;
    Ok(OverwriteInfo { files: files.len(), size: files.iter().map(|f| f.size).sum() })
}

/// MO2's "Create Mod" of `overwrite/`.
#[tauri::command]
pub fn overwrite_to_mod(state: TauriState<'_, AppState>, name: String) -> CmdResult<()> {
    let inst = editable(&state)?;
    let name = name.trim();
    if let Some(why) = mod_install::invalid_name(name) {
        return Err(why.into());
    }
    if inst.mods_dir().join(name).exists() {
        return Err(format!("Мод «{name}» уже есть"));
    }
    mod_install::overwrite_to_mod(&inst, &profile(&inst), name).map_err(|e| {
        log::error!("overwrite to mod {name:?}: {e}");
        format!("Не удалось создать мод: {e}")
    })?;
    log::info!("overwrite became mod {name:?}");
    Ok(())
}

#[tauri::command]
pub fn clear_overwrite(state: TauriState<'_, AppState>) -> CmdResult<()> {
    let inst = editable(&state)?;
    mod_install::clear_overwrite(&inst).map_err(|e| {
        log::error!("clear overwrite: {e}");
        format!("Не удалось очистить overwrite: {e}")
    })?;
    log::info!("overwrite cleared");
    Ok(())
}

/// Logs the game's frameworks wrote into `overwrite/`, newest first.
#[tauri::command]
pub fn overwrite_logs(state: TauriState<'_, AppState>) -> Vec<report::LogFile> {
    report::overwrite_logs(&Instance::new(&state.settings.lock().unwrap().instance_dir))
}

#[tauri::command]
pub fn read_overwrite_log(state: TauriState<'_, AppState>, path: String) -> CmdResult<String> {
    let inst = Instance::new(&state.settings.lock().unwrap().instance_dir);
    report::read_overwrite_log(&inst, &path)
        .map_err(|e| format!("Не удалось прочитать лог: {e}"))?
        .ok_or_else(|| format!("Лога {path} больше нет"))
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
    /// MO2's downloads: archives from Nexus and the ones dropped on the launcher.
    Downloads,
    /// What the game wrote through MO2's virtual file system.
    Overwrite,
}

/// Opens one picture to the webview's asset protocol (its scope is empty otherwise): the player's background.
#[tauri::command]
pub fn allow_background(app: AppHandle, path: String) -> CmdResult<()> {
    let path = PathBuf::from(path);
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if !["gif", "png", "jpg", "jpeg", "webp"].contains(&ext.as_str()) {
        return Err("Это не изображение".into());
    }
    app.asset_protocol_scope().allow_file(&path).map_err(|e| e.to_string())
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
        Folder::Downloads => {
            let dir = Instance::new(&settings.instance_dir).downloads_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            dir
        }
        Folder::Overwrite => Instance::new(&settings.instance_dir).overwrite_dir(),
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

/// MO2's executables besides the one the play button starts.
#[tauri::command]
pub fn executables(state: TauriState<'_, AppState>) -> Vec<mo2::Executable> {
    let inst = Instance::new(&state.settings.lock().unwrap().instance_dir);
    let game = mo2::game_executable(installed_manifest(&inst).is_some_and(|m| m.redmod));
    inst.executables().into_iter().filter(|e| e.title != game).collect()
}

/// Starts the game through MO2, or another of its executables (`executable`, a title).
#[tauri::command]
pub fn launch_game(state: TauriState<'_, AppState>, executable: Option<String>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    if state.author_job.lock().unwrap().is_some() {
        return Err(AUTHOR_JOB_RUNNING.into());
    }
    if executable.is_none() && running_processes().0 {
        return Err("Cyberpunk 2077 уже запущен".into());
    }
    let Some(game_dir) = settings.game_dir.as_deref().filter(|d| game::is_game_dir(d)) else {
        return Err("Не найдена папка Cyberpunk 2077. Укажите её в настройках.".into());
    };
    let inst = Instance::new(&settings.instance_dir);
    let manifest = installed_manifest(&inst);
    let redmod = manifest.as_ref().is_some_and(|m| m.redmod);
    if redmod && executable.is_none() && !game::has_redmod(game_dir) {
        return Err("В сборке есть REDmod-моды, а REDmod не установлен. \
                    Установите бесплатное DLC REDmod в Steam, GOG или Epic."
            .into());
    }
    inst.set_game_path(game_dir).map_err(err)?;
    let profile = profile(&inst);
    snapshot(&inst, &profile, "Перед запуском игры", false);
    let executable = executable.unwrap_or_else(|| mo2::game_executable(redmod).to_owned());
    log::info!("launching {executable:?} (profile {profile}, REDmod {redmod})");
    let is_game = executable == mo2::game_executable(redmod);
    // A window the player opened on their own is not the "locked" one.
    let mo2_was_open = running_processes().1;
    spawn_mo2(&inst, &mo2::run_args(&profile, &executable))?;
    if is_game {
        *state.last_launch.lock().unwrap() = Some(std::time::SystemTime::now());
        if !mo2_was_open {
            crate::lock_window::hide_until_game_over();
        }
    }
    Ok(())
}

/// For a game that hangs: the launcher's own screen has no MO2 window with an Unlock button to fall back on.
#[tauri::command]
pub fn stop_game() {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    for p in sys.processes().values().filter(|p| p.name().to_string_lossy().eq_ignore_ascii_case(GAME_PROCESS)) {
        p.kill();
    }
}

/// Errors the frameworks logged in the game's last run, once it is over; the next run starts afresh.
#[tauri::command]
pub async fn last_run(app: AppHandle) -> CmdResult<Option<diagnose::RunReport>> {
    let state = app.state::<AppState>();
    let Some(since) = state.last_launch.lock().unwrap().take() else { return Ok(None) };
    let inst = Instance::new(&state.settings.lock().unwrap().instance_dir);
    let profile = profile(&inst);
    let report = tauri::async_runtime::spawn_blocking(move || diagnose::last_run(&inst, &profile, since)).await.map_err(err)?;
    log::info!("last run: scripts failed {}, {} problem(s)", report.scripts_failed, report.problems.len());
    Ok(Some(report).filter(|r| !r.is_empty()))
}

#[tauri::command]
pub fn open_mo2(state: TauriState<'_, AppState>) -> CmdResult<()> {
    let settings = state.settings.lock().unwrap().clone();
    if state.author_job.lock().unwrap().is_some() {
        return Err(AUTHOR_JOB_RUNNING.into());
    }
    let inst = Instance::new(&settings.instance_dir);
    spawn_mo2(&inst, &mo2::open_args(&profile(&inst)))
}

fn spawn_mo2(inst: &Instance, args: &[String]) -> CmdResult<()> {
    if !inst.is_installed() {
        return Err("Mod Organizer 2 не установлен".into());
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

/// Switches the launcher to another instance it knows.
#[tauri::command]
pub fn instance_select(state: TauriState<'_, AppState>, dir: PathBuf) -> CmdResult<()> {
    let entry = state.settings.lock().unwrap().instances.iter().find(|i| i.dir == dir).cloned();
    use_instance(&state, entry.ok_or("Эта папка MO2 не добавлена в лаунчер")?)
}

/// Adds the player's own portable MO2 set up for Cyberpunk.
#[tauri::command]
pub fn instance_add(state: TauriState<'_, AppState>, dir: PathBuf) -> CmdResult<()> {
    let inst = Instance::new(&dir);
    if !inst.is_installed() {
        return Err(format!("В папке нет ModOrganizer.exe: {}", dir.display()));
    }
    // A global instance keeps mods and profiles in AppData, where the launcher doesn't look.
    if !inst.is_portable() {
        return Err("Это не портативный MO2 (в папке нет portable.txt): лаунчер работает только с портативными".into());
    }
    let ini = std::fs::read_to_string(inst.ini_path()).unwrap_or_default();
    if !ini.lines().any(|l| l.trim() == "gameName=Cyberpunk 2077") {
        return Err("Этот Mod Organizer 2 настроен не на Cyberpunk 2077".into());
    }
    let name = dir.file_name().map_or_else(|| "Mod Organizer 2".into(), |n| n.to_string_lossy().into_owned());
    use_instance(&state, InstanceEntry { name, dir, build: false })
}

/// Selects the build's instance, adding it at its default place; installing it is `start_update`.
#[tauri::command]
pub fn instance_add_build(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let known = state.settings.lock().unwrap().instances.iter().find(|i| i.build).cloned();
    let entry = match known {
        Some(e) => e,
        None => InstanceEntry {
            name: settings::BUILD_NAME.into(),
            dir: settings::build_dir(&app.path().app_local_data_dir().map_err(err)?),
            build: true,
        },
    };
    use_instance(&state, entry)
}

/// Makes `entry` the active instance, adding it to the list when new.
fn use_instance(state: &AppState, entry: InstanceEntry) -> CmdResult<()> {
    if state.update.lock().unwrap().is_some() || crate::nexus::busy(state) {
        return Err("Дождитесь окончания установки, обновления или проверки".into());
    }
    let mut settings = state.settings.lock().unwrap().clone();
    if !settings.instances.iter().any(|i| i.dir == entry.dir) {
        settings.instances.push(entry.clone());
    }
    settings.instance_dir = entry.dir;
    if let Some(game) = settings.game_dir.as_deref().filter(|d| game::is_game_dir(d)) {
        let inst = Instance::new(&settings.instance_dir);
        if inst.ini_path().is_file() {
            inst.set_game_path(game).map_err(err)?;
        }
    }
    settings.save(&state.settings_path).map_err(err)?;
    log::info!("instance {:?} (build: {})", settings.instance_dir, settings.build());
    *state.manifest.lock().unwrap() = None;
    *state.settings.lock().unwrap() = settings;
    Ok(())
}

/// Downloads the latest official MO2 into a new portable instance for Cyberpunk
/// and switches to it. Progress and outcome arrive as an update's do.
#[tauri::command]
pub fn start_mo2_setup(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let data_dir = app.path().app_local_data_dir().map_err(err)?;
    let dir = data_dir.join("mo2");
    if state.settings.lock().unwrap().instances.iter().any(|i| i.dir == dir) {
        return Err("Mod Organizer 2 уже установлен лаунчером: выберите его в настройках".into());
    }
    if crate::nexus::busy(&state) {
        return Err("Дождитесь окончания проверки или выпуска".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.update.lock().unwrap();
        if slot.is_some() {
            return Err("Установка уже идёт".into());
        }
        *slot = Some(cancel.clone());
    }
    let game_dir = state.settings.lock().unwrap().game_dir.clone().filter(|d| game::is_game_dir(d));

    std::thread::spawn(move || {
        let result = setup_mo2(&app, &data_dir, &dir, game_dir.as_deref(), &cancel);
        let state = app.state::<AppState>();
        *state.update.lock().unwrap() = None;
        let finished = match result {
            Ok(()) => match use_instance(&state, InstanceEntry { name: "Mod Organizer 2".into(), dir, build: false }) {
                Ok(()) => Finished { ok: true, error: None },
                Err(e) => Finished { ok: false, error: Some(e) },
            },
            Err(lyno_core::Error::Cancelled) => Finished { ok: false, error: None },
            Err(e) => {
                log::error!("MO2 setup failed: {e}");
                Finished { ok: false, error: Some(format!("Не удалось установить Mod Organizer 2: {e}")) }
            }
        };
        let _ = app.emit("update-finished", finished);
    });
    Ok(())
}

fn setup_mo2(app: &AppHandle, data_dir: &Path, dir: &Path, game_dir: Option<&Path>, cancel: &AtomicBool) -> lyno_core::Result<()> {
    let emit = |e: install::Event| {
        let _ = app.emit("update-progress", e);
    };
    emit(install::Event::Step { index: 1, total: 2, label: "Скачивание Mod Organizer 2".into() });
    let downloader = Downloader::new();
    let release = mo2::latest_release(&downloader)?;
    log::info!("MO2 {} from {}", release.version, release.url);
    let archive = data_dir.join(format!("Mod.Organizer-{}.7z", release.version));
    let mut last_emit = std::time::Instant::now() - std::time::Duration::from_secs(1);
    downloader.fetch_file(&release.url, release.size, &archive, &|| cancel.load(Ordering::Relaxed), &mut |p| match p {
        PartEvent::Bytes(done) if last_emit.elapsed().as_millis() >= 100 => {
            last_emit = std::time::Instant::now();
            emit(install::Event::Bytes { done, total: release.size });
        }
        PartEvent::Bytes(_) => {}
        PartEvent::Retry { attempt, delay, error } => emit(install::Event::Retry { attempt, delay_secs: delay.as_secs(), error }),
    })?;
    emit(install::Event::Step { index: 2, total: 2, label: "Распаковка Mod Organizer 2".into() });
    mo2::create_portable(&Instance::new(dir), &archive, game_dir)?;
    let _ = std::fs::remove_file(&archive);
    Ok(())
}

pub(crate) fn running_processes() -> (bool, bool) {
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
         Game exe: {exe}\n\
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
        exe = game_dir.and_then(game::exe_version).map_or_else(
            || "unknown".into(),
            |v| format!("patch {} (product {:?}, file {:?})", v.patch(), v.product, v.file)
        ),
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

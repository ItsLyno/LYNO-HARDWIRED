//! Author mode: the author releases from the instance the launcher manages
//! (see `lyno_core::author`). Build and publish run in the background like an
//! update; progress and the outcome arrive as `author-event`s.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lyno_core::author;
use lyno_core::download::Downloader;
use lyno_core::install::{self, Installer};
use lyno_core::github::{GitHub, PublishedBuild};
use lyno_core::manifest::{ChangelogEntry, Manifest};
use lyno_core::mo2::Instance;
use lyno_core::nexus::Nexus;
use lyno_core::package::PackOptions;
use lyno_core::publish::{build, BuildOptions};
use lyno_core::release::{self, github_download_root, release_tag, Channel, Event, Host, PublishOptions};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State as TauriState};

use crate::commands::{err, installed_manifest, installed_manifest_path, running_processes, CmdResult};
use crate::secrets::{self, Secret};
use crate::settings::Settings;
use crate::AppState;

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum AuthorEvent {
    Step { index: usize, total: usize, label: String },
    Bytes { done: u64, total: u64 },
    Log { line: String },
    #[serde(rename_all = "camelCase")]
    Finished { job: Job, ok: bool, error: Option<String> },
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum Job {
    Build,
    Publish,
    Promote,
}

/// A build in `out/` waiting to be published.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltRelease {
    version: String,
    game_version: String,
    notes: Vec<String>,
    /// Packages packed by this build: new or changed. Empty when `restored`.
    repacked: Vec<RepackedRow>,
    upload_size: u64,
    mods: usize,
    warnings: Vec<String>,
    /// Read back from `out/manifest.json` after a restart: only the manifest is known.
    restored: bool,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RepackedRow {
    name: String,
    /// False: a new mod.
    changed: bool,
    size: u64,
    files: Option<lyno_core::publish::FileChanges>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecretsStatus {
    github: bool,
    nexus: bool,
}

fn emit(app: &AppHandle, e: AuthorEvent) {
    let _ = app.emit("author-event", e);
}

fn out_dir(app: &AppHandle, settings: &Settings) -> CmdResult<PathBuf> {
    match &settings.author_out_dir {
        Some(dir) => Ok(dir.clone()),
        None => Ok(app.path().app_local_data_dir().map_err(err)?.join("release-out")),
    }
}

/// Author mode on, nothing else running that touches the instance.
fn ready(state: &AppState) -> CmdResult<Settings> {
    let settings = state.settings.lock().unwrap().clone();
    if !settings.author_mode {
        return Err("Доступно только в режиме автора".into());
    }
    if state.update.lock().unwrap().is_some() || state.verify.lock().unwrap().is_some() {
        return Err("Дождитесь окончания обновления или проверки файлов".into());
    }
    if state.author_job.lock().unwrap().is_some() {
        return Err("Сборка или публикация уже идёт".into());
    }
    Ok(settings)
}

#[tauri::command]
pub fn author_metadata(state: TauriState<'_, AppState>) -> CmdResult<Vec<author::MetadataRow>> {
    let settings = state.settings.lock().unwrap().clone();
    if !settings.author_mode { return Err("Доступно только в режиме автора".into()); }
    let inst = Instance::new(&settings.instance_dir);
    author::metadata(&inst, &crate::commands::profile(&inst), installed_manifest(&inst).as_ref()).map_err(err)
}

#[tauri::command]
pub fn author_save_metadata(state: TauriState<'_, AppState>, folder: String, metadata: lyno_core::meta::AuthorMetadata) -> CmdResult<()> {
    let settings = ready(&state)?;
    if running_processes() != (false, false) { return Err("Закройте игру и Mod Organizer 2 перед изменением метаданных".into()); }
    let inst = Instance::new(&settings.instance_dir);
    author::save_metadata(&inst, &crate::commands::profile(&inst), &folder, &metadata).map_err(err)
}

/// The mod list against the installed build: what the next release changes
/// besides edited files (those come from `verify_build`).
#[tauri::command]
pub fn author_changes(state: TauriState<'_, AppState>) -> CmdResult<author::Pending> {
    let settings = state.settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let installed = installed_manifest(&inst).ok_or("Сборка не установлена")?;
    author::pending(&inst, &installed).map_err(|e| {
        log::error!("author changes: {e}");
        format!("Не удалось прочитать список модов: {e}")
    })
}

/// Records the published build as installed without downloading it: the
/// author has just released it from this instance with `lyno-pack`. Fails if
/// the build has mods this instance doesn't (it was released from elsewhere).
#[tauri::command]
pub fn author_adopt(state: TauriState<'_, AppState>) -> CmdResult<()> {
    let settings = ready(&state)?;
    let manifest = state.manifest.lock().unwrap().clone().ok_or("Нет связи с GitHub: опубликованная версия не загружена")?;
    adopt(&Instance::new(&settings.instance_dir), &manifest, None)
}

fn adopt(inst: &Instance, manifest: &Manifest, out: Option<&std::path::Path>) -> CmdResult<()> {
    author::adopt(inst, manifest, out).map_err(|e| {
        log::error!("adopt build {}: {e}", manifest.build_version);
        format!("Не удалось принять версию {}: {e}", manifest.build_version)
    })?;
    let json = serde_json::to_string_pretty(manifest).map_err(err)?;
    std::fs::write(installed_manifest_path(inst), json).map_err(err)?;
    let _ = std::fs::remove_file(revert_marker(inst));
    log::info!("build {} adopted as installed", manifest.build_version);
    Ok(())
}

/// Holds the published version the author went back from ([`author_revert`]):
/// until the next release the installed build is older than the published one
/// on purpose, and a release is built on from it. Without the mark the same
/// state means a newer build to adopt first.
fn revert_marker(inst: &Instance) -> PathBuf {
    inst.root().join(".lyno").join("author-revert")
}

fn reverted_from(inst: &Instance) -> Option<String> {
    std::fs::read_to_string(revert_marker(inst)).ok().map(|s| s.trim().to_owned()).filter(|s| !s.is_empty())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Versions {
    /// The instance went back from this published version.
    reverted_from: Option<String>,
    /// Builds to go back to, newest first, without the installed one: released ones and ones only in beta.
    /// Empty without a GitHub token.
    builds: Vec<PublishedBuild>,
}

#[tauri::command]
pub async fn author_versions(app: AppHandle) -> CmdResult<Versions> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let inst = Instance::new(&settings.instance_dir);
    let reverted_from = reverted_from(&inst);
    let Some(token) = secrets::get(Secret::GithubToken) else { return Ok(Versions { reverted_from, builds: vec![] }) };
    let installed = installed_manifest(&inst).map(|m| m.build_version);
    let repo = settings.author_repo.clone();
    let builds = tauri::async_runtime::spawn_blocking(move || GitHub::new(&repo, &token).author_builds())
        .await
        .map_err(err)?
        .map_err(|e| format!("Не удалось прочитать историю выпусков: {e}"))?;
    Ok(Versions { reverted_from, builds: builds.into_iter().filter(|b| Some(&b.version) != installed.as_ref()).collect() })
}

/// Installs an earlier published build into the instance like an update, to
/// take back a bad release: build mods get that build's files and switches, mods it doesn't
/// have go under LYNO USER MODS. Players stay on the published build until the
/// author releases a new version from here; nothing is packed for it, every
/// package is in the older releases. Going back to the published build undoes it.
#[tauri::command]
pub async fn author_revert(app: AppHandle, commit: String, beta: bool) -> CmdResult<()> {
    let (settings, published) = {
        let state = app.state::<AppState>();
        let settings = ready(&state)?;
        let published = state.manifest.lock().unwrap().clone().ok_or("Нет связи с GitHub: не видно, какая версия опубликована")?;
        (settings, published)
    };
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2 перед откатом".into());
    }
    let token = secrets::get(Secret::GithubToken).ok_or("Добавьте токен GitHub")?;
    let repo = settings.author_repo.clone();
    let channel = if beta { Channel::Beta } else { Channel::Release };
    let text = tauri::async_runtime::spawn_blocking(move || GitHub::new(&repo, &token).manifest_at(&commit, channel))
        .await
        .map_err(err)?
        .map_err(|e| format!("Не удалось загрузить эту версию: {e}"))?;
    let manifest = Manifest::from_json(&text).map_err(|e| format!("Эту версию лаунчер прочитать не может: {e}"))?;
    log::info!("author goes back from build {} to {}", published.build_version, manifest.build_version);
    let (to, from) = (manifest.build_version.clone(), published.build_version);
    crate::commands::spawn_update(app, manifest, settings, move |inst| {
        let path = revert_marker(inst);
        if to == from {
            let _ = std::fs::remove_file(&path);
            return Ok(());
        }
        std::fs::write(&path, &from).map_err(|e| lyno_core::Error::Io { path, source: e })
    })
}

/// Puts the installed build's version of build mod `id` back into `folder`
/// (its folder in the list now), over the author's edits or a Nexus update,
/// in the background like an update: progress arrives as `update-progress`,
/// the outcome as `update-finished`. The installed build is the one released
/// from this instance, so its package is on GitHub.
#[tauri::command]
pub fn author_rollback(app: AppHandle, id: String, folder: String, keep_settings: bool) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let settings = ready(&state)?;
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2: они держат файлы мода".into());
    }
    let inst = Instance::new(&settings.instance_dir);
    let installed = installed_manifest(&inst).ok_or("Сборка не установлена")?;
    let spec = installed.mod_specs().find(|m| m.id == id).ok_or("Этого мода нет в опубликованной версии: это новый мод выпуска")?;
    let label = spec.title.clone().unwrap_or_else(|| folder.clone());
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.update.lock().unwrap();
        if slot.is_some() {
            return Err("Дождитесь окончания обновления".into());
        }
        *slot = Some(cancel.clone());
    }

    std::thread::spawn(move || {
        log::info!("rollback of {id:?} in {folder:?} to build {} (keep settings: {keep_settings})", installed.build_version);
        let installer = Installer { inst: &inst, manifest: &installed, downloader: &Downloader::new(), cancel: &cancel };
        let mut last_emit = Instant::now() - Duration::from_secs(1);
        let result = installer.rollback(&id, &folder, keep_settings, &mut |event| {
            let is_bytes = matches!(event, install::Event::Bytes { .. });
            if !is_bytes || last_emit.elapsed() >= Duration::from_millis(100) {
                last_emit = Instant::now();
                let _ = app.emit("update-progress", event);
            }
        });
        *app.state::<AppState>().update.lock().unwrap() = None;
        let (ok, error) = match result {
            Ok(()) => {
                log::info!("rollback of {id:?} finished");
                (true, None)
            }
            Err(lyno_core::Error::Cancelled) => {
                log::info!("rollback of {id:?} cancelled");
                (false, None)
            }
            Err(e) => {
                log::error!("rollback of {id:?} failed: {e}");
                (false, Some(format!("Не удалось вернуть опубликованную версию «{label}»: {e}")))
            }
        };
        let _ = app.emit("update-finished", serde_json::json!({ "ok": ok, "error": error }));
    });
    Ok(())
}

#[tauri::command]
pub fn author_secrets() -> SecretsStatus {
    SecretsStatus { github: secrets::get(Secret::GithubToken).is_some(), nexus: secrets::get(Secret::NexusKey).is_some() }
}

/// Saves (`Some`) or forgets (`None`) a secret. A GitHub token is checked
/// first: it must be able to push to the repository.
#[tauri::command]
pub async fn author_set_secret(app: AppHandle, secret: Secret, value: Option<String>) -> CmdResult<()> {
    let Some(value) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else {
        return secrets::delete(secret);
    };
    if let Secret::GithubToken = secret {
        let repo = app.state::<AppState>().settings.lock().unwrap().author_repo.clone();
        let token = value.clone();
        tauri::async_runtime::spawn_blocking(move || GitHub::new(&repo, &token).check_access())
            .await
            .map_err(err)?
            .map_err(|e| format!("Токен не подходит: {e}"))?;
    }
    secrets::set(secret, &value).map_err(|e| format!("Не удалось сохранить в диспетчер учётных данных Windows: {e}"))
}

/// Turns author mode on for whoever can push to the build's repository: the
/// token (`None`: the saved one) must have write access, which GitHub decides.
/// Players see no switch to flip by accident; author mode would stop their
/// updates. Editing `settings.json` by hand still works and still can't
/// publish anything without such a token.
#[tauri::command]
pub async fn author_enable(app: AppHandle, token: Option<String>) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let token = token.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty());
    let check = token.clone().or_else(|| secrets::get(Secret::GithubToken)).ok_or("Вставьте токен GitHub")?;
    let repo = state.settings.lock().unwrap().author_repo.clone();
    tauri::async_runtime::spawn_blocking(move || GitHub::new(&repo, &check).check_access())
        .await
        .map_err(err)?
        .map_err(|e| format!("Токен не подходит: {e}"))?;
    if let Some(t) = &token {
        secrets::set(Secret::GithubToken, t).map_err(|e| format!("Не удалось сохранить в диспетчер учётных данных Windows: {e}"))?;
    }
    let mut settings = state.settings.lock().unwrap().clone();
    settings.author_mode = true;
    settings.save(&state.settings_path).map_err(err)?;
    *state.settings.lock().unwrap() = settings;
    log::info!("author mode on");
    Ok(())
}

/// The build waiting to be published: from this session, or read back from
/// `out/manifest.json` if it is newer than the installed build.
#[tauri::command]
pub fn author_built(app: AppHandle) -> CmdResult<Option<BuiltRelease>> {
    let state = app.state::<AppState>();
    if let Some(b) = state.built.lock().unwrap().clone() {
        return Ok(Some(b));
    }
    let settings = state.settings.lock().unwrap().clone();
    let out = out_dir(&app, &settings)?;
    let Some(m) = std::fs::read_to_string(out.join("manifest.json")).ok().and_then(|t| Manifest::from_json(&t).ok()) else {
        return Ok(None);
    };
    let installed = installed_manifest(&Instance::new(&settings.instance_dir)).map(|m| m.build_version);
    if installed.as_deref() == Some(m.build_version.as_str()) {
        return Ok(None);
    }
    Ok(Some(BuiltRelease {
        notes: m.changelog.iter().find(|c| c.version == m.build_version).map(|c| c.notes.clone()).unwrap_or_default(),
        version: m.build_version.clone(),
        game_version: m.game_version.clone(),
        repacked: vec![],
        upload_size: 0,
        mods: m.mod_specs().count(),
        warnings: vec![],
        restored: true,
    }))
}

/// What the build would warn about from the mod list alone, before packing.
#[tauri::command]
pub async fn author_preflight(app: AppHandle, game_version: String) -> CmdResult<Vec<String>> {
    let settings = app.state::<AppState>().settings.lock().unwrap().clone();
    let out = out_dir(&app, &settings)?;
    let inst = Instance::new(&settings.instance_dir);
    let profile = installed_manifest(&inst).ok_or("Сборка не установлена")?.profile;
    // Reads every meta.ini: off the UI thread.
    tauri::async_runtime::spawn_blocking(move || lyno_core::publish::preflight(inst.root(), &profile, game_version.trim(), &out))
        .await
        .map_err(err)?
        .map_err(|e| format!("Не удалось проверить список модов: {e}"))
}

/// Packs the instance into `out/` in the background against the installed
/// build, which must be the published one: the release is a diff of this
/// instance, and an unaccepted newer build would be silently reverted. Unless
/// the author went back from it on purpose ([`author_revert`]).
#[tauri::command]
pub fn author_build(app: AppHandle, version: String, game_version: String, notes: Vec<String>) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let settings = ready(&state)?;
    if running_processes() != (false, false) {
        return Err("Закройте игру и Mod Organizer 2: MO2 переписывает список модов при выходе".into());
    }
    let (version, game_version) = (version.trim().to_owned(), game_version.trim().to_owned());
    if version.is_empty() || game_version.is_empty() {
        return Err("Укажите версию сборки и версию игры".into());
    }
    let inst = Instance::new(&settings.instance_dir);
    let installed = installed_manifest(&inst).ok_or("Сборка не установлена")?;
    let published = state.manifest.lock().unwrap().clone().ok_or("Нет связи с GitHub: не видно, какая версия опубликована")?;
    if published.build_version != installed.build_version && reverted_from(&inst).as_ref() != Some(&published.build_version) {
        return Err(format!(
            "Опубликована версия {}, а установлена {}. Сначала примите опубликованную версию.",
            published.build_version, installed.build_version
        ));
    }
    if version == published.build_version {
        return Err(format!("Версия {version} уже опубликована, укажите новую"));
    }
    let out = out_dir(&app, &settings)?;
    std::fs::create_dir_all(&out).map_err(|e| format!("{}: {e}", out.display()))?;

    let notes: Vec<String> = notes.into_iter().map(|n| n.trim().to_owned()).filter(|n| !n.is_empty()).collect();
    let mut changelog = Vec::new();
    if !notes.is_empty() {
        let date = time::OffsetDateTime::now_utc().format(time::macros::format_description!("[year]-[month]-[day]")).ok();
        changelog.push(ChangelogEntry { version: version.clone(), date, notes: notes.clone() });
    }
    changelog.extend(published.changelog.iter().filter(|e| e.version != version).cloned());
    let cancel = Arc::new(AtomicBool::new(false));
    let opts = BuildOptions {
        name: published.name.clone(),
        profile: published.profile.clone(),
        build_version: version.clone(),
        game_version: game_version.clone(),
        mo2_version: published.mo2_version.clone(),
        base_url: format!("{}/{}", github_download_root(&settings.author_repo), release_tag(&version)),
        out_dir: out.clone(),
        // Packages are reused from the build these folders hold.
        previous: Some(installed),
        changelog,
        pack: PackOptions::default(),
        hash_cache: true,
        cancel: Some(cancel.clone()),
    };
    *state.author_job.lock().unwrap() = Some(cancel);
    *state.built.lock().unwrap() = None;

    std::thread::spawn(move || {
        log::info!("author build {version} started");
        let result = run_build(&app, &inst, &opts, &notes);
        *app.state::<AppState>().author_job.lock().unwrap() = None;
        let (ok, error) = match result {
            Ok(built) => {
                log::info!("author build {version}: {} package(s), {} bytes to upload", built.repacked.len(), built.upload_size);
                *app.state::<AppState>().built.lock().unwrap() = Some(built);
                (true, None)
            }
            Err(lyno_core::Error::Cancelled) => {
                log::info!("author build cancelled");
                (false, None)
            }
            Err(e) => {
                log::error!("author build failed: {e}");
                (false, Some(format!("Не удалось собрать выпуск: {e}")))
            }
        };
        emit(&app, AuthorEvent::Finished { job: Job::Build, ok, error });
    });
    Ok(())
}

fn run_build(app: &AppHandle, inst: &Instance, opts: &BuildOptions, notes: &[String]) -> lyno_core::Result<BuiltRelease> {
    let mut nexus = Nexus::new(secrets::get(Secret::NexusKey));
    let output = build(inst.root(), opts, &mut |meta| nexus.info(meta), &mut |line| {
        log::info!("build: {line}");
        // Step lines read `[3/120] Mod name: packing 24 MB`.
        let step = line.strip_prefix('[').and_then(|r| r.split_once(']')).and_then(|(n, rest)| {
            let (i, t) = n.split_once('/')?;
            Some((i.parse().ok()?, t.parse().ok()?, rest.trim().to_owned()))
        });
        match step {
            Some((index, total, label)) => emit(app, AuthorEvent::Step { index, total, label }),
            None => emit(app, AuthorEvent::Log { line: line.to_owned() }),
        }
    })?;
    let path = opts.out_dir.join("manifest.json");
    let json = serde_json::to_string_pretty(&output.manifest)?;
    std::fs::write(&path, json + "\n").map_err(|e| lyno_core::Error::Io { path, source: e })?;
    Ok(BuiltRelease {
        version: output.manifest.build_version.clone(),
        game_version: output.manifest.game_version.clone(),
        notes: notes.to_vec(),
        repacked: output
            .repacked
            .iter()
            .map(|r| RepackedRow { name: r.name.clone(), changed: r.changed, size: output.upload_size(&r.name), files: r.files.clone() })
            .collect(),
        upload_size: output.assets.iter().map(|a| a.size).sum(),
        mods: output.manifest.mod_specs().count(),
        warnings: nexus.warnings.iter().chain(&output.warnings).cloned().collect(),
        restored: false,
    })
}

/// Uploads the build in `out/` to GitHub, checks every part, publishes the
/// manifest and records the build as installed. Safe to run again after a
/// failure: uploaded parts are skipped.
#[tauri::command]
pub fn author_publish(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let settings = ready(&state)?;
    let token = secrets::get(Secret::GithubToken).ok_or("Добавьте токен GitHub")?;
    let out = out_dir(&app, &settings)?;
    if !out.join("manifest.json").is_file() {
        return Err("Сначала соберите выпуск".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    *state.author_job.lock().unwrap() = Some(cancel.clone());

    std::thread::spawn(move || {
        let result = run_publish(&app, &settings, &token, &out, &cancel);
        let state = app.state::<AppState>();
        *state.author_job.lock().unwrap() = None;
        let (ok, error) = match result {
            Ok(manifest) => {
                let inst = Instance::new(&settings.instance_dir);
                *state.built.lock().unwrap() = None;
                *state.manifest.lock().unwrap() = Some(manifest.clone());
                match adopt(&inst, &manifest, Some(&out)) {
                    Ok(()) => (true, None),
                    Err(e) => (true, Some(format!("Версия {} опубликована, но не принята: {e}", manifest.build_version))),
                }
            }
            Err(lyno_core::Error::Cancelled) => {
                log::info!("publish cancelled");
                (false, None)
            }
            Err(e) => {
                log::error!("publish failed: {e}");
                (false, Some(format!("Не удалось опубликовать: {e}")))
            }
        };
        emit(&app, AuthorEvent::Finished { job: Job::Publish, ok, error });
    });
    Ok(())
}

/// Versions the two channels hold now; `None`: nothing published there yet.
#[derive(Serialize)]
pub struct Channels {
    beta: Option<String>,
    release: Option<String>,
}

#[tauri::command]
pub async fn author_channels(state: TauriState<'_, AppState>) -> CmdResult<Channels> {
    let settings = state.settings.lock().unwrap().clone();
    if !settings.author_mode {
        return Err("Доступно только в режиме автора".into());
    }
    let token = secrets::get(Secret::GithubToken).ok_or("Добавьте токен GitHub")?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut host = GitHub::new(&settings.author_repo, &token);
        let mut version = |channel| -> lyno_core::Result<Option<String>> {
            Ok(host.published_manifest(channel)?.and_then(|t| Manifest::from_json(&t).ok()).map(|m| m.build_version))
        };
        Ok(Channels { beta: version(Channel::Beta)?, release: version(Channel::Release)? })
    })
    .await
    .map_err(err)?
    .map_err(|e: lyno_core::Error| format!("Не удалось прочитать каналы: {e}"))
}

/// Releases the beta build to everyone: the manifest is copied, no part is uploaded.
#[tauri::command]
pub fn author_promote(app: AppHandle) -> CmdResult<()> {
    let state = app.state::<AppState>();
    let settings = ready(&state)?;
    let token = secrets::get(Secret::GithubToken).ok_or("Добавьте токен GitHub")?;
    *state.author_job.lock().unwrap() = Some(Arc::new(AtomicBool::new(false)));

    std::thread::spawn(move || {
        let mut host = GitHub::new(&settings.author_repo, &token);
        let log_line = |e: Event| match e {
            Event::Current(v) => emit(&app, AuthorEvent::Log { line: format!("Сейчас в релизе версия {}", v.as_deref().unwrap_or("—")) }),
            Event::Checking { done, total } => emit(&app, AuthorEvent::Step { index: done, total, label: "Проверка частей по HTTP".into() }),
            Event::Live { version } => emit(&app, AuthorEvent::Log { line: format!("Версия {version} выпущена в релиз") }),
            _ => {}
        };
        let result = release::promote(&mut host, &Downloader::new(), &log_line);
        *app.state::<AppState>().author_job.lock().unwrap() = None;
        let error = result.err().map(|e| {
            log::error!("promote failed: {e}");
            format!("Не удалось выпустить в релиз: {e}")
        });
        emit(&app, AuthorEvent::Finished { job: Job::Promote, ok: error.is_none(), error });
    });
    Ok(())
}

fn run_publish(app: &AppHandle, settings: &Settings, token: &str, out: &std::path::Path, cancel: &AtomicBool) -> lyno_core::Result<Manifest> {
    let mut host = GitHub::new(&settings.author_repo, token);
    let root = github_download_root(&settings.author_repo);
    let opts = PublishOptions { out, download_root: &root };
    // Byte and check progress arrive from worker threads, many times a second.
    let last = Mutex::new(Instant::now() - Duration::from_secs(1));
    let throttled = |e: AuthorEvent, last_one: bool| {
        let mut t = last.lock().unwrap();
        if last_one || t.elapsed() >= Duration::from_millis(100) {
            *t = Instant::now();
            emit(app, e);
        }
    };
    let log_line = |line: String| {
        log::info!("publish: {line}");
        emit(app, AuthorEvent::Log { line });
    };
    let manifest = release::publish(&mut host, &opts, &Downloader::new(), cancel, &mut |_| true, &|e| match e {
        Event::Current(v) => log_line(format!("Сейчас опубликована версия {}", v.as_deref().unwrap_or("—"))),
        Event::ReleaseCreated { tag } => log_line(format!("Создан релиз {tag}")),
        Event::Uploads { uploaded, total, bytes } => {
            log_line(format!("Частей к загрузке: {} из {total}, {:.1} ГБ", total - uploaded, bytes as f64 / 1e9))
        }
        Event::Uploading { index, count, name, .. } => {
            log::info!("publish: uploading {name}");
            emit(app, AuthorEvent::Step { index, total: count, label: format!("Загрузка {name}") })
        }
        Event::UploadBytes { done, total } => throttled(AuthorEvent::Bytes { done, total }, done == total),
        Event::Checking { done, total } => {
            throttled(AuthorEvent::Step { index: done, total, label: "Проверка частей по HTTP".into() }, done == total)
        }
        Event::Live { version } => log_line(format!("Версия {version} опубликована в бету")),
    })?;
    Ok(manifest)
}

#[tauri::command]
pub fn author_cancel(state: TauriState<'_, AppState>) {
    if let Some(flag) = state.author_job.lock().unwrap().as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
}

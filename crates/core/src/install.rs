//! Applies an [`UpdatePlan`] to an instance: download, unpack, swap.
//!
//! State is saved after every mod, so an interrupted update resumes where
//! it stopped (downloads resume too, see [`crate::download`]). Packages are
//! downloaded in parallel ahead of the install, see [`crate::prefetch`].

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde::Serialize;

use crate::download::Downloader;
use crate::files::{self, FileList};
use crate::manifest::{Manifest, ModEntry, ModSpec, Package};
use crate::meta;
use crate::mo2::Instance;
use crate::modlist::Entry;
use crate::package;
use crate::prefetch::{self, Job, Prefetch, Report};
use crate::rules;
use crate::plan::{Action, UpdatePlan};
use crate::state::{InstalledMod, State};
use crate::tree::{self, FileEntry};
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// Starting step `index` of `total`.
    Step { index: usize, total: usize, label: String },
    /// Overall download progress.
    Bytes { done: u64, total: u64 },
    /// The connection failed; downloads continue after `delay_secs`.
    /// Cleared by the next `Bytes`.
    #[serde(rename_all = "camelCase")]
    Retry { attempt: u32, delay_secs: u64, error: String },
    Done,
}

pub fn state_path(inst: &Instance) -> PathBuf {
    inst.root().join(".lyno").join("state.json")
}

pub struct Installer<'a> {
    pub inst: &'a Instance,
    pub manifest: &'a Manifest,
    pub downloader: &'a Downloader,
    pub cancel: &'a AtomicBool,
}

/// The packages `plan` downloads, in its order; per action, its job.
fn jobs(manifest: &Manifest, plan: &UpdatePlan, cache: &Path) -> (Vec<Job>, Vec<Option<usize>>) {
    let mut jobs = Vec::new();
    let index = plan
        .actions
        .iter()
        .map(|action| {
            let (pkg, name) = match action {
                Action::Base => (&manifest.base, "base"),
                Action::Install { id } | Action::Update { id, .. } | Action::Repair { id, .. } => {
                    let spec = manifest.mod_specs().find(|m| m.id == *id)?;
                    (&spec.package, spec.id.as_str())
                }
                Action::Rename { .. } | Action::Remove { .. } => return None,
            };
            jobs.push(Job::new(pkg, name, cache));
            Some(jobs.len() - 1)
        })
        .collect();
    (jobs, index)
}

fn cache_dir(inst: &Instance) -> PathBuf {
    inst.root().join(".lyno").join("cache")
}

/// Bytes of `plan`'s download already in the cache from an interrupted
/// update, for the UI to show what is left.
pub fn cached_bytes(inst: &Instance, manifest: &Manifest, plan: &UpdatePlan) -> u64 {
    let (jobs, _) = jobs(manifest, plan, &cache_dir(inst));
    jobs.iter().flat_map(|j| &j.parts).map(|(part, path)| prefetch::cached_len(part, path)).sum()
}

impl Installer<'_> {
    pub fn apply(&self, plan: &UpdatePlan, on: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
        let cache = cache_dir(self.inst);
        let (jobs, job_of) = jobs(self.manifest, plan, &cache);
        let fetch = Prefetch::new(&jobs, plan.download_size, self.cancel);
        let on = Mutex::new(on);
        let emit = |e: Event| (on.lock().unwrap())(e);
        let report = |r: Report| {
            emit(match r {
                Report::Bytes { done, total } => Event::Bytes { done, total },
                Report::Retry { attempt, delay, error } => Event::Retry { attempt, delay_secs: delay.as_secs(), error },
            })
        };
        report(fetch.bytes());
        let result = std::thread::scope(|scope| {
            if !jobs.is_empty() {
                for _ in 0..prefetch::PARALLEL_DOWNLOADS {
                    scope.spawn(|| fetch.worker(self.downloader, &report));
                }
            }
            let result = self.install(plan, &fetch, &job_of, &jobs, &emit);
            fetch.finish();
            result
        });
        if result.is_ok() {
            let _ = std::fs::remove_dir_all(&cache);
            emit(Event::Done);
        }
        result
    }

    fn install(
        &self,
        plan: &UpdatePlan,
        fetch: &Prefetch,
        job_of: &[Option<usize>],
        jobs: &[Job],
        on: &dyn Fn(Event),
    ) -> Result<()> {
        let state_file = state_path(self.inst);
        let mut state = State::load(&state_file)?;
        let staging = self.inst.root().join(".lyno").join("staging");
        let backup = self.inst.root().join(".lyno").join("overwrite-backup").join(&self.manifest.build_version);
        let mods_dir = self.inst.mods_dir();
        std::fs::create_dir_all(&mods_dir).map_err(|e| Error::io(&mods_dir, e))?;

        state.begin_update(&self.manifest.build_version);

        let total = plan.actions.len() + 1;
        // Downloads are already in parallel; the packages unpack one at a time.
        let unpack = |i: usize, pkg: &Package, dest: &Path| -> Result<Vec<(FileEntry, String)>> {
            let job = job_of[i].ok_or_else(|| Error::Manifest(format!("no package for action {i}")))?;
            fetch.wait(job)?;
            let paths = jobs[job].paths();
            let hashes = package::unpack(&paths, dest, &pkg.hash)?;
            for p in &paths {
                let _ = std::fs::remove_file(p);
            }
            Ok(hashes)
        };

        for (i, action) in plan.actions.iter().enumerate() {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            on(Event::Step { index: i + 1, total, label: self.label(action) });
            match action {
                Action::Base => {
                    let pkg = &self.manifest.base;
                    let dest = staging.join("base");
                    unpack(i, pkg, &dest)?;
                    let base_files: Vec<String> = tree::list_files(&dest)?.into_iter().map(|f| f.path).collect();
                    package::merge_into(&dest, self.inst.root())?;
                    remove_stale_base_files(self.inst.root(), &state.base_files, &base_files)?;
                    state.base_hash = Some(pkg.hash.clone());
                    state.base_files = base_files;
                    state.base_damaged = false;
                }
                Action::Install { id } | Action::Update { id, .. } | Action::Repair { id, .. } => {
                    let spec = self.spec(id)?;
                    let dest = staging.join(&spec.id);
                    let hashes = unpack(i, &spec.package, &dest)?;
                    if let Action::Repair { from_folder, .. } = action {
                        if !state.mods.get(id).is_some_and(|m| m.reset_settings) {
                            keep_settings(&mods_dir.join(from_folder), &dest, &hashes)?;
                        }
                    }
                    package::swap_folder(&dest, &mods_dir.join(&spec.name))?;
                    files::save(self.inst, &FileList::new(&spec.id, &spec.package.hash, &hashes))?;
                    move_shadowing_files(&self.inst.overwrite_dir(), &mods_dir.join(&spec.name), &backup)?;
                    if let Action::Update { from_folder, .. } | Action::Repair { from_folder, .. } = action {
                        if *from_folder != spec.name {
                            remove_dir(&mods_dir.join(from_folder))?;
                        }
                    }
                    let installed = InstalledMod {
                        folder: spec.name.clone(),
                        hash: spec.package.hash.clone(),
                        damaged: false,
                        reset_settings: false,
                    };
                    state.mods.insert(spec.id.clone(), installed);
                    // A repaired mod is the same version: no "updated" mark.
                    if !matches!(action, Action::Repair { .. }) {
                        let record = state.begin_update(&self.manifest.build_version);
                        if !record.added.contains(id) && !record.updated.contains(id) {
                            match action {
                                Action::Install { .. } => record.added.push(id.clone()),
                                _ => record.updated.push(id.clone()),
                            }
                        }
                    }
                }
                Action::Rename { id, from_folder } => {
                    let spec = self.spec(id)?;
                    let (from, to) = (mods_dir.join(from_folder), mods_dir.join(&spec.name));
                    std::fs::rename(&from, &to).map_err(|e| Error::io(&from, e))?;
                    if let Some(m) = state.mods.get_mut(id) {
                        m.folder = spec.name.clone();
                    }
                }
                Action::Remove { id, folder } => {
                    remove_dir(&mods_dir.join(folder))?;
                    files::remove(self.inst, id)?;
                    state.mods.remove(id);
                    let record = state.begin_update(&self.manifest.build_version);
                    record.added.retain(|a| a != id);
                    record.updated.retain(|a| a != id);
                    if !record.removed.contains(folder) {
                        record.removed.push(folder.clone());
                    }
                }
            }
            state.save(&state_file)?;
        }

        on(Event::Step { index: total, total, label: "Порядок загрузки".into() });
        for e in &self.manifest.mods {
            if let ModEntry::Separator { title, color } = e {
                let dir = mods_dir.join(Entry::separator(title).name);
                std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
                // The player's MO2 shows the author's separator colors too.
                if let Some(color) = color {
                    meta::save_color(&dir.join("meta.ini"), color)?;
                }
            }
        }
        let modlist_path = self.inst.modlist_path(&self.manifest.profile);
        if let Some(dir) = modlist_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        plan.modlist.save(&modlist_path)?;

        state.build_version = Some(self.manifest.build_version.clone());
        state.save(&state_file)?;
        let _ = std::fs::remove_dir_all(&staging);
        Ok(())
    }

    fn spec(&self, id: &str) -> Result<&ModSpec> {
        self.manifest
            .mod_specs()
            .find(|m| m.id == id)
            .ok_or_else(|| Error::Manifest(format!("mod {id:?} is not in the manifest")))
    }

    fn label(&self, action: &Action) -> String {
        let name = |id: &str| self.spec(id).map(|m| m.name.clone()).unwrap_or_else(|_| id.to_owned());
        match action {
            Action::Base => "Mod Organizer 2 и настройки".into(),
            Action::Install { id } => format!("Установка: {}", name(id)),
            Action::Update { id, .. } => format!("Обновление: {}", name(id)),
            Action::Repair { id, .. } => format!("Восстановление: {}", name(id)),
            Action::Rename { id, .. } => format!("Переименование: {}", name(id)),
            Action::Remove { folder, .. } => format!("Удаление: {folder}"),
        }
    }
}

/// Deletes files an older base package installed that the new one dropped,
/// e.g. an MO2 plugin the author removed.
fn remove_stale_base_files(root: &Path, old: &[String], new: &[String]) -> Result<()> {
    let new: std::collections::HashSet<String> = new.iter().map(|p| p.to_lowercase()).collect();
    for rel in old {
        if new.contains(&rel.to_lowercase()) || is_player_file(rel) {
            continue;
        }
        let path = tree::from_slash(root, rel);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(&path, e)),
        }
    }
    Ok(())
}

/// Profile files that may have been shipped by an older build but have
/// since become the player's own (saves, game settings).
pub(crate) fn is_player_file(rel: &str) -> bool {
    let mut parts = rel.splitn(3, '/');
    matches!((parts.next(), parts.next(), parts.next()), (Some("profiles"), Some(_), Some(file)) if rules::is_private_profile_file(file))
}

/// A repair downloads the whole mod, but the player's settings inside it (see
/// [`rules::is_settings`]) are what makes it theirs: copies them from the
/// damaged folder over the fresh package. Only paths the package ships, so a
/// stray file doesn't come back.
fn keep_settings(old: &Path, staging: &Path, files: &[(FileEntry, String)]) -> Result<()> {
    for (f, _) in files.iter().filter(|(f, _)| rules::is_settings(&f.path)) {
        let from = tree::from_slash(old, &f.path);
        if from.is_file() {
            let to = tree::from_slash(staging, &f.path);
            std::fs::copy(&from, &to).map_err(|e| Error::io(&from, e))?;
        }
    }
    Ok(())
}

/// MO2 gives `overwrite/` the highest priority. A file the player's game
/// created there before the build shipped it (typically a mod settings file)
/// would hide the build's version forever, so it is moved to `backup`.
fn move_shadowing_files(overwrite: &Path, mod_dir: &Path, backup: &Path) -> Result<()> {
    if !overwrite.is_dir() {
        return Ok(());
    }
    for f in tree::list_files_with(mod_dir, &rules::is_hashed)? {
        let shadow = tree::from_slash(overwrite, &f.path);
        if !shadow.is_file() {
            continue;
        }
        let to = tree::from_slash(backup, &f.path);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        if to.exists() {
            std::fs::remove_file(&to).map_err(|e| Error::io(&to, e))?;
        }
        std::fs::rename(&shadow, &to).map_err(|e| Error::io(&shadow, e))?;
    }
    Ok(())
}

fn remove_dir(p: &Path) -> Result<()> {
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(p, e)),
    }
}

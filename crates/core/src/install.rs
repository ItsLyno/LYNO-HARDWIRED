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
use crate::mod_install;
use crate::modlist::{Entry, EntryState, ModList};
use crate::package;
use crate::prefetch::{self, Job, Prefetch, Report};
use crate::rules;
use crate::plan::{self, Action, UpdatePlan};
use crate::state::{InstalledMod, OwnCopy, State};
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

/// Manifest of the installed build, saved by the launcher after every successful update.
pub fn installed_manifest_path(inst: &Instance) -> PathBuf {
    inst.root().join(".lyno").join("manifest.json")
}

/// The build is installed into the instance. Without it the whole mod list is
/// the player's: there is no `LYNO USER MODS` section to keep apart.
pub fn has_build(inst: &Instance) -> bool {
    State::load(&state_path(inst)).is_ok_and(|s| s.build_version.is_some())
}

/// The author going to a build, the installed one too, wants its mods as published. The plan
/// only compares package hashes: a build mod whose folder the author deleted, or `edited` ones
/// (found by [`crate::verify`]), would stay as they are. Flagged, the next update downloads them again.
pub fn mark_for_reset(inst: &Instance, edited: &[String]) -> Result<()> {
    let path = state_path(inst);
    let mut state = State::load(&path)?;
    for (id, m) in &mut state.mods {
        m.damaged |= edited.contains(id) || !inst.mods_dir().join(&m.folder).is_dir();
    }
    state.save(&path)
}

/// The installed state, with optional build mods the player deleted in MO2
/// (gone from `list` and from `mods/`) recorded as removed, as by
/// [`remove_mod`]. Not while an update is unfinished: it has installed mods
/// that aren't in the list yet. Nor for a mod marked for repair: the player
/// asked for it back.
pub fn load_state(inst: &Instance, manifest: &Manifest, list: &ModList) -> Result<State> {
    let path = state_path(inst);
    let mut state = State::load(&path)?;
    // Installed by a launcher that didn't record the build's separators: its saved manifest has them.
    if state.build_version.is_some() && state.separators.is_empty() {
        let saved = std::fs::read_to_string(installed_manifest_path(inst)).ok().and_then(|t| Manifest::from_json(&t).ok());
        if let Some(saved) = saved.filter(|m| state.build_version.as_ref() == Some(&m.build_version)) {
            state.separators = saved.separator_folders();
            state.save(&path)?;
        }
    }
    let finished = state.build_version.is_some()
        && state.last_update.as_ref().is_none_or(|u| state.build_version.as_deref() == Some(u.to.as_str()));
    if !finished {
        return Ok(state);
    }
    let gone: Vec<String> = manifest
        .mod_specs()
        .filter(|m| m.optional)
        .filter(|m| {
            state.mods.get(&m.id).is_some_and(|i| !i.damaged && list.get(&i.folder).is_none() && !inst.mods_dir().join(&i.folder).exists())
        })
        .map(|m| m.id.clone())
        .collect();
    for id in &gone {
        files::remove(inst, id)?;
        state.mods.remove(id);
        state.removed.insert(id.clone());
    }
    if !gone.is_empty() {
        state.save(&path)?;
    }
    Ok(state)
}

/// Deletes an optional build mod at the player's wish: its folder and its
/// entry in every profile. Updates no longer bring it back
/// ([`crate::plan::is_removed`]) until [`restore_mod`].
pub fn remove_mod(inst: &Instance, manifest: &Manifest, id: &str) -> Result<()> {
    let spec = manifest.mod_specs().find(|m| m.id == id).ok_or_else(|| Error::Manifest(format!("mod {id:?} is not in the manifest")))?;
    if !spec.optional {
        return Err(Error::Manifest(format!("mod {id:?} is not optional")));
    }
    let path = state_path(inst);
    let mut state = State::load(&path)?;
    let list = crate::modlist::ModList::load(&inst.modlist_path(&manifest.profile))?;
    crate::plan::ensure_not_required(manifest, &state, &list, id)?;
    if let Some(m) = state.mods.get(id) {
        mod_install::remove(inst, &m.folder)?;
        files::remove(inst, id)?;
        state.mods.remove(id);
    }
    state.removed.insert(id.to_owned());
    state.save(&path)
}

/// Id of the optional build mod installed in `folder` (`manifest`: the
/// installed build): the player may put their own version there
/// ([`detach`]). `None`: no build mod, or one the build needs.
pub fn optional_in(inst: &Instance, manifest: &Manifest, folder: &str) -> Result<Option<String>> {
    let state = State::load(&state_path(inst))?;
    let id = state.mods.iter().find(|(_, m)| m.folder == folder).map(|(id, _)| id);
    Ok(id.filter(|id| manifest.mod_specs().any(|m| m.id == **id && m.optional)).cloned())
}

/// The player puts their own version of build mod `id` into its folder: the
/// build lets go of it as of a removed one ([`remove_mod`]), and the folder by
/// its name is the player's (see [`crate::plan::is_players`]).
/// [`restore_mod`] brings the build's version back over it. Its file record stays until the
/// player's files are in: a failed swap gives the mod back to the build as it was.
pub fn detach(inst: &Instance, id: &str) -> Result<()> {
    let path = state_path(inst);
    let mut state = State::load(&path)?;
    state.mods.remove(id);
    state.removed.insert(id.to_owned());
    state.save(&path)
}

/// Undoes [`remove_mod`] or [`detach`]: the next update installs the build's
/// version again, over the player's one.
pub fn restore_mod(inst: &Instance, id: &str) -> Result<()> {
    let path = state_path(inst);
    let mut state = State::load(&path)?;
    if state.removed.remove(id) {
        state.save(&path)?;
    }
    Ok(())
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
        self.move_aside(plan)?;
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

    /// A build mod's new folder taken by a mod of the player's with that name: mostly the same
    /// mod, which the player installed before the author added it. Swapping the build's in
    /// would delete theirs, so theirs moves to `<name> (ваш)` first (in every profile, as MO2
    /// renames) and is switched off at the end ([`Self::own_copies`]). Done before anything
    /// downloads, and recorded at once: an update that fails after it still switches theirs
    /// off when it is resumed. A folder with the record of this very package is ours, left by
    /// an update cut short between the swap and saving the state.
    fn move_aside(&self, plan: &UpdatePlan) -> Result<()> {
        let path = state_path(self.inst);
        let mut state = State::load(&path)?;
        let mods = self.inst.mods_dir();
        for action in &plan.actions {
            let (Action::Install { id } | Action::Update { id, .. }) = action else { continue };
            let spec = self.spec(id)?;
            let ours = state.mods.get(id).is_some_and(|m| m.folder == spec.name) || files::load(self.inst, id, &spec.package.hash).is_some();
            if !ours && mods.join(&spec.name).exists() {
                let to = mod_install::free_folder(&mods, &format!("{} (ваш)", spec.name));
                mod_install::rename(self.inst, &spec.name, &to)?;
                let copy = OwnCopy { id: id.clone(), folder: to, renamed_from: Some(spec.name.clone()) };
                state.begin_update(&self.manifest.build_version).own_copies.push(copy);
                state.save(&path)?;
            }
        }
        Ok(())
    }

    /// The player's mods that are mods of the build now, besides those [`Self::move_aside`]
    /// renamed: the same Nexus mod with files of the new build mod. Both enabled would load
    /// the mod twice (redscript fails on the second copy of a class), with the player's over
    /// the build's; the build's is the one tested with the rest, so it wins and theirs is
    /// switched off, not deleted. Another file of the same Nexus page (an addon) shares no
    /// files and stays on. Only mods new in the build: once the player switches theirs back on,
    /// that is their choice.
    fn own_copies(&self, plan: &UpdatePlan, state: &State, list: &ModList, known: &[OwnCopy]) -> Result<Vec<OwnCopy>> {
        let mods = self.inst.mods_dir();
        let mut out: Vec<OwnCopy> = Vec::new();
        for action in &plan.actions {
            let Action::Install { id } = action else { continue };
            let spec = self.spec(id)?;
            let Some(nexus) = &spec.nexus else { continue };
            let Some(record) = files::load(self.inst, id, &spec.package.hash) else { continue };
            let theirs = list
                .mods()
                .filter(|e| e.state == EntryState::Enabled && plan::is_players(self.manifest, state, e))
                .filter(|e| !known.iter().chain(&out).any(|c| c.folder == e.name));
            let mut found = Vec::new();
            for e in theirs {
                let dir = mods.join(&e.name);
                let meta = dir.join("meta.ini");
                let same_page = meta.is_file() && meta::ModMeta::load(&meta).is_ok_and(|m| m.mod_id == Some(nexus.mod_id));
                if same_page && tree::list_files(&dir)?.iter().any(|f| record.files.contains_key(&f.path)) {
                    found.push(OwnCopy { id: id.clone(), folder: e.name.clone(), renamed_from: None });
                }
            }
            out.extend(found);
        }
        Ok(out)
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
        let before = state.clone();
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
                    let reset_settings = state.mods.get(id).is_some_and(|m| m.reset_settings);
                    if let Action::Update { from_folder, .. } | Action::Repair { from_folder, .. } = action {
                        let old = state.mods.get(id);
                        if !old.is_some_and(|m| m.reset_settings) {
                            let record = old.and_then(|m| files::load(self.inst, id, &m.hash));
                            keep_settings(&mods_dir.join(from_folder), &dest, &hashes, record.as_ref())?;
                        } else {
                            backup_settings(&mods_dir.join(from_folder), &backup.join("mods").join(from_folder))?;
                        }
                    }
                    package::swap_folder(&dest, &mods_dir.join(&spec.name))?;
                    files::save(self.inst, &FileList::new(&spec.id, &spec.package.hash, &hashes))?;
                    move_shadowing_files(&self.inst.overwrite_dir(), &mods_dir.join(&spec.name), &backup, reset_settings)?;
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
                    // Back by the player's wish, or made core by the author.
                    state.removed.remove(id);
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

        // Metadata can change while a content-addressed package stays identical.
        // Stamp the manifest's author fields even when no mod was downloaded.
        for spec in self.manifest.mod_specs().filter(|m| state.mods.contains_key(&m.id)) {
            meta::save_author(&mods_dir.join(&spec.name).join("meta.ini"), &meta::AuthorMetadata {
                id: spec.id.clone(),
                title: spec.title.clone().unwrap_or_default(),
                author: spec.author.clone().unwrap_or_default(),
                description: spec.description.clone().unwrap_or_default(),
                core: !spec.optional,
                requires: spec.requires.clone(),
            })?;
        }
        on(Event::Step { index: total, total, label: "Порядок загрузки".into() });
        for e in &self.manifest.mods {
            if let ModEntry::Separator { title, color } = e {
                let dir = mods_dir.join(Entry::separator(title).name);
                std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
                // The player's MO2 shows the author's separator colors too.
                if let Some(color) = color {
                    meta::save_color(&dir.join("meta.ini"), Some(color))?;
                }
            }
        }
        let modlist_path = self.inst.modlist_path(&self.manifest.profile);
        if let Some(dir) = modlist_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        // A renamed folder of the player's is no longer the one the plan's list knew: the list is
        // worked out again from the one on disk, which the renames updated.
        let record = state.begin_update(&self.manifest.build_version).own_copies.clone();
        let renamed = record.iter().any(|c| c.renamed_from.is_some());
        let mut modlist = match renamed {
            true => plan::target_modlist(self.manifest, &before, &ModList::load(&modlist_path).unwrap_or_default()),
            false => plan.modlist.clone(),
        };
        let found = self.own_copies(plan, &state, &modlist, &record)?;
        state.begin_update(&self.manifest.build_version).own_copies.extend(found);
        // Only copies of mods this run brings: a repair keeps the record of the last update, and a
        // copy the player has switched on again since stays on.
        let brought = |id: &str| plan.actions.iter().any(|a| matches!(a, Action::Install { id: i } | Action::Update { id: i, .. } if i == id));
        for copy in state.begin_update(&self.manifest.build_version).own_copies.iter().filter(|c| brought(&c.id)) {
            if let Some(e) = modlist.entries.iter_mut().find(|e| e.name == copy.folder) {
                e.state = EntryState::Disabled;
            }
        }
        modlist.save(&modlist_path)?;
        // Out of the list now (`plan::is_players`): the folders of separators the build no longer has go too.
        let separators = self.manifest.separator_folders();
        for folder in state.separators.difference(&separators) {
            remove_separator(&mods_dir.join(folder))?;
        }
        state.separators = separators;

        state.build_version = Some(self.manifest.build_version.clone());
        state.save(&state_file)?;
        let _ = std::fs::remove_dir_all(&staging);
        Ok(())
    }

    /// The author's undo for one build mod (see [`crate::author`]): puts the
    /// installed build's package of `id` into `folder`, the mod's folder as it
    /// is now, over their edits or a Nexus update. Old releases stay on GitHub,
    /// so the package downloads even after a newer build went out. Only the
    /// files change: the folder keeps its name, its place and its switch in
    /// the list (a rename or a toggle is a change of its own, see
    /// [`crate::author::pending`]), and `overwrite/` stays as the author left it.
    /// `keep_settings`: settings files the author changed stay theirs, as in a repair.
    pub fn rollback(&self, id: &str, folder: &str, keep_settings: bool, on: &mut (dyn FnMut(Event) + Send)) -> Result<()> {
        let spec = self.spec(id)?;
        let state_file = state_path(self.inst);
        let mut state = State::load(&state_file)?;
        let installed = state.mods.get(id).ok_or_else(|| Error::Manifest(format!("mod {id:?} is not installed")))?.clone();
        let mods_dir = self.inst.mods_dir();
        let dir = mods_dir.join(folder);
        // The folder must be this mod's: by `[LYNO] id`, or the installed folder gone from the disk.
        let ours = match dir.is_dir() {
            true => crate::publish::mod_id(folder, &meta::ModMeta::load(&dir.join("meta.ini")).unwrap_or_default()) == id,
            false => folder == installed.folder,
        };
        if !ours || state.mods.iter().any(|(other, m)| other != id && m.folder == folder) {
            return Err(Error::Manifest(format!("folder {folder:?} is not mod {id:?}")));
        }

        let jobs = [Job::new(&spec.package, &spec.id, &cache_dir(self.inst))];
        let fetch = Prefetch::new(&jobs, spec.package.download_size(), self.cancel);
        let on = Mutex::new(on);
        let emit = |e: Event| (on.lock().unwrap())(e);
        let report = |r: Report| {
            emit(match r {
                Report::Bytes { done, total } => Event::Bytes { done, total },
                Report::Retry { attempt, delay, error } => Event::Retry { attempt, delay_secs: delay.as_secs(), error },
            })
        };
        emit(Event::Step { index: 1, total: 1, label: format!("Возврат опубликованной версии: {folder}") });
        report(fetch.bytes());
        let hashes = std::thread::scope(|scope| {
            for _ in 0..prefetch::PARALLEL_DOWNLOADS {
                scope.spawn(|| fetch.worker(self.downloader, &report));
            }
            let result = fetch.wait(0);
            fetch.finish();
            result
        })
        .and_then(|()| {
            let staging = self.inst.root().join(".lyno").join("staging").join(&spec.id);
            let paths = jobs[0].paths();
            let hashes = package::unpack(&paths, &staging, &spec.package.hash)?;
            for p in &paths {
                let _ = std::fs::remove_file(p);
            }
            if keep_settings {
                let record = files::load(self.inst, id, &installed.hash);
                self::keep_settings(&dir, &staging, &hashes, record.as_ref())?;
            }
            // A renamed folder is the mod by `[LYNO] id`, which the published `meta.ini` may lack.
            let meta_path = staging.join("meta.ini");
            let meta = if meta_path.is_file() { meta::ModMeta::load(&meta_path)? } else { meta::ModMeta::default() };
            if crate::publish::mod_id(folder, &meta) != id {
                meta::ModMeta { lyno_id: Some(id.to_owned()), ..Default::default() }.save(&meta_path)?;
            }
            package::swap_folder(&staging, &dir)?;
            Ok(hashes)
        })?;
        files::save(self.inst, &FileList::new(&spec.id, &spec.package.hash, &hashes))?;
        let restored = InstalledMod { folder: folder.to_owned(), hash: spec.package.hash.clone(), damaged: false, reset_settings: false };
        state.mods.insert(spec.id.clone(), restored);
        state.save(&state_file)?;
        emit(Event::Done);
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

/// An update or a repair replaces the whole mod folder, but the player's
/// settings inside it (see [`rules::is_settings`]) are what makes it theirs:
/// merges JSON/INI with the fresh package, preserving existing values and
/// adding newly shipped keys. Other formats are copied intact. A file still as
/// the build installed it (`record`, the old package's file list) is not the
/// player's, so the new package's version of it comes in. Without a record
/// every existing file counts as the player's. Only paths the package ships,
/// so a stray file doesn't come back.
fn keep_settings(old: &Path, staging: &Path, files: &[(FileEntry, String)], record: Option<&FileList>) -> Result<()> {
    for (f, _) in files.iter().filter(|(f, _)| rules::is_settings(&f.path)) {
        let from = tree::from_slash(old, &f.path);
        if !from.is_file() {
            continue;
        }
        if let Some(shipped) = record.and_then(|r| r.files.get(&f.path)) {
            if crate::hash::blake3_file(&from)? == shipped.blake3 {
                continue;
            }
        }
        let to = tree::from_slash(staging, &f.path);
        crate::settings_merge::merge(&from, &to)?;
    }
    Ok(())
}

/// MO2 gives `overwrite/` the highest priority. A file the player's game
/// created there before the build shipped it would hide the build's version
/// forever, so it is moved to `backup`. Settings receive new JSON/INI keys,
/// or move to the backup too when the player explicitly resets them.
fn move_shadowing_files(overwrite: &Path, mod_dir: &Path, backup: &Path, reset_settings: bool) -> Result<()> {
    if !overwrite.is_dir() {
        return Ok(());
    }
    for f in tree::list_files_with(mod_dir, &|p| rules::is_hashed(p) && !rules::is_generated(p))? {
        let shadow = tree::from_slash(overwrite, &f.path);
        if !shadow.is_file() {
            continue;
        }
        if rules::is_settings(&f.path) && !reset_settings {
            // Merge into a temporary file: the mod folder remains the author's
            // source of truth, while overwrite keeps the player's priority.
            let temp = shadow.with_file_name(format!(".lyno-merge-{}.tmp", uuid::Uuid::new_v4()));
            let result = (|| {
                let author = tree::from_slash(mod_dir, &f.path);
                let mine = std::fs::read(&shadow).map_err(|e| Error::io(&shadow, e))?;
                let theirs = std::fs::read(&author).map_err(|e| Error::io(&author, e))?;
                let merged = crate::settings_merge::merge_bytes(author.extension().and_then(|e| e.to_str()).unwrap_or(""), &mine, &theirs);
                if merged == mine {
                    return Ok(());
                }
                std::fs::write(&temp, merged).map_err(|e| Error::io(&temp, e))?;
                std::fs::rename(&temp, &shadow).map_err(|e| Error::io(&shadow, e))
            })();
            let _ = std::fs::remove_file(&temp);
            result?;
            continue;
        }
        let to = unused_backup_path(tree::from_slash(backup, &f.path));
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::rename(&shadow, &to).map_err(|e| Error::io(&shadow, e))?;
    }
    Ok(())
}

fn backup_settings(from: &Path, backup: &Path) -> Result<()> {
    if !from.is_dir() {
        return Ok(());
    }
    for f in tree::list_files_with(from, &|p| rules::is_settings(p) && !rules::is_generated(p))? {
        let dest = unused_backup_path(tree::from_slash(backup, &f.path));
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| Error::io(parent, e))?;
        }
        let source = tree::from_slash(from, &f.path);
        std::fs::copy(&source, &dest).map_err(|e| Error::io(&source, e))?;
    }
    Ok(())
}

fn unused_backup_path(path: std::path::PathBuf) -> std::path::PathBuf {
    let mut candidate = path.clone();
    let name = path.file_name().unwrap().to_string_lossy();
    let mut index = 1;
    while candidate.exists() {
        candidate = path.with_file_name(format!("{name}.{index}"));
        index += 1;
    }
    candidate
}

/// A separator is an empty folder with MO2's `meta.ini`; one holding anything
/// else (the player's files) stays, MO2 then shows it as a mod.
fn remove_separator(dir: &Path) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Error::io(dir, e)),
    };
    for entry in entries {
        let entry = entry.map_err(|e| Error::io(dir, e))?;
        if !entry.file_name().eq_ignore_ascii_case("meta.ini") {
            return Ok(());
        }
    }
    remove_dir(dir)
}

fn remove_dir(p: &Path) -> Result<()> {
    match std::fs::remove_dir_all(p) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(p, e)),
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    fn write(root: &Path, rel: &str, bytes: &[u8]) {
        let path = tree::from_slash(root, rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn customized_mod_receives_new_author_settings() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("old");
        let staging = dir.path().join("staging");
        write(&old, "config.json", br#"{"volume":1}"#);
        write(&old, "unchanged.json", br#"{"value":1}"#);
        let record = FileList::new("test", "old", &tree::hash_files_with(&old, &rules::is_hashed).unwrap());
        write(&old, "config.json", br#"{"volume":2}"#);
        write(&staging, "config.json", br#"{"volume":3,"newMod":{"enabled":true}}"#);
        write(&staging, "unchanged.json", br#"{"value":3}"#);
        let hashes = tree::hash_files_with(&staging, &rules::is_hashed).unwrap();
        keep_settings(&old, &staging, &hashes, Some(&record)).unwrap();
        let result: serde_json::Value = serde_json::from_slice(&std::fs::read(staging.join("config.json")).unwrap()).unwrap();
        assert_eq!(result, serde_json::json!({"volume":2,"newMod":{"enabled":true}}));
        assert_eq!(std::fs::read(staging.join("unchanged.json")).unwrap(), br#"{"value":3}"#);
    }

    #[test]
    fn overwrite_receives_new_sections_and_reset_restores_author_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let overwrite = dir.path().join("overwrite");
        let author = dir.path().join("mod");
        let backup = dir.path().join("backup");
        let rel = "red4ext/plugins/mod_settings/user.ini";
        write(&overwrite, rel, b"[OldMod]\nvalue=player\n");
        write(&overwrite, "unrelated.json", b"{}");
        write(&overwrite, "r6/cache/final.redscripts", b"cache");
        write(&author, rel, b"[OldMod]\nvalue=author\n[NewMod]\nenabled=true\n");
        move_shadowing_files(&overwrite, &author, &backup, false).unwrap();
        let merged = std::fs::read_to_string(tree::from_slash(&overwrite, rel)).unwrap();
        assert!(merged.contains("value=player") && merged.contains("[NewMod]"));
        assert!(std::fs::read_to_string(tree::from_slash(&author, rel)).unwrap().contains("value=author"));
        move_shadowing_files(&overwrite, &author, &backup, true).unwrap();
        assert!(!tree::from_slash(&overwrite, rel).exists());
        assert_eq!(std::fs::read_to_string(tree::from_slash(&backup, rel)).unwrap(), merged);
        assert!(overwrite.join("unrelated.json").exists());
        assert!(overwrite.join("r6/cache/final.redscripts").exists());
        // Another reset in the same build must retain the previous backup.
        write(&overwrite, rel, b"[OldMod]\nvalue=second\n");
        move_shadowing_files(&overwrite, &author, &backup, true).unwrap();
        assert_eq!(std::fs::read_to_string(tree::from_slash(&backup, rel)).unwrap(), merged);
        assert!(backup.join(format!("{rel}.1")).is_file());
    }
}

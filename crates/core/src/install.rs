//! Applies an [`UpdatePlan`] to an instance: download, unpack, swap.
//!
//! State is saved after every mod, so an interrupted update resumes where
//! it stopped (downloads resume too, see [`crate::download`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use serde::Serialize;

use crate::download::Downloader;
use crate::manifest::{Manifest, ModEntry, ModSpec, Package};
use crate::mo2::Instance;
use crate::modlist::Entry;
use crate::package;
use crate::rules;
use crate::plan::{Action, UpdatePlan};
use crate::state::{InstalledMod, State};
use crate::tree;
use crate::{Error, Result};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Event {
    /// Starting step `index` of `total`.
    Step { index: usize, total: usize, label: String },
    /// Overall download progress.
    Bytes { done: u64, total: u64 },
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

impl Installer<'_> {
    pub fn apply(&self, plan: &UpdatePlan, on: &mut dyn FnMut(Event)) -> Result<()> {
        let state_file = state_path(self.inst);
        let mut state = State::load(&state_file)?;
        let work = self.inst.root().join(".lyno");
        let cache = work.join("cache");
        let staging = work.join("staging");
        let backup = work.join("overwrite-backup").join(&self.manifest.build_version);
        let mods_dir = self.inst.mods_dir();
        std::fs::create_dir_all(&mods_dir).map_err(|e| Error::io(&mods_dir, e))?;

        state.begin_update(&self.manifest.build_version);

        let total_bytes = plan.download_size;
        let mut done_bytes = 0u64;
        let total = plan.actions.len() + 1;

        for (i, action) in plan.actions.iter().enumerate() {
            on(Event::Step { index: i + 1, total, label: self.label(action) });
            match action {
                Action::Base => {
                    let pkg = &self.manifest.base;
                    let dest = staging.join("base");
                    self.fetch_and_unpack(pkg, "base", &cache, &dest, &mut done_bytes, total_bytes, on)?;
                    let files: Vec<String> = tree::list_files(&dest)?.into_iter().map(|f| f.path).collect();
                    package::merge_into(&dest, self.inst.root())?;
                    remove_stale_base_files(self.inst.root(), &state.base_files, &files)?;
                    state.base_hash = Some(pkg.hash.clone());
                    state.base_files = files;
                }
                Action::Install { id } | Action::Update { id, .. } => {
                    let spec = self.spec(id)?;
                    let dest = staging.join(&spec.id);
                    self.fetch_and_unpack(&spec.package, &spec.id, &cache, &dest, &mut done_bytes, total_bytes, on)?;
                    package::swap_folder(&dest, &mods_dir.join(&spec.name))?;
                    move_shadowing_files(&self.inst.overwrite_dir(), &mods_dir.join(&spec.name), &backup)?;
                    if let Action::Update { from_folder, .. } = action {
                        if *from_folder != spec.name {
                            remove_dir(&mods_dir.join(from_folder))?;
                        }
                    }
                    state.mods.insert(spec.id.clone(), InstalledMod { folder: spec.name.clone(), hash: spec.package.hash.clone() });
                    let record = state.begin_update(&self.manifest.build_version);
                    if !record.added.contains(id) && !record.updated.contains(id) {
                        match action {
                            Action::Install { .. } => record.added.push(id.clone()),
                            _ => record.updated.push(id.clone()),
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
            if let ModEntry::Separator { title } = e {
                let dir = mods_dir.join(Entry::separator(title).name);
                std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
            }
        }
        let modlist_path = self.inst.modlist_path(&self.manifest.profile);
        if let Some(dir) = modlist_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        plan.modlist.save(&modlist_path)?;

        state.build_version = Some(self.manifest.build_version.clone());
        state.save(&state_file)?;
        let _ = std::fs::remove_dir_all(&cache);
        let _ = std::fs::remove_dir_all(&staging);
        on(Event::Done);
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn fetch_and_unpack(
        &self,
        pkg: &Package,
        name: &str,
        cache: &Path,
        dest: &Path,
        done: &mut u64,
        total: u64,
        on: &mut dyn FnMut(Event),
    ) -> Result<()> {
        let mut paths = Vec::with_capacity(pkg.parts.len());
        for (i, part) in pkg.parts.iter().enumerate() {
            let path = cache.join(format!("{name}-{}.{:03}", &pkg.hash[..pkg.hash.len().min(16)], i + 1));
            let start = *done;
            let mut got = 0u64;
            self.downloader.fetch_part(part, &path, self.cancel, &mut |n| {
                got += n;
                on(Event::Bytes { done: start + got, total });
            })?;
            *done = start + part.size;
            paths.push(path);
        }
        package::unpack(&paths, dest, &pkg.hash)?;
        for p in &paths {
            let _ = std::fs::remove_file(p);
        }
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
fn is_player_file(rel: &str) -> bool {
    let mut parts = rel.splitn(3, '/');
    matches!((parts.next(), parts.next(), parts.next()), (Some("profiles"), Some(_), Some(file)) if rules::is_private_profile_file(file))
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

//! The author's side of a launcher instance. The author plays in the same kind
//! of instance as players and builds releases from it, so between releases the
//! instance is ahead of the published build on purpose: mods added, removed,
//! edited or reordered. For a player these are damage or foreign mods; for the
//! author they are the next release. An update or a repair would undo them
//! ([`crate::plan`] takes unknown mods for the player's and downloads
//! edited ones again), so the launcher keeps both off in author mode.
//!
//! Content changes inside build mods come from [`crate::verify`], which already
//! compares every file with the record of the installed package. [`pending`]
//! adds what the file check can't see: the mod list itself.
//!
//! After a release is published, [`adopt`] records it as installed: the parts
//! were packed from these very folders, so there is nothing to download.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::Serialize;

use crate::files;
use crate::install::state_path;
use crate::manifest::{Manifest, ModEntry};
use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::publish::{base_files, mod_id, split_user_section};
use crate::state::{InstalledMod, LastUpdate, State};
use crate::{Error, Result};

/// The mod list of the instance against the installed build. Folder names.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    /// Build mods not in the installed build.
    pub added: Vec<String>,
    /// Installed build mods no longer in the build section (or deleted).
    pub removed: Vec<String>,
    /// `(old, new)`: same manifest id, new folder name.
    pub renamed: Vec<(String, String)>,
    /// Switched on or off: the build ships the author's state as the default.
    pub toggled: Vec<String>,
    /// Mods or separators of both lists are in another order, or separators
    /// were added or removed.
    pub reordered: bool,
    /// Under `LYNO USER MODS`: not shipped.
    pub personal: Vec<String>,
}

impl Pending {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.renamed.is_empty()
            && self.toggled.is_empty()
            && !self.reordered
    }
}

/// Compares the mod list of the build's profile with `installed`, the manifest
/// of the installed build. Reads `modlist.txt` and every `meta.ini`, no mod files.
pub fn pending(inst: &Instance, installed: &Manifest) -> Result<Pending> {
    let list = ModList::load(&inst.modlist_path(&installed.profile))?;
    let metas = inst.scan_mods()?;
    let (build, personal) = split_user_section(&list);
    let managed = |e: &&crate::modlist::Entry| e.state != EntryState::Unmanaged;
    let is_mod = |e: &&crate::modlist::Entry| managed(e) && e.separator_title().is_none();

    let was: BTreeMap<&str, &crate::manifest::ModSpec> = installed.mod_specs().map(|m| (m.id.as_str(), m)).collect();
    let mut now = BTreeMap::new();
    let mut pending = Pending { personal: personal.iter().filter(is_mod).map(|e| e.name.clone()).collect(), ..Default::default() };
    for entry in build.iter().filter(is_mod) {
        let id = mod_id(&entry.name, &metas.get(&entry.name).cloned().unwrap_or_default());
        match was.get(id.as_str()) {
            None => pending.added.push(entry.name.clone()),
            Some(spec) => {
                if spec.name != entry.name {
                    pending.renamed.push((spec.name.clone(), entry.name.clone()));
                }
                if spec.enabled != (entry.state == EntryState::Enabled) {
                    pending.toggled.push(entry.name.clone());
                }
            }
        }
        now.insert(id, entry.name.clone());
    }
    pending.removed = installed.mod_specs().filter(|m| !now.contains_key(&m.id)).map(|m| m.name.clone()).collect();

    // Order of what both lists have, by id, with separators by title.
    let ids: HashSet<&str> = now.keys().map(String::as_str).filter(|id| was.contains_key(id)).collect();
    let before: Vec<String> = installed
        .mods
        .iter()
        .filter_map(|e| match e {
            ModEntry::Separator { title, .. } => Some(format!("{title}\0")),
            ModEntry::Mod(m) => ids.contains(m.id.as_str()).then(|| m.id.clone()),
        })
        .collect();
    let after: Vec<String> = build
        .iter()
        .filter(managed)
        .filter_map(|e| match e.separator_title() {
            Some(title) => Some(format!("{title}\0")),
            None => {
                let id = mod_id(&e.name, &metas.get(&e.name).cloned().unwrap_or_default());
                ids.contains(id.as_str()).then_some(id)
            }
        })
        .collect();
    pending.reordered = before != after;
    Ok(pending)
}

/// Records `manifest`, just built from this instance and published, as the
/// installed build without downloading it. `out_dir` is the build's output,
/// left out of the base package like in `build`; `None` when it is unknown
/// (the release was published with `lyno-pack`), which only matters if it
/// sits inside the instance.
///
/// If a mod was edited between packing and publishing, the recorded hash no
/// longer matches its folder and the integrity check reports it as changed:
/// a pending change for the next release, which is what it is.
pub fn adopt(inst: &Instance, manifest: &Manifest, out_dir: Option<&Path>) -> Result<State> {
    for spec in manifest.mod_specs() {
        if !inst.mods_dir().join(&spec.name).is_dir() {
            return Err(Error::Manifest(format!(
                "build {} has mod {:?}, this instance doesn't: it was built from another instance",
                manifest.build_version, spec.name
            )));
        }
    }
    let path = state_path(inst);
    let old = State::load(&path)?;
    let mods: BTreeMap<String, InstalledMod> = manifest
        .mod_specs()
        .map(|m| {
            let installed = InstalledMod { folder: m.name.clone(), hash: m.package.hash.clone(), damaged: false, reset_settings: false };
            (m.id.clone(), installed)
        })
        .collect();
    for id in old.mods.keys().filter(|id| !mods.contains_key(*id)) {
        files::remove(inst, id)?;
    }
    // Records of edited mods name the old package and are ignored from now on;
    // the next integrity check records them again from the folders.
    let record = LastUpdate {
        from: old.build_version.clone(),
        to: manifest.build_version.clone(),
        added: mods.keys().filter(|id| !old.mods.contains_key(*id)).cloned().collect(),
        updated: mods.iter().filter(|(id, m)| old.mods.get(*id).is_some_and(|o| o.hash != m.hash)).map(|(id, _)| id.clone()).collect(),
        removed: old.mods.iter().filter(|(id, _)| !mods.contains_key(*id)).map(|(_, m)| m.folder.clone()).collect(),
    };
    let state = State {
        build_version: Some(manifest.build_version.clone()),
        base_hash: Some(manifest.base.hash.clone()),
        base_files: base_files(inst.root(), &manifest.profile, out_dir)?.into_iter().map(|f| f.path).collect(),
        base_damaged: false,
        mods,
        removed: Default::default(),
        last_update: Some(record),
    };
    state.save(&path)?;
    Ok(state)
}

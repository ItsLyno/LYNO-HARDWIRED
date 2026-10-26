//! The author's side of a launcher instance. The author plays in the same kind
//! of instance as players and builds releases from it, so between releases the
//! instance is ahead of the published build on purpose: mods added, removed,
//! edited or reordered. For a player these are damage or foreign mods; for the
//! author they are the next release. An update or a repair would undo them
//! ([`crate::plan`] takes unknown mods for the player's and downloads
//! edited ones again), so the launcher keeps both off in author mode. The
//! author takes back edits of one mod on purpose, with
//! [`crate::install::Installer::rollback`]: files only, the list stays theirs.
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataRow {
    pub folder: String,
    pub metadata: crate::meta::AuthorMetadata,
}

/// Only actual mods in the build section are dependency candidates.
pub fn metadata(inst: &Instance, profile: &str, published: Option<&Manifest>) -> Result<Vec<MetadataRow>> {
    let list = ModList::load(&inst.modlist_path(profile))?;
    let metas = inst.scan_mods()?;
    let mut core_section = false;
    let mut out = Vec::new();
    for e in split_user_section(&list).0 {
        let meta = metas.get(&e.name).cloned().unwrap_or_default();
        if e.is_separator() { core_section = meta.lyno_core; continue; }
        if e.state == EntryState::Unmanaged { continue; }
        let id = mod_id(&e.name, &meta);
        let previous = published.and_then(|m| m.mod_specs().find(|m| m.id == id));
        out.push(MetadataRow {
            folder: e.name.clone(),
            metadata: crate::meta::AuthorMetadata {
                id,
                title: meta.lyno_title.clone().or_else(|| previous.and_then(|m| m.title.clone())).unwrap_or_default(),
                author: meta.lyno_author.clone().or_else(|| previous.and_then(|m| m.author.clone())).unwrap_or_default(),
                description: meta.lyno_description.clone().unwrap_or_default(),
                core: !meta.lyno_optional && (meta.lyno_core || core_section),
                requires: meta.lyno_requires.clone(),
            },
        });
    }
    Ok(out)
}

pub fn save_metadata(inst: &Instance, profile: &str, folder: &str, value: &crate::meta::AuthorMetadata) -> Result<()> {
    // Resolve through the existing list rather than joining an arbitrary UI path.
    if folder.contains(['/', '\\', ':']) || folder == "." || folder == ".." {
        return Err(Error::Manifest("Недопустимая папка мода".into()));
    }
    let mut rows = metadata(inst, profile, None)?;
    let row = rows.iter_mut().find(|r| r.folder == folder).ok_or_else(|| Error::Manifest("Мод не входит в сборку".into()))?;
    if value.id != row.metadata.id {
        return Err(Error::Manifest("Постоянный ID нельзя менять: на него ссылаются другие моды и установленные сборки".into()));
    }
    row.metadata = value.clone();
    crate::manifest::validate_dependencies(rows.iter().map(|r| (r.metadata.id.as_str(), r.metadata.requires.as_slice())))?;
    crate::meta::save_author(&inst.mods_dir().join(folder).join("meta.ini"), value)
}

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
    pub metadata: Vec<String>,
    /// Mods or separators of both lists are in another order, or separators
    /// were added or removed.
    pub reordered: bool,
    /// Titles of separators the installed build has under another title or
    /// color, or not at all: MO2 knows a separator by its title only, so a
    /// renamed one is a new one.
    pub separators: Vec<String>,
    /// Under `LYNO USER MODS`: not shipped.
    pub personal: Vec<String>,
}

impl Pending {
    pub fn is_empty(&self) -> bool {
        self.added.is_empty()
            && self.removed.is_empty()
            && self.renamed.is_empty()
            && self.toggled.is_empty()
            && self.metadata.is_empty()
            && !self.reordered
            && self.separators.is_empty()
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
    let metadata_rows = metadata(inst, &installed.profile, Some(installed))?;
    let mut pending = Pending { personal: personal.iter().filter(is_mod).map(|e| e.name.clone()).collect(), ..Default::default() };
    for entry in build.iter().filter(is_mod) {
        let id = mod_id(&entry.name, &metas.get(&entry.name).cloned().unwrap_or_default());
        match was.get(id.as_str()) {
            None => pending.added.push(entry.name.clone()),
            Some(spec) => {
                if let Some(row) = metadata_rows.iter().find(|r| r.folder == entry.name) {
                    let value = &row.metadata;
                    if value.core == spec.optional || value.requires != spec.requires
                        || value.description != spec.description.clone().unwrap_or_default()
                        || value.title != spec.title.clone().unwrap_or_default()
                        || value.author != spec.author.clone().unwrap_or_default() {
                        pending.metadata.push(entry.name.clone());
                    }
                }
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

    let was_separators: HashSet<(&str, Option<&str>)> = installed
        .mods
        .iter()
        .filter_map(|e| match e {
            ModEntry::Separator { title, color } => Some((title.as_str(), color.as_deref())),
            ModEntry::Mod(_) => None,
        })
        .collect();
    pending.separators = build
        .iter()
        .filter(managed)
        .filter_map(|e| e.separator_title().map(|title| (title, metas.get(&e.name).and_then(|m| m.color.as_deref()))))
        .filter(|(title, color)| !was_separators.contains(&(*title, *color)))
        .map(|(title, _)| title.to_owned())
        .collect();

    // Order of what both lists have, by id; separators are places in it, their titles are `separators`.
    let ids: HashSet<&str> = now.keys().map(String::as_str).filter(|id| was.contains_key(id)).collect();
    let before: Vec<String> = installed
        .mods
        .iter()
        .filter_map(|e| match e {
            ModEntry::Separator { .. } => Some("\0".to_owned()),
            ModEntry::Mod(m) => ids.contains(m.id.as_str()).then(|| m.id.clone()),
        })
        .collect();
    let after: Vec<String> = build
        .iter()
        .filter(managed)
        .filter_map(|e| match e.separator_title() {
            Some(_) => Some("\0".to_owned()),
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
        own_copies: Vec::new(),
    };
    let state = State {
        build_version: Some(manifest.build_version.clone()),
        base_hash: Some(manifest.base.hash.clone()),
        base_files: base_files(inst.root(), &manifest.profile, out_dir)?.into_iter().map(|f| f.path).collect(),
        base_damaged: false,
        mods,
        removed: Default::default(),
        separators: manifest.separator_folders(),
        last_update: Some(record),
    };
    state.save(&path)?;
    Ok(state)
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use crate::meta::ModMeta;

    fn fixture() -> (tempfile::TempDir, Instance) {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        std::fs::create_dir_all(inst.profile_dir("LYNO")).unwrap();
        for name in ["Core_separator", "Library", "Gameplay", "Other", "LYNO USER MODS_separator", "Private"] {
            std::fs::create_dir_all(inst.mods_dir().join(name)).unwrap();
        }
        std::fs::write(inst.mods_dir().join("Core_separator/meta.ini"), "[LYNO]\ncore=true\n").unwrap();
        std::fs::write(inst.mods_dir().join("Gameplay/meta.ini"), "[General]\nmodid=42\ncustom=keep\n[LYNO]\nfomod=saved\n").unwrap();
        ModList { entries: vec![
            crate::modlist::Entry::separator("Core"), crate::modlist::Entry::enabled("Library"),
            crate::modlist::Entry::enabled("Gameplay"), crate::modlist::Entry::enabled("Other"),
            crate::modlist::Entry::separator("LYNO USER MODS"), crate::modlist::Entry::enabled("Private"),
        ] }.save(&inst.modlist_path("LYNO")).unwrap();
        (dir, inst)
    }

    #[test]
    fn editor_preserves_mo2_and_overrides_inherited_core() {
        let (_dir, inst) = fixture();
        let rows = metadata(&inst, "LYNO", None).unwrap();
        assert_eq!(rows.len(), 3, "no separators or personal mods offered as dependencies");
        let mut value = rows.iter().find(|r| r.folder == "Gameplay").unwrap().metadata.clone();
        assert!(value.core);
        value.core = false;
        value.title = "Геймплей".into();
        value.description = "Только нужные зависимости".into();
        value.requires = vec!["library".into()];
        save_metadata(&inst, "LYNO", "Gameplay", &value).unwrap();
        let meta = ModMeta::load(&inst.mods_dir().join("Gameplay/meta.ini")).unwrap();
        assert!(meta.lyno_optional);
        assert!(!meta.lyno_core);
        assert_eq!(meta.lyno_title.as_deref(), Some("Геймплей"));
        assert_eq!(meta.lyno_requires, ["library"]);
        assert_eq!(meta.mod_id, Some(42));
        assert_eq!(meta.lyno_fomod.as_deref(), Some("saved"));
        assert!(std::fs::read_to_string(inst.mods_dir().join("Gameplay/meta.ini")).unwrap().contains("custom=keep"));
        assert!(!metadata(&inst, "LYNO", None).unwrap().iter().find(|r| r.folder == "Gameplay").unwrap().metadata.core);
    }

    #[test]
    fn editor_rejects_invalid_graph_and_ini_injection_without_writing() {
        let (_dir, inst) = fixture();
        let original = std::fs::read(inst.mods_dir().join("Gameplay/meta.ini")).unwrap();
        let mut value = metadata(&inst, "LYNO", None).unwrap().into_iter().find(|r| r.folder == "Gameplay").unwrap().metadata;
        for deps in [vec!["missing".into()], vec!["private".into()], vec!["gameplay".into()], vec!["library".into(), "library".into()]] {
            value.requires = deps;
            assert!(save_metadata(&inst, "LYNO", "Gameplay", &value).is_err());
        }
        value.requires.clear();
        value.title = "title\n[LYNO]\ncore=true".into();
        assert!(save_metadata(&inst, "LYNO", "Gameplay", &value).is_err());
        assert_eq!(std::fs::read(inst.mods_dir().join("Gameplay/meta.ini")).unwrap(), original);
        value.title.clear();
        value.id = "changed".into();
        assert!(save_metadata(&inst, "LYNO", "Gameplay", &value).is_err());
        assert!(save_metadata(&inst, "LYNO", "../outside", &value).is_err());
    }

    #[test]
    fn editor_rejects_transitive_cycles() {
        let (_dir, inst) = fixture();
        let mut rows = metadata(&inst, "LYNO", None).unwrap();
        let value = &mut rows.iter_mut().find(|r| r.folder == "Gameplay").unwrap().metadata;
        value.requires = vec!["library".into()];
        save_metadata(&inst, "LYNO", "Gameplay", value).unwrap();
        let value = &mut rows.iter_mut().find(|r| r.folder == "Library").unwrap().metadata;
        value.requires = vec!["gameplay".into()];
        assert!(save_metadata(&inst, "LYNO", "Library", value).is_err());
    }
}

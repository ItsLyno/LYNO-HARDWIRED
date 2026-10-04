//! Turns "what the manifest wants" + "what is in the instance" into a list
//! of concrete actions and the resulting `modlist.txt`.

use std::collections::{BTreeMap, HashSet};

use serde::Serialize;

use crate::manifest::{Manifest, ModEntry, ModSpec};
use crate::meta::ModMeta;
use crate::modlist::{Entry, EntryState, ModList};

/// Separator placed above mods the user added by hand; they always stay
/// at the bottom (highest priority) and are never touched by updates.
pub const USER_SEPARATOR: &str = "LYNO USER MODS";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Action {
    /// Download and install a mod that isn't present.
    Install { id: String, name: String },
    /// Replace an installed mod with a different Nexus file.
    Update { id: String, from_folder: String, name: String, from_file_id: Option<u64>, to_file_id: u64 },
    /// Same file, new folder name.
    Rename { id: String, from_folder: String, name: String },
    /// Managed mod no longer in the build.
    Remove { folder: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdatePlan {
    pub actions: Vec<Action>,
    #[serde(skip)]
    pub modlist: ModList,
    /// True when only the load order / enabled flags change.
    pub modlist_changed: bool,
}

impl UpdatePlan {
    pub fn is_up_to_date(&self) -> bool {
        self.actions.is_empty() && !self.modlist_changed
    }

    pub fn downloads(&self) -> impl Iterator<Item = &str> {
        self.actions.iter().filter_map(|a| match a {
            Action::Install { id, .. } | Action::Update { id, .. } => Some(id.as_str()),
            _ => None,
        })
    }
}

pub fn plan(manifest: &Manifest, installed: &BTreeMap<String, ModMeta>, current: &ModList) -> UpdatePlan {
    let by_lyno_id: BTreeMap<&str, (&str, &ModMeta)> = installed
        .iter()
        .filter_map(|(folder, meta)| meta.lyno_id.as_deref().map(|id| (id, (folder.as_str(), meta))))
        .collect();

    let mut actions = Vec::new();
    for spec in manifest.mod_specs() {
        actions.extend(action_for(spec, by_lyno_id.get(spec.id.as_str()).copied()));
    }

    let wanted: HashSet<&str> = manifest.mod_specs().map(|m| m.id.as_str()).collect();
    for (id, (folder, _)) in &by_lyno_id {
        if !wanted.contains(id) {
            actions.push(Action::Remove { folder: (*folder).to_owned() });
        }
    }

    let modlist = target_modlist(manifest, installed, current);
    let modlist_changed = modlist != *current;
    UpdatePlan { actions, modlist, modlist_changed }
}

fn action_for(spec: &ModSpec, installed: Option<(&str, &ModMeta)>) -> Option<Action> {
    let Some((folder, meta)) = installed else {
        return Some(Action::Install { id: spec.id.clone(), name: spec.name.clone() });
    };
    if meta.file_id != Some(spec.nexus.file_id) {
        return Some(Action::Update {
            id: spec.id.clone(),
            from_folder: folder.to_owned(),
            name: spec.name.clone(),
            from_file_id: meta.file_id,
            to_file_id: spec.nexus.file_id,
        });
    }
    (folder != spec.name).then(|| Action::Rename {
        id: spec.id.clone(),
        from_folder: folder.to_owned(),
        name: spec.name.clone(),
    })
}

fn target_modlist(manifest: &Manifest, installed: &BTreeMap<String, ModMeta>, current: &ModList) -> ModList {
    let managed: HashSet<&str> = installed
        .iter()
        .filter(|(_, m)| m.is_managed())
        .map(|(f, _)| f.as_str())
        .collect();
    let build_separators: HashSet<String> = manifest
        .mods
        .iter()
        .filter_map(|e| match e {
            ModEntry::Separator { title } => Some(Entry::separator(title).name),
            ModEntry::Mod(_) => None,
        })
        .collect();
    let user_sep = Entry::separator(USER_SEPARATOR);

    let mut entries: Vec<Entry> = current
        .entries
        .iter()
        .filter(|e| e.state == EntryState::Unmanaged)
        .cloned()
        .collect();

    entries.extend(manifest.mods.iter().map(|e| match e {
        ModEntry::Separator { title } => Entry::separator(title),
        ModEntry::Mod(m) if m.enabled => Entry::enabled(&m.name),
        ModEntry::Mod(m) => Entry::disabled(&m.name),
    }));

    let user: Vec<Entry> = current
        .entries
        .iter()
        .filter(|e| {
            e.state != EntryState::Unmanaged
                && !managed.contains(e.name.as_str())
                && !build_separators.contains(&e.name)
                && e.name != user_sep.name
        })
        .cloned()
        .collect();
    if !user.is_empty() {
        entries.push(user_sep);
        entries.extend(user);
    }

    ModList { entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{NexusSource, SCHEMA_VERSION};

    fn spec(id: &str, file_id: u64) -> ModSpec {
        ModSpec {
            id: id.into(),
            name: format!("{id} folder"),
            enabled: true,
            version: None,
            author: None,
            nexus: NexusSource {
                game: "cyberpunk2077".into(),
                mod_id: 1,
                file_id,
                file_name: String::new(),
                size: 0,
                md5: None,
            },
            recipe: vec![],
        }
    }

    fn manifest(mods: Vec<ModEntry>) -> Manifest {
        Manifest {
            schema: SCHEMA_VERSION,
            name: "LYNO".into(),
            build_version: "1".into(),
            game_version: "2.21".into(),
            mo2_version: "2.5.3".into(),
            profile: "LYNO".into(),
            changelog: vec![],
            mods,
            own_files: vec![],
        }
    }

    fn managed(id: &str, file_id: u64) -> ModMeta {
        ModMeta { lyno_id: Some(id.into()), file_id: Some(file_id), ..Default::default() }
    }

    #[test]
    fn fresh_install_installs_everything() {
        let m = manifest(vec![
            ModEntry::Separator { title: "Core".into() },
            ModEntry::Mod(spec("cet", 10)),
            ModEntry::Mod(spec("r4x", 20)),
        ]);
        let p = plan(&m, &BTreeMap::new(), &ModList::default());
        assert_eq!(p.downloads().collect::<Vec<_>>(), ["cet", "r4x"]);
        let names: Vec<_> = p.modlist.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Core_separator", "cet folder", "r4x folder"]);
        assert!(p.modlist_changed);
    }

    #[test]
    fn diff_update_rename_remove_and_keep_user_mods() {
        let m = manifest(vec![
            ModEntry::Mod(spec("cet", 11)),
            ModEntry::Mod(spec("r4x", 20)),
            ModEntry::Mod(spec("keep", 30)),
        ]);
        let installed = BTreeMap::from([
            ("cet folder".to_owned(), managed("cet", 10)),
            ("old r4x".to_owned(), managed("r4x", 20)),
            ("keep folder".to_owned(), managed("keep", 30)),
            ("gone".to_owned(), managed("gone", 1)),
            ("My Tweak".to_owned(), ModMeta::default()),
        ]);
        let current = ModList {
            entries: vec![
                Entry { name: "DLC: EP1".into(), state: EntryState::Unmanaged },
                Entry::enabled("cet folder"),
                Entry::enabled("old r4x"),
                Entry::enabled("gone"),
                Entry::enabled("keep folder"),
                Entry::disabled("My Tweak"),
            ],
        };

        let p = plan(&m, &installed, &current);
        assert_eq!(
            p.actions,
            [
                Action::Update {
                    id: "cet".into(),
                    from_folder: "cet folder".into(),
                    name: "cet folder".into(),
                    from_file_id: Some(10),
                    to_file_id: 11
                },
                Action::Rename { id: "r4x".into(), from_folder: "old r4x".into(), name: "r4x folder".into() },
                Action::Remove { folder: "gone".into() },
            ]
        );
        let names: Vec<_> = p.modlist.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["DLC: EP1", "cet folder", "r4x folder", "keep folder", "LYNO USER MODS_separator", "My Tweak"]
        );
        assert_eq!(p.modlist.get("My Tweak").unwrap().state, EntryState::Disabled);
    }

    #[test]
    fn up_to_date_is_stable() {
        let m = manifest(vec![ModEntry::Mod(spec("cet", 10))]);
        let installed = BTreeMap::from([("cet folder".to_owned(), managed("cet", 10))]);
        let first = plan(&m, &installed, &ModList::default());
        let second = plan(&m, &installed, &first.modlist);
        assert!(second.is_up_to_date(), "{second:?}");
    }
}

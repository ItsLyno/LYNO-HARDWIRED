//! Turns "what the manifest wants" + "what is installed" into concrete
//! actions and the resulting `modlist.txt`.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::manifest::{Manifest, ModEntry, ModSpec};
use crate::modlist::{Entry, EntryState, ModList};
use crate::state::State;
use crate::{Error, Result};

/// Separator of the player's section at the bottom (highest priority): where
/// their new mods and their separators go. Their mods may also sit among the
/// build's (see `target_modlist`); updates never touch them either way.
pub const USER_SEPARATOR: &str = "LYNO USER MODS";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "action", rename_all = "camelCase")]
pub enum Action {
    /// Download and unpack the base package (MO2 + instance config).
    Base,
    /// Download and install a mod that isn't present.
    Install { id: String },
    /// Content changed: download the new package and replace the folder.
    Update { id: String, from_folder: String },
    /// Same package, but the installed folder is damaged (see
    /// [`crate::verify`]): download it again and replace the folder.
    Repair { id: String, from_folder: String },
    /// Same content, new folder name.
    Rename { id: String, from_folder: String },
    /// Managed mod no longer in the build.
    Remove { id: String, folder: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub actions: Vec<Action>,
    pub modlist: ModList,
    pub modlist_changed: bool,
    /// Bytes to download.
    pub download_size: u64,
}

impl UpdatePlan {
    pub fn is_up_to_date(&self) -> bool {
        self.actions.is_empty() && !self.modlist_changed
    }
}

pub fn plan(manifest: &Manifest, state: &State, current: &ModList) -> UpdatePlan {
    let mut actions = Vec::new();
    let mut download_size = 0;

    if state.base_hash.as_deref() != Some(manifest.base.hash.as_str()) || state.base_damaged {
        actions.push(Action::Base);
        download_size += manifest.base.download_size();
    }

    for spec in manifest.mod_specs().filter(|m| !is_removed(m, state)) {
        if let Some(a) = action_for(spec, state) {
            if matches!(a, Action::Install { .. } | Action::Update { .. } | Action::Repair { .. }) {
                download_size += spec.package.download_size();
            }
            actions.push(a);
        }
    }

    let wanted: HashSet<&str> = manifest.mod_specs().map(|m| m.id.as_str()).collect();
    for (id, installed) in &state.mods {
        if !wanted.contains(id.as_str()) {
            actions.push(Action::Remove { id: id.clone(), folder: installed.folder.clone() });
        }
    }

    let modlist = target_modlist(manifest, state, current);
    let modlist_changed = modlist != *current;
    UpdatePlan { actions, modlist, modlist_changed, download_size }
}

fn action_for(spec: &ModSpec, state: &State) -> Option<Action> {
    let Some(installed) = state.mods.get(&spec.id) else {
        return Some(Action::Install { id: spec.id.clone() });
    };
    if installed.hash != spec.package.hash {
        return Some(Action::Update { id: spec.id.clone(), from_folder: installed.folder.clone() });
    }
    if installed.damaged {
        return Some(Action::Repair { id: spec.id.clone(), from_folder: installed.folder.clone() });
    }
    (installed.folder != spec.name).then(|| Action::Rename { id: spec.id.clone(), from_folder: installed.folder.clone() })
}

/// The player removed this optional build mod: no update brings it back. One
/// the author has made core since comes back: the build needs it.
pub fn is_removed(spec: &ModSpec, state: &State) -> bool {
    spec.optional && state.removed.contains(&spec.id)
}

/// Whether a build mod ends up enabled. An installed optional mod keeps its
/// current state in `modlist.txt`, whether the player set it in the launcher
/// or in MO2. Core mods follow the manifest, so a framework switched off by
/// accident comes back with the next update.
pub fn is_enabled(spec: &ModSpec, state: &State, current: &ModList) -> bool {
    let chosen = || {
        let folder = &state.mods.get(&spec.id)?.folder;
        Some(current.get(folder)?.state == EntryState::Enabled)
    };
    spec.optional.then(chosen).flatten().unwrap_or(spec.enabled)
}

/// The player's switch for an optional build mod. Only edits `list`: the
/// mod stays installed, and [`plan`] keeps the choice across updates.
pub fn set_enabled(manifest: &Manifest, state: &State, list: &mut ModList, id: &str, enabled: bool) -> Result<()> {
    let spec = manifest
        .mod_specs()
        .find(|m| m.id == id)
        .ok_or_else(|| Error::Manifest(format!("mod {id:?} is not in the manifest")))?;
    if !spec.optional {
        return Err(Error::Manifest(format!("mod {id:?} is not optional")));
    }
    let folder = &state.mods.get(id).ok_or_else(|| Error::Manifest(format!("mod {id:?} is not installed")))?.folder;
    let entry = list
        .entries
        .iter_mut()
        .find(|e| e.name == *folder)
        .ok_or_else(|| Error::Manifest(format!("mod folder {folder:?} is not in modlist.txt")))?;
    entry.state = if enabled { EntryState::Enabled } else { EntryState::Disabled };
    Ok(())
}

/// A player's entry of the list: neither a build mod (installed or coming
/// with this manifest) nor a build separator, nor one MO2 doesn't manage.
pub fn is_players(manifest: &Manifest, state: &State, e: &Entry) -> bool {
    e.state != EntryState::Unmanaged
        && !state.is_managed_folder(&e.name)
        && !manifest.mods.iter().any(|m| match m {
            ModEntry::Separator { title, .. } => Entry::separator(title).name == e.name,
            ModEntry::Mod(m) => m.name == e.name && !is_removed(m, state),
        })
}

/// Build entries follow the manifest; the player's keep their place. A
/// player's mod in the build section sticks to the build entry right above
/// it (a mod by id, so a renamed folder keeps it), or to the nearest one above
/// that the build kept; above every build entry it stays at the top. What is
/// under `LYNO USER MODS` stays at the bottom, and so do separators of the
/// player found in the build section: MO2 would put build mods under them.
fn target_modlist(manifest: &Manifest, state: &State, current: &ModList) -> ModList {
    let folder_of: HashMap<&str, &str> = manifest.mod_specs().map(|m| (m.id.as_str(), m.name.as_str())).collect();
    let user_sep = Entry::separator(USER_SEPARATOR);
    let tail_at = current.entries.iter().position(|e| e.name == user_sep.name).unwrap_or(current.entries.len());
    let (head, tail) = current.entries.split_at(tail_at);

    let mut stuck: HashMap<String, Vec<Entry>> = HashMap::new();
    let mut user = Vec::new();
    let mut anchor = String::new();
    for e in head {
        if is_players(manifest, state, e) {
            match e.is_separator() {
                true => user.push(e.clone()),
                false => stuck.entry(anchor.clone()).or_default().push(e.clone()),
            }
        } else if let Some((id, _)) = state.mods.iter().find(|(_, m)| m.folder == e.name) {
            // A build mod the manifest dropped goes away: its player's mods move up to the anchor before it.
            if let Some(folder) = folder_of.get(id.as_str()) {
                anchor = folder.to_string();
            }
        } else if e.state != EntryState::Unmanaged {
            anchor = e.name.clone();
        }
    }
    user.extend(tail.iter().filter(|e| is_players(manifest, state, e) && e.name != user_sep.name).cloned());

    let mut entries: Vec<Entry> = current.entries.iter().filter(|e| e.state == EntryState::Unmanaged).cloned().collect();
    entries.extend(stuck.remove("").unwrap_or_default());
    for e in &manifest.mods {
        let entry = match e {
            ModEntry::Separator { title, .. } => Some(Entry::separator(title)),
            ModEntry::Mod(m) if is_removed(m, state) => None,
            ModEntry::Mod(m) if is_enabled(m, state, current) => Some(Entry::enabled(&m.name)),
            ModEntry::Mod(m) => Some(Entry::disabled(&m.name)),
        };
        if let Some(entry) = entry {
            let mine = stuck.remove(&entry.name).unwrap_or_default();
            entries.push(entry);
            entries.extend(mine);
        }
    }
    if !user.is_empty() {
        entries.push(user_sep);
        entries.extend(user);
    }

    ModList { entries }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::{manifest, spec};
    use crate::state::InstalledMod;

    fn state(base: Option<&str>, mods: &[(&str, &str, &str)]) -> State {
        State {
            build_version: None,
            base_hash: base.map(Into::into),
            base_files: vec![],
            base_damaged: false,
            last_update: None,
            removed: Default::default(),
            mods: mods
                .iter()
                .map(|(id, folder, hash)| (id.to_string(), InstalledMod { folder: folder.to_string(), hash: hash.to_string(), damaged: false, reset_settings: false }))
                .collect(),
        }
    }

    #[test]
    fn fresh_install_installs_everything() {
        let m = manifest(vec![
            ModEntry::Separator { title: "Core".into(), color: None },
            ModEntry::Mod(spec("cet", "h1")),
            ModEntry::Mod(spec("r4x", "h2")),
        ]);
        let p = plan(&m, &State::default(), &ModList::default());
        assert_eq!(
            p.actions,
            [Action::Base, Action::Install { id: "cet".into() }, Action::Install { id: "r4x".into() }]
        );
        assert_eq!(p.download_size, 15);
        let names: Vec<_> = p.modlist.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["Core_separator", "cet folder", "r4x folder"]);
    }

    #[test]
    fn diff_update_rename_remove_and_keep_user_mods() {
        let m = manifest(vec![
            ModEntry::Mod(spec("cet", "new")),
            ModEntry::Mod(spec("r4x", "same")),
            ModEntry::Mod(spec("keep", "k")),
        ]);
        let st = state(
            Some("base"),
            &[("cet", "cet folder", "old"), ("r4x", "old r4x", "same"), ("keep", "keep folder", "k"), ("gone", "gone", "g")],
        );
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

        let p = plan(&m, &st, &current);
        assert_eq!(
            p.actions,
            [
                Action::Update { id: "cet".into(), from_folder: "cet folder".into() },
                Action::Rename { id: "r4x".into(), from_folder: "old r4x".into() },
                Action::Remove { id: "gone".into(), folder: "gone".into() },
            ]
        );
        assert_eq!(p.download_size, 5);
        let names: Vec<_> = p.modlist.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["DLC: EP1", "cet folder", "r4x folder", "keep folder", "My Tweak"], "the player's mod keeps its place");
        assert_eq!(p.modlist.get("My Tweak").unwrap().state, EntryState::Disabled);
    }

    #[test]
    fn up_to_date_is_stable() {
        let m = manifest(vec![ModEntry::Mod(spec("cet", "h"))]);
        let st = state(Some("base"), &[("cet", "cet folder", "h")]);
        let first = plan(&m, &st, &ModList::default());
        let second = plan(&m, &st, &first.modlist);
        assert!(second.is_up_to_date(), "{second:?}");
    }

    #[test]
    fn damaged_mods_and_base_are_downloaded_again() {
        let m = manifest(vec![ModEntry::Mod(spec("cet", "h")), ModEntry::Mod(spec("r4x", "new")), ModEntry::Mod(spec("ok", "o"))]);
        let mut st = state(Some("base"), &[("cet", "cet folder", "h"), ("r4x", "r4x folder", "old"), ("ok", "ok folder", "o")]);
        st.base_damaged = true;
        st.mods.get_mut("cet").unwrap().damaged = true;
        st.mods.get_mut("r4x").unwrap().damaged = true;

        let p = plan(&m, &st, &ModList::default());
        assert_eq!(
            p.actions,
            [
                Action::Base,
                Action::Repair { id: "cet".into(), from_folder: "cet folder".into() },
                Action::Update { id: "r4x".into(), from_folder: "r4x folder".into() },
            ],
            "a damaged mod with a newer package is just updated"
        );
        assert_eq!(p.download_size, 15);
    }

    #[test]
    fn players_mods_stick_to_the_build_entry_above_them() {
        let m = manifest(vec![
            ModEntry::Separator { title: "Weapons".into(), color: None },
            ModEntry::Mod(spec("gun", "g")),
            ModEntry::Mod(spec("new", "n")),
            ModEntry::Mod(spec("knife", "k")),
            ModEntry::Separator { title: "Cars".into(), color: None },
            ModEntry::Mod(spec("car", "c")),
        ]);
        let st = state(
            Some("base"),
            &[("gun", "old gun", "g"), ("gone", "gone", "x"), ("knife", "knife folder", "k"), ("car", "car folder", "c")],
        );
        let current = ModList {
            entries: vec![
                Entry::enabled("At Top"),
                Entry::separator("Weapons"),
                Entry::enabled("My Sight"),
                Entry::enabled("old gun"),
                Entry::disabled("My Ammo"),
                Entry::enabled("gone"),
                Entry::enabled("After Gone"),
                Entry::enabled("knife folder"),
                Entry::separator("My Stuff in MO2"),
                Entry::separator("Cars"),
                Entry::enabled("car folder"),
                Entry::separator(USER_SEPARATOR),
                Entry::enabled("Mine"),
            ],
        };

        let p = plan(&m, &st, &current);
        let names: Vec<_> = p.modlist.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "At Top",
                "Weapons_separator",
                "My Sight",
                "gun folder",
                "My Ammo",
                "After Gone",
                "new folder",
                "knife folder",
                "Cars_separator",
                "car folder",
                "LYNO USER MODS_separator",
                "My Stuff in MO2_separator",
                "Mine",
            ],
            "renamed anchor keeps its mods, a removed one hands them up, separators of the player go down"
        );
        assert_eq!(p.modlist.get("My Ammo").unwrap().state, EntryState::Disabled);

        let st = state(Some("base"), &[("gun", "gun folder", "g"), ("new", "new folder", "n"), ("knife", "knife folder", "k"), ("car", "car folder", "c")]);
        assert!(plan(&m, &st, &p.modlist).is_up_to_date(), "the player's places are not an update");
    }

    fn optional(id: &str, hash: &str, enabled: bool) -> ModSpec {
        ModSpec { optional: true, enabled, ..spec(id, hash) }
    }

    #[test]
    fn optional_mods_keep_the_players_choice() {
        let m = manifest(vec![
            ModEntry::Mod(spec("cet", "h")),
            ModEntry::Mod(optional("hd", "new", true)),
            ModEntry::Mod(optional("lut", "l", false)),
            ModEntry::Mod(optional("fresh", "f", true)),
        ]);
        let st = state(Some("base"), &[("cet", "cet folder", "h"), ("hd", "old hd", "old"), ("lut", "lut folder", "l")]);
        // The player switched CET and HD off and the LUT on in MO2.
        let current = ModList {
            entries: vec![Entry::disabled("cet folder"), Entry::disabled("old hd"), Entry::enabled("lut folder")],
        };

        let p = plan(&m, &st, &current);
        let state_of = |name: &str| p.modlist.get(name).unwrap().state;
        assert_eq!(state_of("cet folder"), EntryState::Enabled, "required mods follow the manifest");
        assert_eq!(state_of("hd folder"), EntryState::Disabled, "choice survives an update with a rename");
        assert_eq!(state_of("lut folder"), EntryState::Enabled);
        assert_eq!(state_of("fresh folder"), EntryState::Enabled, "new optional mod gets the author's default");
    }

    #[test]
    fn removed_optional_mods_stay_away() {
        let m = manifest(vec![
            ModEntry::Mod(spec("cet", "h")),
            ModEntry::Mod(optional("hd", "new", true)),
            ModEntry::Mod(optional("lut", "l", true)),
        ]);
        let mut st = state(Some("base"), &[("cet", "cet folder", "h")]);
        st.removed = ["hd".to_string(), "cet".to_string()].into();
        let current = ModList { entries: vec![Entry::enabled("cet folder"), Entry::enabled("hd folder")] };

        let p = plan(&m, &st, &current);
        assert_eq!(p.actions, [Action::Install { id: "lut".into() }], "an update of a removed mod is not for this player");
        let names: Vec<_> = p.modlist.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            ["cet folder", "hd folder", "lut folder"],
            "a core mod ignores the mark; a folder by the removed mod's name is the player's own and stays"
        );
    }

    #[test]
    fn set_enabled_toggles_only_installed_optional_mods() {
        let m = manifest(vec![ModEntry::Mod(spec("cet", "h")), ModEntry::Mod(optional("hd", "h", true))]);
        let st = state(Some("base"), &[("cet", "cet folder", "h"), ("hd", "hd folder", "h")]);
        let mut list = plan(&m, &st, &ModList::default()).modlist;

        set_enabled(&m, &st, &mut list, "hd", false).unwrap();
        assert_eq!(list.get("hd folder").unwrap().state, EntryState::Disabled);
        assert!(plan(&m, &st, &list).is_up_to_date(), "the choice is not an update");

        assert!(set_enabled(&m, &st, &mut list, "cet", false).is_err());
        assert!(set_enabled(&m, &state(Some("base"), &[]), &mut list, "hd", true).is_err());
        assert!(set_enabled(&m, &st, &mut list, "nope", true).is_err());
    }
}

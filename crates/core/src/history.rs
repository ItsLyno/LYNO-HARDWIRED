//! Snapshots of a profile's `modlist.txt` taken before the launcher changes it,
//! so a player who switched off the wrong mods, dragged half the list or got a
//! broken list from an update can go back. Kept in `.lyno/history/<profile>.json`,
//! newest first; never in the profile folder, which the build ships.
//!
//! Only the list goes back (order and on/off), not mod files: a deleted mod
//! stays deleted, one installed since stays installed, switched off.

use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::manifest::Manifest;
use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::state::State;
use crate::{Error, Result};

const KEEP: usize = 30;
/// Changes this close to the previous snapshot go into it: switching ten selected mods
/// one by one is one change for the player, and must not push out the older snapshots.
const MERGE_MS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    /// When it was taken, ms since epoch; unique within a profile.
    pub id: u64,
    /// The change that came after it ("Before switching off X"), in the player's words.
    pub label: String,
    /// Changes merged into it after the first (see [`MERGE_MS`]).
    #[serde(default)]
    pub more: u32,
    /// Later small changes may merge into it.
    #[serde(default)]
    pub merge: bool,
    /// The installed build when it was taken: a list from before an update doesn't fit the new build.
    #[serde(default)]
    pub build: Option<String>,
    /// `modlist.txt` as it was.
    pub text: String,
}

/// How a snapshot differs from the list now.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Diff {
    /// Mods that would switch on or off.
    pub switched: usize,
    /// Enabled mods the snapshot doesn't have (installed since): they would be switched off.
    pub off: usize,
    /// Mods of the snapshot deleted since: they don't come back.
    pub gone: usize,
    /// The order of the mods in both differs.
    pub moved: bool,
}

fn path(inst: &Instance, profile: &str) -> PathBuf {
    inst.root().join(".lyno").join("history").join(format!("{profile}.json"))
}

/// Newest first; empty when there is none or it doesn't read.
pub fn list(inst: &Instance, profile: &str) -> Vec<Snapshot> {
    std::fs::read_to_string(path(inst, profile)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save(inst: &Instance, profile: &str, snapshots: &[Snapshot]) -> Result<()> {
    let p = path(inst, profile);
    let dir = p.parent().expect("history file has a folder");
    std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec(snapshots)?).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, &p).map_err(|e| Error::io(&p, e))
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

/// Remembers the list as it is now, before a change labelled `label`. Nothing when it is the
/// same as the newest snapshot. `merge`: a small change, joined to a small one just before.
pub fn record(inst: &Instance, profile: &str, label: &str, merge: bool) -> Result<()> {
    let Ok(text) = std::fs::read_to_string(inst.modlist_path(profile)) else { return Ok(()) };
    let mut snapshots = list(inst, profile);
    let now = now_ms();
    if let Some(newest) = snapshots.first_mut() {
        if newest.text == text {
            return Ok(());
        }
        if merge && newest.merge && now.saturating_sub(newest.id) < MERGE_MS {
            newest.more += 1;
            return save(inst, profile, &snapshots);
        }
    }
    let build = State::load(&crate::install::state_path(inst)).ok().and_then(|s| s.build_version);
    // Two in the same millisecond keep apart: ids are what restore picks by.
    let id = snapshots.first().map_or(now, |n| now.max(n.id + 1));
    snapshots.insert(0, Snapshot { id, label: label.to_owned(), more: 0, merge, build, text });
    snapshots.truncate(KEEP);
    save(inst, profile, &snapshots)
}

/// What going back to `snapshot` would change in `current`.
pub fn diff(inst: &Instance, snapshot: &Snapshot, current: &ModList) -> Diff {
    let Ok(old) = ModList::parse(&snapshot.text, &inst.modlist_path("")) else { return Diff::default() };
    let mods = inst.mods_dir();
    let names = |l: &ModList| l.mods().map(|e| e.name.clone()).collect::<HashSet<_>>();
    let (in_old, in_now) = (names(&old), names(current));
    let order = |l: &ModList, other: &HashSet<String>| l.mods().filter(|e| other.contains(&e.name)).map(|e| e.name.clone()).collect::<Vec<_>>();
    Diff {
        switched: old.mods().filter(|e| current.get(&e.name).is_some_and(|c| c.state != e.state)).count(),
        off: current.mods().filter(|e| e.state == EntryState::Enabled && !in_old.contains(&e.name)).count(),
        gone: old.mods().filter(|e| !mods.join(&e.name).is_dir()).count(),
        moved: order(&old, &in_now) != order(current, &in_old),
    }
}

/// The list of snapshot `id` as it can be now: entries whose folders are gone left out, mods
/// installed since added switched off at the end. With `build` (an installed build, not the
/// author's), build entries then follow the build as on an update: the snapshot only brings
/// back the player's choices, never a build section the build doesn't have.
pub fn restored(inst: &Instance, profile: &str, id: u64, build: Option<(&Manifest, &State)>) -> Result<ModList> {
    let snapshot = list(inst, profile)
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| Error::Manifest(format!("no snapshot {id} of profile {profile:?}")))?;
    let path = inst.modlist_path(profile);
    let current = ModList::load(&path)?;
    let mut list = ModList::parse(&snapshot.text, &path)?;
    let mods = inst.mods_dir();
    list.entries.retain(|e| e.state == EntryState::Unmanaged || mods.join(&e.name).is_dir());
    let known: HashSet<String> = list.entries.iter().map(|e| e.name.clone()).collect();
    for e in current.entries.iter().filter(|e| !known.contains(&e.name)) {
        let mut e = e.clone();
        if e.state == EntryState::Enabled {
            e.state = EntryState::Disabled;
        }
        list.entries.push(e);
    }
    Ok(match build {
        Some((manifest, state)) => crate::plan::target_modlist(manifest, state, &list),
        None => list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::Entry;

    fn setup(names: &[&str]) -> (tempfile::TempDir, Instance) {
        let tmp = tempfile::tempdir().unwrap();
        let inst = Instance::new(tmp.path());
        for n in names {
            std::fs::create_dir_all(inst.mods_dir().join(n)).unwrap();
        }
        std::fs::create_dir_all(inst.profile_dir("P")).unwrap();
        (tmp, inst)
    }

    fn set(inst: &Instance, entries: Vec<Entry>) {
        ModList { entries }.save(&inst.modlist_path("P")).unwrap();
    }

    fn names(l: &ModList) -> Vec<(String, EntryState)> {
        l.entries.iter().map(|e| (e.name.clone(), e.state)).collect()
    }

    #[test]
    fn records_skips_repeats_and_merges_small_changes() {
        let (_tmp, inst) = setup(&["A", "B"]);
        record(&inst, "P", "nothing yet", false).unwrap();
        assert!(list(&inst, "P").is_empty(), "no modlist.txt, nothing to remember");

        set(&inst, vec![Entry::enabled("A"), Entry::enabled("B")]);
        record(&inst, "P", "before switching off A", true).unwrap();
        record(&inst, "P", "same list again", true).unwrap();
        assert_eq!(list(&inst, "P").len(), 1);

        set(&inst, vec![Entry::disabled("A"), Entry::enabled("B")]);
        record(&inst, "P", "before switching off B", true).unwrap();
        let all = list(&inst, "P");
        assert_eq!((all.len(), all[0].label.as_str(), all[0].more), (1, "before switching off A", 1), "merged into the first");

        // An update is never merged: it is what a player most wants to undo.
        set(&inst, vec![Entry::disabled("A"), Entry::disabled("B")]);
        record(&inst, "P", "before the update", false).unwrap();
        let all = list(&inst, "P");
        assert_eq!(all.iter().map(|s| s.label.as_str()).collect::<Vec<_>>(), ["before the update", "before switching off A"]);
        assert!(all[0].id > all[1].id);
    }

    #[test]
    fn keeps_the_newest() {
        let (_tmp, inst) = setup(&["A"]);
        for i in 0..KEEP + 5 {
            let e = if i % 2 == 0 { Entry::enabled("A") } else { Entry::disabled("A") };
            set(&inst, vec![e]);
            record(&inst, "P", &format!("change {i}"), false).unwrap();
        }
        let all = list(&inst, "P");
        assert_eq!(all.len(), KEEP);
        assert_eq!(all[0].label, format!("change {}", KEEP + 4));
    }

    #[test]
    fn restores_what_can_come_back() {
        let (_tmp, inst) = setup(&["A", "B", "C", "Sep_separator"]);
        set(&inst, vec![Entry::separator("Sep"), Entry::enabled("A"), Entry::disabled("B"), Entry::enabled("Gone")]);
        record(&inst, "P", "before", false).unwrap();
        let id = list(&inst, "P")[0].id;
        // Since then: B on and above A, C installed, Gone deleted.
        set(&inst, vec![Entry::separator("Sep"), Entry::enabled("B"), Entry::enabled("A"), Entry::enabled("C")]);

        let now = ModList::load(&inst.modlist_path("P")).unwrap();
        assert_eq!(diff(&inst, &list(&inst, "P")[0], &now), Diff { switched: 1, off: 1, gone: 1, moved: true });

        let back = restored(&inst, "P", id, None).unwrap();
        assert_eq!(
            names(&back),
            [
                ("Sep_separator".to_owned(), EntryState::Disabled),
                ("A".to_owned(), EntryState::Enabled),
                ("B".to_owned(), EntryState::Disabled),
                ("C".to_owned(), EntryState::Disabled),
            ]
        );
        assert!(restored(&inst, "P", id + 1, None).is_err());
    }

    #[test]
    fn a_build_section_follows_the_build() {
        use crate::manifest::tests::{manifest, spec};
        use crate::manifest::ModEntry;
        use crate::state::InstalledMod;
        let (_tmp, inst) = setup(&["cet folder", "opt folder", "Mine", "LYNO USER MODS_separator"]);
        let mut opt = spec("opt", "h2");
        opt.optional = true;
        let m = manifest(vec![ModEntry::Mod(spec("cet", "h1")), ModEntry::Mod(opt)]);
        let installed = |folder: &str| InstalledMod { folder: folder.into(), hash: String::new(), damaged: false, reset_settings: false };
        let state = State {
            mods: [("cet".to_owned(), installed("cet folder")), ("opt".to_owned(), installed("opt folder"))].into(),
            ..Default::default()
        };
        // The player had switched off the optional mod and a core one (in MO2), and put their mod among the build's.
        set(&inst, vec![Entry::disabled("cet folder"), Entry::enabled("Mine"), Entry::disabled("opt folder")]);
        record(&inst, "P", "before", false).unwrap();
        let id = list(&inst, "P")[0].id;
        set(&inst, vec![Entry::enabled("cet folder"), Entry::enabled("opt folder"), Entry::separator("LYNO USER MODS"), Entry::enabled("Mine")]);

        let back = restored(&inst, "P", id, Some((&m, &state))).unwrap();
        assert_eq!(
            names(&back),
            [
                ("cet folder".to_owned(), EntryState::Enabled),
                ("opt folder".to_owned(), EntryState::Disabled),
                ("LYNO USER MODS_separator".to_owned(), EntryState::Disabled),
                ("Mine".to_owned(), EntryState::Enabled),
            ],
            "the core mod stays on, the player's choice for the optional one comes back"
        );
    }
}

//! Which enabled mod's file the game gets where several mods ship the same path,
//! as MO2's VFS resolves it: the highest priority wins, `overwrite/` above every mod.
//!
//! Unlike MO2's conflicts tab, every overlap is told from the winner's side: a mod
//! in the middle of three with a file "overwrites" the lowest one in MO2, though its
//! file never reaches the game. Here it only loses, to the one whose file does.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::tree;
use crate::Result;

/// The name `overwrite/` goes by in [`ModConflicts`]; `<` can't be in a folder name on Windows.
pub const OVERWRITE: &str = "<overwrite>";

/// One enabled mod's overlaps with the others; only mods that have any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModConflicts {
    /// Folder under `mods/`, or [`OVERWRITE`].
    pub mod_folder: String,
    /// Files of this mod that are in the game though lower ones have them too, by the mod they hide.
    pub wins: Vec<Overlap>,
    /// Files of this mod a higher one hides, by the mod whose file the game gets.
    pub loses: Vec<Overlap>,
    /// All the mod's files: when every one loses, the mod does nothing in the game.
    pub files: usize,
    pub lost: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overlap {
    pub mod_folder: String,
    /// Relative paths with `/`, sorted.
    pub files: Vec<String>,
}

/// MO2 leaves out a mod's `meta.ini` and anything renamed to `*.mohidden` (its "hide file").
fn is_visible(path: &str) -> bool {
    path != "meta.ini" && !path.split('/').any(|part| part.to_ascii_lowercase().ends_with(".mohidden"))
}

/// Enabled mods of `list` in priority order, then `overwrite/`. Windows paths ignore case, so do overlaps.
pub fn scan(inst: &Instance, list: &ModList) -> Result<Vec<ModConflicts>> {
    let mut sources: Vec<(String, std::path::PathBuf)> = list
        .mods()
        .filter(|e| e.state == EntryState::Enabled)
        .map(|e| (e.name.clone(), inst.mods_dir().join(&e.name)))
        .filter(|(_, dir)| dir.is_dir())
        .collect();
    let overwrite = inst.overwrite_dir();
    if overwrite.is_dir() {
        sources.push((OVERWRITE.to_owned(), overwrite));
    }
    // Lowest first, so the last source of a path is the one the game gets.
    let mut owners: HashMap<String, (String, Vec<usize>)> = HashMap::new();
    let mut counts = vec![0; sources.len()];
    for (idx, (_, dir)) in sources.iter().enumerate() {
        for f in tree::list_files_with(dir, &is_visible)? {
            counts[idx] += 1;
            let entry = owners.entry(f.path.to_lowercase()).or_insert_with(|| (String::new(), Vec::new()));
            entry.0 = f.path;
            entry.1.push(idx);
        }
    }
    let mut wins: Vec<BTreeMap<usize, Vec<String>>> = vec![BTreeMap::new(); sources.len()];
    let mut loses: Vec<BTreeMap<usize, Vec<String>>> = vec![BTreeMap::new(); sources.len()];
    for (path, idxs) in owners.into_values().filter(|(_, idxs)| idxs.len() > 1) {
        let (&winner, losers) = idxs.split_last().expect("more than one owner");
        for &loser in losers {
            wins[winner].entry(loser).or_default().push(path.clone());
            loses[loser].entry(winner).or_default().push(path.clone());
        }
    }
    let overlaps = |by: BTreeMap<usize, Vec<String>>| -> Vec<Overlap> {
        // Nearest in priority first: the ones a player is likely to move around.
        by.into_iter()
            .map(|(idx, mut files)| {
                files.sort();
                Overlap { mod_folder: sources[idx].0.clone(), files }
            })
            .collect()
    };
    let mut out = Vec::new();
    for (idx, (w, l)) in wins.into_iter().zip(loses).enumerate() {
        if w.is_empty() && l.is_empty() {
            continue;
        }
        let lost = l.values().map(Vec::len).sum();
        let mut wins = overlaps(w);
        wins.reverse();
        out.push(ModConflicts { mod_folder: sources[idx].0.clone(), wins, loses: overlaps(l), files: counts[idx], lost });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::Entry;
    use std::path::Path;

    fn write(root: &Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, "").unwrap();
    }

    fn files(o: &Overlap) -> (&str, Vec<&str>) {
        (o.mod_folder.as_str(), o.files.iter().map(String::as_str).collect())
    }

    #[test]
    fn the_highest_priority_wins_each_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let inst = Instance::new(root);
        write(root, "mods/Low/r6/scripts/a.reds");
        write(root, "mods/Low/r6/scripts/b.reds");
        write(root, "mods/Low/meta.ini");
        write(root, "mods/Mid/r6/scripts/a.reds");
        write(root, "mods/Mid/meta.ini");
        write(root, "mods/High/R6/Scripts/A.reds");
        write(root, "mods/High/r6/scripts/b.reds.mohidden");
        write(root, "mods/Off/r6/scripts/b.reds");
        write(root, "mods/Alone/r6/scripts/c.reds");
        write(root, "overwrite/r6/scripts/b.reds");
        let list = ModList {
            entries: vec![
                Entry::enabled("Low"),
                Entry::separator("Group"),
                Entry::enabled("Mid"),
                Entry::enabled("Alone"),
                Entry::enabled("High"),
                Entry::disabled("Off"),
            ],
        };

        let all = scan(&inst, &list).unwrap();
        let by: HashMap<_, _> = all.iter().map(|c| (c.mod_folder.as_str(), c)).collect();
        assert_eq!(all.len(), 4, "only mods with overlaps: {all:?}");

        let low = by["Low"];
        assert!(low.wins.is_empty());
        let loses: Vec<_> = low.loses.iter().map(files).collect();
        assert_eq!(loses, [("High", vec!["R6/Scripts/A.reds"]), (OVERWRITE, vec!["r6/scripts/b.reds"])]);
        assert_eq!((low.files, low.lost), (2, 2));

        // Mid's file never reaches the game: it only loses, to High, not wins over Low.
        let mid = by["Mid"];
        assert!(mid.wins.is_empty());
        assert_eq!((mid.files, mid.lost), (1, 1));

        let high = by["High"];
        let wins: Vec<_> = high.wins.iter().map(files).collect();
        assert_eq!(wins, [("Mid", vec!["R6/Scripts/A.reds"]), ("Low", vec!["R6/Scripts/A.reds"])]);
        assert!(high.loses.is_empty());
        assert_eq!((high.files, high.lost), (1, 0));

        assert_eq!(by[OVERWRITE].wins.iter().map(files).collect::<Vec<_>>(), [("Low", vec!["r6/scripts/b.reds"])]);
    }
}

//! Which enabled mod's file the game gets where several mods ship the same path,
//! as MO2's VFS resolves it: the highest priority wins, `overwrite/` above every mod.
//!
//! Unlike MO2's conflicts tab, every overlap is told from the winner's side: a mod
//! in the middle of three with a file "overwrites" the lowest one in MO2, though its
//! file never reaches the game. Here it only loses, to the one whose file does.
//!
//! Most Cyberpunk conflicts MO2 doesn't see at all: two `.archive` files of
//! different names replacing the same resource (a mesh, a texture). The game
//! loads `archive/pc/mod/*.archive` in ASCII order of the file name (`#`, digits
//! and capitals first), or in the order of `archive/pc/mod/modlist.txt`, which the
//! MO2 plugin writes before a launch with `enforce_archive_load_order`; the first
//! archive loaded with a resource wins it.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::tree;
use crate::Result;

/// The name `overwrite/` goes by in [`ModConflicts`]; `<` can't be in a folder name on Windows.
pub const OVERWRITE: &str = "<overwrite>";

const ARCHIVE_DIR: &str = "archive/pc/mod/";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflicts {
    /// Mods with any conflict, lowest priority first, [`OVERWRITE`] last.
    pub mods: Vec<String>,
    pub files: Vec<ModConflicts>,
    pub archives: Vec<ArchiveConflicts>,
    pub archive_order: ArchiveOrder,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ArchiveOrder {
    /// No `modlist.txt`: ASCII order of the file names.
    Alphabetical,
    /// `enforce_archive_load_order`: lowest MO2 priority loads first, so it wins.
    Mo2,
    /// With `reverse_archive_load_order` too: the highest MO2 priority wins, as with loose files.
    Mo2Reversed,
}

/// One mod's archives sharing resources with archives of other mods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveConflicts {
    pub mod_folder: String,
    /// Resources the game takes from this mod's archive, not the other mod's.
    pub wins: Vec<ArchiveOverlap>,
    /// Resources of this mod's archive the game takes from the other mod's.
    pub loses: Vec<ArchiveOverlap>,
    /// File names of its archives that lose every resource: they change nothing in the game.
    pub dead: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ArchiveOverlap {
    /// File name of this mod's archive.
    pub archive: String,
    pub mod_folder: String,
    /// File name of the other mod's archive.
    pub other: String,
    /// Hashes of the resource paths (archives keep no names), decimal as WolvenKit shows them, sorted.
    pub resources: Vec<String>,
}

/// MO2 leaves out a mod's `meta.ini` and anything renamed to `*.mohidden` (its "hide file").
fn is_visible(path: &str) -> bool {
    path != "meta.ini" && !path.split('/').any(|part| part.to_ascii_lowercase().ends_with(".mohidden"))
}

/// Enabled mods of `list` in priority order, then `overwrite/`. Windows paths ignore case, so do overlaps.
pub fn scan(inst: &Instance, list: &ModList) -> Result<Conflicts> {
    let mut sources: Vec<(String, PathBuf)> = list
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
    let archive_order = archive_order(inst);
    let archives = archive_conflicts(&sources, &owners, archive_order);
    let files = file_conflicts(&sources, &owners, &counts);
    let has = |name: &str| files.iter().any(|m| m.mod_folder == name) || archives.iter().any(|m| m.mod_folder == name);
    let mods = sources.iter().map(|(name, _)| name).filter(|name| has(name)).cloned().collect();
    Ok(Conflicts { mods, files, archives, archive_order })
}

fn file_conflicts(sources: &[(String, PathBuf)], owners: &HashMap<String, (String, Vec<usize>)>, counts: &[usize]) -> Vec<ModConflicts> {
    let mut wins: Vec<BTreeMap<usize, Vec<String>>> = vec![BTreeMap::new(); sources.len()];
    let mut loses: Vec<BTreeMap<usize, Vec<String>>> = vec![BTreeMap::new(); sources.len()];
    for (path, idxs) in owners.values().filter(|(_, idxs)| idxs.len() > 1) {
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
    out
}

/// The MO2 plugin's settings, `[Plugins]` of `ModOrganizer.ini` (`Cyberpunk%202077%20Support%20Plugin\<key>=true`).
fn archive_order(inst: &Instance) -> ArchiveOrder {
    let text = std::fs::read_to_string(inst.ini_path()).unwrap_or_default();
    let on = |key: &str| {
        let suffix = format!("\\{key}=true");
        text.lines().any(|l| l.trim().starts_with("Cyberpunk") && l.trim().ends_with(&suffix))
    };
    match (on("enforce_archive_load_order"), on("reverse_archive_load_order")) {
        (false, _) => ArchiveOrder::Alphabetical,
        (true, false) => ArchiveOrder::Mo2,
        (true, true) => ArchiveOrder::Mo2Reversed,
    }
}

/// An archive the game loads: the VFS winner of its name.
struct Loaded {
    source: usize,
    name: String,
    path: PathBuf,
}

/// `name` of `archive/pc/mod/<name>.archive`; archives in subfolders aren't loaded.
fn archive_name(path: &str) -> Option<&str> {
    let name = path.strip_prefix(ARCHIVE_DIR)?;
    (!name.contains('/') && name.ends_with(".archive")).then_some(name)
}

/// Archives in the order the game loads them.
fn load_order(sources: &[(String, PathBuf)], owners: &HashMap<String, (String, Vec<usize>)>, order: ArchiveOrder) -> Vec<Loaded> {
    let mut loaded: Vec<Loaded> = owners
        .iter()
        .filter_map(|(key, (path, idxs))| {
            archive_name(key)?;
            let source = *idxs.last().expect("an owner");
            let name = path.rsplit('/').next().expect("a file name").to_owned();
            Some(Loaded { source, name, path: sources[source].1.join(path) })
        })
        .collect();
    // The game compares bytes: `#`, digits and capitals sort before lowercase.
    loaded.sort_by(|a, b| a.name.cmp(&b.name));
    if order != ArchiveOrder::Alphabetical {
        // The plugin lists every enabled mod's archives by priority, a mod's own as Windows lists a folder;
        // overwrite/ isn't a mod. ponytail: archives the list misses assumed to load after it, alphabetically.
        let mut mods: Vec<usize> = (0..sources.len()).filter(|&i| sources[i].0 != OVERWRITE).collect();
        if order == ArchiveOrder::Mo2Reversed {
            mods.reverse();
        }
        let mut rank: HashMap<String, usize> = HashMap::new();
        for idx in mods {
            let mut names: Vec<&str> = owners.iter().filter(|(_, (_, idxs))| idxs.contains(&idx)).filter_map(|(key, _)| archive_name(key)).collect();
            names.sort();
            for name in names {
                let next = rank.len();
                rank.entry(name.to_owned()).or_insert(next);
            }
        }
        loaded.sort_by_key(|a| rank.get(&a.name.to_lowercase()).copied().unwrap_or(usize::MAX));
    }
    loaded
}

fn archive_conflicts(sources: &[(String, PathBuf)], owners: &HashMap<String, (String, Vec<usize>)>, order: ArchiveOrder) -> Vec<ArchiveConflicts> {
    let loaded = load_order(sources, owners, order);
    // Unreadable or not an archive: the game skips it too, nothing to show.
    let hashes: Vec<Vec<u64>> = loaded.iter().map(|a| resource_hashes(&a.path).unwrap_or_default()).collect();
    let mut first: HashMap<u64, usize> = HashMap::new();
    let mut lost = vec![0usize; loaded.len()];
    // (this archive, other archive) → resources, by the mod of this archive.
    let mut wins: Vec<BTreeMap<(usize, usize), Vec<u64>>> = vec![BTreeMap::new(); sources.len()];
    let mut loses: Vec<BTreeMap<(usize, usize), Vec<u64>>> = vec![BTreeMap::new(); sources.len()];
    for (idx, list) in hashes.iter().enumerate() {
        for &hash in list {
            let Some(&winner) = first.get(&hash) else {
                first.insert(hash, idx);
                continue;
            };
            lost[idx] += 1;
            let (w, l) = (loaded[winner].source, loaded[idx].source);
            // A mod's own archives overriding each other is how its author built it.
            if w != l {
                wins[w].entry((winner, idx)).or_default().push(hash);
                loses[l].entry((idx, winner)).or_default().push(hash);
            }
        }
    }
    let overlaps = |by: BTreeMap<(usize, usize), Vec<u64>>| -> Vec<ArchiveOverlap> {
        by.into_iter()
            .map(|((this, other), mut resources)| {
                resources.sort_unstable();
                ArchiveOverlap {
                    archive: loaded[this].name.clone(),
                    mod_folder: sources[loaded[other].source].0.clone(),
                    other: loaded[other].name.clone(),
                    resources: resources.iter().map(u64::to_string).collect(),
                }
            })
            .collect()
    };
    let mut out = Vec::new();
    for (idx, (w, l)) in wins.into_iter().zip(loses).enumerate() {
        if w.is_empty() && l.is_empty() {
            continue;
        }
        let dead = (0..loaded.len())
            .filter(|&a| loaded[a].source == idx && !hashes[a].is_empty() && lost[a] == hashes[a].len())
            .map(|a| loaded[a].name.clone())
            .collect();
        out.push(ArchiveConflicts { mod_folder: sources[idx].0.clone(), wins: overlaps(w), loses: overlaps(l), dead });
    }
    out
}

/// Path hashes of an RDAR archive's resources, from its index alone (WolvenKit's `ArchiveReader`):
/// a 40-byte header with the index offset at 8, an index of 28 bytes with the entry count at 16,
/// then 56-byte entries, each starting with the FNV-1a 64 hash of the resource's path.
fn resource_hashes(path: &Path) -> std::io::Result<Vec<u64>> {
    let invalid = || std::io::Error::from(std::io::ErrorKind::InvalidData);
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let mut header = [0u8; 40];
    f.read_exact(&mut header)?;
    if &header[..4] != b"RDAR" {
        return Err(invalid());
    }
    let index_at = u64::from_le_bytes(header[8..16].try_into().unwrap());
    f.seek(SeekFrom::Start(index_at))?;
    let mut index = [0u8; 28];
    f.read_exact(&mut index)?;
    let count = u64::from(u32::from_le_bytes(index[16..20].try_into().unwrap()));
    if index_at + 28 + count * 56 > len {
        return Err(invalid());
    }
    let mut entries = vec![0u8; count as usize * 56];
    f.read_exact(&mut entries)?;
    Ok(entries.as_chunks::<56>().0.iter().map(|e| u64::from_le_bytes(e[..8].try_into().unwrap())).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::Entry;

    fn write_bytes(root: &Path, rel: &str, data: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    fn write(root: &Path, rel: &str) {
        write_bytes(root, rel, b"");
    }

    /// An archive with these resources and no data, laid out as the game's.
    fn rdar(hashes: &[u64]) -> Vec<u8> {
        let mut out = b"RDAR".to_vec();
        out.extend(12u32.to_le_bytes());
        out.extend(44u64.to_le_bytes());
        out.extend((28 + 56 * hashes.len() as u32).to_le_bytes());
        out.extend([0u8; 20]);
        out.extend(0u32.to_le_bytes()); // custom data length
        out.extend(8u32.to_le_bytes());
        out.extend((56 * hashes.len() as u32).to_le_bytes());
        out.extend(0u64.to_le_bytes());
        out.extend((hashes.len() as u32).to_le_bytes());
        out.extend([0u8; 8]);
        for h in hashes {
            out.extend(h.to_le_bytes());
            out.extend([0u8; 48]);
        }
        out
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

        let all = scan(&inst, &list).unwrap().files;
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

    /// Every resource an archive takes from another one: (winner mod, its archive, loser mod, its archive, hashes).
    fn taken(c: &Conflicts) -> Vec<(String, String, String, String, Vec<&str>)> {
        let mut out: Vec<_> = c
            .archives
            .iter()
            .flat_map(|m| {
                m.wins.iter().map(|o| {
                    (m.mod_folder.clone(), o.archive.clone(), o.mod_folder.clone(), o.other.clone(), o.resources.iter().map(String::as_str).collect())
                })
            })
            .collect();
        out.sort();
        out
    }

    fn t(w: &str, wa: &str, l: &str, la: &str, r: &[&'static str]) -> (String, String, String, String, Vec<&'static str>) {
        (w.into(), wa.into(), l.into(), la.into(), r.to_vec())
    }

    #[test]
    fn the_first_archive_loaded_wins_each_resource() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let inst = Instance::new(root);
        write_bytes(root, "mods/Zeta/archive/pc/mod/a_patch.archive", &rdar(&[1, 2, 3]));
        // Hidden by Alpha's file of the same name: the game never loads it.
        write_bytes(root, "mods/Old/archive/pc/mod/z_main.archive", &rdar(&[4, 5]));
        write_bytes(root, "mods/Alpha/archive/pc/mod/z_main.archive", &rdar(&[2, 3, 4]));
        write_bytes(root, "mods/Fix/archive/pc/mod/#fix.archive", &rdar(&[3]));
        write_bytes(root, "mods/Upper/archive/pc/mod/B.archive", &rdar(&[4, 6]));
        write_bytes(root, "mods/Upper/archive/pc/mod/sub/skipped.archive", &rdar(&[1]));
        write_bytes(root, "mods/Upper/archive/pc/mod/broken.archive", b"not an archive");
        let list = ModList { entries: ["Zeta", "Old", "Alpha", "Fix", "Upper"].into_iter().map(Entry::enabled).collect() };

        // ASCII: #fix, B, a_patch, z_main.
        let c = scan(&inst, &list).unwrap();
        assert_eq!(c.archive_order, ArchiveOrder::Alphabetical);
        assert_eq!(
            taken(&c),
            [
                t("Fix", "#fix.archive", "Alpha", "z_main.archive", &["3"]),
                t("Fix", "#fix.archive", "Zeta", "a_patch.archive", &["3"]),
                t("Upper", "B.archive", "Alpha", "z_main.archive", &["4"]),
                t("Zeta", "a_patch.archive", "Alpha", "z_main.archive", &["2"]),
            ]
        );
        let alpha = c.archives.iter().find(|m| m.mod_folder == "Alpha").unwrap();
        assert_eq!(alpha.dead, ["z_main.archive"]);
        assert_eq!(alpha.loses.len(), 3);
        assert_eq!(c.mods, ["Zeta", "Old", "Alpha", "Fix", "Upper"], "Old by its hidden file");

        // The plugin's list: lowest MO2 priority first and winning.
        let ini = "[Plugins]\r\nCyberpunk%202077%20Support%20Plugin\\enforce_archive_load_order=true\r\n";
        std::fs::write(inst.ini_path(), ini).unwrap();
        let c = scan(&inst, &list).unwrap();
        assert_eq!(c.archive_order, ArchiveOrder::Mo2);
        assert_eq!(
            taken(&c),
            [
                t("Alpha", "z_main.archive", "Upper", "B.archive", &["4"]),
                t("Zeta", "a_patch.archive", "Alpha", "z_main.archive", &["2", "3"]),
                t("Zeta", "a_patch.archive", "Fix", "#fix.archive", &["3"]),
            ]
        );
        assert_eq!(c.archives.iter().find(|m| m.mod_folder == "Fix").unwrap().dead, ["#fix.archive"]);

        std::fs::write(inst.ini_path(), format!("{ini}Cyberpunk%202077%20Support%20Plugin\\reverse_archive_load_order=true\r\n")).unwrap();
        let c = scan(&inst, &list).unwrap();
        assert_eq!(c.archive_order, ArchiveOrder::Mo2Reversed);
        assert_eq!(
            taken(&c),
            [
                t("Alpha", "z_main.archive", "Zeta", "a_patch.archive", &["2"]),
                t("Fix", "#fix.archive", "Alpha", "z_main.archive", &["3"]),
                t("Fix", "#fix.archive", "Zeta", "a_patch.archive", &["3"]),
                t("Upper", "B.archive", "Alpha", "z_main.archive", &["4"]),
            ]
        );
    }
}

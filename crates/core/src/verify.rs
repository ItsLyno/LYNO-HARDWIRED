//! Integrity check of an installed build and marking what needs repair.
//!
//! A mod folder is checked against the tree hash of its package, the same
//! filter as [`crate::package::unpack`] plus [`rules::is_generated`]: the
//! package never contains generated files, but the game may write logs or
//! caches into a mod folder, and those are not damage.
//!
//! The manifest has no per-file hashes, so a damaged mod can only be
//! repaired as a whole (download the package again). The check can't tell a
//! deleted file from a settings file the game rewrote in place either: a mod
//! whose shipped config the player changed in game shows up as changed, and
//! repairing it resets the config. The launcher lets the player choose what
//! to repair.
//!
//! The base package shares the instance root with files MO2 rewrites on its
//! own (`ModOrganizer.ini`, profile settings), so only missing base files
//! count as damage.
//!
//! Repair itself is an update: [`mark`] flags the selection in `state.json`
//! and [`crate::plan::plan`] turns the flags into [`crate::plan::Action::Repair`]
//! and [`crate::plan::Action::Base`], so it is resumable and crash-safe like
//! any update.

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::install::{is_player_file, Event};
use crate::mo2::Instance;
use crate::rules;
use crate::state::State;
use crate::tree::{self, FileEntry};
use crate::{Error, Result};

/// Missing base files listed in the report; the rest is only counted.
const LISTED_FILES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Mods checked, not counting the base package.
    pub checked: usize,
    pub damaged: Vec<Damaged>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Damaged {
    /// Manifest mod id; `None` for the base package (MO2 and its config).
    pub id: Option<String>,
    /// Folder under `mods/`, empty for the base package.
    pub folder: String,
    pub problem: Problem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Problem {
    /// The mod folder is gone.
    MissingFolder,
    /// Files were added, removed or changed.
    Changed,
    /// Base package files are gone; `files` lists the first few.
    MissingFiles { count: usize, files: Vec<String> },
}

/// Checks every installed build mod and the base package. Reads every file
/// of every build mod, so it takes as long as reading the whole build from
/// disk; progress arrives as [`Event::Step`] per mod and [`Event::Bytes`].
pub fn verify(inst: &Instance, state: &State, cancel: &AtomicBool, on: &mut dyn FnMut(Event)) -> Result<Report> {
    let mut damaged = Vec::new();

    let missing: Vec<&String> = state
        .base_files
        .iter()
        .filter(|rel| !is_player_file(rel) && !tree::from_slash(inst.root(), rel).is_file())
        .collect();
    if !missing.is_empty() {
        damaged.push(Damaged {
            id: None,
            folder: String::new(),
            problem: Problem::MissingFiles {
                count: missing.len(),
                files: missing.iter().take(LISTED_FILES).map(|s| s.to_string()).collect(),
            },
        });
    }

    // List everything first so progress has a total.
    let keep = |p: &str| rules::is_hashed(p) && !rules::is_generated(p);
    let mut mods = Vec::with_capacity(state.mods.len());
    let mut total_bytes = 0;
    for (id, m) in &state.mods {
        let dir = inst.mods_dir().join(&m.folder);
        if !dir.is_dir() {
            damaged.push(Damaged { id: Some(id.clone()), folder: m.folder.clone(), problem: Problem::MissingFolder });
            continue;
        }
        let files = tree::list_files_with(&dir, &keep)?;
        total_bytes += files.iter().map(|f| f.size).sum::<u64>();
        mods.push((id, m, dir, files));
    }

    let total = mods.len();
    let mut done = 0u64;
    for (i, (id, m, dir, files)) in mods.into_iter().enumerate() {
        on(Event::Step { index: i + 1, total, label: m.folder.clone() });
        let mut hashes = Vec::with_capacity(files.len());
        for f in &files {
            let path = tree::from_slash(&dir, &f.path);
            let start = done;
            hashes.push(hash_file(&path, cancel, &mut |n| on(Event::Bytes { done: start + n, total: total_bytes }))?);
            done = start + f.size;
        }
        let got = tree::combine(files.iter().zip(&hashes).map(|(f, h): (&FileEntry, &String)| (f, h.as_str())));
        if got.hash != m.hash {
            damaged.push(Damaged { id: Some(id.clone()), folder: m.folder.clone(), problem: Problem::Changed });
        }
    }

    on(Event::Done);
    Ok(Report { checked: state.mods.len(), damaged })
}

/// Flags exactly the selected mods (manifest ids) and, with `base`, the base
/// package for the next update to download again; clears every other flag.
/// Ids that are not installed are ignored.
pub fn mark(state: &mut State, ids: &[String], base: bool) {
    let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    for (id, m) in &mut state.mods {
        m.damaged = ids.contains(id.as_str());
    }
    state.base_damaged = base;
}

/// BLAKE3 of a file with byte progress (bytes read so far) and cancellation:
/// a single `.archive` can be several GB.
fn hash_file(path: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> Result<String> {
    let file = File::open(path).map_err(|e| Error::io(path, e))?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut read = 0u64;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        let n = reader.read(&mut buf).map_err(|e| Error::io(path, e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        read += n as u64;
        progress(read);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::InstalledMod;
    use crate::tree::tree_hash_with;

    fn write(root: &Path, rel: &str, data: &str) {
        let p = tree::from_slash(root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    fn installed(inst: &Instance, folder: &str) -> InstalledMod {
        let hash = tree_hash_with(&inst.mods_dir().join(folder), &rules::is_hashed).unwrap().hash;
        InstalledMod { folder: folder.into(), hash, damaged: false }
    }

    #[test]
    fn finds_changed_and_missing_mods_and_base_files() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        write(dir.path(), "ModOrganizer.exe", "MZ");
        write(dir.path(), "ModOrganizer.ini", "[General]");
        for name in ["Intact", "Broken", "Gone"] {
            write(&inst.mods_dir(), &format!("{name}/archive/pc/mod/{name}.archive"), name);
            write(&inst.mods_dir(), &format!("{name}/meta.ini"), "[General]");
        }
        let mut state = State {
            base_files: ["ModOrganizer.exe", "ModOrganizer.ini", "plugins/x.dll", "profiles/LYNO/UserSettings.json"]
                .map(String::from)
                .into(),
            ..Default::default()
        };
        for (id, name) in [("intact", "Intact"), ("broken", "Broken"), ("gone", "Gone")] {
            state.mods.insert(id.into(), installed(&inst, name));
        }

        // MO2 and the game touch files that are not damage.
        write(&inst.mods_dir(), "Intact/meta.ini", "[General]\nlastNexusQuery=now");
        write(&inst.mods_dir(), "Intact/r6/logs/redscript.log", "log");
        write(&inst.mods_dir(), "Intact/r6/cache/final.redscripts", "cache");
        // Antivirus ate a file; the player deleted a folder.
        std::fs::remove_file(inst.mods_dir().join("Broken/archive/pc/mod/Broken.archive")).unwrap();
        std::fs::remove_dir_all(inst.mods_dir().join("Gone")).unwrap();

        let mut events = Vec::new();
        let report = verify(&inst, &state, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();
        assert_eq!(report.checked, 3);
        assert_eq!(
            report.damaged,
            [
                Damaged {
                    id: None,
                    folder: String::new(),
                    problem: Problem::MissingFiles { count: 1, files: vec!["plugins/x.dll".into()] },
                },
                Damaged { id: Some("gone".into()), folder: "Gone".into(), problem: Problem::MissingFolder },
                Damaged { id: Some("broken".into()), folder: "Broken".into(), problem: Problem::Changed },
            ]
        );
        assert!(matches!(events.last(), Some(Event::Done)));
        assert!(events.iter().any(|e| matches!(e, Event::Step { total: 2, .. })));

        let cancelled = verify(&inst, &state, &AtomicBool::new(true), &mut |_| {});
        assert!(matches!(cancelled, Err(Error::Cancelled)));
    }

    #[test]
    fn mark_flags_exactly_the_selection() {
        let mut state = State::default();
        for id in ["a", "b"] {
            state.mods.insert(id.into(), InstalledMod { folder: id.into(), hash: "h".into(), damaged: true });
        }
        mark(&mut state, &["a".into(), "not-installed".into()], true);
        assert!(state.mods["a"].damaged && !state.mods["b"].damaged && state.base_damaged);
        mark(&mut state, &[], false);
        assert!(!state.mods["a"].damaged && !state.base_damaged);
    }
}

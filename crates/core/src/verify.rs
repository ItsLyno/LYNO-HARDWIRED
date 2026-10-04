//! Integrity check of an installed build and marking what needs repair.
//!
//! Each mod folder is compared file by file with the record taken when the
//! mod was unpacked (see [`crate::files`]), with the same filter as
//! [`crate::package::unpack`] plus [`rules::is_generated`]: the package never
//! contains generated files, but the game may write logs or caches into a mod
//! folder, and those are not damage. A changed settings file
//! ([`rules::is_settings`]) is not damage either: the game rewrites shipped
//! configs in place. It is reported apart, and a repair keeps it.
//!
//! A mod installed before records existed has only the package tree hash:
//! if it matches, the pass records the files; if not, the mod is reported as
//! [`Problem::Changed`] without detail, and the player decides.
//!
//! The base package shares the instance root with files MO2 rewrites on its
//! own (`ModOrganizer.ini`, profile settings), so only missing base files
//! count as damage.
//!
//! A damaged mod is repaired as a whole: a package is one `tar.zst` stream,
//! there is no way to download one file of it. Repair itself is an update:
//! [`mark`] flags the selection in `state.json` and [`crate::plan::plan`]
//! turns the flags into [`crate::plan::Action::Repair`] and
//! [`crate::plan::Action::Base`], so it is resumable and crash-safe like any
//! update.

use std::collections::{BTreeMap, HashSet};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;

use crate::files::{self, FileList};
use crate::install::{is_player_file, Event};
use crate::mo2::Instance;
use crate::rules;
use crate::state::State;
use crate::tree::{self, FileEntry};
use crate::{Error, Result};

/// Paths listed per kind of problem; the rest is only counted.
const LISTED_FILES: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    /// Mods checked, not counting the base package.
    pub checked: usize,
    pub damaged: Vec<Damaged>,
    /// Mods (damaged or not) with settings files the player changed.
    pub customized: Vec<Customized>,
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
#[serde(rename_all = "camelCase")]
pub struct Customized {
    pub id: String,
    pub folder: String,
    pub files: Files,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Problem {
    /// The mod folder is gone.
    MissingFolder,
    /// No file record (installed by an older launcher) and the tree hash
    /// doesn't match: files were added, removed or changed, settings included.
    Changed,
    /// Paths relative to the mod folder (the instance root for the base package).
    Files { missing: Files, changed: Files, added: Files },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Files {
    pub count: usize,
    /// The first [`LISTED_FILES`] paths.
    pub sample: Vec<String>,
}

impl Files {
    fn of(paths: Vec<String>) -> Self {
        Self { count: paths.len(), sample: paths.into_iter().take(LISTED_FILES).collect() }
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Checks every installed build mod and the base package. Reads every file
/// of every build mod, so it takes as long as reading the whole build from
/// disk; progress arrives as [`Event::Step`] per mod and [`Event::Bytes`].
/// Writes file records for intact mods that have none.
pub fn verify(inst: &Instance, state: &State, cancel: &AtomicBool, on: &mut dyn FnMut(Event)) -> Result<Report> {
    let mut damaged = Vec::new();
    let mut customized = Vec::new();

    let missing: Vec<String> = state
        .base_files
        .iter()
        .filter(|rel| !is_player_file(rel) && !tree::from_slash(inst.root(), rel).is_file())
        .cloned()
        .collect();
    if !missing.is_empty() {
        damaged.push(Damaged {
            id: None,
            folder: String::new(),
            problem: Problem::Files { missing: Files::of(missing), changed: Files::default(), added: Files::default() },
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
        let mut current = Vec::with_capacity(files.len());
        for f in files {
            let path = tree::from_slash(&dir, &f.path);
            let start = done;
            let hash = hash_file(&path, cancel, &mut |n| on(Event::Bytes { done: start + n, total: total_bytes }))?;
            done = start + f.size;
            current.push((f, hash));
        }

        let Some(record) = files::load(inst, id, &m.hash) else {
            if tree::combine_pairs(&current).hash == m.hash {
                files::save(inst, &FileList::new(id, &m.hash, &current))?;
            } else {
                damaged.push(Damaged { id: Some(id.clone()), folder: m.folder.clone(), problem: Problem::Changed });
            }
            continue;
        };
        let diff = compare(&record, &current);
        if !diff.settings.is_empty() {
            customized.push(Customized { id: id.clone(), folder: m.folder.clone(), files: Files::of(diff.settings) });
        }
        if !(diff.missing.is_empty() && diff.changed.is_empty() && diff.added.is_empty()) {
            let problem =
                Problem::Files { missing: Files::of(diff.missing), changed: Files::of(diff.changed), added: Files::of(diff.added) };
            damaged.push(Damaged { id: Some(id.clone()), folder: m.folder.clone(), problem });
        }
    }

    on(Event::Done);
    Ok(Report { checked: state.mods.len(), damaged, customized })
}

#[derive(Default)]
struct Diff {
    missing: Vec<String>,
    changed: Vec<String>,
    added: Vec<String>,
    /// Changed settings files: not damage.
    settings: Vec<String>,
}

/// A new settings file in a mod folder is ignored like a changed one: some
/// mods write their config next to themselves when it is first saved.
fn compare(record: &FileList, current: &[(FileEntry, String)]) -> Diff {
    let mut diff = Diff::default();
    let now: BTreeMap<&str, &str> = current.iter().map(|(f, h)| (f.path.as_str(), h.as_str())).collect();
    for (path, want) in &record.files {
        match now.get(path.as_str()) {
            None => diff.missing.push(path.clone()),
            Some(h) if *h == want.blake3 => {}
            Some(_) if rules::is_settings(path) => diff.settings.push(path.clone()),
            Some(_) => diff.changed.push(path.clone()),
        }
    }
    for path in now.keys() {
        if !record.files.contains_key(*path) && !rules::is_settings(path) {
            diff.added.push(path.to_string());
        }
    }
    diff
}

/// Flags exactly the selected mods (manifest ids) and, with `base`, the base
/// package for the next update to download again; clears every other flag.
/// With `reset_settings` the repair brings back the build's settings files
/// too. Ids that are not installed are ignored.
pub fn mark(state: &mut State, ids: &[String], base: bool, reset_settings: bool) {
    let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    for (id, m) in &mut state.mods {
        m.damaged = ids.contains(id.as_str());
        m.reset_settings = m.damaged && reset_settings;
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

    fn write(root: &Path, rel: &str, data: &str) {
        let p = tree::from_slash(root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    fn installed(inst: &Instance, id: &str, folder: &str, record: bool) -> InstalledMod {
        let files = tree::hash_files_with(&inst.mods_dir().join(folder), &rules::is_hashed).unwrap();
        let hash = tree::combine_pairs(&files).hash;
        if record {
            files::save(inst, &FileList::new(id, &hash, &files)).unwrap();
        }
        InstalledMod { folder: folder.into(), hash, damaged: false, reset_settings: false }
    }

    fn files_of(paths: &[&str]) -> Files {
        Files::of(paths.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn finds_damage_per_file_and_tells_settings_apart() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        write(dir.path(), "ModOrganizer.exe", "MZ");
        write(dir.path(), "ModOrganizer.ini", "[General]");
        for name in ["Intact", "Broken", "Gone", "Tuned"] {
            write(&inst.mods_dir(), &format!("{name}/archive/pc/mod/{name}.archive"), name);
            write(&inst.mods_dir(), &format!("{name}/bin/x64/plugins/{name}.dll"), name);
            write(&inst.mods_dir(), &format!("{name}/bin/x64/plugins/config.json"), "{}");
            write(&inst.mods_dir(), &format!("{name}/meta.ini"), "[General]");
        }
        let mut state = State {
            base_files: ["ModOrganizer.exe", "ModOrganizer.ini", "plugins/x.dll", "profiles/LYNO/UserSettings.json"]
                .map(String::from)
                .into(),
            ..Default::default()
        };
        for (id, name) in [("intact", "Intact"), ("broken", "Broken"), ("gone", "Gone"), ("tuned", "Tuned")] {
            state.mods.insert(id.into(), installed(&inst, id, name, true));
        }

        // MO2 and the game touch files that are not damage.
        write(&inst.mods_dir(), "Intact/meta.ini", "[General]\nlastNexusQuery=now");
        write(&inst.mods_dir(), "Intact/r6/logs/redscript.log", "log");
        write(&inst.mods_dir(), "Intact/r6/cache/final.redscripts", "cache");
        // The player changed settings in game.
        write(&inst.mods_dir(), "Tuned/bin/x64/plugins/config.json", "{\"fov\": 90}");
        write(&inst.mods_dir(), "Tuned/bin/x64/plugins/new-settings.json", "{}");
        // Antivirus ate a file, something replaced another and dropped a stray one,
        // the player changed a setting there too; a folder is gone.
        std::fs::remove_file(inst.mods_dir().join("Broken/archive/pc/mod/Broken.archive")).unwrap();
        write(&inst.mods_dir(), "Broken/bin/x64/plugins/Broken.dll", "patched");
        write(&inst.mods_dir(), "Broken/archive/pc/mod/stray.archive", "x");
        write(&inst.mods_dir(), "Broken/bin/x64/plugins/config.json", "{\"fov\": 80}");
        std::fs::remove_dir_all(inst.mods_dir().join("Gone")).unwrap();

        let mut events = Vec::new();
        let report = verify(&inst, &state, &AtomicBool::new(false), &mut |e| events.push(e)).unwrap();
        assert_eq!(report.checked, 4);
        assert_eq!(
            report.damaged,
            [
                Damaged {
                    id: None,
                    folder: String::new(),
                    problem: Problem::Files { missing: files_of(&["plugins/x.dll"]), changed: Files::default(), added: Files::default() },
                },
                Damaged { id: Some("gone".into()), folder: "Gone".into(), problem: Problem::MissingFolder },
                Damaged {
                    id: Some("broken".into()),
                    folder: "Broken".into(),
                    problem: Problem::Files {
                        missing: files_of(&["archive/pc/mod/Broken.archive"]),
                        changed: files_of(&["bin/x64/plugins/Broken.dll"]),
                        added: files_of(&["archive/pc/mod/stray.archive"]),
                    },
                },
            ]
        );
        let customized: Vec<_> = report.customized.iter().map(|c| (c.id.as_str(), c.files.sample.clone())).collect();
        assert_eq!(
            customized,
            [("broken", vec!["bin/x64/plugins/config.json".to_string()]), ("tuned", vec!["bin/x64/plugins/config.json".to_string()])]
        );
        assert!(matches!(events.last(), Some(Event::Done)));
        assert!(events.iter().any(|e| matches!(e, Event::Step { total: 3, .. })));

        let cancelled = verify(&inst, &state, &AtomicBool::new(true), &mut |_| {});
        assert!(matches!(cancelled, Err(Error::Cancelled)));
    }

    #[test]
    fn mods_without_records_get_them_when_intact() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        for name in ["Old", "OldBroken"] {
            write(&inst.mods_dir(), &format!("{name}/x.archive"), name);
            write(&inst.mods_dir(), &format!("{name}/config.json"), "{}");
        }
        let mut state = State::default();
        let old = installed(&inst, "old", "Old", false);
        state.mods.insert("old".into(), old.clone());
        state.mods.insert("old-broken".into(), installed(&inst, "old-broken", "OldBroken", false));
        // Without a record a changed setting can't be told from damage.
        write(&inst.mods_dir(), "OldBroken/config.json", "{\"fov\": 90}");

        let report = verify(&inst, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(
            report.damaged,
            [Damaged { id: Some("old-broken".into()), folder: "OldBroken".into(), problem: Problem::Changed }]
        );
        assert!(files::load(&inst, "old", &old.hash).is_some(), "intact mod recorded");
        assert!(files::load(&inst, "old-broken", &state.mods["old-broken"].hash).is_none());

        // With the record, the next check tells a setting from damage.
        write(&inst.mods_dir(), "Old/config.json", "{\"fov\": 90}");
        let report = verify(&inst, &state, &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(report.damaged.len(), 1);
        assert_eq!(report.customized[0].id, "old");
    }

    #[test]
    fn mark_flags_exactly_the_selection() {
        let mut state = State::default();
        for id in ["a", "b"] {
            state.mods.insert(id.into(), InstalledMod { folder: id.into(), hash: "h".into(), damaged: true, reset_settings: true });
        }
        mark(&mut state, &["a".into(), "not-installed".into()], true, true);
        assert!(state.mods["a"].damaged && state.mods["a"].reset_settings && state.base_damaged);
        assert!(!state.mods["b"].damaged && !state.mods["b"].reset_settings);
        mark(&mut state, &["a".into()], false, false);
        assert!(state.mods["a"].damaged && !state.mods["a"].reset_settings && !state.base_damaged);
    }
}

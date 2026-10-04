//! Per-file hashes of installed build mods, `<instance>/.lyno/files/*.json`.
//!
//! The manifest only has a tree hash per package: enough to tell a mod is
//! not intact, not which file. Per-file hashes in the manifest would make it
//! megabytes for a build of hundreds of mods, fetched on every launch, so the
//! launcher records them itself: [`crate::package::unpack`] hashes every file
//! anyway, and [`crate::verify`] records mods that predate this from its first
//! full pass. A record names the package it was taken from; one for another
//! package (the mod was updated since) is ignored.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::mo2::Instance;
use crate::tree::FileEntry;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileList {
    /// Manifest mod id.
    pub id: String,
    /// Tree hash of the package the files came from.
    pub package: String,
    /// Keyed by path relative to the mod folder, with `/`; hashed files only
    /// (see [`crate::rules::is_hashed`]).
    pub files: BTreeMap<String, FileHash>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileHash {
    pub size: u64,
    pub blake3: String,
}

impl FileList {
    pub fn new(id: &str, package: &str, files: &[(FileEntry, String)]) -> Self {
        let files = files.iter().map(|(f, h)| (f.path.clone(), FileHash { size: f.size, blake3: h.clone() })).collect();
        Self { id: id.to_owned(), package: package.to_owned(), files }
    }
}

/// Ids come from the author's `meta.ini` and may hold anything; the file
/// name is derived from a hash, the id is kept inside.
fn path(inst: &Instance, id: &str) -> PathBuf {
    let name = &blake3::hash(id.as_bytes()).to_hex()[..16];
    inst.root().join(".lyno").join("files").join(format!("{name}.json"))
}

/// The record of mod `id` taken from `package`; `None` if there is none, it
/// is unreadable or it belongs to another package.
pub fn load(inst: &Instance, id: &str, package: &str) -> Option<FileList> {
    let text = std::fs::read_to_string(path(inst, id)).ok()?;
    let list: FileList = serde_json::from_str(&text).ok()?;
    (list.id == id && list.package == package).then_some(list)
}

pub fn save(inst: &Instance, list: &FileList) -> Result<()> {
    let path = path(inst, &list.id);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_string(list)?).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, &path).map_err(|e| Error::io(&path, e))
}

pub fn remove(inst: &Instance, id: &str) -> Result<()> {
    let path = path(inst, id);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::io(&path, e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_stale_records() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        let entry = FileEntry { path: "a/b.archive".into(), size: 3, mtime: 0 };
        let list = FileList::new("odd/id:..", "pkg1", &[(entry, "h".into())]);
        save(&inst, &list).unwrap();

        assert_eq!(load(&inst, "odd/id:..", "pkg1"), Some(list));
        assert_eq!(load(&inst, "odd/id:..", "pkg2"), None, "record of an older package");
        assert_eq!(load(&inst, "other", "pkg1"), None);
        remove(&inst, "odd/id:..").unwrap();
        remove(&inst, "odd/id:..").unwrap();
        assert_eq!(load(&inst, "odd/id:..", "pkg1"), None);
    }
}

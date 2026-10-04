//! Tree hash of a folder: identifies a package by its unpacked content.
//!
//! `blake3( for each file sorted by path: "<path>\0<blake3 hex>\n" )`, paths
//! relative to the root with forward slashes. Empty folders don't count.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::hash::blake3_file;
use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    /// Relative path with forward slashes.
    pub path: String,
    pub size: u64,
    /// Modification time in nanoseconds since the Unix epoch, 0 if unknown.
    pub mtime: u64,
}

/// Regular files under `root`, sorted by relative path.
pub fn list_files(root: &Path) -> Result<Vec<FileEntry>> {
    list_files_with(root, &|_| true)
}

/// Like [`list_files`], keeping only paths for which `keep` returns true.
pub fn list_files_with(root: &Path, keep: &dyn Fn(&str) -> bool) -> Result<Vec<FileEntry>> {
    let mut out = Vec::new();
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|e| {
            let path = e.path().map(Path::to_path_buf).unwrap_or_else(|| root.to_path_buf());
            Error::io(path, e.into())
        })?;
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path().strip_prefix(root).expect("walkdir stays under root");
        let path = to_slash(rel);
        if !keep(&path) {
            continue;
        }
        let meta = entry.metadata().map_err(|e| Error::io(entry.path(), e.into()))?;
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        out.push(FileEntry { path, size: meta.len(), mtime });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeHash {
    pub hash: String,
    pub size: u64,
    pub files: usize,
}

pub fn tree_hash(root: &Path) -> Result<TreeHash> {
    tree_hash_with(root, &|_| true)
}

pub fn tree_hash_with(root: &Path, keep: &dyn Fn(&str) -> bool) -> Result<TreeHash> {
    Ok(combine_pairs(&hash_files_with(root, keep)?))
}

/// Files under `root` kept by `keep`, sorted by path, with their BLAKE3.
pub fn hash_files_with(root: &Path, keep: &dyn Fn(&str) -> bool) -> Result<Vec<(FileEntry, String)>> {
    list_files_with(root, keep)?
        .into_iter()
        .map(|f| {
            let h = blake3_file(&from_slash(root, &f.path))?;
            Ok((f, h))
        })
        .collect()
}

/// [`combine`] over the output of [`hash_files_with`].
pub fn combine_pairs(files: &[(FileEntry, String)]) -> TreeHash {
    combine(files.iter().map(|(f, h)| (f, h.as_str())))
}

/// Tree hash from per-file hashes; `files` must be sorted by path.
pub fn combine<'a>(files: impl IntoIterator<Item = (&'a FileEntry, &'a str)>) -> TreeHash {
    let mut hasher = blake3::Hasher::new();
    let (mut size, mut count) = (0, 0);
    for (f, h) in files {
        hasher.update(f.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(h.as_bytes());
        hasher.update(b"\n");
        size += f.size;
        count += 1;
    }
    TreeHash { hash: hasher.finalize().to_hex().to_string(), size, files: count }
}

pub fn to_slash(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

pub fn from_slash(root: &Path, rel: &str) -> PathBuf {
    rel.split('/').fold(root.to_path_buf(), |p, c| p.join(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, data: &str) {
        let p = from_slash(root, rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[test]
    fn hash_depends_on_content_and_paths_only() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(a.path(), "archive/pc/mod/x.archive", "x");
        write(a.path(), "meta.ini", "m");
        // Same files written in a different order, plus an empty dir.
        write(b.path(), "meta.ini", "m");
        write(b.path(), "archive/pc/mod/x.archive", "x");
        std::fs::create_dir_all(b.path().join("empty")).unwrap();

        let ha = tree_hash(a.path()).unwrap();
        assert_eq!(ha, tree_hash(b.path()).unwrap());
        assert_eq!((ha.size, ha.files), (2, 2));

        write(b.path(), "meta.ini", "changed");
        assert_ne!(ha.hash, tree_hash(b.path()).unwrap().hash);
    }

    #[test]
    fn lists_with_forward_slashes() {
        let a = tempfile::tempdir().unwrap();
        write(a.path(), "r6/scripts/a.reds", "1");
        let files = list_files(a.path()).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!((files[0].path.as_str(), files[0].size), ("r6/scripts/a.reds", 1));
        assert!(files[0].mtime > 0);
    }
}

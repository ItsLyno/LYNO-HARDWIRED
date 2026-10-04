//! Per-file hash cache for the author tool.
//!
//! A build has hundreds of mods and hundreds of gigabytes; reading all of it on
//! every `lyno-pack build` just to find the few mods that changed is what makes
//! it slow. A file whose size and modification time are unchanged keeps its
//! hash. Never used to verify downloads: [`crate::package::unpack`] always
//! hashes the real content.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::tree::FileEntry;
use crate::{Error, Result};

/// Saved at least this often, so an interrupted build keeps most of its work.
const SAVE_INTERVAL: Duration = Duration::from_secs(30);

pub struct HashCache {
    path: Option<PathBuf>,
    entries: HashMap<String, Cached>,
    dirty: bool,
    last_save: Instant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Cached {
    size: u64,
    mtime: u64,
    blake3: String,
}

impl HashCache {
    /// Loads the cache at `path`; a missing or unreadable file starts empty.
    pub fn load(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self { path: Some(path), entries, dirty: false, last_save: Instant::now() }
    }

    /// A cache that remembers nothing: every file is read.
    pub fn disabled() -> Self {
        Self { path: None, entries: HashMap::new(), dirty: false, last_save: Instant::now() }
    }

    /// `key` identifies the file across runs (path relative to the instance).
    pub fn get(&self, key: &str, f: &FileEntry) -> Option<&str> {
        self.entries
            .get(key)
            .filter(|c| f.mtime != 0 && c.size == f.size && c.mtime == f.mtime)
            .map(|c| c.blake3.as_str())
    }

    pub fn insert(&mut self, key: String, f: &FileEntry, blake3: String) {
        if self.path.is_none() || f.mtime == 0 {
            return;
        }
        self.entries.insert(key, Cached { size: f.size, mtime: f.mtime, blake3 });
        self.dirty = true;
    }

    pub fn save_if_due(&mut self) -> Result<()> {
        if self.last_save.elapsed() >= SAVE_INTERVAL {
            self.save()?;
        }
        Ok(())
    }

    pub fn save(&mut self) -> Result<()> {
        let Some(path) = self.path.as_deref().filter(|_| self.dirty) else { return Ok(()) };
        write_atomic(path, &serde_json::to_string(&self.entries)?)?;
        self.dirty = false;
        self.last_save = Instant::now();
        Ok(())
    }
}

fn write_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| Error::io(&tmp, e))?;
    std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(size: u64, mtime: u64) -> FileEntry {
        FileEntry { path: "a".into(), size, mtime }
    }

    #[test]
    fn hit_only_when_size_and_mtime_match_and_survives_reload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".lyno/pack-cache.json");
        let mut c = HashCache::load(&path);
        c.insert("mods/A/a".into(), &entry(5, 100), "h".into());
        assert_eq!(c.get("mods/A/a", &entry(5, 100)), Some("h"));
        assert_eq!(c.get("mods/A/a", &entry(6, 100)), None);
        assert_eq!(c.get("mods/A/a", &entry(5, 101)), None);
        assert_eq!(c.get("mods/B/a", &entry(5, 100)), None);
        c.save().unwrap();
        assert_eq!(HashCache::load(&path).get("mods/A/a", &entry(5, 100)), Some("h"));
    }

    #[test]
    fn unknown_mtime_and_disabled_never_hit() {
        let mut c = HashCache::load(tempfile::tempdir().unwrap().path().join("c.json"));
        c.insert("k".into(), &entry(5, 0), "h".into());
        assert_eq!(c.get("k", &entry(5, 0)), None);
        let mut d = HashCache::disabled();
        d.insert("k".into(), &entry(5, 1), "h".into());
        assert_eq!(d.get("k", &entry(5, 1)), None);
    }
}

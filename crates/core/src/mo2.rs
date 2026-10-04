//! A portable MO2 instance managed by the launcher.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::meta::ModMeta;
use crate::{Error, Result};

pub const EXE_NAME: &str = "ModOrganizer.exe";
/// Name of the executable entry MO2's Cyberpunk plugin registers.
pub const GAME_EXECUTABLE: &str = "Cyberpunk 2077";

#[derive(Debug, Clone)]
pub struct Instance {
    root: PathBuf,
}

impl Instance {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn exe(&self) -> PathBuf {
        self.root.join(EXE_NAME)
    }

    /// MO2 2.4+ treats a directory with `portable.txt` as a portable instance.
    pub fn is_portable(&self) -> bool {
        self.root.join("portable.txt").is_file()
    }

    pub fn is_installed(&self) -> bool {
        self.exe().is_file()
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.root.join("mods")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }

    pub fn overwrite_dir(&self) -> PathBuf {
        self.root.join("overwrite")
    }

    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.root.join("profiles").join(profile)
    }

    pub fn modlist_path(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("modlist.txt")
    }

    /// Reads `meta.ini` of every folder under `mods/`, keyed by folder name.
    /// Folders without a `meta.ini` (separators, hand-made mods) get defaults.
    pub fn scan_mods(&self) -> Result<BTreeMap<String, ModMeta>> {
        let dir = self.mods_dir();
        let mut out = BTreeMap::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            if !entry.file_type().map_err(|e| Error::io(entry.path(), e))?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta_path = entry.path().join("meta.ini");
            let meta = if meta_path.is_file() { ModMeta::load(&meta_path)? } else { ModMeta::default() };
            out.insert(name, meta);
        }
        Ok(out)
    }
}

/// Arguments for `ModOrganizer.exe` to start a configured executable
/// inside MO2's virtual file system.
///
/// `ModOrganizer.exe -p <profile> run -e <executable>`; when MO2 is already
/// open the command is forwarded to the running instance.
pub fn run_args(profile: &str, executable: &str) -> Vec<String> {
    vec!["-p".into(), profile.into(), "run".into(), "-e".into(), executable.into()]
}

/// Arguments to open MO2's own window on the given profile.
pub fn open_args(profile: &str) -> Vec<String> {
    vec!["-p".into(), profile.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_run_args() {
        assert_eq!(run_args("LYNO", GAME_EXECUTABLE), ["-p", "LYNO", "run", "-e", "Cyberpunk 2077"]);
    }

    #[test]
    fn scans_mod_folders() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        std::fs::create_dir_all(inst.mods_dir().join("CET")).unwrap();
        std::fs::create_dir_all(inst.mods_dir().join("Core_separator")).unwrap();
        std::fs::write(
            inst.mods_dir().join("CET/meta.ini"),
            "[General]\nmodid=107\n[LYNO]\nid=cet\n",
        )
        .unwrap();
        std::fs::write(inst.mods_dir().join("stray.txt"), "").unwrap();

        let mods = inst.scan_mods().unwrap();
        assert_eq!(mods.len(), 2);
        assert_eq!(mods["CET"].lyno_id.as_deref(), Some("cet"));
        assert_eq!(mods["Core_separator"], ModMeta::default());
    }
}

//! What the launcher has installed, kept at `<instance>/.lyno/state.json`.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    pub build_version: Option<String>,
    pub base_hash: Option<String>,
    /// Files of the installed base package, relative to the instance root.
    /// Lets an update remove what a newer base no longer has.
    #[serde(default)]
    pub base_files: Vec<String>,
    /// Keyed by manifest mod id.
    pub mods: BTreeMap<String, InstalledMod>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMod {
    pub folder: String,
    pub hash: String,
}

impl State {
    /// A missing file means nothing is installed yet.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => Ok(serde_json::from_str(&text)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(self)?).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
    }

    pub fn is_managed_folder(&self, folder: &str) -> bool {
        self.mods.values().any(|m| m.folder == folder)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_is_empty_and_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(".lyno/state.json");
        assert_eq!(State::load(&p).unwrap(), State::default());

        let mut s = State { build_version: Some("1.0".into()), ..Default::default() };
        s.mods.insert("cet".into(), InstalledMod { folder: "CET".into(), hash: "h".into() });
        s.save(&p).unwrap();
        assert_eq!(State::load(&p).unwrap(), s);
        assert!(s.is_managed_folder("CET"));
    }
}

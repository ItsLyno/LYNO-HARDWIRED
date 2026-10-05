//! What the launcher has installed, kept at `<instance>/.lyno/state.json`.

use std::collections::{BTreeMap, BTreeSet};
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
    /// A base file went missing (see [`crate::verify`]): the next update
    /// unpacks the base package again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub base_damaged: bool,
    /// Keyed by manifest mod id.
    pub mods: BTreeMap<String, InstalledMod>,
    /// Optional build mods the player removed (manifest ids): updates don't
    /// bring them back until the player asks (see [`crate::plan::is_removed`]).
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub removed: BTreeSet<String>,
    /// What the latest update changed, for "added / updated in 1.4.0" marks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_update: Option<LastUpdate>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LastUpdate {
    /// Build version before the update; `None` for the first install.
    pub from: Option<String>,
    pub to: String,
    /// Manifest ids.
    pub added: Vec<String>,
    pub updated: Vec<String>,
    /// Folder names: removed mods are no longer in the manifest.
    pub removed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledMod {
    pub folder: String,
    pub hash: String,
    /// The folder no longer matches `hash` (see [`crate::verify`]): the next
    /// update downloads the package again.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub damaged: bool,
    /// With `damaged`: the repair also brings back the build's settings files
    /// instead of keeping the player's (see [`crate::rules::is_settings`]).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reset_settings: bool,
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

    /// The record for an update to `version`. An interrupted update keeps its
    /// record, so resuming it (possibly towards a newer build) adds to it
    /// rather than forgetting the mods already done.
    pub fn begin_update(&mut self, version: &str) -> &mut LastUpdate {
        let unfinished = |u: &LastUpdate| u.to == version || self.build_version.as_deref() != Some(u.to.as_str());
        if !self.last_update.as_ref().is_some_and(unfinished) {
            self.last_update = Some(LastUpdate { from: self.build_version.clone(), ..Default::default() });
        }
        let u = self.last_update.as_mut().unwrap();
        u.to = version.to_owned();
        u
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
        s.mods.insert("cet".into(), InstalledMod { folder: "CET".into(), hash: "h".into(), damaged: false, reset_settings: false });
        s.save(&p).unwrap();
        assert_eq!(State::load(&p).unwrap(), s);
        assert!(s.is_managed_folder("CET"));
    }

    #[test]
    fn update_record_survives_interruption() {
        let mut s = State { build_version: Some("1.0".into()), ..Default::default() };
        s.begin_update("1.1").added.push("a".into());
        // Interrupted before 1.1 finished, resumed towards 1.2.
        s.begin_update("1.2").updated.push("b".into());
        assert_eq!(
            s.last_update,
            Some(LastUpdate {
                from: Some("1.0".into()),
                to: "1.2".into(),
                added: vec!["a".into()],
                updated: vec!["b".into()],
                removed: vec![]
            })
        );
        // 1.2 finished; the next update starts a new record.
        s.build_version = Some("1.2".into());
        s.begin_update("1.3");
        assert_eq!(s.last_update.as_ref().unwrap().from.as_deref(), Some("1.2"));
        assert!(s.last_update.as_ref().unwrap().added.is_empty());
    }
}

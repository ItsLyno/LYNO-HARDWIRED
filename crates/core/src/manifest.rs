//! The build manifest published by the build author (`manifest.json`).
//!
//! It never contains third-party mod files: mods are referenced by Nexus
//! ids, and `recipe` describes how files from the downloaded archive are
//! laid out inside the MO2 mod folder.

use std::collections::HashSet;
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    /// Build version shown to users, e.g. "1.4.0".
    pub build_version: String,
    /// Required `Cyberpunk2077.exe` file version, e.g. "3.0.78.57301".
    pub game_version: String,
    /// Minimum MO2 version, e.g. "2.5.3".
    pub mo2_version: String,
    /// MO2 profile the launcher manages.
    pub profile: String,
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
    /// In MO2 UI order (lowest priority first).
    pub mods: Vec<ModEntry>,
    /// Author-owned files (configs, ini, plugin settings) hosted by the author.
    #[serde(default)]
    pub own_files: Vec<OwnFile>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogEntry {
    pub version: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModEntry {
    Separator { title: String },
    Mod(ModSpec),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSpec {
    /// Stable id across build versions (survives renames).
    pub id: String,
    /// Folder name under `mods/`.
    pub name: String,
    pub enabled: bool,
    pub version: Option<String>,
    pub author: Option<String>,
    pub nexus: NexusSource,
    #[serde(default)]
    pub recipe: Vec<RecipeItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusSource {
    pub game: String,
    pub mod_id: u64,
    pub file_id: u64,
    pub file_name: String,
    pub size: u64,
    pub md5: Option<String>,
}

impl NexusSource {
    pub fn page_url(&self) -> String {
        format!("https://www.nexusmods.com/{}/mods/{}", self.game, self.mod_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeItem {
    /// Path inside the archive (forward slashes).
    pub from: String,
    /// Path inside the MO2 mod folder (forward slashes).
    pub to: String,
    pub blake3: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnFile {
    /// Path relative to the MO2 instance root.
    pub path: String,
    pub url: String,
    pub blake3: String,
    pub size: u64,
}

impl Manifest {
    pub fn from_json(text: &str) -> Result<Self> {
        let m: Self = serde_json::from_str(text)?;
        m.validate()?;
        Ok(m)
    }

    pub fn mod_specs(&self) -> impl Iterator<Item = &ModSpec> {
        self.mods.iter().filter_map(|e| match e {
            ModEntry::Mod(m) => Some(m),
            ModEntry::Separator { .. } => None,
        })
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA_VERSION {
            return Err(Error::Manifest(format!(
                "unsupported schema {} (launcher supports {SCHEMA_VERSION})",
                self.schema
            )));
        }
        let mut ids = HashSet::new();
        let mut names = HashSet::new();
        for m in self.mod_specs() {
            if !ids.insert(m.id.as_str()) {
                return Err(Error::Manifest(format!("duplicate mod id {:?}", m.id)));
            }
            if !names.insert(m.name.to_lowercase()) {
                return Err(Error::Manifest(format!("duplicate mod folder {:?}", m.name)));
            }
            if !is_safe_relative(&m.name) || m.name.contains(['/', '\\']) {
                return Err(Error::Manifest(format!("bad mod folder name {:?}", m.name)));
            }
            for r in &m.recipe {
                if !is_safe_relative(&r.to) {
                    return Err(Error::Manifest(format!("mod {:?}: unsafe path {:?}", m.id, r.to)));
                }
            }
        }
        for f in &self.own_files {
            if !is_safe_relative(&f.path) {
                return Err(Error::Manifest(format!("unsafe own file path {:?}", f.path)));
            }
        }
        Ok(())
    }
}

/// Rejects absolute paths and `..` so a manifest can't write outside the instance.
fn is_safe_relative(p: &str) -> bool {
    let normalized = p.replace('\\', "/");
    if normalized.is_empty() || normalized.contains(':') {
        return false;
    }
    Path::new(&normalized)
        .components()
        .all(|c| matches!(c, Component::Normal(_)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(id: &str, file_id: u64) -> ModSpec {
        ModSpec {
            id: id.into(),
            name: id.into(),
            enabled: true,
            version: Some("1.0".into()),
            author: None,
            nexus: NexusSource {
                game: "cyberpunk2077".into(),
                mod_id: 1,
                file_id,
                file_name: format!("{id}.zip"),
                size: 10,
                md5: None,
            },
            recipe: vec![],
        }
    }

    fn manifest(mods: Vec<ModEntry>) -> Manifest {
        Manifest {
            schema: SCHEMA_VERSION,
            name: "LYNO".into(),
            build_version: "1.0.0".into(),
            game_version: "2.21".into(),
            mo2_version: "2.5.3".into(),
            profile: "LYNO".into(),
            changelog: vec![],
            mods,
            own_files: vec![],
        }
    }

    #[test]
    fn json_roundtrip() {
        let m = manifest(vec![
            ModEntry::Separator { title: "Core".into() },
            ModEntry::Mod(spec("cet", 1)),
        ]);
        let json = serde_json::to_string_pretty(&m).unwrap();
        assert!(json.contains("\"kind\": \"separator\""));
        assert_eq!(Manifest::from_json(&json).unwrap(), m);
    }

    #[test]
    fn rejects_duplicates_and_traversal() {
        let dup = manifest(vec![ModEntry::Mod(spec("a", 1)), ModEntry::Mod(spec("a", 2))]);
        assert!(dup.validate().is_err());

        let mut bad = spec("b", 1);
        bad.recipe.push(RecipeItem {
            from: "x".into(),
            to: "../../evil.dll".into(),
            blake3: String::new(),
            size: 0,
        });
        assert!(manifest(vec![ModEntry::Mod(bad)]).validate().is_err());

        assert!(!is_safe_relative("C:/Windows"));
        assert!(!is_safe_relative("/etc"));
        assert!(is_safe_relative("archive/pc/mod/x.archive"));
    }
}

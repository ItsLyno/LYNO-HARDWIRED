//! The build manifest (`build/manifest.json` in the repository).
//!
//! The whole build is distributed through GitHub Releases: a base package
//! (portable MO2 + instance config) and one package per mod. Packages are
//! content-addressed by their tree hash, so an unchanged mod keeps pointing
//! at the asset uploaded with an older release.

use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path};

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

pub const SCHEMA_VERSION: u32 = 4;

/// The MO2 profile of the Coop mod set: the author's, from which `publish` records `ModSpec::coop`,
/// and the one the launcher makes next to the build's to start the game in Coop.
pub const COOP_PROFILE: &str = "Coop";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema: u32,
    pub name: String,
    /// Build version shown to users, e.g. "1.4.0".
    pub build_version: String,
    /// Required `Cyberpunk2077.exe` version, e.g. "2.31".
    pub game_version: String,
    /// The patch read from the author's `Cyberpunk2077.exe` at build time
    /// ([`crate::game::ExeVersion::patch`]): the launcher compares the player's exe
    /// with it, so the check doesn't depend on how `game_version` was typed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game_exe: Option<String>,
    pub mo2_version: String,
    /// MO2 profile the build lives in.
    pub profile: String,
    /// The build has REDmod mods (`mods/<name>/info.json`): the game must be
    /// started with `-modded` so MO2 deploys them, and the free REDmod DLC
    /// must be installed.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub redmod: bool,
    #[serde(default)]
    pub changelog: Vec<ChangelogEntry>,
    /// Portable MO2 and instance config, unpacked into the instance root.
    pub base: Package,
    /// In MO2 UI order (lowest priority first).
    pub mods: Vec<ModEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangelogEntry {
    pub version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ModEntry {
    Separator {
        title: String,
        /// `#rrggbb` from the separator's `meta.ini` in the author's MO2. Old launchers ignore it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        color: Option<String>,
    },
    Mod(ModSpec),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModSpec {
    /// Stable id across build versions (survives folder renames).
    pub id: String,
    /// Folder name under `mods/`.
    pub name: String,
    pub enabled: bool,
    /// The player may switch the mod on or off; `enabled` is only the default.
    /// The choice lives in the player's `modlist.txt` and survives updates
    /// (see [`crate::plan::is_enabled`]). False only for core mods: `[LYNO]
    /// core=true` on the mod or on its separator in the author's MO2.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub optional: bool,
    /// On in the build's [`COOP_PROFILE`], as the author set it in their MO2 profile of that name.
    /// `None` in builds without one. Unlike `enabled` the player can't change it: in Coop everybody
    /// plays the same set, see [`crate::plan::coop_modlist`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub coop: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Confirmed hard dependencies, by stable build mod id. Nexus suggestions are separate.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Title on Nexus, when it differs from the folder name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nexus: Option<NexusRef>,
    pub package: Package,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NexusRef {
    pub game: String,
    pub mod_id: u64,
}

impl NexusRef {
    pub fn url(&self) -> String {
        format!("https://www.nexusmods.com/{}/mods/{}", self.game, self.mod_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    /// Tree hash of the unpacked folder (see [`crate::tree`]), without
    /// `meta.ini` (see [`crate::rules::is_hashed`]).
    pub hash: String,
    /// Unpacked size in bytes.
    pub size: u64,
    /// `tar.zst` stream split into release assets, in order.
    pub parts: Vec<Part>,
}

impl Package {
    pub fn download_size(&self) -> u64 {
        self.parts.iter().map(|p| p.size).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Part {
    pub url: String,
    pub size: u64,
    pub blake3: String,
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

    /// The build has a Coop mod set (the author's Coop profile was there when it was made).
    pub fn has_coop(&self) -> bool {
        self.mod_specs().any(|m| m.coop.is_some())
    }

    /// Folders of the build's separators in `mods/` (`<title>_separator`).
    pub fn separator_folders(&self) -> BTreeSet<String> {
        self.mods
            .iter()
            .filter_map(|e| match e {
                ModEntry::Separator { title, .. } => Some(crate::modlist::Entry::separator(title).name),
                ModEntry::Mod(_) => None,
            })
            .collect()
    }

    pub fn validate(&self) -> Result<()> {
        if self.schema != SCHEMA_VERSION && self.schema != 3 {
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
            if !is_safe_folder_name(&m.name) {
                return Err(Error::Manifest(format!("bad mod folder name {:?}", m.name)));
            }
        }
        validate_dependencies(self.mod_specs().map(|m| (m.id.as_str(), m.requires.as_slice())))?;
        for e in &self.mods {
            if let ModEntry::Separator { title, color } = e {
                if !is_safe_folder_name(title) {
                    return Err(Error::Manifest(format!("bad separator title {:?}", title)));
                }
                // The UI puts it into a style attribute: only a plain hex color gets through.
                if color.as_deref().is_some_and(|c| crate::meta::parse_hex_color(c).is_none()) {
                    return Err(Error::Manifest(format!("bad separator color {:?}", color)));
                }
            }
        }
        Ok(())
    }
}

/// Reject broken references and cycles before author metadata or a release is saved.
pub fn validate_dependencies<'a>(mods: impl Iterator<Item = (&'a str, &'a [String])>) -> Result<()> {
    let mut graph = std::collections::HashMap::new();
    for (id, requires) in mods {
        if id.trim().is_empty() || graph.insert(id, requires).is_some() {
            return Err(Error::Manifest(format!("Повторяющийся или пустой ID мода: {id}")));
        }
    }
    for (&id, requires) in &graph {
        let mut unique = HashSet::new();
        for dep in *requires {
            if !graph.contains_key(dep.as_str()) {
                return Err(Error::Manifest(format!("{id}: отсутствует обязательный мод {dep}")));
            }
            if !unique.insert(dep) {
                return Err(Error::Manifest(format!("{id}: повторяющаяся зависимость {dep}")));
            }
        }
        let mut pending: Vec<&str> = requires.iter().map(String::as_str).collect();
        let mut visited = HashSet::new();
        while let Some(dep) = pending.pop() {
            if dep == id {
                return Err(Error::Manifest(format!("Циклическая зависимость: {id}")));
            }
            if visited.insert(dep) {
                if let Some(next) = graph.get(dep) { pending.extend(next.iter().map(String::as_str)); }
            }
        }
    }
    Ok(())
}

/// A single path component: no separators, no `..`, no drive letters.
fn is_safe_folder_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains(['/', '\\', ':'])
        && matches!(Path::new(name).components().next(), Some(Component::Normal(_)))
        && Path::new(name).components().count() == 1
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn package(hash: &str) -> Package {
        Package {
            hash: hash.into(),
            size: 10,
            parts: vec![Part { url: format!("https://example.invalid/{hash}"), size: 5, blake3: "x".into() }],
        }
    }

    pub fn spec(id: &str, hash: &str) -> ModSpec {
        ModSpec {
            id: id.into(),
            name: format!("{id} folder"),
            enabled: true,
            optional: false,
            coop: None,
            description: None,
            requires: vec![],
            version: Some("1.0".into()),
            author: Some("someone".into()),
            title: None,
            nexus: Some(NexusRef { game: "cyberpunk2077".into(), mod_id: 107 }),
            package: package(hash),
        }
    }

    pub fn manifest(mods: Vec<ModEntry>) -> Manifest {
        Manifest {
            schema: SCHEMA_VERSION,
            name: "LYNO".into(),
            build_version: "1.0.0".into(),
            game_version: "2.31".into(),
            game_exe: None,
            mo2_version: "2.5.2".into(),
            profile: "LYNO".into(),
            redmod: false,
            changelog: vec![],
            base: package("base"),
            mods,
        }
    }

    #[test]
    fn json_roundtrip() {
        let m = manifest(vec![ModEntry::Separator { title: "Core".into(), color: None }, ModEntry::Mod(spec("cet", "h1"))]);
        let json = serde_json::to_string_pretty(&m).unwrap();
        assert!(json.contains("\"kind\": \"separator\""));
        assert_eq!(Manifest::from_json(&json).unwrap(), m);
    }

    #[test]
    fn rejects_duplicates_and_traversal() {
        let dup = manifest(vec![ModEntry::Mod(spec("a", "1")), ModEntry::Mod(spec("a", "2"))]);
        assert!(dup.validate().is_err());

        let mut bad = spec("b", "1");
        bad.name = "../evil".into();
        assert!(manifest(vec![ModEntry::Mod(bad)]).validate().is_err());

        assert!(!is_safe_folder_name(".."));
        assert!(!is_safe_folder_name("C:"));
        assert!(!is_safe_folder_name("a/b"));
        assert!(is_safe_folder_name("Cyber Engine Tweaks"));
    }

    #[test]
    fn nexus_url() {
        let n = NexusRef { game: "cyberpunk2077".into(), mod_id: 107 };
        assert_eq!(n.url(), "https://www.nexusmods.com/cyberpunk2077/mods/107");
    }
}

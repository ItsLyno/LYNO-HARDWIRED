//! Build-author side: turns an MO2 instance into a manifest + release assets.

use std::path::{Path, PathBuf};

use crate::manifest::{ChangelogEntry, Manifest, ModEntry, ModSpec, NexusRef, Package, Part, SCHEMA_VERSION};
use crate::meta::ModMeta;
use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::package::{self, PackOptions, PackedPart};
use crate::tree;
use crate::Result;

/// Top-level instance folders that never go into the base package.
const BASE_EXCLUDED: &[&str] = &["mods", "downloads", "overwrite", "logs", "crashDumps", "webcache", ".lyno"];

pub struct BuildOptions {
    pub name: String,
    pub profile: String,
    pub build_version: String,
    pub game_version: String,
    pub mo2_version: String,
    /// Where the assets will be downloadable from, e.g.
    /// `https://github.com/<owner>/<repo>/releases/download/build-1.4.0`.
    pub base_url: String,
    pub out_dir: PathBuf,
    /// Previously published manifest: unchanged packages are reused.
    pub previous: Option<Manifest>,
    pub changelog: Vec<ChangelogEntry>,
    pub pack: PackOptions,
}

/// Extra info for a mod, e.g. from the Nexus API.
#[derive(Debug, Clone, Default)]
pub struct ModInfo {
    pub author: Option<String>,
    pub title: Option<String>,
}

pub struct BuildOutput {
    pub manifest: Manifest,
    /// Newly written assets to upload to the release.
    pub assets: Vec<PackedPart>,
    pub warnings: Vec<String>,
}

pub fn build(
    root: &Path,
    opts: &BuildOptions,
    info: &mut dyn FnMut(&ModMeta) -> ModInfo,
    log: &mut dyn FnMut(&str),
) -> Result<BuildOutput> {
    let inst = Instance::new(root);
    let list = ModList::load(&inst.modlist_path(&opts.profile))?;
    let metas = inst.scan_mods()?;
    let mut assets = Vec::new();
    let mut warnings = Vec::new();

    let profile = opts.profile.clone();
    // Never pack our own output if it sits inside the instance.
    let out_rel = std::path::absolute(&opts.out_dir)
        .ok()
        .zip(std::path::absolute(root).ok())
        .and_then(|(out, root)| out.strip_prefix(root).ok().map(tree::to_slash))
        .filter(|r| !r.is_empty());
    let keep_base = move |p: &str| {
        if out_rel.as_deref().is_some_and(|o| p == o || p.starts_with(&format!("{o}/"))) {
            return false;
        }
        let mut parts = p.split('/');
        let first = parts.next().unwrap_or_default();
        if BASE_EXCLUDED.iter().any(|x| x.eq_ignore_ascii_case(first)) {
            return false;
        }
        if first != "profiles" {
            return true;
        }
        // Only the build's profile (other profiles are the author's own),
        // and without modlist.txt: the manifest defines the load order.
        parts.next() == Some(profile.as_str()) && parts.next().is_some_and(|f| !f.eq_ignore_ascii_case("modlist.txt"))
    };
    log("base: hashing");
    let base_hash = tree::tree_hash_with(root, &keep_base)?;
    let previous_base = opts.previous.as_ref().map(|m| &m.base).filter(|b| b.hash == base_hash.hash);
    let base = match previous_base {
        Some(b) => b.clone(),
        None => {
            log("base: packing");
            let files = tree::list_files_with(root, &keep_base)?;
            let (pkg, parts) = pack_one(root, &files, "base", &base_hash, opts)?;
            assets.extend(parts);
            pkg
        }
    };

    let mut mods = Vec::new();
    for entry in &list.entries {
        if entry.state == EntryState::Unmanaged {
            continue;
        }
        if let Some(title) = entry.separator_title() {
            mods.push(ModEntry::Separator { title: title.to_owned() });
            continue;
        }
        let folder = inst.mods_dir().join(&entry.name);
        if !folder.is_dir() {
            warnings.push(format!("{:?} is in modlist.txt but has no folder, skipped", entry.name));
            continue;
        }
        let meta = metas.get(&entry.name).cloned().unwrap_or_default();
        let id = meta.lyno_id.clone().unwrap_or_else(|| slug(&entry.name));

        log(&format!("{}: hashing", entry.name));
        let hash = tree::tree_hash(&folder)?;
        let reused = opts
            .previous
            .as_ref()
            .and_then(|m| m.mod_specs().find(|s| s.id == id))
            .map(|s| s.package.clone())
            .filter(|p| p.hash == hash.hash);
        let package = match reused {
            Some(p) => p,
            None => {
                log(&format!("{}: packing {} MB", entry.name, hash.size / 1_000_000));
                let files = tree::list_files(&folder)?;
                let (pkg, parts) = pack_one(&folder, &files, &id, &hash, opts)?;
                assets.extend(parts);
                pkg
            }
        };

        let nexus = match (meta.mod_id, meta.game_name.as_deref()) {
            (Some(mod_id), game) => Some(NexusRef { game: game.unwrap_or("cyberpunk2077").to_lowercase(), mod_id }),
            (None, _) => {
                warnings.push(format!("{:?}: no Nexus mod id in meta.ini, no link in the launcher", entry.name));
                None
            }
        };
        let extra = if nexus.is_some() { info(&meta) } else { ModInfo::default() };
        mods.push(ModEntry::Mod(ModSpec {
            id,
            name: entry.name.clone(),
            enabled: entry.state == EntryState::Enabled,
            version: meta.version.clone(),
            author: extra.author,
            title: extra.title.filter(|t| *t != entry.name),
            nexus,
            package,
        }));
    }

    let manifest = Manifest {
        schema: SCHEMA_VERSION,
        name: opts.name.clone(),
        build_version: opts.build_version.clone(),
        game_version: opts.game_version.clone(),
        mo2_version: opts.mo2_version.clone(),
        profile: opts.profile.clone(),
        changelog: opts.changelog.clone(),
        base,
        mods,
    };
    manifest.validate()?;
    Ok(BuildOutput { manifest, assets, warnings })
}

fn pack_one(
    root: &Path,
    files: &[tree::FileEntry],
    id: &str,
    hash: &tree::TreeHash,
    opts: &BuildOptions,
) -> Result<(Package, Vec<PackedPart>)> {
    let name = format!("{id}-{}", &hash.hash[..16]);
    let parts = package::pack(root, files, &opts.out_dir, &name, &opts.pack)?;
    let base_url = opts.base_url.trim_end_matches('/');
    let pkg = Package {
        hash: hash.hash.clone(),
        size: hash.size,
        parts: parts
            .iter()
            .map(|p| Part { url: format!("{base_url}/{}", p.file_name), size: p.size, blake3: p.blake3.clone() })
            .collect(),
    };
    Ok((pkg, parts))
}

/// Folder name → stable, URL-safe id.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() {
        // Non-latin folder names: fall back to a hash.
        blake3::hash(name.as_bytes()).to_hex()[..12].to_owned()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(slug("Cyber Engine Tweaks (CET)"), "cyber-engine-tweaks-cet");
        assert_eq!(slug("Мой мод").len(), 12);
    }
}

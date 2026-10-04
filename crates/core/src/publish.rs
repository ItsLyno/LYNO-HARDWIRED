//! Build-author side: turns an MO2 instance into a manifest + release assets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::download::Downloader;
use crate::hash_cache::HashCache;
use crate::manifest::{ChangelogEntry, Manifest, ModEntry, ModSpec, NexusRef, Package, Part, SCHEMA_VERSION};
use crate::meta::ModMeta;
use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::package::{self, PackOptions, PackedPart};
use crate::tree::FileEntry;
use crate::{rules, tree};
use crate::{Error, Result};

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
    /// Remember file hashes in `<instance>/.lyno/pack-cache.json` between runs.
    pub hash_cache: bool,
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
    /// Packages that had to be repacked, with the reason.
    pub repacked: Vec<Repacked>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repacked {
    /// Mod folder name, or `base`.
    pub name: String,
    /// False: not in the previous manifest at all.
    pub changed: bool,
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
    let mut warnings = Vec::new();
    let cache = if opts.hash_cache { HashCache::load(root.join(".lyno").join("pack-cache.json")) } else { HashCache::disabled() };
    let mut packer = Packer { opts, cache, assets: Vec::new(), repacked: Vec::new() };
    let steps = 1 + list.entries.iter().filter(|e| e.state != EntryState::Unmanaged && e.separator_title().is_none()).count();

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
        if rules::is_generated(p) {
            return false;
        }
        let (first, rest) = p.split_once('/').unwrap_or((p, ""));
        if BASE_EXCLUDED.iter().any(|x| x.eq_ignore_ascii_case(first)) {
            return false;
        }
        if first != "profiles" {
            return true;
        }
        // Only the build's profile (other profiles are the author's own),
        // without the author's private files.
        rest.split_once('/')
            .is_some_and(|(name, file)| name == profile && !rules::is_private_profile_file(file))
    };
    warnings.extend(profile_warnings(&inst.profile_dir(&opts.profile)));
    let files = tree::list_files_with(root, &keep_base)?;
    let previous_base = opts.previous.as_ref().map(|m| &m.base);
    let base_label = format!("[1/{steps}] base ({})", base_breakdown(&files));
    let base = packer.package(root, "", &files, "base", "base", &base_label, previous_base, log)?;

    let mut mods = Vec::new();
    let mut redmod = false;
    let mut step = 1;
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
        step += 1;
        let meta = metas.get(&entry.name).cloned().unwrap_or_default();
        let id = meta.lyno_id.clone().unwrap_or_else(|| slug(&entry.name));

        let (generated, files): (Vec<_>, Vec<_>) =
            tree::list_files(&folder)?.into_iter().partition(|f| rules::is_generated(&f.path));
        if let Some(first) = generated.first() {
            warnings.push(format!(
                "{:?}: {} generated file(s) not shipped (logs, r6/cache, load order), e.g. {}",
                entry.name,
                generated.len(),
                first.path
            ));
        }
        redmod |= entry.state == EntryState::Enabled && has_redmod(&folder);

        let previous = opts.previous.as_ref().and_then(|m| m.mod_specs().find(|s| s.id == id)).map(|s| &s.package);
        let prefix = format!("mods/{}/", entry.name);
        let step_label = format!("[{step}/{steps}] {}", entry.name);
        let package = packer.package(&folder, &prefix, &files, &id, &entry.name, &step_label, previous, log)?;

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
            optional: meta.lyno_optional,
            version: meta.version.clone(),
            author: extra.author,
            title: extra.title.filter(|t| *t != entry.name),
            nexus,
            package,
        }));
    }

    warnings.extend(overwrite_warning(&inst.overwrite_dir())?);
    packer.cache.save()?;
    let Packer { assets, repacked, .. } = packer;
    remove_stray_files(&opts.out_dir, &assets)?;

    let manifest = Manifest {
        schema: SCHEMA_VERSION,
        name: opts.name.clone(),
        build_version: opts.build_version.clone(),
        game_version: opts.game_version.clone(),
        mo2_version: opts.mo2_version.clone(),
        profile: opts.profile.clone(),
        redmod,
        changelog: opts.changelog.clone(),
        base,
        mods,
    };
    manifest.validate()?;
    Ok(BuildOutput { manifest, assets, repacked, warnings })
}

/// Checks every part of `manifest` over HTTP, including parts reused from
/// older releases, before the manifest goes live. Returns one message per
/// broken part; empty means every player can download the build.
pub fn check_published(manifest: &Manifest, downloader: &Downloader, progress: &(dyn Fn(usize, usize) + Sync)) -> Vec<String> {
    let parts: Vec<&Part> = std::iter::once(&manifest.base)
        .chain(manifest.mod_specs().map(|m| &m.package))
        .flat_map(|p| &p.parts)
        .collect();
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let errors = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..CHECK_THREADS.min(parts.len()) {
            s.spawn(|| {
                while let Some(part) = parts.get(next.fetch_add(1, Ordering::Relaxed)) {
                    if let Err(e) = downloader.check_part(part) {
                        errors.lock().unwrap().push(e.to_string());
                    }
                    progress(done.fetch_add(1, Ordering::Relaxed) + 1, parts.len());
                }
            });
        }
    });
    let mut errors = errors.into_inner().unwrap();
    errors.sort();
    errors
}

const CHECK_THREADS: usize = 8;

/// Size of the base package by top-level entry, largest first. The base is
/// everything in the instance root outside `BASE_EXCLUDED`, so a stray folder
/// there (a game copy, old output, backups) silently becomes gigabytes to read
/// and upload on every build.
fn base_breakdown(files: &[FileEntry]) -> String {
    let mut sizes: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for f in files {
        let top = f.path.split_once('/').map_or(f.path.as_str(), |(first, _)| first);
        *sizes.entry(top).or_default() += f.size;
    }
    let mut sizes: Vec<_> = sizes.into_iter().collect();
    sizes.sort_by_key(|(_, size)| std::cmp::Reverse(*size));
    let total: u64 = sizes.iter().map(|(_, s)| s).sum();
    let top: Vec<String> = sizes.iter().take(5).map(|(name, s)| format!("{name} {} MB", s / 1_000_000)).collect();
    format!("{} files, {} MB; largest: {}", files.len(), total / 1_000_000, top.join(", "))
}

/// REDmod mods keep their content in `mods/<name>/` with an `info.json`.
fn has_redmod(folder: &Path) -> bool {
    std::fs::read_dir(folder.join("mods"))
        .into_iter()
        .flatten()
        .flatten()
        .any(|e| e.path().join("info.json").is_file())
}

/// Files the game created in `overwrite/` that are not regenerated on their
/// own: usually mod settings, which only ship once moved into a mod.
fn overwrite_warning(dir: &Path) -> Result<Option<String>> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let files = tree::list_files_with(dir, &|p| !rules::is_generated(p))?;
    Ok(files.first().map(|first| {
        format!(
            "overwrite/ has {} file(s) that are not shipped, e.g. {}. If these are mod settings, \
             move them into a mod (MO2: right-click Overwrite > Create Mod)",
            files.len(),
            first.path
        )
    }))
}

/// Profile options whose files stay on the author's machine.
fn profile_warnings(dir: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(dir.join("settings.ini")).unwrap_or_default();
    let enabled = |key: &str| {
        text.lines()
            .filter_map(|l| l.trim().split_once('='))
            .any(|(k, v)| k.trim() == key && v.trim().eq_ignore_ascii_case("true"))
    };
    let mut out = Vec::new();
    if enabled("LocalSettings") {
        out.push(
            "profile uses profile-specific game settings: UserSettings.json is not shipped \
             (graphics depend on the player's hardware), disable it in the profile options"
                .to_owned(),
        );
    }
    if enabled("LocalSaves") {
        out.push("profile uses profile-specific saves: saves/ is not shipped, disable it in the profile options".to_owned());
    }
    out
}

/// Produces packages while doing as little disk work as possible: the author's
/// disk, not the CPU, is what makes a build of hundreds of mods slow.
///
/// - A package whose hash matches the previous manifest is reused (nothing to upload).
/// - Hashes come from the [`HashCache`] when files are unchanged, so an
///   unchanged mod is not read at all.
/// - A new or changed package is read once: files are hashed while packed.
/// - Packages finished by an interrupted run (`<name>.parts.json` next to the
///   parts) are picked up instead of packed again.
struct Packer<'a> {
    opts: &'a BuildOptions,
    cache: HashCache,
    assets: Vec<PackedPart>,
    repacked: Vec<Repacked>,
}

/// Written next to the parts once a package is complete.
#[derive(Serialize, Deserialize)]
struct PartRecord {
    file_name: String,
    size: u64,
    blake3: String,
}

impl Packer<'_> {
    /// `files` are the shipped files under `root`, sorted; `prefix` is `root`
    /// relative to the instance (hash cache key).
    #[allow(clippy::too_many_arguments)]
    fn package(
        &mut self,
        root: &Path,
        prefix: &str,
        files: &[FileEntry],
        id: &str,
        name: &str,
        step: &str,
        previous: Option<&Package>,
        log: &mut dyn FnMut(&str),
    ) -> Result<Package> {
        let hashed: Vec<&FileEntry> = files.iter().filter(|f| rules::is_hashed(&f.path)).collect();
        let mut known = self.cached_hash(prefix, &hashed);
        if known.is_none() && previous.is_some() {
            log(&format!("{step}: hashing {} MB", mb(&hashed)));
            known = Some(self.read_hash(root, prefix, &hashed)?);
        }
        if let Some(hash) = &known {
            if let Some(p) = previous.filter(|p| p.hash == hash.hash) {
                return Ok(p.clone());
            }
            if let Some(parts) = self.resume(&package_name(id, hash)) {
                log(&format!("{step}: already packed by an earlier run"));
                self.repacked.push(Repacked { name: name.to_owned(), changed: previous.is_some() });
                return Ok(self.finish(hash, parts));
            }
        }

        log(&format!("{step}: packing {} MB", mb(&hashed)));
        let out = &self.opts.out_dir;
        // The final name needs the hash, which is only known once every file is read.
        let packed = package::pack(root, files, out, &format!("{id}.packing"), &self.opts.pack)?;
        for (f, h) in files.iter().zip(&packed.file_hashes) {
            self.cache.insert(format!("{prefix}{}", f.path), f, h.clone());
        }
        let hash = tree::combine(
            files.iter().zip(&packed.file_hashes).filter(|(f, _)| rules::is_hashed(&f.path)).map(|(f, h)| (f, h.as_str())),
        );
        let parts = rename_parts(out, packed.parts, &package_name(id, &hash))?;
        let records: Vec<PartRecord> = parts
            .iter()
            .map(|p| PartRecord { file_name: p.file_name.clone(), size: p.size, blake3: p.blake3.clone() })
            .collect();
        let record_path = out.join(format!("{}.parts.json", package_name(id, &hash)));
        std::fs::write(&record_path, serde_json::to_string_pretty(&records)?).map_err(|e| Error::io(&record_path, e))?;
        self.cache.save_if_due()?;
        self.repacked.push(Repacked { name: name.to_owned(), changed: previous.is_some() });
        Ok(self.finish(&hash, parts))
    }

    /// Tree hash from the cache alone, `None` if any file has to be read.
    fn cached_hash(&self, prefix: &str, files: &[&FileEntry]) -> Option<tree::TreeHash> {
        let hashes: Option<Vec<&str>> = files.iter().map(|f| self.cache.get(&format!("{prefix}{}", f.path), f)).collect();
        Some(tree::combine(files.iter().copied().zip(hashes?)))
    }

    fn read_hash(&mut self, root: &Path, prefix: &str, files: &[&FileEntry]) -> Result<tree::TreeHash> {
        let mut hashes = Vec::with_capacity(files.len());
        for f in files {
            let key = format!("{prefix}{}", f.path);
            let h = match self.cache.get(&key, f) {
                Some(h) => h.to_owned(),
                None => {
                    let h = crate::hash::blake3_file(&tree::from_slash(root, &f.path))?;
                    self.cache.insert(key, f, h.clone());
                    h
                }
            };
            hashes.push(h);
        }
        self.cache.save_if_due()?;
        Ok(tree::combine(files.iter().copied().zip(hashes.iter().map(String::as_str))))
    }

    /// Parts of `name` left complete by an earlier run, if all are still there.
    fn resume(&self, name: &str) -> Option<Vec<PackedPart>> {
        let out = &self.opts.out_dir;
        let text = std::fs::read_to_string(out.join(format!("{name}.parts.json"))).ok()?;
        let records: Vec<PartRecord> = serde_json::from_str(&text).ok()?;
        records
            .into_iter()
            .map(|r| {
                let path = out.join(&r.file_name);
                let ok = std::fs::metadata(&path).is_ok_and(|m| m.len() == r.size);
                ok.then_some(PackedPart { file_name: r.file_name, path, size: r.size, blake3: r.blake3 })
            })
            .collect()
    }

    fn finish(&mut self, hash: &tree::TreeHash, parts: Vec<PackedPart>) -> Package {
        let base_url = self.opts.base_url.trim_end_matches('/');
        let pkg = Package {
            hash: hash.hash.clone(),
            size: hash.size,
            parts: parts
                .iter()
                .map(|p| Part { url: format!("{base_url}/{}", p.file_name), size: p.size, blake3: p.blake3.clone() })
                .collect(),
        };
        self.assets.extend(parts);
        pkg
    }
}

fn package_name(id: &str, hash: &tree::TreeHash) -> String {
    format!("{id}-{}", &hash.hash[..16])
}

fn mb(files: &[&FileEntry]) -> u64 {
    files.iter().map(|f| f.size).sum::<u64>() / 1_000_000
}

fn rename_parts(out: &Path, parts: Vec<PackedPart>, name: &str) -> Result<Vec<PackedPart>> {
    parts
        .into_iter()
        .enumerate()
        .map(|(i, p)| {
            let file_name = format!("{name}.tar.zst.{:03}", i + 1);
            let path = out.join(&file_name);
            std::fs::rename(&p.path, &path).map_err(|e| Error::io(&p.path, e))?;
            Ok(PackedPart { file_name, path, ..p })
        })
        .collect()
}

/// Leaves only this build's assets in `out`: parts and records of earlier or
/// interrupted runs would otherwise be uploaded again by `out/*.tar.zst.*`.
fn remove_stray_files(out: &Path, assets: &[PackedPart]) -> Result<()> {
    let keep: HashSet<&str> = assets.iter().map(|a| a.file_name.as_str()).collect();
    let packages: HashSet<&str> = assets.iter().filter_map(|a| a.file_name.split_once(".tar.zst.").map(|(n, _)| n)).collect();
    let Ok(entries) = std::fs::read_dir(out) else { return Ok(()) };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let stray = match (is_part_file(&name), name.strip_suffix(".parts.json")) {
            (true, _) => !keep.contains(name.as_str()),
            (false, Some(package)) => !packages.contains(package),
            (false, None) => false,
        };
        if stray {
            std::fs::remove_file(entry.path()).map_err(|e| Error::io(entry.path(), e))?;
        }
    }
    Ok(())
}

/// `<name>.tar.zst.NNN`
fn is_part_file(name: &str) -> bool {
    name.rsplit_once(".tar.zst.").is_some_and(|(_, n)| n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()))
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

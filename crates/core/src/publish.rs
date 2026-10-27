//! Build-author side: turns an MO2 instance into a manifest + release assets.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::download::Downloader;
use crate::files::{self, FileList};
use crate::hash_cache::HashCache;
use crate::manifest::{ChangelogEntry, Manifest, ModEntry, ModSpec, NexusRef, Package, Part, SCHEMA_VERSION};
use crate::meta::ModMeta;
use crate::mo2::Instance;
use crate::modlist::{Entry, EntryState, ModList};
use crate::package::{self, PackOptions, PackedPart};
use crate::plan::USER_SEPARATOR;
use crate::tree::FileEntry;
use crate::verify::Files;
use crate::{rules, tree};
use crate::{Error, Result};

/// Top-level instance folders that never go into the base package.
const BASE_EXCLUDED: &[&str] = &["mods", "downloads", "overwrite", "logs", "crashDumps", "webcache", ".lyno"];

/// Folders and root files MO2 itself brings. Anything else in the instance root (a work folder,
/// notes, a game copy) is the author's and stays out of the base: it would reach every player.
const MO2_FOLDERS: &[&str] = &[
    "profiles", "dlls", "plugins", "stylesheets", "translations", "resources", "qml", "licenses", "tutorials",
    "platforms", "styles", "imageformats", "iconengines", "tls", "generic", "networkinformation", "sqldrivers",
    "platformthemes", "loot", "explorer++", "buttons", "mode",
];
const MO2_ROOT_EXTENSIONS: &[&str] = &["exe", "dll", "ini", "qss", "png", "chm", "qm", "json"];

fn is_mo2_path(first: &str, rest: &str) -> bool {
    if rest.is_empty() {
        if first.eq_ignore_ascii_case("portable.txt") {
            return true;
        }
        let ext = first.rsplit_once('.').map_or("", |(_, e)| e);
        return MO2_ROOT_EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(ext));
    }
    MO2_FOLDERS.iter().any(|x| x.eq_ignore_ascii_case(first))
}

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
    /// Checked before each package: a package being packed is finished first.
    /// A cancelled build resumes like an interrupted one.
    pub cancel: Option<Arc<AtomicBool>>,
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

impl BuildOutput {
    /// Bytes to upload for a repacked package (`Repacked::name`).
    pub fn upload_size(&self, name: &str) -> u64 {
        let (id, pkg) = self
            .manifest
            .mod_specs()
            .find(|m| m.name == name)
            .map_or(("base", &self.manifest.base), |m| (m.id.as_str(), &m.package));
        // Part names: `package_name` + `.tar.zst.NNN`.
        let prefix = format!("{id}-{}.", &pkg.hash[..16]);
        self.assets.iter().filter(|a| a.file_name.starts_with(&prefix)).map(|a| a.size).sum()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repacked {
    /// Mod folder name, or `base`.
    pub name: String,
    /// False: not in the previous manifest at all.
    pub changed: bool,
    /// Against the file record of the installed package ([`crate::files`]);
    /// `None` for a new mod, the base, or a mod without a current record.
    pub files: Option<FileChanges>,
}

/// Files of a changed mod against the published package. Paths relative to the mod folder.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileChanges {
    pub changed: Files,
    pub added: Files,
    pub missing: Files,
    /// Settings files changed or added: every player gets them as the defaults.
    pub settings: Files,
}

/// Unlike a player's, a new settings file of the author's is a change: it ships.
fn file_changes(record: &FileList, current: &[(FileEntry, String)]) -> FileChanges {
    let mut diff = crate::verify::compare(record, current);
    let new_settings = current.iter().map(|(f, _)| &f.path).filter(|p| rules::is_settings(p) && !record.files.contains_key(*p));
    diff.settings.extend(new_settings.cloned());
    FileChanges {
        changed: Files::of(diff.changed),
        added: Files::of(diff.added),
        missing: Files::of(diff.missing),
        settings: Files::of(diff.settings),
    }
}

/// File record of a package packed into `out`, taken from the hashes packing
/// computed: once the build is published, [`crate::author::adopt`] makes it the
/// installed record, so the next build can tell which files changed.
pub(crate) fn file_record_path(out: &Path, id: &str, hash: &str) -> PathBuf {
    out.join(format!("{id}-{}.files.json", &hash[..16]))
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
    let mut warnings = preflight(root, &opts.profile, &opts.game_version, &opts.out_dir)?;
    let cache = if opts.hash_cache { HashCache::load(root.join(".lyno").join("pack-cache.json")) } else { HashCache::disabled() };
    let mut packer = Packer { inst: &inst, opts, cache, assets: Vec::new(), repacked: Vec::new() };
    let (entries, _) = split_user_section(&list);
    let is_mod = |e: &&Entry| e.state != EntryState::Unmanaged && e.separator_title().is_none();
    let steps = 1 + entries.iter().filter(is_mod).count();

    let mut files = base_files(root, &opts.profile, Some(&opts.out_dir))?;
    warnings.extend(base_warnings(&files));
    let staged_ini = stage_mo2_ini(root, &mut files)?;
    let base_source = |p: &str| match &staged_ini {
        Some(staged) if p == MO2_INI => staged.clone(),
        _ => tree::from_slash(root, p),
    };
    let previous_base = opts.previous.as_ref().map(|m| &m.base);
    let base_label = format!("[1/{steps}] base ({})", base_breakdown(&files));
    let base = packer.package(&base_source, "", &files, "base", "base", &base_label, previous_base, log)?;

    let mut mods = Vec::new();
    let mut redmod = false;
    let mut step = 1;
    // `[LYNO] core=true` of the separator the current mod sits under.
    let mut core_section = false;
    for entry in entries {
        if entry.state == EntryState::Unmanaged {
            continue;
        }
        if let Some(title) = entry.separator_title() {
            let color = metas.get(&entry.name).and_then(|m| m.color.clone());
            core_section = metas.get(&entry.name).is_some_and(|m| m.lyno_core);
            mods.push(ModEntry::Separator { title: title.to_owned(), color });
            continue;
        }
        let folder = inst.mods_dir().join(&entry.name);
        if !folder.is_dir() {
            continue;
        }
        step += 1;
        let meta = metas.get(&entry.name).cloned().unwrap_or_default();
        let id = mod_id(&entry.name, &meta);

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

        if opts.cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
            packer.cache.save()?;
            return Err(Error::Cancelled);
        }
        let previous_spec = opts.previous.as_ref().and_then(|m| m.mod_specs().find(|s| s.id == id));
        let previous = previous_spec.map(|s| &s.package);
        let prefix = format!("mods/{}/", entry.name);
        let step_label = format!("[{step}/{steps}] {}", entry.name);
        let source = |p: &str| tree::from_slash(&folder, p);
        let package = packer.package(&source, &prefix, &files, &id, &entry.name, &step_label, previous, log)?;

        let nexus = meta.mod_id.map(|mod_id| NexusRef {
            game: meta.game_name.as_deref().unwrap_or("cyberpunk2077").to_lowercase(),
            mod_id,
        });
        let mut extra = if nexus.is_some() { info(&meta) } else { ModInfo::default() };
        // Without a Nexus key (or with Nexus down) the same Nexus page keeps
        // the author and title it had, instead of losing them for players.
        if let Some(prev) = previous_spec.filter(|p| p.nexus.is_some() && p.nexus == nexus) {
            extra.author = extra.author.or_else(|| prev.author.clone());
            extra.title = extra.title.or_else(|| prev.title.clone());
        }
        mods.push(ModEntry::Mod(ModSpec {
            id,
            name: entry.name.clone(),
            enabled: entry.state == EntryState::Enabled,
            optional: is_optional(&meta, core_section),
            version: meta.version.clone(),
            description: meta.lyno_description.clone(),
            requires: meta.lyno_requires.clone(),
            author: meta.lyno_author.clone().or(extra.author),
            title: meta.lyno_title.clone().or(extra.title).filter(|t| *t != entry.name),
            nexus,
            package,
        }));
    }

    let game_exe = game_patch(&inst);
    packer.cache.save()?;
    let Packer { assets, repacked, .. } = packer;
    remove_stray_files(&opts.out_dir, &assets)?;

    let manifest = Manifest {
        schema: SCHEMA_VERSION,
        name: opts.name.clone(),
        build_version: opts.build_version.clone(),
        game_version: opts.game_version.clone(),
        game_exe,
        mo2_version: opts.mo2_version.clone(),
        profile: opts.profile.clone(),
        redmod,
        changelog: opts.changelog.clone(),
        base,
        mods,
    };
    manifest.validate()?;
    for m in manifest.mod_specs().filter(|m| m.enabled) {
        for dep in &m.requires {
            if manifest.mod_specs().any(|d| d.id == *dep && !d.enabled) {
                return Err(Error::Manifest(format!("{}: обязательный мод {dep} выключен", m.name)));
            }
        }
    }
    Ok(BuildOutput { manifest, assets, repacked, warnings })
}

/// What [`build`] warns about from the mod list, `meta.ini` files and the
/// profile alone, without listing or reading mod files: a real build packs for
/// hours, and a forgotten flag is cheaper to see before that. Only generated
/// files in mods and an oversized base are left to the build.
pub fn preflight(root: &Path, profile: &str, game_version: &str, out_dir: &Path) -> Result<Vec<String>> {
    let inst = Instance::new(root);
    let list = ModList::load(&inst.modlist_path(profile))?;
    let metas = inst.scan_mods()?;
    let mut warnings = Vec::new();
    let (entries, personal) = split_user_section(&list);
    let is_mod = |e: &&Entry| e.state != EntryState::Unmanaged && e.separator_title().is_none();
    let personal: Vec<&str> = personal.iter().filter(is_mod).map(|e| e.name.as_str()).collect();
    if !personal.is_empty() {
        warnings.push(format!("{} mod(s) under {USER_SEPARATOR} are not shipped: {}", personal.len(), personal.join(", ")));
    }
    warnings.extend(profile_warnings(&inst.profile_dir(profile)));
    warnings.extend(skipped_root_warning(root, out_dir));

    // (id, folder, enabled, optional, requires) of the mods the build ships.
    let mut mods = Vec::new();
    let mut core_section = false;
    for entry in entries.iter().filter(|e| e.state != EntryState::Unmanaged) {
        let meta = metas.get(&entry.name).cloned().unwrap_or_default();
        if entry.separator_title().is_some() {
            core_section = meta.lyno_core;
            continue;
        }
        if !inst.mods_dir().join(&entry.name).is_dir() {
            warnings.push(format!("{:?} is in modlist.txt but has no folder, skipped", entry.name));
            continue;
        }
        if meta.mod_id.is_none() {
            warnings.push(format!("{:?}: no Nexus mod id in meta.ini, no link in the launcher", entry.name));
        }
        let enabled = entry.state == EntryState::Enabled;
        mods.push((mod_id(&entry.name, &meta), &entry.name, enabled, is_optional(&meta, core_section), meta.lyno_requires));
    }
    // Most likely a forgotten flag rather than a build without frameworks.
    if !mods.is_empty() && mods.iter().all(|m| m.3) {
        warnings.push("no core mods ([LYNO] core=true on a separator or a mod): players can switch off every mod".into());
    }
    for (_, name, _, _, requires) in mods.iter().filter(|m| m.2) {
        for dep in requires.iter().filter(|dep| mods.iter().any(|d| d.0 == **dep && !d.2)) {
            warnings.push(format!("{name:?} requires {dep}, which is switched off: the build will fail"));
        }
    }
    warnings.extend(overwrite_warning(&inst.overwrite_dir())?);
    if let Some(exe) = game_patch(&inst) {
        if crate::game::compare_patch(&exe, game_version) != Some(std::cmp::Ordering::Equal) {
            warnings.push(format!(
                "game version {game_version:?} given, but the game this instance runs is patch {exe}: players are checked against {exe}"
            ));
        }
    }
    Ok(warnings)
}

/// Patch of the game this instance runs, from its exe.
fn game_patch(inst: &Instance) -> Option<String> {
    inst.game_path().and_then(|d| crate::game::exe_version(&d)).map(|v| v.patch())
}

/// Build entries and the author's own ones. The author plays in a launcher
/// instance like any player: what sits under `LYNO USER MODS` is theirs
/// (tests, personal tweaks), not the build's. A mod goes into the build once it
/// is moved above the separator.
pub fn split_user_section(list: &ModList) -> (&[Entry], &[Entry]) {
    let user_section = list.entries.iter().position(|e| e.separator_title() == Some(USER_SEPARATOR));
    list.entries.split_at(user_section.unwrap_or(list.entries.len()))
}

/// Manifest id of a mod folder: `[LYNO] id=` survives renaming the folder.
pub fn mod_id(folder: &str, meta: &ModMeta) -> String {
    meta.lyno_id.clone().unwrap_or_else(|| slug(folder))
}

/// Files of the base package: MO2 and the build's profile (only what MO2 itself ships:
/// [`MO2_FOLDERS`]), without what the game and MO2 generate, the author's other profiles and private files, and
/// `out_dir` when it sits inside the instance.
pub fn base_files(root: &Path, profile: &str, out_dir: Option<&Path>) -> Result<Vec<FileEntry>> {
    let out_rel = out_dir
        .and_then(|o| std::path::absolute(o).ok())
        .zip(std::path::absolute(root).ok())
        .and_then(|(out, root)| out.strip_prefix(root).ok().map(tree::to_slash))
        .filter(|r| !r.is_empty());
    let keep = |p: &str| {
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
        // Dot folders at the root are tools' (`.git`, `.pnpm-store`, `.vs`), never MO2's.
        if first.starts_with('.') && !rest.is_empty() {
            return false;
        }
        // Python writes bytecode next to MO2's Python plugins on every machine.
        if p.split('/').any(|c| c.eq_ignore_ascii_case("__pycache__")) {
            return false;
        }
        if !is_mo2_path(first, rest) {
            return false;
        }
        if !first.eq_ignore_ascii_case("profiles") {
            return true;
        }
        rest.split_once('/').is_some_and(|(name, file)| name == profile && !rules::is_private_profile_file(file))
    };
    tree::list_files_with(root, &keep)
}

const MO2_INI: &str = "ModOrganizer.ini";

/// Ships `ModOrganizer.ini` without the state of the author's MO2 window
/// ([`crate::mo2::strip_ui_state`]): a copy in `.lyno/pack/` replaces it in
/// `files`. The copy is rewritten only when its content changes, so its
/// size and mtime keep the hash cache valid between builds.
fn stage_mo2_ini(root: &Path, files: &mut [FileEntry]) -> Result<Option<PathBuf>> {
    let Some(entry) = files.iter_mut().find(|f| f.path == MO2_INI) else { return Ok(None) };
    let src = root.join(MO2_INI);
    let text = std::fs::read(&src).map_err(|e| Error::io(&src, e))?;
    // Qt writes UTF-8; anything else ships untouched.
    let Ok(text) = String::from_utf8(text) else { return Ok(None) };
    let stripped = crate::mo2::strip_ui_state(&text);
    let dir = root.join(".lyno").join("pack");
    let staged = dir.join(MO2_INI);
    if std::fs::read_to_string(&staged).ok().as_deref() != Some(stripped.as_str()) {
        std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
        std::fs::write(&staged, &stripped).map_err(|e| Error::io(&staged, e))?;
    }
    let fresh = tree::list_files_with(&dir, &|p| p == MO2_INI)?;
    let Some(fresh) = fresh.into_iter().next() else { return Ok(None) };
    entry.size = fresh.size;
    entry.mtime = fresh.mtime;
    Ok(Some(staged))
}

/// Top-level folders of the base this large are not MO2 (all of MO2 is a few
/// hundred MB): a game copy, a dev toolchain, backups. Every player would
/// download them.
const BASE_FOLDER_WARN_SIZE: u64 = 500_000_000;

/// Root entries that stay out of the base. If MO2 itself needs one (a new Qt folder), the
/// build would ship an MO2 that doesn't start: this is the only place the author sees it.
fn skipped_root_warning(root: &Path, out_dir: &Path) -> Option<String> {
    let out = std::path::absolute(out_dir).ok();
    let mut names: Vec<String> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .filter(|e| out.as_deref() != std::path::absolute(e.path()).ok().as_deref())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| {
            let rest = if root.join(n).is_dir() { "x" } else { "" };
            !n.starts_with('.')
                && !BASE_EXCLUDED.iter().any(|x| x.eq_ignore_ascii_case(n))
                && !is_mo2_path(n, rest)
        })
        .collect();
    names.sort();
    (!names.is_empty()).then(|| format!("not MO2's, left out of the base: {} (add to MO2_FOLDERS if MO2 needs it)", names.join(", ")))
}

fn base_warnings(files: &[FileEntry]) -> Vec<String> {
    let mut sizes: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for f in files {
        if let Some((first, _)) = f.path.split_once('/') {
            *sizes.entry(first).or_default() += f.size;
        }
    }
    sizes
        .into_iter()
        .filter(|&(_, size)| size >= BASE_FOLDER_WARN_SIZE)
        .map(|(name, size)| {
            format!("{name}/ in the instance root is {} MB and goes into the base package: move it out unless MO2 needs it", size / 1_000_000)
        })
        .collect()
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
/// Players may switch off every build mod except the core: frameworks and
/// libraries other mods need, without which the game crashes or scripts fail
/// to compile. `optional=true` lets a mod inside a core section be switched
/// off anyway (an add-on the author ships disabled, say).
fn is_optional(meta: &ModMeta, core_section: bool) -> bool {
    meta.lyno_optional || !(meta.lyno_core || core_section)
}

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
    inst: &'a Instance,
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
    /// `files` are the shipped files, sorted, read from `source(path)`;
    /// `prefix` is their folder relative to the instance (hash cache key).
    #[allow(clippy::too_many_arguments)]
    fn package(
        &mut self,
        source: &dyn Fn(&str) -> PathBuf,
        prefix: &str,
        files: &[FileEntry],
        id: &str,
        name: &str,
        step: &str,
        previous: Option<&Package>,
        log: &mut dyn FnMut(&str),
    ) -> Result<Package> {
        let hashed: Vec<&FileEntry> = files.iter().filter(|f| rules::is_hashed(&f.path)).collect();
        let mut known = self.cached_hashes(prefix, &hashed);
        if known.is_none() && previous.is_some() {
            log(&format!("{step}: hashing {} MB", mb(&hashed)));
            known = Some(self.read_hashes(source, prefix, &hashed)?);
        }
        if let Some(current) = &known {
            let hash = tree::combine_pairs(current);
            if let Some(p) = previous.filter(|p| p.hash == hash.hash) {
                return Ok(p.clone());
            }
            if let Some(parts) = self.resume(&package_name(id, &hash)) {
                log(&format!("{step}: already packed by an earlier run"));
                self.record_repacked(id, name, previous, &hash, current)?;
                return Ok(self.finish(&hash, parts));
            }
        }

        log(&format!("{step}: packing {} MB", mb(&hashed)));
        let out = &self.opts.out_dir;
        // The final name needs the hash, which is only known once every file is read.
        let packed = package::pack_from(source, files, out, &format!("{id}.packing"), &self.opts.pack)?;
        for (f, h) in files.iter().zip(&packed.file_hashes) {
            self.cache.insert(format!("{prefix}{}", f.path), f, h.clone());
        }
        let current: Vec<(FileEntry, String)> =
            files.iter().zip(packed.file_hashes).filter(|(f, _)| rules::is_hashed(&f.path)).map(|(f, h)| (f.clone(), h)).collect();
        let hash = tree::combine_pairs(&current);
        let parts = rename_parts(out, packed.parts, &package_name(id, &hash))?;
        let records: Vec<PartRecord> = parts
            .iter()
            .map(|p| PartRecord { file_name: p.file_name.clone(), size: p.size, blake3: p.blake3.clone() })
            .collect();
        let record_path = out.join(format!("{}.parts.json", package_name(id, &hash)));
        std::fs::write(&record_path, serde_json::to_string_pretty(&records)?).map_err(|e| Error::io(&record_path, e))?;
        self.cache.save_if_due()?;
        self.record_repacked(id, name, previous, &hash, &current)?;
        Ok(self.finish(&hash, parts))
    }

    /// Writes the package's file record into `out` ([`file_record_path`]) and
    /// tells what changed against the installed one.
    fn record_repacked(
        &mut self,
        id: &str,
        name: &str,
        previous: Option<&Package>,
        hash: &tree::TreeHash,
        current: &[(FileEntry, String)],
    ) -> Result<()> {
        let path = file_record_path(&self.opts.out_dir, id, &hash.hash);
        std::fs::write(&path, serde_json::to_string(&FileList::new(id, &hash.hash, current))?).map_err(|e| Error::io(&path, e))?;
        let files = previous.and_then(|p| files::load(self.inst, id, &p.hash)).map(|record| file_changes(&record, current));
        self.repacked.push(Repacked { name: name.to_owned(), changed: previous.is_some(), files });
        Ok(())
    }

    /// Per-file hashes from the cache alone, `None` if any file has to be read.
    fn cached_hashes(&self, prefix: &str, files: &[&FileEntry]) -> Option<Vec<(FileEntry, String)>> {
        files.iter().map(|f| Some(((*f).clone(), self.cache.get(&format!("{prefix}{}", f.path), f)?.to_owned()))).collect()
    }

    fn read_hashes(&mut self, source: &dyn Fn(&str) -> PathBuf, prefix: &str, files: &[&FileEntry]) -> Result<Vec<(FileEntry, String)>> {
        let mut hashes = Vec::with_capacity(files.len());
        for f in files {
            let key = format!("{prefix}{}", f.path);
            let h = match self.cache.get(&key, f) {
                Some(h) => h.to_owned(),
                None => {
                    let h = crate::hash::blake3_file(&source(&f.path))?;
                    self.cache.insert(key, f, h.clone());
                    h
                }
            };
            hashes.push(((*f).clone(), h));
        }
        self.cache.save_if_due()?;
        Ok(hashes)
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
        let record = name.strip_suffix(".parts.json").or_else(|| name.strip_suffix(".files.json"));
        let stray = match (is_part_file(&name), record) {
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

    #[test]
    fn everything_outside_core_is_optional() {
        let meta = |optional, core| ModMeta { lyno_optional: optional, lyno_core: core, ..Default::default() };
        assert!(is_optional(&meta(false, false), false));
        assert!(!is_optional(&meta(false, false), true), "a mod under a core separator");
        assert!(!is_optional(&meta(false, true), false), "a core mod outside a core section");
        assert!(is_optional(&meta(true, false), true), "optional=true wins over the section");
    }
}

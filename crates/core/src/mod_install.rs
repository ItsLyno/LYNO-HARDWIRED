//! Installs a mod archive downloaded from Nexus into the instance, the way
//! MO2 would: the files into `mods/<name>/`, the Nexus record into its
//! `meta.ini`, the folder into `modlist.txt`, and the archive into MO2's
//! `downloads/` with a `.meta` next to it, so MO2 shows it in its Downloads
//! tab and can reinstall it.
//!
//! What MO2 asks the player, the install asks too and stops halfway, the
//! archive waiting in `downloads/`: a FOMOD installer ([`Outcome::Fomod`],
//! [`open_fomod`], [`install_fomod`]) or a layout no rule recognizes
//! ([`Outcome::Manual`], [`install_root`], MO2's manual installer). With MO2
//! or the game running the install waits for them to close
//! ([`Outcome::Deferred`]).
//!
//! A Nexus download only goes to `downloads/` ([`fetch`]): the player drags it
//! onto the list, and confirms when it would replace an installed mod.
//!
//! An archive the player brings from disk ([`Download::from_file`]) is
//! installed from where it is: it is theirs, not MO2's download.
//!
//! MO2 keeps `modlist.txt` in memory and writes it back on exit, so the
//! caller makes sure MO2 is closed before [`install`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;

use crate::archive::{self, Kind, Layout};
use crate::download::{Downloader, PartEvent};
use crate::fomod::{self, FileItem, FileState, Installer, Selection};
use crate::manifest::Manifest;
use crate::meta::ModMeta;
use crate::mo2::Instance;
use crate::modlist::{Entry, EntryState, ModList};
use crate::nexus::NexusApi;
use crate::nxm::NxmLink;
use crate::package;
use crate::plan::USER_SEPARATOR;
use crate::tracking::{self, Cache};
use crate::{Error, Result};

/// A file from Nexus, as MO2 records it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Download {
    pub game: String,
    pub mod_id: u64,
    pub file_id: u64,
    /// Mod page title, the folder name of a new mod.
    pub mod_name: String,
    /// File title on the files tab ("Main File").
    pub file_title: String,
    pub version: Option<String>,
    /// Archive name.
    pub file_name: String,
    /// An archive from the player's disk: installed from where it lies,
    /// never moved into `downloads/`.
    pub local: bool,
}

impl Download {
    /// Folder of a new mod. A page often has a main file and addons; by the page
    /// title alone they all land in `Name`, `Name (2)`, … and can't be told apart.
    pub fn folder(&self) -> String {
        let title = self.file_title.trim();
        if title.is_empty() || title.eq_ignore_ascii_case(self.mod_name.trim()) {
            self.mod_name.clone()
        } else if title.to_lowercase().contains(&self.mod_name.trim().to_lowercase()) {
            title.to_owned()
        } else {
            format!("{} - {title}", self.mod_name.trim())
        }
    }

    /// An archive from disk. Nexus names its files `Name-modid-version-timestamp`;
    /// like MO2, the mod page and version are taken from such a name.
    pub fn from_file(path: &Path) -> Self {
        let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let parts: Vec<&str> = stem.split('-').collect();
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
        let nexus = (parts.len() >= 4 && digits(parts[parts.len() - 1]) && parts[parts.len() - 1].len() >= 9)
            .then(|| (1..parts.len() - 2).find(|&i| digits(parts[i])))
            .flatten();
        let (mod_name, mod_id, version) = match nexus {
            Some(i) => (
                parts[..i].join("-").trim().to_owned(),
                parts[i].parse().unwrap_or(0),
                Some(parts[i + 1..parts.len() - 1].join(".")).filter(|v| !v.is_empty()),
            ),
            None => (stem.trim().to_owned(), 0, None),
        };
        Self {
            game: if mod_id != 0 { DEFAULT_GAME.into() } else { String::new() },
            mod_id,
            file_id: 0,
            mod_name,
            file_title: String::new(),
            version,
            file_name,
            local: true,
        }
    }
}

const DEFAULT_GAME: &str = "cyberpunk2077";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Replace this mod folder: an update or a reinstall. Keeps its place and
    /// state in `modlist.txt` and the rest of its `meta.ini` (`[LYNO] id`).
    Replace(String),
    /// The player's own version of optional build mod `id`, over its folder
    /// as [`Target::Replace`]: the build lets go of it ([`crate::install::detach`]).
    Own { folder: String, id: String },
    /// A new folder. `personal`: under `LYNO USER MODS`; otherwise at the end
    /// of the build section (the author's next release). `after`: right
    /// after this entry of the list (dropped there), within the player's
    /// section for a personal mod.
    New { personal: bool, after: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    Installed { folder: String },
    /// In MO2's downloads, for the player to drag onto the list; `replaces`:
    /// the installed mod it is another version of.
    Downloaded { replaces: Option<String> },
    /// In MO2's downloads: the player installs it there.
    Mo2 { reason: archive::Mo2Reason },
    /// MO2 or the game is running, and MO2 would overwrite `modlist.txt` on
    /// exit: the archive waits in its downloads, the install follows once
    /// they close.
    Deferred {
        #[serde(skip)]
        archive: PathBuf,
        #[serde(skip)]
        target: Target,
    },
    /// No rule recognizes the layout: the player picks the folder that is
    /// the mod ([`archive::roots`]).
    Manual {
        #[serde(skip)]
        archive: PathBuf,
        #[serde(skip)]
        target: Target,
    },
    /// A FOMOD installer: the archive is in MO2's downloads and waits for the
    /// player's choice.
    Fomod {
        #[serde(skip)]
        archive: PathBuf,
        #[serde(skip)]
        target: Target,
    },
}

/// Where [`fetch`] is in its work.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Progress {
    /// File info is known: the job can show its name.
    #[serde(rename_all = "camelCase")]
    Resolved { mod_name: String, file_title: String, version: Option<String>, size: u64, replaces: Option<String> },
    Bytes { done: u64, total: u64 },
    #[serde(rename_all = "camelCase")]
    Retry { attempt: u32, delay_secs: u64, error: String },
}

/// What [`fetch`] needs to know about the instance.
pub struct Context<'a> {
    pub inst: &'a Instance,
    pub profile: &'a str,
    /// The author updates build mods; a player only their own and the optional build mods
    /// of `manifest` (the installed build), which then become theirs.
    pub author: bool,
    pub manifest: Option<&'a Manifest>,
    pub now: u64,
}

/// The core build mod an nxm link would replace on a player's instance: the
/// build needs its version.
#[derive(Debug)]
pub struct BuildMod(pub String);

/// Downloads the file of an nxm link (or of a Premium update, `link.key`
/// empty) into MO2's `downloads/`. Installing it is the player's call.
pub fn fetch(
    api: &NexusApi,
    downloader: &Downloader,
    ctx: &Context,
    link: &NxmLink,
    cancel: &dyn Fn() -> bool,
    on: &mut dyn FnMut(Progress),
) -> Result<std::result::Result<(Download, Outcome), BuildMod>> {
    let files = api.files(&link.game, link.mod_id)?;
    let file = files
        .files
        .iter()
        .find(|f| f.file_id == link.file_id)
        .cloned()
        .ok_or_else(|| Error::Nexus { status: 404, message: format!("file {} is not on mod page {}", link.file_id, link.mod_id) })?;
    let page = api.mod_page(&link.game, link.mod_id)?;
    let size = file.size().ok_or_else(|| Error::Download(format!("Nexus gives no size for {}", file.file_name)))?;

    let (tracked, _) = match ctx.inst.modlist_path(ctx.profile).is_file() {
        true => tracking::tracked_mods(ctx.inst, ctx.profile)?,
        false => Default::default(),
    };
    let replaces = tracking::target(&tracked, &files, &link.game, link.mod_id, link.file_id);
    if let Some(t) = replaces.filter(|t| t.managed && !ctx.author) {
        if ctx.manifest.map(|m| crate::install::optional_in(ctx.inst, m, &t.folder)).transpose()?.flatten().is_none() {
            return Ok(Err(BuildMod(t.folder.clone())));
        }
    }
    let dl = Download {
        game: link.game.clone(),
        mod_id: link.mod_id,
        file_id: link.file_id,
        mod_name: page.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| file.name.clone()),
        file_title: file.name.clone(),
        version: file.version.clone().or(page.version.clone()),
        file_name: file.file_name.clone(),
        local: false,
    };
    on(Progress::Resolved {
        mod_name: dl.mod_name.clone(),
        file_title: dl.file_title.clone(),
        version: dl.version.clone(),
        size,
        replaces: replaces.map(|t| t.folder.clone()),
    });

    let permit = link.key.as_deref().zip(link.expires);
    let links = api.download_links(&link.game, link.mod_id, link.file_id, permit)?;
    let url = links.first().ok_or_else(|| Error::Download(format!("Nexus gave no download link for {}", dl.file_name)))?;
    // `size_kb` alone is rounded (27845 bytes come as 27 KB): only the CDN knows
    // the exact size the download is checked against.
    let size = match file.size_in_bytes {
        Some(n) => n,
        None => downloader.content_length(url)?,
    };
    let scratch = ctx.inst.root().join(".lyno").join("nexus-downloads").join(folder_name(&dl.file_name));
    downloader.fetch_file(url, size, &scratch, cancel, &mut |e| match e {
        PartEvent::Bytes(done) => on(Progress::Bytes { done, total: size }),
        PartEvent::Retry { attempt, delay, error } => on(Progress::Retry { attempt, delay_secs: delay.as_secs(), error }),
    })?;

    to_downloads(ctx.inst, &scratch, &dl, false)?;
    let outcome = Outcome::Downloaded { replaces: replaces.map(|t| t.folder.clone()) };
    // The page was just read: its mods show as current without another request.
    let mut cache = Cache::load(&Cache::path(ctx.inst));
    cache.record(&link.game, link.mod_id, files, ctx.now);
    cache.save()?;
    Ok(Ok((dl, outcome)))
}

/// Folder names MO2 and Windows accept.
pub fn folder_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_end_matches(['.', ' ']).to_owned();
    if trimmed.is_empty() {
        "Nexus mod".into()
    } else if trimmed.ends_with("_separator") {
        format!("{trimmed} mod")
    } else {
        trimmed
    }
}

/// `name`, or `name (2)`, … if a folder by that name exists.
fn free_folder(mods: &Path, name: &str) -> String {
    let mut candidate = name.to_owned();
    let mut n = 2;
    while mods.join(&candidate).exists() {
        candidate = format!("{name} ({n})");
        n += 1;
    }
    candidate
}

/// Installs `archive` (downloaded to a scratch place, in `downloads/` or
/// the player's own) for `target`; a Nexus download ends in `downloads/`.
pub fn install(inst: &Instance, profile: &str, archive_path: &Path, dl: &Download, target: &Target) -> Result<Outcome> {
    let kind = archive::kind(archive_path)?;
    let layout = match kind {
        Kind::Zip | Kind::SevenZip | Kind::Rar => archive::layout(&archive::entries(archive_path, kind)?),
        Kind::Unknown => Layout::Mo2(archive::Mo2Reason::Format),
    };
    let files = match layout {
        Layout::Files(files) => files,
        Layout::Fomod { .. } => {
            let archive = keep(inst, archive_path, dl, false)?;
            // An installer the launcher can't read: the player picks the files like in MO2's manual installer.
            return Ok(match open_fomod(inst, &archive, target) {
                Ok(_) => Outcome::Fomod { archive, target: target.clone() },
                Err(_) => Outcome::Manual { archive, target: target.clone() },
            });
        }
        Layout::Mo2(archive::Mo2Reason::Format) => {
            keep(inst, archive_path, dl, false)?;
            return Ok(Outcome::Mo2 { reason: archive::Mo2Reason::Format });
        }
        Layout::Mo2(_) => {
            let archive = keep(inst, archive_path, dl, false)?;
            return Ok(Outcome::Manual { archive, target: target.clone() });
        }
    };
    place(inst, profile, archive_path, kind, &files, dl, target, None)
}

/// Folders of an archive waiting in [`Outcome::Manual`].
pub fn roots(archive_path: &Path) -> Result<Vec<archive::Root>> {
    let kind = archive::kind(archive_path)?;
    Ok(archive::roots(&archive::entries(archive_path, kind)?))
}

/// Installs the folder `root` of the archive as the mod (MO2's manual
/// installer, "set data directory").
pub fn install_root(inst: &Instance, profile: &str, archive_path: &Path, dl: &Download, target: &Target, root: &str) -> Result<Outcome> {
    let kind = archive::kind(archive_path)?;
    let files = archive::map_under(&archive::entries(archive_path, kind)?, root);
    if files.is_empty() {
        return Err(Error::Fomod(format!("nothing to install under {root:?}")));
    }
    place(inst, profile, archive_path, kind, &files, dl, target, None)
}

/// Where the archive stays: a Nexus download goes to MO2's `downloads/`
/// with its `.meta`, the player's own file is left alone.
fn keep(inst: &Instance, archive_path: &Path, dl: &Download, installed: bool) -> Result<PathBuf> {
    if dl.local {
        Ok(archive_path.to_owned())
    } else {
        to_downloads(inst, archive_path, dl, installed)
    }
}

/// Unpacks `files` of the archive into the target folder, records the mod in
/// `meta.ini` and `modlist.txt`, and moves the archive into `downloads/`.
/// `fomod`: the remembered FOMOD choice, `None` drops an old one.
#[allow(clippy::too_many_arguments)]
fn place(
    inst: &Instance,
    profile: &str,
    archive_path: &Path,
    kind: Kind,
    files: &[(String, String)],
    dl: &Download,
    target: &Target,
    fomod: Option<&str>,
) -> Result<Outcome> {
    let mods = inst.mods_dir();
    std::fs::create_dir_all(&mods).map_err(|e| Error::io(&mods, e))?;
    let folder = match target {
        Target::Replace(f) | Target::Own { folder: f, .. } => f.clone(),
        Target::New { .. } => free_folder(&mods, &folder_name(&dl.folder())),
    };
    let staging = inst.root().join(".lyno").join("staging").join("nexus");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
    }
    archive::extract(archive_path, kind, files, &staging)?;

    let old_meta = mods.join(&folder).join("meta.ini");
    let meta_path = staging.join("meta.ini");
    if !matches!(target, Target::New { .. }) && old_meta.is_file() {
        std::fs::copy(&old_meta, &meta_path).map_err(|e| Error::io(&old_meta, e))?;
    }
    // An archive from disk with no Nexus name records only itself.
    let nexus = dl.mod_id != 0;
    let meta = ModMeta {
        game_name: nexus.then(|| dl.game.clone()),
        mod_id: nexus.then_some(dl.mod_id),
        file_id: (dl.file_id != 0).then_some(dl.file_id),
        version: dl.version.clone(),
        installation_file: Some(dl.file_name.clone()),
        repository: nexus.then(|| "Nexus".into()),
        ..Default::default()
    };
    meta.save(&meta_path)?;
    crate::meta::save_fomod(&meta_path, fomod)?;
    // Before the swap: a crash in between leaves the build's files as the player's, never the
    // player's as a damaged build mod that a repair would overwrite.
    if let Target::Own { id, .. } = target {
        crate::install::detach(inst, id)?;
    }
    package::swap_folder(&staging, &mods.join(&folder))?;

    let list_path = inst.modlist_path(profile);
    let mut list = if list_path.is_file() { ModList::load(&list_path)? } else { ModList::default() };
    if list.get(&folder).is_none() {
        let (personal, after) = match target {
            Target::New { personal, after } => (*personal, after.as_deref()),
            Target::Replace(_) | Target::Own { .. } => (true, None),
        };
        let personal = personal && crate::install::has_build(inst);
        insert(&mut list, Entry::enabled(&folder), personal, after);
        if let Some(dir) = list_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        list.save(&list_path)?;
    }
    keep(inst, archive_path, dl, true)?;
    Ok(Outcome::Installed { folder })
}

/// A FOMOD installer read from an archive in `downloads/`.
pub struct Fomod {
    pub installer: Installer,
    pub archive: PathBuf,
    kind: Kind,
    /// The folder that holds `fomod/`, as in the archive (`""` or `Wrapper/`).
    root: String,
    entries: Vec<String>,
    /// The choice remembered in the `meta.ini` of the mod it replaces.
    pub previous: Option<Selection>,
}

/// The config is a few KB; a bigger one is not an installer.
const MAX_CONFIG: u64 = 4 << 20;
/// Images are previews: a huge one is skipped rather than sent to the UI.
const MAX_IMAGE: u64 = 3 << 20;

pub fn open_fomod(inst: &Instance, archive_path: &Path, target: &Target) -> Result<Fomod> {
    let kind = archive::kind(archive_path)?;
    let entries = archive::entries(archive_path, kind)?;
    let Layout::Fomod { root } = archive::layout(&entries) else {
        return Err(Error::Fomod(format!("{} has no fomod/ModuleConfig.xml", archive_path.display())));
    };
    let config = entries
        .iter()
        .find(|e| archive::fomod_root(e) == Some(root.as_str()))
        .cloned()
        .ok_or_else(|| Error::Fomod("ModuleConfig.xml not found".into()))?;
    let bytes = archive::read(archive_path, kind, std::slice::from_ref(&config), MAX_CONFIG)?
        .remove(&config)
        .ok_or_else(|| Error::Fomod("ModuleConfig.xml is too big".into()))?;
    let installer = Installer::parse(&bytes)?;
    let previous = match target {
        Target::Replace(folder) | Target::Own { folder, .. } => {
            let meta = inst.mods_dir().join(folder).join("meta.ini");
            let saved = meta.is_file().then(|| ModMeta::load(&meta)).transpose()?.and_then(|m| m.lyno_fomod);
            saved.and_then(|v| fomod::decode_saved(&v)).map(|s| installer.restore(&s))
        }
        Target::New { .. } => None,
    };
    Ok(Fomod { installer, archive: archive_path.to_owned(), kind, root, entries, previous })
}

impl Fomod {
    /// Archive entry of an installer path: authors on Windows mix the case.
    fn entry(&self, rel: &str) -> Option<&String> {
        let full = format!("{}{rel}", self.root);
        self.entries.iter().find(|e| e.eq_ignore_ascii_case(&full))
    }

    /// The installer's images as `data:` URLs, by installer path. Formats a
    /// browser can't show (`.dds`) and oversized files are left out.
    pub fn images(&self) -> Result<HashMap<String, String>> {
        let wanted: Vec<(String, String, &str)> = self
            .installer
            .images()
            .into_iter()
            .filter_map(|rel| {
                let mime = match rel.rsplit_once('.')?.1.to_ascii_lowercase().as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "gif" => "image/gif",
                    "webp" => "image/webp",
                    "bmp" => "image/bmp",
                    _ => return None,
                };
                Some((self.entry(&rel)?.clone(), rel, mime))
            })
            .collect();
        let names: Vec<String> = wanted.iter().map(|(e, _, _)| e.clone()).collect();
        let mut data = archive::read(&self.archive, self.kind, &names, MAX_IMAGE)?;
        let engine = base64::engine::general_purpose::STANDARD;
        Ok(wanted
            .into_iter()
            .filter_map(|(entry, rel, mime)| Some((rel, format!("data:{mime};base64,{}", engine.encode(data.remove(&entry)?)))))
            .collect())
    }

    /// `(archive path, path in the mod folder)` of the installer's items.
    /// A later item wins a destination over an earlier one.
    fn map(&self, items: &[FileItem]) -> Result<Vec<(String, String)>> {
        let mut out: Vec<(String, String)> = Vec::new();
        let mut at: HashMap<String, usize> = HashMap::new();
        let mut add = |entry: &str, dest: String| -> Result<()> {
            let Some(dest) = archive::place(&dest) else { return Ok(()) };
            if !archive::is_safe(&dest) || !archive::is_safe(entry) {
                return Err(Error::Fomod(format!("unsafe path {entry:?} -> {dest:?}")));
            }
            match at.get(&dest.to_ascii_lowercase()) {
                Some(&i) => out[i] = (entry.to_owned(), dest),
                None => {
                    at.insert(dest.to_ascii_lowercase(), out.len());
                    out.push((entry.to_owned(), dest));
                }
            }
            Ok(())
        };
        let join = |base: &str, rel: &str| if base.is_empty() { rel.to_owned() } else { format!("{base}/{rel}") };
        for item in items {
            if item.folder {
                let prefix = if item.source.is_empty() { self.root.clone() } else { format!("{}{}/", self.root, item.source) };
                let base = item.destination.clone().unwrap_or_else(|| item.source.clone());
                for e in &self.entries {
                    let Some(rel) = e.get(prefix.len()..).filter(|_| e.is_char_boundary(prefix.len()) && e[..prefix.len()].eq_ignore_ascii_case(&prefix)) else {
                        continue;
                    };
                    // The whole archive as a folder would install the installer too.
                    if item.source.is_empty() && rel.to_ascii_lowercase().starts_with("fomod/") {
                        continue;
                    }
                    add(e, join(&base, rel))?;
                }
            } else {
                let entry = self.entry(&item.source).ok_or_else(|| Error::Fomod(format!("{} is not in the archive", item.source)))?;
                let dest = match item.destination.as_deref() {
                    None => item.source.clone(),
                    Some("") => item.source.rsplit('/').next().unwrap_or_default().to_owned(),
                    Some(d) => d.to_owned(),
                };
                add(entry, dest)?;
            }
        }
        Ok(out)
    }
}

/// `fileDependency`: a file is active when an enabled mod of the profile has
/// it, inactive when only a disabled one does.
pub fn file_states(inst: &Instance, profile: &str) -> impl Fn(&str) -> FileState {
    let list = ModList::load(&inst.modlist_path(profile)).unwrap_or_default();
    let mods = inst.mods_dir();
    let folders: Vec<(PathBuf, bool)> =
        list.mods().map(|e| (mods.join(&e.name), e.state == EntryState::Enabled)).collect();
    move |path: &str| {
        let mut state = FileState::Missing;
        for (dir, enabled) in &folders {
            if crate::tree::from_slash(dir, path).is_file() {
                if *enabled {
                    return FileState::Active;
                }
                state = FileState::Inactive;
            }
        }
        state
    }
}

/// Installs the archive of `fomod` with the player's `selection` and
/// remembers the choice in the mod's `meta.ini`.
pub fn install_fomod(inst: &Instance, profile: &str, fomod: &Fomod, dl: &Download, target: &Target, selection: &Selection) -> Result<Outcome> {
    let states = file_states(inst, profile);
    let ev = fomod.installer.evaluate(selection, &states);
    if let Some(step) = ev.valid.iter().position(|v| !v) {
        return Err(Error::Fomod(format!("step {:?}: a group has a wrong number of options picked", fomod.installer.steps[step].name)));
    }
    let files = fomod.map(&fomod.installer.files(&ev, &states))?;
    if files.is_empty() {
        return Err(Error::Fomod("the chosen options install no files".into()));
    }
    let saved = fomod::encode_saved(&fomod.installer.save(&ev));
    place(inst, profile, &fomod.archive, fomod.kind, &files, dl, target, Some(&saved))
}

/// A new mod of the player goes to the very bottom (highest priority, like
/// MO2 does), under `LYNO USER MODS`; one of the author's build to the end
/// of the build section. Dropped on the list (`after`), it goes there: a
/// player's mod among the build's too, [`crate::plan`] keeps it in place.
fn insert(list: &mut ModList, entry: Entry, personal: bool, after: Option<&str>) {
    if let Some(at) = after.and_then(|a| list.entries.iter().position(|e| e.name == a)) {
        list.entries.insert(at + 1, entry);
        return;
    }
    let user = list.entries.iter().position(|e| e.separator_title() == Some(USER_SEPARATOR));
    match (personal, user) {
        (true, Some(_)) => list.entries.push(entry),
        (true, None) => {
            list.entries.push(Entry::separator(USER_SEPARATOR));
            list.entries.push(entry);
        }
        (false, Some(at)) => list.entries.insert(at, entry),
        (false, None) => list.entries.push(entry),
    }
}

/// Moves the archive into MO2's `downloads/` with the `.meta` MO2 writes
/// for a Nexus download; `installed` marks it as installed in MO2's list.
pub fn to_downloads(inst: &Instance, archive_path: &Path, dl: &Download, installed: bool) -> Result<PathBuf> {
    let dir = inst.downloads_dir();
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let name = folder_name(&dl.file_name);
    let dest = dir.join(&name);
    if dest != archive_path {
        if dest.exists() {
            std::fs::remove_file(&dest).map_err(|e| Error::io(&dest, e))?;
        }
        if std::fs::rename(archive_path, &dest).is_err() {
            // Another drive: copy instead.
            std::fs::copy(archive_path, &dest).map_err(|e| Error::io(&dest, e))?;
            let _ = std::fs::remove_file(archive_path);
        }
    }
    let meta = dir.join(format!("{name}.meta"));
    // MO2's own `.meta` of the same file has more (url, category): keep it, flip the flag.
    if dest == archive_path && meta.is_file() {
        let old = std::fs::read_to_string(&meta).map_err(|e| Error::io(&meta, e))?;
        let flag = format!("installed={installed}");
        let mut found = false;
        let mut lines: Vec<String> = old
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("installed=") {
                    found = true;
                    flag.clone()
                } else {
                    l.to_owned()
                }
            })
            .collect();
        if !found {
            let at = lines.iter().position(|l| l.trim() == "[General]").map_or(lines.len(), |i| i + 1);
            lines.insert(at, flag);
        }
        std::fs::write(&meta, lines.join("\r\n") + "\r\n").map_err(|e| Error::io(&meta, e))?;
        return Ok(dest);
    }
    let text = format!(
        "[General]\r\ngameName={game}\r\nmodID={mod_id}\r\nfileID={file_id}\r\nurl=\r\nname={title}\r\ndescription=\r\n\
         modName={mod_name}\r\nversion={version}\r\nnewestVersion=\r\nfileCategory=0\r\ncategory=0\r\nrepository=Nexus\r\n\
         installed={installed}\r\nuninstalled=false\r\npaused=false\r\nremoved=false\r\n",
        game = dl.game,
        mod_id = dl.mod_id,
        file_id = dl.file_id,
        title = ini_value(&dl.file_title),
        mod_name = ini_value(&dl.mod_name),
        version = ini_value(dl.version.as_deref().unwrap_or_default()),
    );
    std::fs::write(&meta, text).map_err(|e| Error::io(&meta, e))?;
    Ok(dest)
}

/// An archive in MO2's `downloads/`, for the launcher's downloads list.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadItem {
    pub file_name: String,
    pub size: u64,
    /// Unix seconds of the last change.
    pub modified: u64,
    /// From the `.meta` MO2 or the launcher wrote; the file name otherwise.
    pub mod_name: String,
    pub file_title: Option<String>,
    pub version: Option<String>,
    pub mod_id: u64,
    pub file_id: u64,
    pub installed: bool,
}

const ARCHIVE_EXTS: &[&str] = &["zip", "7z", "rar"];

/// The newest archives of `downloads/`, newest first. MO2's hidden ones
/// (`removed=true`) are left out, as in its Downloads tab.
pub fn recent_downloads(inst: &Instance, limit: usize) -> Result<Vec<DownloadItem>> {
    let dir = inst.downloads_dir();
    let Ok(read) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut out = Vec::new();
    for entry in read {
        let entry = entry.map_err(|e| Error::io(&dir, e))?;
        let path = entry.path();
        let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if !ARCHIVE_EXTS.contains(&ext.as_str()) || !path.is_file() {
            continue;
        }
        let meta = entry.metadata().map_err(|e| Error::io(&path, e))?;
        let ini = read_meta(&path);
        if ini.get("removed").is_some_and(|v| v == "true") {
            continue;
        }
        let dl = download_from(&path, &ini);
        out.push(DownloadItem {
            file_name: dl.file_name.clone(),
            size: meta.len(),
            modified: meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()),
            mod_name: dl.mod_name,
            file_title: Some(dl.file_title).filter(|t| !t.is_empty()),
            version: dl.version,
            mod_id: dl.mod_id,
            file_id: dl.file_id,
            installed: ini.get("installed").is_some_and(|v| v == "true"),
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.file_name.cmp(&b.file_name)));
    out.truncate(limit);
    Ok(out)
}

/// `[General]` of `<archive>.meta`, unquoted. Empty without one.
fn read_meta(archive_path: &Path) -> HashMap<String, String> {
    let mut path = archive_path.as_os_str().to_owned();
    path.push(".meta");
    let Ok(text) = std::fs::read_to_string(&path) else { return HashMap::new() };
    let mut general = false;
    let mut out = HashMap::new();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            general = line.eq_ignore_ascii_case("[General]");
        } else if let (true, Some((k, v))) = (general, line.split_once('=')) {
            let v = v.trim();
            let v = match v.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                Some(q) => q.replace("\\\"", "\"").replace("\\\\", "\\"),
                None => v.to_owned(),
            };
            out.insert(k.trim().to_owned(), v);
        }
    }
    out
}

fn download_from(path: &Path, ini: &HashMap<String, String>) -> Download {
    let guessed = Download::from_file(path);
    let num = |k: &str| ini.get(k).and_then(|v| v.parse::<u64>().ok()).filter(|n| *n > 0);
    let text = |k: &str| ini.get(k).map(|v| v.trim().to_owned()).filter(|v| !v.is_empty());
    let mod_id = num("modID").unwrap_or(guessed.mod_id);
    Download {
        game: text("gameName").map(|g| g.to_lowercase()).unwrap_or(if mod_id != 0 { DEFAULT_GAME.into() } else { String::new() }),
        mod_id,
        file_id: num("fileID").unwrap_or(0),
        mod_name: text("modName").unwrap_or(guessed.mod_name),
        file_title: text("name").unwrap_or_default(),
        version: text("version").or(guessed.version),
        file_name: guessed.file_name,
        // Already MO2's: stays in `downloads/`, its `.meta` gets the installed flag.
        local: ini.is_empty(),
    }
}

/// What a file in `downloads/` (or from the player's disk) knows about itself.
pub fn download_for(path: &Path) -> Download {
    download_from(path, &read_meta(path))
}

/// Which mod a downloaded archive goes to, without asking Nexus: the mod
/// installed from the same file or one it replaces (by the files the last
/// check recorded), else a new one. A player's optional build mod (`manifest`:
/// the installed build) becomes theirs; `Err`: a core one.
pub fn target_for(
    inst: &Instance,
    profile: &str,
    dl: &Download,
    author: bool,
    manifest: Option<&Manifest>,
    after: Option<String>,
) -> Result<std::result::Result<Target, BuildMod>> {
    let new = Target::New { personal: !author, after };
    if dl.mod_id == 0 || !inst.modlist_path(profile).is_file() {
        return Ok(Ok(new));
    }
    let (tracked, _) = tracking::tracked_mods(inst, profile)?;
    let cache = Cache::load(&Cache::path(inst));
    let files = cache.get(&dl.game, dl.mod_id).map(|c| c.files.clone()).unwrap_or_default();
    // A file id of 0 (guessed from the name) matches no install: a new mod, as MO2 does.
    let replaces = (dl.file_id != 0).then(|| tracking::target(&tracked, &files, &dl.game, dl.mod_id, dl.file_id)).flatten();
    Ok(match replaces {
        Some(t) if t.managed && !author => match manifest.map(|m| crate::install::optional_in(inst, m, &t.folder)).transpose()?.flatten() {
            Some(id) => Ok(Target::Own { folder: t.folder.clone(), id }),
            None => Err(BuildMod(t.folder.clone())),
        },
        Some(t) => Ok(Target::Replace(t.folder.clone())),
        None => Ok(new),
    })
}

/// The archive a mod was installed from, to install it again: MO2 records a
/// file name in `downloads/` or, for one installed from elsewhere, a full path.
pub fn source_archive(inst: &Instance, folder: &str) -> Option<PathBuf> {
    let meta = ModMeta::load(&inst.mods_dir().join(folder).join("meta.ini")).ok()?;
    let file = PathBuf::from(meta.installation_file.filter(|f| !f.is_empty())?);
    let path = if file.is_absolute() { file } else { inst.downloads_dir().join(file) };
    path.is_file().then_some(path)
}

/// `modlist.txt` of every profile: MO2 drops or renames a mod in all of them,
/// or a profile not open now would lose the mod's place and state.
fn each_modlist(inst: &Instance, mut f: impl FnMut(&mut ModList) -> bool) -> Result<()> {
    let dir = inst.root().join("profiles");
    let Ok(profiles) = std::fs::read_dir(&dir) else { return Ok(()) };
    for p in profiles {
        let path = p.map_err(|e| Error::io(&dir, e))?.path().join("modlist.txt");
        if path.is_file() {
            let mut list = ModList::load(&path)?;
            if f(&mut list) {
                list.save(&path)?;
            }
        }
    }
    Ok(())
}

/// Deletes a mod: its folder and its entry in every profile. The folder is
/// first moved aside, so a file held open by another program fails the
/// delete before the lists change rather than leaving half a mod behind.
pub fn remove(inst: &Instance, folder: &str) -> Result<()> {
    let dir = inst.mods_dir().join(folder);
    let trash = inst.root().join(".lyno").join("staging").join("deleted");
    if trash.exists() {
        std::fs::remove_dir_all(&trash).map_err(|e| Error::io(&trash, e))?;
    }
    std::fs::create_dir_all(&trash).map_err(|e| Error::io(&trash, e))?;
    // Already gone from the disk: only the lists still name it.
    if dir.exists() {
        std::fs::remove_dir(&trash).map_err(|e| Error::io(&trash, e))?;
        std::fs::rename(&dir, &trash).map_err(|e| Error::io(&dir, e))?;
    }
    each_modlist(inst, |list| {
        let len = list.entries.len();
        list.entries.retain(|e| e.name != folder);
        list.entries.len() != len
    })?;
    std::fs::remove_dir_all(&trash).map_err(|e| Error::io(&trash, e))
}

/// Why a folder name can't be a mod's, in words for the player.
pub fn invalid_name(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("Название не может быть пустым")
    } else if name.contains(['<', '>', ':', '"', '/', '\\', '|', '?', '*']) || name.chars().any(char::is_control) {
        Some("Windows не разрешает в названии папки символы < > : \" / \\ | ? *")
    } else if name.ends_with(['.', ' ']) || name.starts_with(' ') {
        Some("Название не может начинаться с пробела или заканчиваться точкой и пробелом")
    } else if Entry::enabled(name).is_separator() {
        Some("Так MO2 называет разделители: выберите другое название")
    } else {
        None
    }
}

/// Renames a mod's folder and its entry in every profile, keeping its place
/// and state. `to` passed [`invalid_name`] and names no other mod.
pub fn rename(inst: &Instance, from: &str, to: &str) -> Result<()> {
    let mods = inst.mods_dir();
    std::fs::rename(mods.join(from), mods.join(to)).map_err(|e| Error::io(mods.join(from), e))?;
    each_modlist(inst, |list| {
        let mut changed = false;
        for e in list.entries.iter_mut().filter(|e| e.name == from) {
            e.name = to.to_owned();
            changed = true;
        }
        changed
    })
}

/// Index of the player's first entry: right under `LYNO USER MODS`, or
/// without the build past the unmanaged entries (DLC) MO2 keeps lowest.
/// `None`: the build is installed and the player has no section yet.
fn user_start(list: &ModList, has_build: bool) -> Option<usize> {
    match list.entries.iter().position(|e| e.separator_title() == Some(USER_SEPARATOR)) {
        Some(sep) => Some(sep + 1),
        None if has_build => None,
        None => Some(list.entries.iter().take_while(|e| e.state == EntryState::Unmanaged).count()),
    }
}

/// Whether `name` (a mod or a separator) is in the player's section.
pub fn is_users(list: &ModList, has_build: bool, name: &str) -> bool {
    user_start(list, has_build).is_some_and(|s| list.entries[s..].iter().any(|e| e.name == name && e.state != EntryState::Unmanaged))
}

/// Moves an entry of the player right below `after` (`None`: the end of the
/// list). Their mod goes anywhere, among the build's too ([`crate::plan`]
/// keeps it there); their separator only within their section, or MO2 would
/// put build mods under it: an `after` outside means the section's top, as
/// does one not in the list. `false`: `name` is the build's (`managed`: a
/// build mod's folder; the build's separators are the ones above the section).
pub fn move_entry(list: &mut ModList, has_build: bool, managed: impl Fn(&str) -> bool, name: &str, after: Option<&str>) -> bool {
    let pos = |list: &ModList, n: &str| list.entries.iter().position(|e| e.name == n && e.state != EntryState::Unmanaged);
    let Some(from) = pos(list, name) else { return false };
    let separator = list.entries[from].is_separator();
    if managed(name) || separator && !user_start(list, has_build).is_some_and(|s| from >= s) {
        return false;
    }
    let entry = list.entries.remove(from);
    let start = user_start(list, has_build);
    let to = match (after, after.and_then(|a| pos(list, a))) {
        (_, Some(i)) if !separator || start.is_some_and(|s| i + 1 >= s) => i + 1,
        (None, _) => list.entries.len(),
        _ => start.unwrap_or_else(|| {
            list.entries.push(Entry::separator(USER_SEPARATOR));
            list.entries.len()
        }),
    };
    list.entries.insert(to, entry);
    true
}

/// Adds a separator of the player into the profile's list below `after` (as
/// in [`move_entry`]). MO2 makes one an empty folder `<title>_separator`; it
/// writes the `meta.ini` itself. Returns the folder.
pub fn add_separator(inst: &Instance, profile: &str, title: &str, after: Option<&str>) -> Result<String> {
    let entry = Entry::separator(title);
    let name = entry.name.clone();
    let dir = inst.mods_dir().join(&name);
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    let path = inst.modlist_path(profile);
    let mut list = ModList::load(&path)?;
    let has_build = crate::install::has_build(inst);
    if user_start(&list, has_build).is_none() {
        list.entries.push(Entry::separator(USER_SEPARATOR));
    }
    list.entries.push(entry);
    move_entry(&mut list, has_build, |_| false, &name, after);
    list.save(&path)?;
    Ok(name)
}

/// Files in `overwrite/`: what the game and its tools wrote through MO2's
/// virtual file system (CET and RED4ext configs, logs), above every mod.
pub fn overwrite_files(inst: &Instance) -> Result<Vec<crate::tree::FileEntry>> {
    let dir = inst.overwrite_dir();
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    crate::tree::list_files(&dir)
}

/// MO2's "Create Mod" of `overwrite/`: its folder becomes the mod `name`,
/// enabled at the end of the player's section; `overwrite/` starts empty.
/// `name` passed [`invalid_name`] and names no other mod.
pub fn overwrite_to_mod(inst: &Instance, profile: &str, name: &str) -> Result<()> {
    let from = inst.overwrite_dir();
    let mods = inst.mods_dir();
    std::fs::create_dir_all(&mods).map_err(|e| Error::io(&mods, e))?;
    std::fs::rename(&from, mods.join(name)).map_err(|e| Error::io(&from, e))?;
    std::fs::create_dir_all(&from).map_err(|e| Error::io(&from, e))?;
    let path = inst.modlist_path(profile);
    let mut list = ModList::load(&path)?;
    insert(&mut list, Entry::enabled(name), crate::install::has_build(inst), None);
    list.save(&path)
}

/// Empties `overwrite/`. Moved aside first, like [`remove`]: a file held open
/// fails the whole clear instead of leaving half of it.
pub fn clear_overwrite(inst: &Instance) -> Result<()> {
    let dir = inst.overwrite_dir();
    if !dir.is_dir() {
        return Ok(());
    }
    let trash = inst.root().join(".lyno").join("staging").join("overwrite-cleared");
    if trash.exists() {
        std::fs::remove_dir_all(&trash).map_err(|e| Error::io(&trash, e))?;
    }
    std::fs::create_dir_all(trash.parent().unwrap()).map_err(|e| Error::io(&trash, e))?;
    std::fs::rename(&dir, &trash).map_err(|e| Error::io(&dir, e))?;
    std::fs::create_dir_all(&dir).map_err(|e| Error::io(&dir, e))?;
    std::fs::remove_dir_all(&trash).map_err(|e| Error::io(&trash, e))
}

/// Qt reads an unquoted value up to the line end; quotes keep commas and
/// semicolons, which Qt would otherwise split or cut at.
fn ini_value(v: &str) -> String {
    let v = v.replace(['\r', '\n'], " ");
    if v.contains([',', ';', '"', '=']) {
        format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    fn download(file_id: u64, version: &str) -> Download {
        Download {
            game: "cyberpunk2077".into(),
            mod_id: 42,
            file_id,
            mod_name: "Cool: Mod".into(),
            file_title: "Cool: Mod".into(),
            version: Some(version.into()),
            file_name: format!("Cool Mod-42-{file_id}.zip"),
            local: false,
        }
    }

    /// An instance with the build installed.
    fn instance(list: &str) -> (tempfile::TempDir, Instance) {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        std::fs::create_dir_all(inst.profile_dir("LYNO")).unwrap();
        std::fs::write(inst.modlist_path("LYNO"), list).unwrap();
        let state = crate::state::State { build_version: Some("1.0".into()), ..Default::default() };
        state.save(&crate::install::state_path(&inst)).unwrap();
        (dir, inst)
    }

    fn names(inst: &Instance) -> Vec<String> {
        ModList::load(&inst.modlist_path("LYNO")).unwrap().entries.into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn without_the_build_a_mod_goes_to_the_end_of_the_list() {
        let (dir, inst) = instance("+Mine\r\n");
        std::fs::remove_file(crate::install::state_path(&inst)).unwrap();
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("Cool/archive/pc/mod/a.archive", b"v1")]);
        install(&inst, "LYNO", &archive, &download(1, "1.0"), &Target::New { personal: true, after: None }).unwrap();
        assert_eq!(names(&inst), ["Mine", "Cool_ Mod"]);
    }

    #[test]
    fn installs_new_personal_mod_and_updates_it_in_place() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("Cool/archive/pc/mod/a.archive", b"v1")]);
        let out = install(&inst, "LYNO", &archive, &download(1, "1.0"), &Target::New { personal: true, after: None }).unwrap();
        assert_eq!(out, Outcome::Installed { folder: "Cool_ Mod".into() });
        assert_eq!(names(&inst), ["Build Mod", "LYNO USER MODS_separator", "Cool_ Mod"]);
        let meta = ModMeta::load(&inst.mods_dir().join("Cool_ Mod/meta.ini")).unwrap();
        assert_eq!((meta.mod_id, meta.file_id, meta.version.as_deref()), (Some(42), Some(1), Some("1.0")));
        assert!(inst.downloads_dir().join("Cool Mod-42-1.zip").is_file());
        let dl_meta = std::fs::read_to_string(inst.downloads_dir().join("Cool Mod-42-1.zip.meta")).unwrap();
        assert!(dl_meta.contains("modID=42\r\n") && dl_meta.contains("installed=true"), "{dl_meta}");

        // The author's [LYNO] id survives an update; the list keeps the place and state.
        let ini = inst.mods_dir().join("Cool_ Mod/meta.ini");
        std::fs::write(&ini, std::fs::read_to_string(&ini).unwrap() + "\n[LYNO]\nid=cool\n").unwrap();
        let mut list = ModList::load(&inst.modlist_path("LYNO")).unwrap();
        list.entries[2].state = crate::modlist::EntryState::Disabled;
        list.save(&inst.modlist_path("LYNO")).unwrap();
        let archive = dir.path().join("dl2.zip");
        zip_with(&archive, &[("archive/pc/mod/b.archive", b"v2")]);
        install(&inst, "LYNO", &archive, &download(2, "2.0"), &Target::Replace("Cool_ Mod".into())).unwrap();
        let folder = inst.mods_dir().join("Cool_ Mod");
        assert!(!folder.join("archive/pc/mod/a.archive").exists(), "old files are replaced");
        assert_eq!(std::fs::read(folder.join("archive/pc/mod/b.archive")).unwrap(), b"v2");
        let meta = ModMeta::load(&folder.join("meta.ini")).unwrap();
        assert_eq!((meta.file_id, meta.version.as_deref(), meta.lyno_id.as_deref()), (Some(2), Some("2.0"), Some("cool")));
        let list = ModList::load(&inst.modlist_path("LYNO")).unwrap();
        assert_eq!(list.get("Cool_ Mod").unwrap().state, crate::modlist::EntryState::Disabled);
        assert_eq!(list.entries.len(), 3);
    }

    #[test]
    fn author_mod_goes_to_the_end_of_the_build_section() {
        let (dir, inst) = instance("+Mine\r\n-LYNO USER MODS_separator\r\n+Build Mod\r\n");
        std::fs::create_dir_all(inst.mods_dir().join("Cool_ Mod")).unwrap();
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("r6/scripts/x.reds", b"x")]);
        let out = install(&inst, "LYNO", &archive, &download(1, "1"), &Target::New { personal: false, after: None }).unwrap();
        assert_eq!(out, Outcome::Installed { folder: "Cool_ Mod (2)".into() });
        assert_eq!(names(&inst), ["Build Mod", "Cool_ Mod (2)", "LYNO USER MODS_separator", "Mine"]);
    }

    #[test]
    fn unreadable_fomod_asks_for_the_mod_folder() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("fomod/ModuleConfig.xml", b"<nope/>"), ("A/archive/pc/mod/a.archive", b"a")]);
        let target = Target::New { personal: true, after: None };
        let out = install(&inst, "LYNO", &archive, &download(1, "1"), &target).unwrap();
        let kept = inst.downloads_dir().join("Cool Mod-42-1.zip");
        assert_eq!(out, Outcome::Manual { archive: kept.clone(), target: target.clone() });
        let roots = roots(&kept).unwrap();
        assert!(roots.iter().any(|r| r.path == "A/" && r.valid), "{roots:?}");
        let out = install_root(&inst, "LYNO", &kept, &download(1, "1"), &target, "A/").unwrap();
        assert_eq!(out, Outcome::Installed { folder: "Cool_ Mod".into() });
        assert_eq!(std::fs::read(inst.mods_dir().join("Cool_ Mod/archive/pc/mod/a.archive")).unwrap(), b"a");
        assert!(install_root(&inst, "LYNO", &kept, &download(1, "1"), &target, "Nope/").is_err());
        assert_eq!(names(&inst), ["Build Mod", "LYNO USER MODS_separator", "Cool_ Mod"]);
        let meta = std::fs::read_to_string(inst.downloads_dir().join("Cool Mod-42-1.zip.meta")).unwrap();
        assert!(meta.contains("installed=true\r\n") && meta.contains("modName=Cool: Mod\r\n"), "{meta}");
        assert!(!archive.exists());
    }

    const CONFIG: &str = r#"<config><moduleName>Cool</moduleName>
      <requiredInstallFiles><folder source="Core" destination="" /></requiredInstallFiles>
      <installSteps order="Explicit"><installStep name="Look"><optionalFileGroups>
        <group name="Color" type="SelectExactlyOne"><plugins order="Explicit">
          <plugin name="Red"><description>red</description><image path="fomod\red.png" />
            <files><file source="Options\Red\color.archive" destination="archive/pc/mod/color.archive" /></files>
            <typeDescriptor><type name="Optional" /></typeDescriptor></plugin>
          <plugin name="Blue"><description>blue</description>
            <files><folder source="options/blue" destination="" /></files>
            <typeDescriptor><type name="Optional" /></typeDescriptor></plugin>
        </plugins></group>
      </optionalFileGroups></installStep></installSteps></config>"#;

    fn fomod_zip(path: &Path) {
        zip_with(
            path,
            &[
                ("Cool/fomod/ModuleConfig.xml", CONFIG.as_bytes()),
                ("Cool/fomod/red.png", b"PNG"),
                ("Cool/Core/r6/scripts/core.reds", b"core"),
                ("Cool/Core/readme.txt", b"dropped like MO2 does"),
                ("Cool/Options/Red/color.archive", b"red"),
                ("Cool/Options/Blue/blue.archive", b"blue"),
            ],
        );
    }

    #[test]
    fn fomod_waits_for_a_choice_and_remembers_it() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("dl.zip");
        fomod_zip(&archive);
        let target = Target::New { personal: true, after: None };
        let out = install(&inst, "LYNO", &archive, &download(1, "1"), &target).unwrap();
        let Outcome::Fomod { archive: kept, target: t } = out else { panic!("{out:?}") };
        assert_eq!(t, target);
        assert_eq!(kept, inst.downloads_dir().join("Cool Mod-42-1.zip"));
        assert_eq!(names(&inst), ["Build Mod"], "nothing installed before the choice");

        let f = open_fomod(&inst, &kept, &target).unwrap();
        assert_eq!(f.previous, None);
        assert_eq!(f.images().unwrap()["fomod/red.png"], "data:image/png;base64,UE5H");
        // Blue: a folder in another case than the archive.
        let out = install_fomod(&inst, "LYNO", &f, &download(1, "1"), &target, &vec![Some(vec![vec![1]])]).unwrap();
        assert_eq!(out, Outcome::Installed { folder: "Cool_ Mod".into() });
        let folder = inst.mods_dir().join("Cool_ Mod");
        assert_eq!(std::fs::read(folder.join("r6/scripts/core.reds")).unwrap(), b"core");
        assert_eq!(std::fs::read(folder.join("archive/pc/mod/blue.archive")).unwrap(), b"blue");
        assert!(!folder.join("archive/pc/mod/color.archive").exists());
        assert!(!folder.join("readme.txt").exists());
        assert!(!folder.join("fomod").exists());
        let meta = ModMeta::load(&folder.join("meta.ini")).unwrap();
        assert!(meta.lyno_fomod.is_some());
        let dl_meta = std::fs::read_to_string(inst.downloads_dir().join("Cool Mod-42-1.zip.meta")).unwrap();
        assert!(dl_meta.contains("installed=true"), "{dl_meta}");
        assert_eq!(names(&inst), ["Build Mod", "LYNO USER MODS_separator", "Cool_ Mod"]);

        // The next version starts from the old choice.
        let archive = dir.path().join("dl2.zip");
        fomod_zip(&archive);
        let replace = Target::Replace("Cool_ Mod".into());
        let Outcome::Fomod { archive: kept, .. } = install(&inst, "LYNO", &archive, &download(2, "2"), &replace).unwrap() else { panic!() };
        let f = open_fomod(&inst, &kept, &replace).unwrap();
        assert_eq!(f.previous, Some(vec![Some(vec![vec![1]])]));
        install_fomod(&inst, "LYNO", &f, &download(2, "2"), &replace, &vec![Some(vec![vec![0]])]).unwrap();
        assert_eq!(std::fs::read(folder.join("archive/pc/mod/color.archive")).unwrap(), b"red");
        assert!(!folder.join("archive/pc/mod/blue.archive").exists(), "the old option is gone");
        // A choice the group doesn't allow is refused.
        let err = install_fomod(&inst, "LYNO", &f, &download(2, "2"), &replace, &vec![Some(vec![vec![]])]);
        assert!(matches!(err, Err(Error::Fomod(_))), "{err:?}");
    }

    #[test]
    fn file_dependencies_look_at_enabled_mods() {
        let (_dir, inst) = instance("+On\r\n-Off\r\n");
        for (m, f) in [("On", "a.archive"), ("Off", "b.archive")] {
            std::fs::create_dir_all(inst.mods_dir().join(m).join("archive/pc/mod")).unwrap();
            std::fs::write(inst.mods_dir().join(m).join("archive/pc/mod").join(f), b"").unwrap();
        }
        let states = file_states(&inst, "LYNO");
        assert_eq!(states("archive/pc/mod/a.archive"), FileState::Active);
        assert_eq!(states("archive/pc/mod/b.archive"), FileState::Inactive);
        assert_eq!(states("archive/pc/mod/c.archive"), FileState::Missing);
    }

    #[test]
    fn guesses_nexus_files_by_name() {
        let dl = Download::from_file(Path::new("C:/Users/v/Desktop/Better Lightning - HDR-15520-3-2-1735000000.7z"));
        assert_eq!((dl.mod_name.as_str(), dl.mod_id, dl.version.as_deref()), ("Better Lightning - HDR", 15520, Some("3.2")));
        assert_eq!((dl.game.as_str(), dl.file_id, dl.local), ("cyberpunk2077", 0, true));
        let plain = Download::from_file(Path::new("my-cool-mod.zip"));
        assert_eq!((plain.mod_name.as_str(), plain.mod_id, plain.version), ("my-cool-mod", 0, None));
        assert_eq!(plain.file_name, "my-cool-mod.zip");
    }

    #[test]
    fn dropped_mods_land_where_dropped() {
        let list = |text: &str| ModList::parse(text, Path::new("m")).unwrap();
        let names = |l: &ModList| l.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        // UI order: A, B, separator, Mine, Other.
        let base = "+Other\r\n+Mine\r\n-LYNO USER MODS_separator\r\n+B\r\n+A\r\n";
        let mut l = list(base);
        insert(&mut l, Entry::enabled("New"), true, Some("Mine"));
        assert_eq!(names(&l), ["A", "B", "LYNO USER MODS_separator", "Mine", "New", "Other"]);
        // Among build mods too, the player's and the author's alike.
        for personal in [true, false] {
            let mut l = list(base);
            insert(&mut l, Entry::enabled("New"), personal, Some("A"));
            assert_eq!(names(&l), ["A", "New", "B", "LYNO USER MODS_separator", "Mine", "Other"]);
        }
        let mut l = list(base);
        insert(&mut l, Entry::enabled("New"), true, Some("Gone"));
        assert_eq!(names(&l).last().unwrap(), "New");
    }

    #[test]
    fn players_own_archive_stays_where_it_is() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("my-mod.zip");
        zip_with(&archive, &[("r6/scripts/x.reds", b"x")]);
        let dl = Download::from_file(&archive);
        let target = Target::New { personal: true, after: Some("Build Mod".into()) };
        assert_eq!(install(&inst, "LYNO", &archive, &dl, &target).unwrap(), Outcome::Installed { folder: "my-mod".into() });
        assert!(archive.is_file(), "not moved");
        assert!(!inst.downloads_dir().exists());
        let meta = std::fs::read_to_string(inst.mods_dir().join("my-mod/meta.ini")).unwrap();
        assert!(meta.contains("installationFile=my-mod.zip") && !meta.contains("modid") && !meta.contains("repository"), "{meta}");
    }

    #[test]
    fn downloads_list_and_targets() {
        let (dir, inst) = instance("+Cool_ Mod\r\n-LYNO USER MODS_separator\r\n");
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("archive/pc/mod/a.archive", b"1")]);
        install(&inst, "LYNO", &archive, &download(7, "1.0"), &Target::New { personal: true, after: None }).unwrap();
        // MO2's own download, hidden ones and stray files.
        std::fs::write(inst.downloads_dir().join("Other-9-2-0-1735000000.rar"), b"Rar!").unwrap();
        std::fs::write(
            inst.downloads_dir().join("Other-9-2-0-1735000000.rar.meta"),
            "[General]\r\nmodID=9\r\nfileID=90\r\nmodName=\"Other, the mod\"\r\nname=Main\r\nversion=2.0\r\ninstalled=false\r\nurl=keep me\r\n",
        )
        .unwrap();
        std::fs::write(inst.downloads_dir().join("hidden.zip"), b"PK").unwrap();
        std::fs::write(inst.downloads_dir().join("hidden.zip.meta"), "[General]\r\nremoved=true\r\n").unwrap();
        std::fs::write(inst.downloads_dir().join("notes.txt"), b"").unwrap();

        let items = recent_downloads(&inst, 10).unwrap();
        let mut seen: Vec<(&str, &str, u64, bool)> = items.iter().map(|i| (i.file_name.as_str(), i.mod_name.as_str(), i.mod_id, i.installed)).collect();
        seen.sort();
        assert_eq!(seen, [("Cool Mod-42-7.zip", "Cool: Mod", 42, true), ("Other-9-2-0-1735000000.rar", "Other, the mod", 9, false)]);
        assert_eq!(recent_downloads(&inst, 1).unwrap().len(), 1);

        // The same file again replaces its mod; another file of the page is a new mod.
        let same = download_for(&inst.downloads_dir().join("Cool Mod-42-7.zip"));
        assert_eq!((same.mod_id, same.file_id, same.local), (42, 7, false));
        assert_eq!(target_for(&inst, "LYNO", &same, false, None, None).unwrap().unwrap(), Target::Replace("Cool_ Mod".into()));
        let other = download_for(&inst.downloads_dir().join("Other-9-2-0-1735000000.rar"));
        assert_eq!((other.mod_name.as_str(), other.file_id, other.version.as_deref()), ("Other, the mod", 90, Some("2.0")));
        assert_eq!(target_for(&inst, "LYNO", &other, false, None, Some("x".into())).unwrap().unwrap(), Target::New { personal: true, after: Some("x".into()) });

        // Installing MO2's download keeps its `.meta` and only flips the flag.
        let p = inst.downloads_dir().join("Other-9-2-0-1735000000.rar");
        to_downloads(&inst, &p, &other, true).unwrap();
        let meta = std::fs::read_to_string(inst.downloads_dir().join("Other-9-2-0-1735000000.rar.meta")).unwrap();
        assert!(meta.contains("installed=true") && meta.contains("url=keep me") && !meta.contains("installed=false"), "{meta}");
    }

    #[test]
    fn cleans_folder_names() {
        assert_eq!(folder_name("A/B: C?"), "A_B_ C_");
        assert_eq!(folder_name(" trailing. "), "trailing");
        assert_eq!(folder_name("x_separator"), "x_separator mod");
        assert_eq!(folder_name(""), "Nexus mod");
    }

    #[test]
    fn names_addons_apart_from_the_main_file() {
        let dl = |title: &str| Download { mod_name: "Cool Mod".into(), file_title: title.into(), ..Default::default() };
        assert_eq!(dl("").folder(), "Cool Mod");
        assert_eq!(dl("cool mod").folder(), "Cool Mod");
        assert_eq!(dl("Cool Mod - Extra Outfits").folder(), "Cool Mod - Extra Outfits");
        assert_eq!(dl("Extra Outfits").folder(), "Cool Mod - Extra Outfits");
    }

    #[test]
    fn remove_and_rename_reach_every_profile() {
        let (_d, inst) = instance("+Keep\n-Old\n");
        std::fs::create_dir_all(inst.profile_dir("Other")).unwrap();
        std::fs::write(inst.modlist_path("Other"), "+Old\n+Gone\n").unwrap();
        for m in ["Keep", "Old", "Gone"] {
            std::fs::create_dir_all(inst.mods_dir().join(m)).unwrap();
        }
        std::fs::write(inst.mods_dir().join("Gone/file.archive"), b"x").unwrap();

        rename(&inst, "Old", "New").unwrap();
        remove(&inst, "Gone").unwrap();
        assert!(inst.mods_dir().join("New").is_dir() && !inst.mods_dir().join("Gone").exists());
        assert_eq!(names(&inst), ["New", "Keep"]);
        let other = ModList::load(&inst.modlist_path("Other")).unwrap();
        assert_eq!(other.entries, [Entry::enabled("New")]);
        // The state goes with the name.
        assert_eq!(ModList::load(&inst.modlist_path("LYNO")).unwrap().get("New").unwrap().state, EntryState::Disabled);

        assert!(invalid_name("a/b").is_some() && invalid_name("x.").is_some() && invalid_name("Foo_separator").is_some());
        assert!(invalid_name("Cool Mod 1.2").is_none());
    }

    #[test]
    fn moves_players_entries() {
        let list = |text: &str| ModList::parse(text, Path::new("modlist.txt")).unwrap();
        let order = |l: &ModList| l.entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
        let managed = |n: &str| n.starts_with("Build");
        // Highest priority first in the file; UI order is the reverse.
        let mut l = list("+C\n+B\n-Mine_separator\n+A\n-LYNO USER MODS_separator\n+Build 2\n-Guns_separator\n+Build\n");
        assert!(move_entry(&mut l, true, managed, "C", Some("A")));
        assert_eq!(order(&l), ["Build", "Guns_separator", "Build 2", "LYNO USER MODS_separator", "A", "C", "Mine_separator", "B"]);
        // A mod goes among the build's, under a build separator too.
        assert!(move_entry(&mut l, true, managed, "B", Some("Guns_separator")));
        assert_eq!(order(&l), ["Build", "Guns_separator", "B", "Build 2", "LYNO USER MODS_separator", "A", "C", "Mine_separator"]);
        assert!(move_entry(&mut l, true, managed, "B", None));
        assert_eq!(order(&l)[7], "B");
        // A separator of the player stays in their section: dropped on the build's, it goes to its top.
        assert!(move_entry(&mut l, true, managed, "Mine_separator", Some("Build")));
        assert_eq!(order(&l)[4], "Mine_separator");
        assert!(!move_entry(&mut l, true, managed, "Build", None), "a build mod stays");
        assert!(!move_entry(&mut l, true, managed, "Guns_separator", None), "so does a build separator");
        assert!(is_users(&l, true, "Mine_separator") && !is_users(&l, true, "Build"));
        assert!(!move_entry(&mut l, true, managed, "LYNO USER MODS_separator", None));

        // Without the build every mod is the player's, DLC entries stay lowest.
        let mut l = list("+B\n+A\n*DLC: EP1\n");
        assert!(move_entry(&mut l, false, |_| false, "B", Some("nothing")));
        assert_eq!(order(&l), ["DLC: EP1", "B", "A"]);
        assert!(!move_entry(&mut l, false, |_| false, "DLC: EP1", None));
    }

    #[test]
    fn adds_a_separator_and_makes_a_mod_of_overwrite() {
        let (_d, inst) = instance("+Build\n");
        let sep = add_separator(&inst, "LYNO", "Mine", None).unwrap();
        assert_eq!(sep, "Mine_separator");
        assert!(inst.mods_dir().join(&sep).is_dir());
        assert_eq!(names(&inst), ["Build", USER_SEPARATOR_FOLDER, "Mine_separator"]);

        std::fs::create_dir_all(inst.overwrite_dir().join("r6/logs")).unwrap();
        std::fs::write(inst.overwrite_dir().join("r6/logs/a.log"), b"x").unwrap();
        assert_eq!(overwrite_files(&inst).unwrap().len(), 1);
        overwrite_to_mod(&inst, "LYNO", "From Overwrite").unwrap();
        assert!(inst.mods_dir().join("From Overwrite/r6/logs/a.log").is_file());
        assert!(overwrite_files(&inst).unwrap().is_empty() && inst.overwrite_dir().is_dir());
        assert_eq!(names(&inst)[3], "From Overwrite");
        assert_eq!(ModList::load(&inst.modlist_path("LYNO")).unwrap().get("From Overwrite").unwrap().state, EntryState::Enabled);

        std::fs::write(inst.overwrite_dir().join("b.ini"), b"x").unwrap();
        clear_overwrite(&inst).unwrap();
        assert!(overwrite_files(&inst).unwrap().is_empty() && inst.overwrite_dir().is_dir());
    }

    const USER_SEPARATOR_FOLDER: &str = "LYNO USER MODS_separator";
}

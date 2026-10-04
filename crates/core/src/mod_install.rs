//! Installs a mod archive downloaded from Nexus into the instance, the way
//! MO2 would: the files into `mods/<name>/`, the Nexus record into its
//! `meta.ini`, the folder into `modlist.txt`, and the archive into MO2's
//! `downloads/` with a `.meta` next to it, so MO2 shows it in its Downloads
//! tab and can reinstall it. Archives the launcher can't install
//! ([`archive::Layout::Mo2`]) only get the last step: the player installs
//! them in MO2, which reads RAR.
//!
//! A FOMOD installer stops halfway: the archive goes to `downloads/` and
//! [`Outcome::Fomod`] asks the player to choose ([`open_fomod`]), then
//! [`install_fomod`] installs the choice like any other archive.
//!
//! MO2 keeps `modlist.txt` in memory and writes it back on exit, so the
//! caller makes sure MO2 is closed before [`install`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;

use crate::archive::{self, Kind, Layout};
use crate::download::{Downloader, PartEvent};
use crate::fomod::{self, FileItem, FileState, Installer, Selection};
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Replace this mod folder: an update or a reinstall. Keeps its place and
    /// state in `modlist.txt` and the rest of its `meta.ini` (`[LYNO] id`).
    Replace(String),
    /// A new folder. `personal`: under `LYNO USER MODS`; otherwise at the end
    /// of the build section (the author's next release).
    New { personal: bool },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Outcome {
    Installed { folder: String },
    /// In MO2's downloads: the player installs it there.
    Mo2 { reason: archive::Mo2Reason },
    /// MO2 is open and would overwrite `modlist.txt` on exit: the archive is
    /// in its downloads instead, for the player to install there.
    Mo2Open,
    /// A FOMOD installer: the archive is in MO2's downloads and waits for the
    /// player's choice.
    Fomod {
        #[serde(skip)]
        archive: PathBuf,
        #[serde(skip)]
        target: Target,
    },
}

/// Where [`fetch_and_install`] is in its work.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Progress {
    /// File info is known: the job can show its name.
    #[serde(rename_all = "camelCase")]
    Resolved { mod_name: String, file_title: String, version: Option<String>, size: u64, replaces: Option<String> },
    Bytes { done: u64, total: u64 },
    #[serde(rename_all = "camelCase")]
    Retry { attempt: u32, delay_secs: u64, error: String },
    Installing,
}

/// What [`fetch_and_install`] needs to know about the instance.
pub struct Context<'a> {
    pub inst: &'a Instance,
    pub profile: &'a str,
    /// The author updates build mods; a player only their own.
    pub author: bool,
    /// MO2 is running: download, but leave the install to MO2.
    pub mo2_running: bool,
    pub now: u64,
}

/// The build mod an nxm link would replace on a player's instance: updated
/// with the build, not from Nexus.
#[derive(Debug)]
pub struct BuildMod(pub String);

/// Downloads the file of an nxm link (or of a Premium update, `link.key`
/// empty) and installs it over the mod it replaces or as a new one.
pub fn fetch_and_install(
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
        return Ok(Err(BuildMod(t.folder.clone())));
    }
    let target = match replaces {
        Some(t) => Target::Replace(t.folder.clone()),
        None => Target::New { personal: !ctx.author },
    };
    let dl = Download {
        game: link.game.clone(),
        mod_id: link.mod_id,
        file_id: link.file_id,
        mod_name: page.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| file.name.clone()),
        file_title: file.name.clone(),
        version: file.version.clone().or(page.version.clone()),
        file_name: file.file_name.clone(),
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
    let scratch = ctx.inst.root().join(".lyno").join("nexus-downloads").join(folder_name(&dl.file_name));
    downloader.fetch_file(url, size, &scratch, cancel, &mut |e| match e {
        PartEvent::Bytes(done) => on(Progress::Bytes { done, total: size }),
        PartEvent::Retry { attempt, delay, error } => on(Progress::Retry { attempt, delay_secs: delay.as_secs(), error }),
    })?;

    on(Progress::Installing);
    let outcome = if ctx.mo2_running {
        to_downloads(ctx.inst, &scratch, &dl, false)?;
        Outcome::Mo2Open
    } else {
        install(ctx.inst, ctx.profile, &scratch, &dl, &target)?
    };
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

/// Installs `archive` (downloaded to a scratch place) for `target`, then
/// moves it into `downloads/`.
pub fn install(inst: &Instance, profile: &str, archive_path: &Path, dl: &Download, target: &Target) -> Result<Outcome> {
    let kind = archive::kind(archive_path)?;
    let layout = match kind {
        Kind::Zip | Kind::SevenZip => archive::layout(&archive::entries(archive_path, kind)?),
        Kind::Rar | Kind::Unknown => Layout::Mo2(archive::Mo2Reason::Format),
    };
    let files = match layout {
        Layout::Files(files) => files,
        Layout::Fomod { .. } => {
            let archive = to_downloads(inst, archive_path, dl, false)?;
            // An installer the launcher can't read is still one MO2 may.
            return Ok(match open_fomod(inst, &archive, target) {
                Ok(_) => Outcome::Fomod { archive, target: target.clone() },
                Err(_) => Outcome::Mo2 { reason: archive::Mo2Reason::Fomod },
            });
        }
        Layout::Mo2(reason) => {
            to_downloads(inst, archive_path, dl, false)?;
            return Ok(Outcome::Mo2 { reason });
        }
    };
    place(inst, profile, archive_path, kind, &files, dl, target, None)
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
        Target::Replace(f) => f.clone(),
        Target::New { .. } => free_folder(&mods, &folder_name(&dl.mod_name)),
    };
    let staging = inst.root().join(".lyno").join("staging").join("nexus");
    if staging.exists() {
        std::fs::remove_dir_all(&staging).map_err(|e| Error::io(&staging, e))?;
    }
    archive::extract(archive_path, kind, files, &staging)?;

    let old_meta = mods.join(&folder).join("meta.ini");
    let meta_path = staging.join("meta.ini");
    if matches!(target, Target::Replace(_)) && old_meta.is_file() {
        std::fs::copy(&old_meta, &meta_path).map_err(|e| Error::io(&old_meta, e))?;
    }
    let meta = ModMeta {
        game_name: Some(dl.game.clone()),
        mod_id: Some(dl.mod_id),
        file_id: Some(dl.file_id),
        version: dl.version.clone(),
        installation_file: Some(dl.file_name.clone()),
        repository: Some("Nexus".into()),
        ..Default::default()
    };
    meta.save(&meta_path)?;
    crate::meta::save_fomod(&meta_path, fomod)?;
    package::swap_folder(&staging, &mods.join(&folder))?;

    let list_path = inst.modlist_path(profile);
    let mut list = if list_path.is_file() { ModList::load(&list_path)? } else { ModList::default() };
    if list.get(&folder).is_none() {
        let personal = !matches!(target, Target::New { personal: false });
        insert(&mut list, Entry::enabled(&folder), personal);
        if let Some(dir) = list_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        list.save(&list_path)?;
    }
    to_downloads(inst, archive_path, dl, true)?;
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
        Target::Replace(folder) => {
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
/// of the build section.
fn insert(list: &mut ModList, entry: Entry, personal: bool) {
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
            file_title: "Main File".into(),
            version: Some(version.into()),
            file_name: format!("Cool Mod-42-{file_id}.zip"),
        }
    }

    fn instance(list: &str) -> (tempfile::TempDir, Instance) {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        std::fs::create_dir_all(inst.profile_dir("LYNO")).unwrap();
        std::fs::write(inst.modlist_path("LYNO"), list).unwrap();
        (dir, inst)
    }

    fn names(inst: &Instance) -> Vec<String> {
        ModList::load(&inst.modlist_path("LYNO")).unwrap().entries.into_iter().map(|e| e.name).collect()
    }

    #[test]
    fn installs_new_personal_mod_and_updates_it_in_place() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("Cool/archive/pc/mod/a.archive", b"v1")]);
        let out = install(&inst, "LYNO", &archive, &download(1, "1.0"), &Target::New { personal: true }).unwrap();
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
        let out = install(&inst, "LYNO", &archive, &download(1, "1"), &Target::New { personal: false }).unwrap();
        assert_eq!(out, Outcome::Installed { folder: "Cool_ Mod (2)".into() });
        assert_eq!(names(&inst), ["Build Mod", "Cool_ Mod (2)", "LYNO USER MODS_separator", "Mine"]);
    }

    #[test]
    fn unreadable_fomod_goes_to_mo2_downloads() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("fomod/ModuleConfig.xml", b"<nope/>"), ("A/archive/pc/mod/a.archive", b"a")]);
        let out = install(&inst, "LYNO", &archive, &download(1, "1"), &Target::New { personal: true }).unwrap();
        assert_eq!(out, Outcome::Mo2 { reason: archive::Mo2Reason::Fomod });
        assert_eq!(names(&inst), ["Build Mod"]);
        let meta = std::fs::read_to_string(inst.downloads_dir().join("Cool Mod-42-1.zip.meta")).unwrap();
        assert!(meta.contains("installed=false\r\n") && meta.contains("modName=Cool: Mod\r\n"), "{meta}");
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
        let target = Target::New { personal: true };
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
    fn cleans_folder_names() {
        assert_eq!(folder_name("A/B: C?"), "A_B_ C_");
        assert_eq!(folder_name(" trailing. "), "trailing");
        assert_eq!(folder_name("x_separator"), "x_separator mod");
        assert_eq!(folder_name(""), "Nexus mod");
    }
}

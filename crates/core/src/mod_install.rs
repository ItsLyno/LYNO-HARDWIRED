//! Installs a mod archive downloaded from Nexus into the instance, the way
//! MO2 would: the files into `mods/<name>/`, the Nexus record into its
//! `meta.ini`, the folder into `modlist.txt`, and the archive into MO2's
//! `downloads/` with a `.meta` next to it, so MO2 shows it in its Downloads
//! tab and can reinstall it. Archives the launcher can't install
//! ([`archive::Layout::Mo2`]) only get the last step: the player installs
//! them in MO2, which runs FOMOD installers and reads RAR.
//!
//! MO2 keeps `modlist.txt` in memory and writes it back on exit, so the
//! caller makes sure MO2 is closed before [`install`].

use std::path::{Path, PathBuf};

use crate::archive::{self, Layout};
use crate::download::{Downloader, PartEvent};
use crate::meta::ModMeta;
use crate::mo2::Instance;
use crate::modlist::{Entry, ModList};
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
        archive::Kind::Zip | archive::Kind::SevenZip => archive::layout(&archive::entries(archive_path, kind)?),
        archive::Kind::Rar | archive::Kind::Unknown => Layout::Mo2(archive::Mo2Reason::Format),
    };
    let files = match layout {
        Layout::Files(files) => files,
        Layout::Mo2(reason) => {
            to_downloads(inst, archive_path, dl, false)?;
            return Ok(Outcome::Mo2 { reason });
        }
    };

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
    archive::extract(archive_path, kind, &files, &staging)?;

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
    fn fomod_goes_to_mo2_downloads() {
        let (dir, inst) = instance("+Build Mod\r\n");
        let archive = dir.path().join("dl.zip");
        zip_with(&archive, &[("fomod/ModuleConfig.xml", b"<config/>"), ("A/archive/pc/mod/a.archive", b"a")]);
        let out = install(&inst, "LYNO", &archive, &download(1, "1"), &Target::New { personal: true }).unwrap();
        assert_eq!(out, Outcome::Mo2 { reason: archive::Mo2Reason::Fomod });
        assert_eq!(names(&inst), ["Build Mod"]);
        let meta = std::fs::read_to_string(inst.downloads_dir().join("Cool Mod-42-1.zip.meta")).unwrap();
        assert!(meta.contains("installed=false\r\n") && meta.contains("modName=Cool: Mod\r\n"), "{meta}");
        assert!(!archive.exists());
    }

    #[test]
    fn cleans_folder_names() {
        assert_eq!(folder_name("A/B: C?"), "A_B_ C_");
        assert_eq!(folder_name(" trailing. "), "trailing");
        assert_eq!(folder_name("x_separator"), "x_separator mod");
        assert_eq!(folder_name(""), "Nexus mod");
    }
}

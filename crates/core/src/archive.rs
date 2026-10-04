//! Mod archives downloaded from Nexus: which ones the launcher installs
//! itself and where their files go in the mod folder.
//!
//! The rules are those of MO2's Cyberpunk plugin (`CyberpunkModDataChecker`
//! in `docs/reference/mo2-basic-games/game_cyberpunk2077.py`): a mod is
//! valid when its top level has one of the game's data folders; loose
//! `.archive` / `.xl` files go to `archive/pc/mod/`; images and text files at
//! the top are dropped. MO2's quick installer also unwraps an archive that
//! wraps everything in one folder (`ModName-1.2/archive/...`).
//!
//! Everything else goes to MO2: FOMOD installers ask the player questions,
//! RAR has no pure-Rust decoder, and an unrecognized layout needs a person to
//! look at it.

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

use crate::{Error, Result};

/// Top-level folders of a valid Cyberpunk mod.
const DATA_DIRS: &[&str] = &["archive", "engine", "r6", "mods", "red4ext", "bin"];
/// Loose files that MO2's checker moves into `archive/pc/mod/`.
const MOVED_TO_ARCHIVE: &[&str] = &[".archive", ".xl"];
const ARCHIVE_MOD_DIR: &str = "archive/pc/mod/";
/// Top-level files MO2's checker deletes: screenshots and readmes.
const DROPPED: &[&str] = &[".gif", ".jpg", ".jpeg", ".jxl", ".md", ".png", ".txt", ".webp"];
/// Wrapper folders to look through before giving up.
const MAX_DEPTH: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Zip,
    SevenZip,
    Rar,
    Unknown,
}

/// By content, not extension: authors upload `.zip` files that are 7z.
pub fn kind(path: &Path) -> Result<Kind> {
    let mut magic = [0u8; 6];
    let mut f = File::open(path).map_err(|e| Error::io(path, e))?;
    let n = f.read(&mut magic).map_err(|e| Error::io(path, e))?;
    Ok(match &magic[..n] {
        [b'P', b'K', 3, 4, ..] | [b'P', b'K', 5, 6, ..] => Kind::Zip,
        [b'7', b'z', 0xBC, 0xAF, 0x27, 0x1C] => Kind::SevenZip,
        [b'R', b'a', b'r', b'!', ..] => Kind::Rar,
        _ => Kind::Unknown,
    })
}

/// Paths of the files in an archive, with `/` separators.
pub fn entries(path: &Path, kind: Kind) -> Result<Vec<String>> {
    match kind {
        Kind::Zip => {
            let mut zip = open_zip(path)?;
            let mut out = Vec::with_capacity(zip.len());
            for i in 0..zip.len() {
                let f = zip.by_index_raw(i).map_err(|e| bad(path, e))?;
                if !f.is_dir() {
                    out.push(normalize(f.name()));
                }
            }
            Ok(out)
        }
        Kind::SevenZip => {
            let archive = sevenz_rust2::Archive::open(path).map_err(|e| bad(path, e))?;
            Ok(archive.files.iter().filter(|f| !f.is_directory()).map(|f| normalize(f.name())).collect())
        }
        Kind::Rar | Kind::Unknown => Err(bad(path, "not a zip or 7z archive")),
    }
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<File>> {
    let f = File::open(path).map_err(|e| Error::io(path, e))?;
    zip::ZipArchive::new(f).map_err(|e| bad(path, e))
}

fn bad(path: &Path, e: impl std::fmt::Display) -> Error {
    Error::Parse { path: path.to_owned(), message: e.to_string() }
}

/// Windows-made archives use `\`.
fn normalize(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_owned()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Layout {
    /// `(path in the archive, path in the mod folder)`.
    Files(Vec<(String, String)>),
    /// MO2 installs it.
    Mo2(Mo2Reason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mo2Reason {
    /// A FOMOD installer (`fomod/ModuleConfig.xml`): options to choose.
    Fomod,
    /// RAR or not an archive.
    Format,
    /// No game data folder found.
    Layout,
}

fn ext_in(name: &str, exts: &[&str]) -> bool {
    let lower = name.to_lowercase();
    exts.iter().any(|e| lower.ends_with(e))
}

/// A path the archive must not write outside the mod folder with.
fn is_safe(rel: &str) -> bool {
    !rel.is_empty() && !rel.contains(':') && rel.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Where the files of an archive go, following MO2's Cyberpunk checker.
pub fn layout(entries: &[String]) -> Layout {
    if entries.iter().any(|e| e.to_lowercase().ends_with("fomod/moduleconfig.xml")) {
        return Layout::Mo2(Mo2Reason::Fomod);
    }
    let mut prefix = String::new();
    for _ in 0..=MAX_DEPTH {
        let mut dirs = BTreeSet::new();
        let mut files = Vec::new();
        for e in entries.iter().filter_map(|e| e.strip_prefix(prefix.as_str())) {
            match e.split_once('/') {
                Some((dir, _)) => {
                    dirs.insert(dir.to_owned());
                }
                None => files.push(e),
            }
        }
        let kept: Vec<&&str> = files.iter().filter(|f| !ext_in(f, DROPPED)).collect();
        let has_data_dir = dirs.iter().any(|d| DATA_DIRS.contains(&d.to_lowercase().as_str()));
        let loose_archives = kept.iter().any(|f| ext_in(f, MOVED_TO_ARCHIVE));
        if has_data_dir || loose_archives {
            return map_files(entries, &prefix);
        }
        match (dirs.len(), kept.is_empty()) {
            (1, true) => {
                prefix.push_str(dirs.first().unwrap());
                prefix.push('/');
            }
            _ => break,
        }
    }
    Layout::Mo2(Mo2Reason::Layout)
}

fn map_files(entries: &[String], prefix: &str) -> Layout {
    let mut out = Vec::new();
    for e in entries {
        let Some(rel) = e.strip_prefix(prefix) else { continue };
        let target = if rel.contains('/') {
            rel.to_owned()
        } else if ext_in(rel, DROPPED) {
            continue;
        } else if ext_in(rel, MOVED_TO_ARCHIVE) {
            format!("{ARCHIVE_MOD_DIR}{rel}")
        } else {
            rel.to_owned()
        };
        if !is_safe(e) || !is_safe(&target) {
            return Layout::Mo2(Mo2Reason::Layout);
        }
        out.push((e.clone(), target));
    }
    if out.is_empty() {
        return Layout::Mo2(Mo2Reason::Layout);
    }
    Layout::Files(out)
}

/// Writes the mapped files of the archive into `dest`.
pub fn extract(path: &Path, kind: Kind, files: &[(String, String)], dest: &Path) -> Result<()> {
    let map: HashMap<&str, &str> = files.iter().map(|(a, b)| (a.as_str(), b.as_str())).collect();
    let mut written = 0;
    let mut write = |name: &str, reader: &mut dyn Read| -> Result<()> {
        let Some(rel) = map.get(name) else { return Ok(()) };
        let out = crate::tree::from_slash(dest, rel);
        if let Some(dir) = out.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let mut f = File::create(&out).map_err(|e| Error::io(&out, e))?;
        std::io::copy(reader, &mut f).map_err(|e| Error::io(&out, e))?;
        f.flush().map_err(|e| Error::io(&out, e))?;
        written += 1;
        Ok(())
    };
    match kind {
        Kind::Zip => {
            let mut zip = open_zip(path)?;
            for i in 0..zip.len() {
                let mut f = zip.by_index(i).map_err(|e| bad(path, e))?;
                if f.is_dir() {
                    continue;
                }
                let name = normalize(f.name());
                write(&name, &mut f)?;
            }
        }
        Kind::SevenZip => {
            let mut reader =
                sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(|e| bad(path, e))?;
            let mut failed = None;
            reader
                .for_each_entries(|entry, r| {
                    if entry.is_directory() {
                        return Ok(true);
                    }
                    let name = normalize(entry.name());
                    // A solid block is one stream: a skipped file still has to be read past.
                    if !map.contains_key(name.as_str()) {
                        return std::io::copy(r, &mut std::io::sink()).map(|_| true).map_err(Into::into);
                    }
                    if let Err(e) = write(&name, r) {
                        failed = Some(e);
                        return Ok(false);
                    }
                    Ok(true)
                })
                .map_err(|e| bad(path, e))?;
            if let Some(e) = failed {
                return Err(e);
            }
        }
        Kind::Rar | Kind::Unknown => return Err(bad(path, "not a zip or 7z archive")),
    }
    if written != files.len() {
        return Err(bad(path, format!("{written} of {} files extracted", files.len())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn targets(l: &Layout) -> Vec<&str> {
        match l {
            Layout::Files(f) => f.iter().map(|(_, t)| t.as_str()).collect(),
            Layout::Mo2(r) => panic!("{r:?}"),
        }
    }

    #[test]
    fn data_folders_at_the_top_install_as_is() {
        let l = layout(&names(&["archive/pc/mod/a.archive", "r6/scripts/x.reds", "readme.txt", "notes.ini"]));
        assert_eq!(targets(&l), ["archive/pc/mod/a.archive", "r6/scripts/x.reds", "notes.ini"]);
    }

    #[test]
    fn unwraps_a_wrapper_folder() {
        let l = layout(&names(&["Cool Mod-1.2/red4ext/plugins/x/x.dll", "Cool Mod-1.2/README.md", "info.txt"]));
        assert_eq!(targets(&l), ["red4ext/plugins/x/x.dll"]);
        let deep = layout(&names(&["a/b/c/bin/x64/plugins/cyber_engine_tweaks/mods/m/init.lua"]));
        assert_eq!(targets(&deep), ["bin/x64/plugins/cyber_engine_tweaks/mods/m/init.lua"]);
    }

    #[test]
    fn moves_loose_archives() {
        let l = layout(&names(&["Mod/thing.archive", "Mod/thing.archive.xl", "Mod/preview.png"]));
        assert_eq!(targets(&l), ["archive/pc/mod/thing.archive", "archive/pc/mod/thing.archive.xl"]);
    }

    #[test]
    fn data_folder_names_ignore_case() {
        assert_eq!(targets(&layout(&names(&["Archive/pc/mod/a.archive"]))), ["Archive/pc/mod/a.archive"]);
    }

    #[test]
    fn hands_fomod_and_unknown_layouts_to_mo2() {
        let fomod = layout(&names(&["fomod/ModuleConfig.xml", "Option A/archive/pc/mod/a.archive"]));
        assert_eq!(fomod, Layout::Mo2(Mo2Reason::Fomod));
        assert_eq!(layout(&names(&["Wrapper/FOMOD/moduleconfig.xml"])), Layout::Mo2(Mo2Reason::Fomod));
        assert_eq!(layout(&names(&["Option A/archive/x.archive", "Option B/archive/x.archive"])), Layout::Mo2(Mo2Reason::Layout));
        assert_eq!(layout(&names(&["readme.txt"])), Layout::Mo2(Mo2Reason::Layout));
        assert_eq!(layout(&[]), Layout::Mo2(Mo2Reason::Layout));
    }

    #[test]
    fn refuses_paths_out_of_the_folder() {
        assert_eq!(layout(&names(&["archive/../../evil.dll", "archive/a.archive"])), Layout::Mo2(Mo2Reason::Layout));
        assert_eq!(layout(&names(&["archive/C:/evil.dll"])), Layout::Mo2(Mo2Reason::Layout));
    }

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let mut w = zip::ZipWriter::new(File::create(path).unwrap());
        for (name, data) in files {
            w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn extracts_zip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("mod.zip");
        zip_with(&path, &[("Mod\\archive\\pc\\mod\\a.archive", b"A"), ("Mod/readme.txt", b"R")]);
        assert_eq!(kind(&path).unwrap(), Kind::Zip);
        let list = entries(&path, Kind::Zip).unwrap();
        let Layout::Files(files) = layout(&list) else { panic!() };
        let dest = dir.path().join("out");
        extract(&path, Kind::Zip, &files, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("archive/pc/mod/a.archive")).unwrap(), b"A");
        assert!(!dest.join("readme.txt").exists());
    }

    #[test]
    fn extracts_7z() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("r6/scripts")).unwrap();
        std::fs::write(src.join("r6/scripts/x.reds"), b"X").unwrap();
        std::fs::write(src.join("loose.archive"), b"L").unwrap();
        // Skipped, and in the same solid block as the files after it.
        std::fs::write(src.join("a readme.txt"), b"skip me").unwrap();
        let path = dir.path().join("mod.zip"); // a 7z under a .zip name
        sevenz_rust2::compress_to_path(&src, &path).unwrap();
        assert_eq!(kind(&path).unwrap(), Kind::SevenZip);
        let list = entries(&path, Kind::SevenZip).unwrap();
        let Layout::Files(files) = layout(&list) else { panic!("{list:?}") };
        let dest = dir.path().join("out");
        extract(&path, Kind::SevenZip, &files, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("r6/scripts/x.reds")).unwrap(), b"X");
        assert_eq!(std::fs::read(dest.join("archive/pc/mod/loose.archive")).unwrap(), b"L");
    }

    #[test]
    fn detects_rar_and_garbage() {
        let dir = tempfile::tempdir().unwrap();
        let rar = dir.path().join("a.rar");
        std::fs::write(&rar, b"Rar!\x1a\x07\x00rest").unwrap();
        assert_eq!(kind(&rar).unwrap(), Kind::Rar);
        let txt = dir.path().join("a.txt");
        std::fs::write(&txt, b"hi").unwrap();
        assert_eq!(kind(&txt).unwrap(), Kind::Unknown);
    }
}

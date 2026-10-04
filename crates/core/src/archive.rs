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
//! FOMOD installers ask the player questions: [`Layout::Fomod`], see
//! [`crate::fomod`]. An unrecognized layout needs a person to look at it:
//! like MO2's manual installer, the player picks the folder that is the mod
//! ([`roots`], [`map_under`]). RAR is read with RARLAB's UnRAR library, as
//! 7-Zip (MO2's `archive.dll`) does.

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
/// Lowercase; the archive's own case is kept for reading.
pub const FOMOD_CONFIG: &str = "fomod/moduleconfig.xml";
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
        Kind::Rar => {
            let list = unrar::Archive::new(path).open_for_listing().map_err(|e| bad(path, e))?;
            let mut out = Vec::new();
            for entry in list {
                let entry = entry.map_err(|e| bad(path, e))?;
                if entry.is_file() {
                    out.push(normalize(&entry.filename.to_string_lossy()));
                }
            }
            Ok(out)
        }
        Kind::Unknown => Err(bad(path, "not a zip, 7z or rar archive")),
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
    /// A FOMOD installer; `root` is the folder that holds `fomod/` (`""` or
    /// `Wrapper/`), its paths are relative to it.
    Fomod { root: String },
    /// MO2 installs it.
    Mo2(Mo2Reason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Mo2Reason {
    /// A FOMOD installer the launcher can't read.
    Fomod,
    /// Not an archive the launcher reads.
    Format,
    /// No game data folder found.
    Layout,
}

fn ext_in(name: &str, exts: &[&str]) -> bool {
    let lower = name.to_lowercase();
    exts.iter().any(|e| lower.ends_with(e))
}

/// A path the archive must not write outside the mod folder with.
pub fn is_safe(rel: &str) -> bool {
    !rel.is_empty() && !rel.contains(':') && rel.split('/').all(|c| !c.is_empty() && c != "." && c != "..")
}

/// Where the files of an archive go, following MO2's Cyberpunk checker.
pub fn layout(entries: &[String]) -> Layout {
    // The shallowest one: an installer may ship examples of others.
    if let Some(root) = entries.iter().filter_map(|e| fomod_root(e)).min_by_key(|r| r.matches('/').count()) {
        return Layout::Fomod { root: root.to_owned() };
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

/// `Wrapper/` of `Wrapper/fomod/ModuleConfig.xml`, any case.
pub fn fomod_root(entry: &str) -> Option<&str> {
    let at = entry.len().checked_sub(FOMOD_CONFIG.len())?;
    let matches = entry.is_char_boundary(at) && entry[at..].eq_ignore_ascii_case(FOMOD_CONFIG);
    (matches && (at == 0 || entry[..at].ends_with('/'))).then(|| &entry[..at])
}

/// Where a file goes in the mod folder, `rel` being its path from the mod
/// root: MO2's checker moves loose archives and drops loose images and text.
pub fn place(rel: &str) -> Option<String> {
    if rel.contains('/') {
        Some(rel.to_owned())
    } else if ext_in(rel, DROPPED) {
        None
    } else if ext_in(rel, MOVED_TO_ARCHIVE) {
        Some(format!("{ARCHIVE_MOD_DIR}{rel}"))
    } else {
        Some(rel.to_owned())
    }
}

fn map_files(entries: &[String], prefix: &str) -> Layout {
    let mut out = Vec::new();
    for e in entries {
        let Some(rel) = e.strip_prefix(prefix) else { continue };
        let Some(target) = place(rel) else { continue };
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

/// Writes the mapped files of the archive into `dest`. A file may be mapped
/// to several places (FOMOD options that share a file).
pub fn extract(path: &Path, kind: Kind, files: &[(String, String)], dest: &Path) -> Result<()> {
    let mut map: HashMap<&str, Vec<&str>> = HashMap::new();
    for (a, b) in files {
        map.entry(a.as_str()).or_default().push(b.as_str());
    }
    let mut written = 0;
    let mut write = |name: &str, reader: &mut dyn Read| -> Result<()> {
        let Some(targets) = map.get(name) else { return Ok(()) };
        let outs: Vec<_> = targets.iter().map(|rel| crate::tree::from_slash(dest, rel)).collect();
        for out in &outs {
            if let Some(dir) = out.parent() {
                std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
            }
        }
        let mut f = File::create(&outs[0]).map_err(|e| Error::io(&outs[0], e))?;
        std::io::copy(reader, &mut f).map_err(|e| Error::io(&outs[0], e))?;
        f.flush().map_err(|e| Error::io(&outs[0], e))?;
        for out in &outs[1..] {
            std::fs::copy(&outs[0], out).map_err(|e| Error::io(out, e))?;
        }
        written += outs.len();
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
        Kind::Rar => {
            let mut rar = unrar::Archive::new(path).open_for_processing().map_err(|e| bad(path, e))?;
            while let Some(header) = rar.read_header().map_err(|e| bad(path, e))? {
                let name = normalize(&header.entry().filename.to_string_lossy());
                let targets = map.get(name.as_str()).filter(|_| header.entry().is_file());
                let Some(targets) = targets else {
                    rar = header.skip().map_err(|e| bad(path, e))?;
                    continue;
                };
                // Straight to disk: a Cyberpunk .archive can be gigabytes.
                let outs: Vec<_> = targets.iter().map(|rel| crate::tree::from_slash(dest, rel)).collect();
                for out in &outs {
                    if let Some(dir) = out.parent() {
                        std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
                    }
                }
                rar = header.extract_to(&outs[0]).map_err(|e| bad(path, e))?;
                for out in &outs[1..] {
                    std::fs::copy(&outs[0], out).map_err(|e| Error::io(out, e))?;
                }
                written += outs.len();
            }
        }
        Kind::Unknown => return Err(bad(path, "not a zip, 7z or rar archive")),
    }
    if written != files.len() {
        return Err(bad(path, format!("{written} of {} files extracted", files.len())));
    }
    Ok(())
}

/// Reads the named files (archive paths) into memory, skipping any larger
/// than `max_size`. For the FOMOD config and its images.
pub fn read(path: &Path, kind: Kind, names: &[String], max_size: u64) -> Result<HashMap<String, Vec<u8>>> {
    let wanted: std::collections::HashSet<&str> = names.iter().map(String::as_str).collect();
    let mut out = HashMap::new();
    match kind {
        Kind::Zip => {
            let mut zip = open_zip(path)?;
            for i in 0..zip.len() {
                let mut f = zip.by_index(i).map_err(|e| bad(path, e))?;
                let name = normalize(f.name());
                if f.is_dir() || !wanted.contains(name.as_str()) || f.size() > max_size {
                    continue;
                }
                let mut buf = Vec::with_capacity(f.size() as usize);
                f.read_to_end(&mut buf).map_err(|e| bad(path, e))?;
                out.insert(name, buf);
            }
        }
        Kind::SevenZip => {
            let mut reader =
                sevenz_rust2::ArchiveReader::open(path, sevenz_rust2::Password::empty()).map_err(|e| bad(path, e))?;
            reader
                .for_each_entries(|entry, r| {
                    let name = normalize(entry.name());
                    if entry.is_directory() || !wanted.contains(name.as_str()) || entry.size() > max_size {
                        // A solid block is one stream: a skipped file still has to be read past.
                        return std::io::copy(r, &mut std::io::sink()).map(|_| true).map_err(Into::into);
                    }
                    let mut buf = Vec::new();
                    r.read_to_end(&mut buf)?;
                    out.insert(name, buf);
                    // Stop once everything is read: the rest of a big archive is just decompression time.
                    Ok(out.len() < wanted.len())
                })
                .map_err(|e| bad(path, e))?;
        }
        Kind::Rar => {
            let mut rar = unrar::Archive::new(path).open_for_processing().map_err(|e| bad(path, e))?;
            while let Some(header) = rar.read_header().map_err(|e| bad(path, e))? {
                let entry = header.entry();
                let name = normalize(&entry.filename.to_string_lossy());
                if entry.is_file() && wanted.contains(name.as_str()) && entry.unpacked_size <= max_size {
                    let (buf, next) = header.read().map_err(|e| bad(path, e))?;
                    out.insert(name, buf);
                    rar = next;
                } else {
                    rar = header.skip().map_err(|e| bad(path, e))?;
                }
            }
        }
        Kind::Unknown => return Err(bad(path, "not a zip, 7z or rar archive")),
    }
    Ok(out)
}

/// A folder of an archive the player may pick as the mod (MO2's manual
/// installer: "set data directory").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Root {
    /// `""` for the archive itself, else `Folder/Sub/`.
    pub path: String,
    /// Files under it.
    pub files: usize,
    /// Has a game data folder or loose `.archive` files at its top: what
    /// MO2's Cyberpunk checker accepts.
    pub valid: bool,
}

/// Every folder of the archive, in path order, with the archive itself first.
pub fn roots(entries: &[String]) -> Vec<Root> {
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    dirs.insert(String::new());
    for e in entries {
        let mut at = 0;
        while let Some(i) = e[at..].find('/') {
            at += i + 1;
            dirs.insert(e[..at].to_owned());
        }
    }
    dirs.into_iter()
        .map(|path| {
            let under: Vec<&str> = entries.iter().filter_map(|e| e.strip_prefix(path.as_str())).collect();
            let valid = under.iter().any(|rel| match rel.split_once('/') {
                Some((dir, _)) => DATA_DIRS.contains(&dir.to_lowercase().as_str()),
                None => ext_in(rel, MOVED_TO_ARCHIVE),
            });
            Root { files: under.len(), valid, path }
        })
        .collect()
}

/// Files under `root` as the mod, with MO2's fixes at the top ([`place`]).
/// Empty when nothing is left or a path would leave the mod folder.
pub fn map_under(entries: &[String], root: &str) -> Vec<(String, String)> {
    match map_files(entries, root) {
        Layout::Files(f) => f,
        _ => Vec::new(),
    }
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
            other => panic!("{other:?}"),
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
        assert_eq!(fomod, Layout::Fomod { root: String::new() });
        assert_eq!(layout(&names(&["Wrapper/FOMOD/moduleconfig.xml"])), Layout::Fomod { root: "Wrapper/".into() });
        let nested = layout(&names(&["W/docs/example/fomod/ModuleConfig.xml", "W/fomod/ModuleConfig.xml"]));
        assert_eq!(nested, Layout::Fomod { root: "W/".into() });
        assert!(matches!(layout(&names(&["notfomod/ModuleConfig.xml", "archive/a.archive"])), Layout::Files(_)));
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

        let read = read(&path, Kind::Zip, &names(&["Mod/readme.txt", "Mod/nope"]), 1024).unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read["Mod/readme.txt"], b"R");
        let twice = vec![("Mod/readme.txt".to_owned(), "a.txt".to_owned()), ("Mod/readme.txt".to_owned(), "b/b.txt".to_owned())];
        let dest = dir.path().join("twice");
        extract(&path, Kind::Zip, &twice, &dest).unwrap();
        assert_eq!(std::fs::read(dest.join("b/b.txt")).unwrap(), b"R");
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
        let read = read(&path, Kind::SevenZip, &names(&["r6/scripts/x.reds"]), 1024).unwrap();
        assert_eq!(read["r6/scripts/x.reds"], b"X");
    }

    #[test]
    fn reads_and_extracts_rar() {
        // `rar a -s`: a solid archive, so skipped files must be read past.
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/mod.rar");
        assert_eq!(kind(&path).unwrap(), Kind::Rar);
        let list = entries(&path, Kind::Rar).unwrap();
        assert_eq!(list.len(), 3, "{list:?}");
        let Layout::Files(files) = layout(&list) else { panic!("{list:?}") };
        let dir = tempfile::tempdir().unwrap();
        extract(&path, Kind::Rar, &files, dir.path()).unwrap();
        assert_eq!(std::fs::read(dir.path().join("archive/pc/mod/a.archive")).unwrap(), b"A");
        assert_eq!(std::fs::read(dir.path().join("archive/pc/mod/b.archive.xl")).unwrap(), b"B");
        assert!(!dir.path().join("readme.txt").exists());
        let read = read(&path, Kind::Rar, &names(&["Cool Mod/readme.txt"]), 1024).unwrap();
        assert_eq!(read["Cool Mod/readme.txt"], b"readme");
    }

    #[test]
    fn manual_roots() {
        let list = names(&["Pick/Option A/archive/pc/mod/a.archive", "Pick/Option B/r6/scripts/b.reds", "Pick/readme.txt"]);
        assert_eq!(layout(&list), Layout::Mo2(Mo2Reason::Layout));
        let r = roots(&list);
        let paths: Vec<(&str, usize, bool)> = r.iter().map(|r| (r.path.as_str(), r.files, r.valid)).collect();
        assert_eq!(
            paths,
            [
                ("", 3, false),
                ("Pick/", 3, false),
                ("Pick/Option A/", 1, true),
                ("Pick/Option A/archive/", 1, false),
                ("Pick/Option A/archive/pc/", 1, false),
                ("Pick/Option A/archive/pc/mod/", 1, true),
                ("Pick/Option B/", 1, true),
                ("Pick/Option B/r6/", 1, false),
                ("Pick/Option B/r6/scripts/", 1, false),
            ]
        );
        assert_eq!(map_under(&list, "Pick/Option B/"), [("Pick/Option B/r6/scripts/b.reds".to_owned(), "r6/scripts/b.reds".to_owned())]);
        // Loose archives picked as the mod go where MO2 puts them.
        assert_eq!(map_under(&list, "Pick/Option A/archive/pc/mod/")[0].1, "archive/pc/mod/a.archive");
        assert!(map_under(&list, "Nope/").is_empty());
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

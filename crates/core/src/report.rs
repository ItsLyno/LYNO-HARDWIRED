//! Diagnostic report: one zip a player sends to the build author instead of
//! describing a crash in words.
//!
//! Under MO2 the frameworks' logs don't land in the game folder: USVFS sends
//! every new file to `overwrite/` (see `docs/cyberpunk-mo2.md`), so RED4ext,
//! CET and redscript logs are collected from there. The game folder is still
//! searched for players who once started the game without MO2.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;

use crate::mo2::Instance;
use crate::{Error, Result};

/// Logs grow without bound (USVFS, CET at debug level); the tail is what matters.
pub const MAX_FILE: u64 = 2 * 1024 * 1024;

/// MO2 writes a new `usvfs-*.log` per run; only the latest few are useful.
const MO2_LOGS: usize = 5;

/// Game-relative folders where frameworks write logs when run without MO2.
const GAME_LOG_DIRS: &[&str] = &["r6/logs", "red4ext/logs", "red4ext/plugins", "bin/x64/plugins/cyber_engine_tweaks"];

pub struct ReportInput<'a> {
    /// Free-form text written as `summary.txt` (versions, paths, status).
    pub summary: String,
    pub inst: &'a Instance,
    pub profile: &'a str,
    pub game_dir: Option<&'a Path>,
    /// Additional files, e.g. the launcher's own logs, as (name in zip, path).
    pub extra: Vec<(String, PathBuf)>,
}

/// Writes the report to `dest` and returns the names of the files inside.
/// Missing files are skipped: a report from a broken install is the point.
pub fn write_report(dest: &Path, input: &ReportInput) -> Result<Vec<String>> {
    let inst = input.inst;
    let root = inst.root();
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for rel in [".lyno/state.json", ".lyno/manifest.json", "ModOrganizer.ini"] {
        files.push((format!("instance/{rel}"), root.join(rel)));
    }
    for name in ["modlist.txt", "settings.ini"] {
        files.push((format!("instance/profiles/{}/{name}", input.profile), inst.profile_dir(input.profile).join(name)));
    }
    let mut mo2_logs = logs_in(&root.join("logs"), 1);
    mo2_logs.sort_by_key(|p| std::cmp::Reverse(std::fs::metadata(p).and_then(|m| m.modified()).ok()));
    files.extend(mo2_logs.into_iter().take(MO2_LOGS).map(|p| (zip_name("instance", root, &p), p)));
    files.extend(logs_in(&inst.overwrite_dir(), usize::MAX).into_iter().map(|p| (zip_name("instance", root, &p), p)));
    if let Some(game) = input.game_dir {
        for dir in GAME_LOG_DIRS {
            files.extend(logs_in(&game.join(dir), 3).into_iter().map(|p| (zip_name("game", game, &p), p)));
        }
    }
    files.extend(input.extra.iter().cloned());

    let out = File::create(dest).map_err(|e| Error::io(dest, e))?;
    let mut zip = zip::ZipWriter::new(out);
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let zip_err = |e: zip::result::ZipError| Error::io(dest, std::io::Error::other(e));
    let mut names = vec!["summary.txt".to_owned()];
    zip.start_file("summary.txt", opts).map_err(zip_err)?;
    zip.write_all(input.summary.as_bytes()).map_err(|e| Error::io(dest, e))?;
    for (name, path) in files {
        let Ok(data) = read_tail(&path, MAX_FILE) else { continue };
        zip.start_file(name.as_str(), opts).map_err(zip_err)?;
        zip.write_all(&data).map_err(|e| Error::io(dest, e))?;
        names.push(name);
    }
    zip.finish().map_err(zip_err)?;
    Ok(names)
}

/// `*.log` files under `dir`, at most `depth` levels deep.
fn logs_in(dir: &Path, depth: usize) -> Vec<PathBuf> {
    walkdir::WalkDir::new(dir)
        .max_depth(depth)
        .into_iter()
        .flatten()
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("log")))
        .map(|e| e.into_path())
        .collect()
}

fn zip_name(prefix: &str, root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    let rel: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect();
    format!("{prefix}/{}", rel.join("/"))
}

/// The last `max` bytes of the file, marked when cut.
fn read_tail(path: &Path, max: u64) -> std::io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let mut data = Vec::new();
    if len > max {
        data.extend_from_slice(format!("[... first {} bytes cut ...]\n", len - max).as_bytes());
        file.seek(SeekFrom::Start(len - max))?;
    }
    file.read_to_end(&mut data)?;
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, data: &[u8]) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[test]
    fn collects_logs_and_state() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("inst");
        let game = tmp.path().join("game");
        write(&root, ".lyno/state.json", b"{}");
        write(&root, "profiles/LYNO/modlist.txt", b"+CET\n");
        write(&root, "logs/mo_interface.log", b"mo2");
        write(&root, "overwrite/red4ext/logs/red4ext.log", b"r4e");
        write(&root, "overwrite/bin/x64/plugins/cyber_engine_tweaks/cyber_engine_tweaks.log", &[b'x'; 100]);
        write(&root, "overwrite/r6/cache/final.redscripts", b"not a log");
        write(&game, "r6/logs/redscript_rCURRENT.log", b"redscript");
        write(&game, "archive/pc/content/huge.log", b"not a log dir");
        let launcher_log = tmp.path().join("launcher.log");
        std::fs::write(&launcher_log, b"launcher").unwrap();

        let dest = tmp.path().join("report.zip");
        let inst = Instance::new(&root);
        let input = ReportInput {
            summary: "LYNO".into(),
            inst: &inst,
            profile: "LYNO",
            game_dir: Some(&game),
            extra: vec![("launcher/launcher.log".into(), launcher_log)],
        };
        let mut names = write_report(&dest, &input).unwrap();
        names.sort();
        assert_eq!(
            names,
            [
                "game/r6/logs/redscript_rCURRENT.log",
                "instance/.lyno/state.json",
                "instance/logs/mo_interface.log",
                "instance/overwrite/bin/x64/plugins/cyber_engine_tweaks/cyber_engine_tweaks.log",
                "instance/overwrite/red4ext/logs/red4ext.log",
                "instance/profiles/LYNO/modlist.txt",
                "launcher/launcher.log",
                "summary.txt",
            ]
        );
        let mut zip = zip::ZipArchive::new(File::open(&dest).unwrap()).unwrap();
        let mut text = String::new();
        zip.by_name("instance/overwrite/red4ext/logs/red4ext.log").unwrap().read_to_string(&mut text).unwrap();
        assert_eq!(text, "r4e");
    }

    #[test]
    fn keeps_the_tail_of_big_files() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.log");
        std::fs::write(&p, b"0123456789").unwrap();
        assert_eq!(read_tail(&p, 4).unwrap(), b"[... first 6 bytes cut ...]\n6789");
        assert_eq!(read_tail(&p, 10).unwrap(), b"0123456789");
    }
}

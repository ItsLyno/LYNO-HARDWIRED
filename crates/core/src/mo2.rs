//! A portable MO2 instance managed by the launcher.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::meta::ModMeta;
use crate::{Error, Result};

pub const EXE_NAME: &str = "ModOrganizer.exe";
/// Executable entries MO2's Cyberpunk plugin registers. The REDmod one adds
/// `-modded` and deploys REDmod mods before the game starts.
pub const GAME_EXECUTABLE: &str = "Cyberpunk 2077";
pub const GAME_EXECUTABLE_REDMOD: &str = "Cyberpunk 2077 (REDmod)";

/// The MO2 executable entry to start the build with.
pub fn game_executable(redmod: bool) -> &'static str {
    if redmod {
        GAME_EXECUTABLE_REDMOD
    } else {
        GAME_EXECUTABLE
    }
}

#[derive(Debug, Clone)]
pub struct Instance {
    root: PathBuf,
}

impl Instance {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn exe(&self) -> PathBuf {
        self.root.join(EXE_NAME)
    }

    /// MO2 2.4+ treats a directory with `portable.txt` as a portable instance.
    pub fn is_portable(&self) -> bool {
        self.root.join("portable.txt").is_file()
    }

    pub fn is_installed(&self) -> bool {
        self.exe().is_file()
    }

    pub fn mods_dir(&self) -> PathBuf {
        self.root.join("mods")
    }

    pub fn downloads_dir(&self) -> PathBuf {
        self.root.join("downloads")
    }

    pub fn overwrite_dir(&self) -> PathBuf {
        self.root.join("overwrite")
    }

    pub fn profile_dir(&self, profile: &str) -> PathBuf {
        self.root.join("profiles").join(profile)
    }

    pub fn modlist_path(&self, profile: &str) -> PathBuf {
        self.profile_dir(profile).join("modlist.txt")
    }

    /// Reads `meta.ini` of every folder under `mods/`, keyed by folder name.
    /// Folders without a `meta.ini` (separators, hand-made mods) get defaults.
    pub fn scan_mods(&self) -> Result<BTreeMap<String, ModMeta>> {
        let dir = self.mods_dir();
        let mut out = BTreeMap::new();
        if !dir.exists() {
            return Ok(out);
        }
        for entry in std::fs::read_dir(&dir).map_err(|e| Error::io(&dir, e))? {
            let entry = entry.map_err(|e| Error::io(&dir, e))?;
            if !entry.file_type().map_err(|e| Error::io(entry.path(), e))?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let meta_path = entry.path().join("meta.ini");
            let meta = if meta_path.is_file() { ModMeta::load(&meta_path)? } else { ModMeta::default() };
            out.insert(name, meta);
        }
        Ok(out)
    }
}

impl Instance {
    pub fn ini_path(&self) -> PathBuf {
        self.root.join("ModOrganizer.ini")
    }

    /// Points the instance at the user's game folder.
    ///
    /// `ModOrganizer.ini` comes from the author's machine: besides
    /// `gamePath` it holds absolute paths in executables, so every
    /// occurrence of the old game path is rewritten.
    pub fn set_game_path(&self, game_dir: &Path) -> Result<()> {
        let path = self.ini_path();
        let text = std::fs::read_to_string(&path).map_err(|e| Error::io(&path, e))?;
        let new = game_dir.to_string_lossy().replace('\\', "/").trim_end_matches('/').to_owned();
        let updated = rewrite_game_path(&text, &new);
        if updated != text {
            std::fs::write(&path, updated).map_err(|e| Error::io(&path, e))?;
        }
        Ok(())
    }
}

/// Sections of `ModOrganizer.ini` that are the state of the author's MO2
/// window, not the build: window and column layout (`Geometry`), expanded
/// separators and selected tabs (`Widgets`), last folders of file dialogs
/// (`recentDirectories`), Nexus CDN servers with the day last seen
/// (`Servers`). MO2 rewrites them on every run, so shipped as is they changed
/// the base package, and its upload, with every build.
const UI_STATE_SECTIONS: &[&str] = &["Geometry", "Widgets", "recentDirectories", "Servers"];

/// `ModOrganizer.ini` without [`UI_STATE_SECTIONS`]; MO2 starts with
/// defaults for them. Line based, so every other line stays byte for byte
/// (Qt escapes and `@ByteArray` values survive no ini rewriter unchanged).
pub fn strip_ui_state(ini: &str) -> String {
    let mut skip = false;
    let mut out = String::with_capacity(ini.len());
    // Blank lines are laid out anew, one before each section as Qt writes
    // them: where a dropped section sat must leave no trace in the file.
    for line in ini.split_inclusive('\n') {
        let t = line.trim();
        if let Some(section) = t.strip_prefix('[').and_then(|r| r.strip_suffix(']')) {
            skip = UI_STATE_SECTIONS.iter().any(|s| s.eq_ignore_ascii_case(section));
            if !skip && !out.is_empty() {
                out.push_str(if line.ends_with("\r\n") { "\r\n" } else { "\n" });
            }
        }
        if !skip && !t.is_empty() {
            out.push_str(line);
        }
    }
    out
}

fn rewrite_game_path(ini: &str, new: &str) -> String {
    let old = ini.lines().find_map(|l| {
        l.trim()
            .strip_prefix("gamePath=")
            .map(|v| v.trim_start_matches("@ByteArray(").trim_end_matches(')').trim_end_matches('/').to_owned())
    });
    let new_line = format!("gamePath=@ByteArray({new})");
    let mut out = match old.as_deref() {
        Some(old) if !old.is_empty() => {
            let old_fwd = old.replace('\\', "/");
            // Qt ini escapes backslashes, so paths may appear as C:\\Games\\...
            let old_bs = old_fwd.replace('/', "\\");
            let old_bs2 = old_fwd.replace('/', "\\\\");
            let new_bs = new.replace('/', "\\");
            let new_bs2 = new.replace('/', "\\\\");
            ini.replace(&old_bs2, &new_bs2).replace(&old_bs, &new_bs).replace(&old_fwd, new)
        }
        _ => ini.to_owned(),
    };
    // Normalize the gamePath line itself.
    out = out
        .lines()
        .map(|l| if l.trim_start().starts_with("gamePath=") { new_line.clone() } else { l.to_owned() })
        .collect::<Vec<_>>()
        .join("\r\n");
    if !out.contains("gamePath=") {
        out = out.replacen("[General]", &format!("[General]\r\n{new_line}"), 1);
    }
    out + "\r\n"
}

/// Arguments for `ModOrganizer.exe` to start a configured executable
/// inside MO2's virtual file system.
///
/// `ModOrganizer.exe -p <profile> run -e <executable>`; when MO2 is already
/// open the command is forwarded to the running instance.
pub fn run_args(profile: &str, executable: &str) -> Vec<String> {
    vec!["-p".into(), profile.into(), "run".into(), "-e".into(), executable.into()]
}

/// Arguments to open MO2's own window on the given profile.
pub fn open_args(profile: &str) -> Vec<String> {
    vec!["-p".into(), profile.into()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_game_paths() {
        let ini = "[General]\r\ngamePath=@ByteArray(D:/SteamLibrary/steamapps/common/Cyberpunk 2077)\r\n\
            [customExecutables]\r\n1\\binary=D:/SteamLibrary/steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe\r\n\
            2\\workingDirectory=D:\\\\SteamLibrary\\\\steamapps\\\\common\\\\Cyberpunk 2077\r\n";
        let out = rewrite_game_path(ini, "C:/Games/Cyberpunk 2077");
        assert!(out.contains("gamePath=@ByteArray(C:/Games/Cyberpunk 2077)\r\n"), "{out}");
        assert!(out.contains("1\\binary=C:/Games/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe"), "{out}");
        assert!(out.contains("2\\workingDirectory=C:\\\\Games\\\\Cyberpunk 2077"), "{out}");
        assert!(!out.contains("SteamLibrary"));
    }

    #[test]
    fn adds_missing_game_path() {
        let out = rewrite_game_path("[General]\r\nversion=2.5.2\r\n", "C:/G");
        assert!(out.starts_with("[General]\r\ngamePath=@ByteArray(C:/G)\r\nversion=2.5.2"), "{out}");
    }

    #[test]
    fn builds_run_args() {
        assert_eq!(run_args("LYNO", GAME_EXECUTABLE), ["-p", "LYNO", "run", "-e", "Cyberpunk 2077"]);
    }

    #[test]
    fn scans_mod_folders() {
        let dir = tempfile::tempdir().unwrap();
        let inst = Instance::new(dir.path());
        std::fs::create_dir_all(inst.mods_dir().join("CET")).unwrap();
        std::fs::create_dir_all(inst.mods_dir().join("Core_separator")).unwrap();
        std::fs::write(
            inst.mods_dir().join("CET/meta.ini"),
            "[General]\nmodid=107\n[LYNO]\nid=cet\n",
        )
        .unwrap();
        std::fs::write(inst.mods_dir().join("stray.txt"), "").unwrap();

        let mods = inst.scan_mods().unwrap();
        assert_eq!(mods.len(), 2);
        assert_eq!(mods["CET"].lyno_id.as_deref(), Some("cet"));
        assert_eq!(mods["Core_separator"], ModMeta::default());
    }

    #[test]
    fn strips_ui_state_of_mo2_ini() {
        let ini = "[General]\r\ngameName=Cyberpunk 2077\r\n\r\n[Geometry]\r\nMainWindow_geometry=@ByteArray(\\x1)\r\n\r\n\
                   [customExecutables]\r\nsize=1\r\n1\\title=Cyberpunk 2077\r\n\r\n[Widgets]\r\nMainWindow_modList_index=a, b\r\n\r\n\
                   [Servers]\r\n1\\lastSeen=2026-10-04\r\n\r\n[Plugins]\r\nx\\y=true\r\n";
        assert_eq!(
            strip_ui_state(ini),
            "[General]\r\ngameName=Cyberpunk 2077\r\n\r\n[customExecutables]\r\nsize=1\r\n1\\title=Cyberpunk 2077\r\n\r\n\
             [Plugins]\r\nx\\y=true\r\n"
        );
        // A dropped section at the end leaves no blank line behind either.
        assert_eq!(strip_ui_state("[General]\r\na=1\r\n\r\n[Widgets]\r\nb=2\r\n"), "[General]\r\na=1\r\n");
        assert_eq!(strip_ui_state("[General]\nversion=2.5.2"), "[General]\nversion=2.5.2");
    }
}

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

    /// The profile MO2 opened last: the one of an instance without the build.
    pub fn selected_profile(&self) -> Option<String> {
        let text = std::fs::read_to_string(self.ini_path()).ok()?;
        text.lines().find_map(|l| {
            let v = l.trim().strip_prefix("selected_profile=")?;
            let v = v.strip_prefix("@ByteArray(").and_then(|v| v.strip_suffix(')')).unwrap_or(v);
            (!v.is_empty()).then(|| v.to_owned())
        })
    }

    /// Points the instance at the user's game folder.
    ///
    /// `ModOrganizer.ini` comes from the author's machine: besides
    /// `gamePath` it holds absolute paths in executables, so every
    /// occurrence of the old game path is rewritten. Called before every
    /// launch too: MO2 starts the game by `binary=`, not by `gamePath`.
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

/// Game files the executables of a Cyberpunk instance start; the folder in front of them is a game folder.
const GAME_BINARIES: &[&str] = &["/bin/x64/Cyberpunk2077.exe", "/tools/redmod/bin/redMod.exe", "/REDprelauncher.exe"];

fn rewrite_game_path(ini: &str, new: &str) -> String {
    // MO2 saves paths with native separators, Qt-escaped: `D:\\Games\\Cyberpunk 2077`.
    let norm = |v: &str| {
        let v = v.trim().trim_start_matches("@ByteArray(").trim_end_matches(')');
        v.replace("\\\\", "/").replace('\\', "/").trim_end_matches('/').to_owned()
    };
    // Old game folders: gamePath, and the folders of the game's executables. Those can differ from
    // gamePath: once gamePath is rewritten (by an older launcher, or the player in MO2), it no longer
    // leads to the author's folder still in `binary=`.
    let mut olds: Vec<String> = Vec::new();
    for line in ini.lines() {
        let line = line.trim();
        let old = if let Some(v) = line.strip_prefix("gamePath=") {
            Some(norm(v))
        } else if let Some((_, v)) = line.split_once("\\binary=") {
            let v = norm(v);
            GAME_BINARIES.iter().find_map(|b| v.strip_suffix(b).map(str::to_owned))
        } else {
            None
        };
        if let Some(old) = old.filter(|o| !o.is_empty() && o != new && !olds.contains(o)) {
            olds.push(old);
        }
    }
    let new_bs = new.replace('/', "\\");
    let new_bs2 = new.replace('/', "\\\\");
    let new_line = format!("gamePath=@ByteArray({new})");
    let mut out = ini.to_owned();
    for old_fwd in &olds {
        // Qt ini escapes backslashes, so paths may appear as C:\\Games\\...
        let old_bs = old_fwd.replace('/', "\\");
        let old_bs2 = old_fwd.replace('/', "\\\\");
        out = out.replace(&old_bs2, &new_bs2).replace(&old_bs, &new_bs).replace(old_fwd.as_str(), new);
    }
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

/// `gameName` of the Cyberpunk plugin.
const GAME_NAME: &str = "Cyberpunk 2077";

/// The Cyberpunk plugin MO2 2.5.2 ships is 2.3.1: it predates forced load
/// libraries, so CET and RED4ext would not load. 3.0.0, the build's copy, runs on
/// 2.5.2's `basic_games`; 3.0.1 needs a newer one (`BasicLocalSavegames(self)`).
/// MIT, see `assets/LICENSE-basic_games`.
const CYBERPUNK_PLUGIN: &[u8] = include_bytes!("../assets/game_cyberpunk2077.py");
const CYBERPUNK_PLUGIN_PATH: &str = "plugins/basic_games/games/game_cyberpunk2077.py";

/// Profile of an instance the launcher creates without the build.
pub const DEFAULT_PROFILE: &str = "Default";

const MO2_LATEST: &str = "https://api.github.com/repos/ModOrganizer2/modorganizer/releases/latest";

/// The portable archive of an official MO2 release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub size: u64,
}

/// Latest MO2 release on GitHub. MO2 2.5+ ships the Cyberpunk game plugin, so
/// an instance without the build needs nothing else.
pub fn latest_release(dl: &crate::download::Downloader) -> Result<Release> {
    let json: serde_json::Value = serde_json::from_str(&dl.get_text(MO2_LATEST)?)?;
    pick_release(&json).ok_or_else(|| Error::Download(format!("{MO2_LATEST}: no portable archive in the release")))
}

/// `Mod.Organizer-2.5.2.7z`; next to it are the installer and `-pdbs`, `-src` archives.
fn pick_release(json: &serde_json::Value) -> Option<Release> {
    let asset = json["assets"].as_array()?.iter().find(|a| {
        a["name"]
            .as_str()
            .and_then(|n| n.strip_prefix("Mod.Organizer-")?.strip_suffix(".7z"))
            .is_some_and(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_digit() || c == '.'))
    })?;
    Some(Release {
        version: json["tag_name"].as_str()?.trim_start_matches('v').to_owned(),
        url: asset["browser_download_url"].as_str()?.to_owned(),
        size: asset["size"].as_u64()?,
    })
}

/// Unpacks MO2's portable archive into `inst` (which must not exist yet) and
/// sets it up for Cyberpunk, so MO2 starts without its instance wizard. Goes
/// through `<root>.part`: a failed attempt leaves no half instance behind.
pub fn create_portable(inst: &Instance, archive_path: &Path, game_dir: Option<&Path>) -> Result<()> {
    let root = inst.root();
    if root.exists() {
        return Err(Error::Io { path: root.to_owned(), source: std::io::ErrorKind::AlreadyExists.into() });
    }
    let part = PathBuf::from(format!("{}.part", root.display()));
    if part.exists() {
        std::fs::remove_dir_all(&part).map_err(|e| Error::io(&part, e))?;
    }
    let kind = crate::archive::kind(archive_path)?;
    let entries = crate::archive::entries(archive_path, kind)?;
    let exe = entries
        .iter()
        .find(|e| e.rsplit('/').next().is_some_and(|n| n.eq_ignore_ascii_case(EXE_NAME)))
        .ok_or_else(|| Error::Parse { path: archive_path.to_owned(), message: format!("no {EXE_NAME}") })?;
    let prefix = &exe[..exe.len() - EXE_NAME.len()];
    let files: Vec<(String, String)> = entries
        .iter()
        .filter_map(|e| Some((e.clone(), e.strip_prefix(prefix)?.to_owned())))
        .filter(|(_, rel)| crate::archive::is_safe(rel))
        .collect();
    crate::archive::extract(archive_path, kind, &files, &part)?;

    let staged = Instance::new(&part);
    let write = |path: PathBuf, text: &str| std::fs::write(&path, text).map_err(|e| Error::io(&path, e));
    write(part.join("portable.txt"), "")?;
    let plugin = part.join(CYBERPUNK_PLUGIN_PATH);
    std::fs::write(&plugin, CYBERPUNK_PLUGIN).map_err(|e| Error::io(&plugin, e))?;
    write(
        staged.ini_path(),
        &format!("[General]\r\ngameName={GAME_NAME}\r\nselected_profile=@ByteArray({DEFAULT_PROFILE})\r\n"),
    )?;
    if let Some(dir) = game_dir {
        staged.set_game_path(dir)?;
    }
    let profile = staged.profile_dir(DEFAULT_PROFILE);
    std::fs::create_dir_all(&profile).map_err(|e| Error::io(&profile, e))?;
    crate::modlist::ModList::default().save(&staged.modlist_path(DEFAULT_PROFILE))?;
    std::fs::rename(&part, root).map_err(|e| Error::io(root, e))
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
    fn rewrites_game_paths_saved_with_backslashes() {
        let ini = "[General]\r\ngamePath=@ByteArray(D:\\\\SteamLibrary\\\\steamapps\\\\common\\\\Cyberpunk 2077)\r\n\
            [customExecutables]\r\n1\\binary=D:/SteamLibrary/steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe\r\n\
            1\\workingDirectory=D:\\\\SteamLibrary\\\\steamapps\\\\common\\\\Cyberpunk 2077\r\n";
        let out = rewrite_game_path(ini, "C:/Games/Cyberpunk 2077");
        assert!(out.contains("gamePath=@ByteArray(C:/Games/Cyberpunk 2077)\r\n"), "{out}");
        assert!(out.contains("1\\binary=C:/Games/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe"), "{out}");
        assert!(out.contains("1\\workingDirectory=C:\\\\Games\\\\Cyberpunk 2077"), "{out}");
        assert!(!out.contains("SteamLibrary"), "{out}");
    }

    #[test]
    fn rewrites_executables_left_behind_a_rewritten_game_path() {
        // gamePath already the player's, executables still the author's: what older launchers left.
        let ini = "[General]\r\ngamePath=@ByteArray(D:\\\\SteamLibrary\\\\steamapps\\\\common\\\\Cyberpunk 2077)\r\n\
            [customExecutables]\r\n\
            1\\binary=C:/Program Files (x86)/Steam/steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe\r\n\
            1\\workingDirectory=C:/Program Files (x86)/Steam/steamapps/common/Cyberpunk 2077/bin/x64\r\n\
            3\\binary=C:/Program Files (x86)/Steam/steamapps/common/Cyberpunk 2077/tools/redmod/bin/redMod.exe\r\n\
            3\\workingDirectory=C:\\\\Program Files (x86)\\\\Steam\\\\steamapps\\\\common\\\\Cyberpunk 2077\\\\tools\\\\redmod\\\\bin\r\n\
            6\\binary=C:/Windows/System32/cmd.exe\r\n";
        let out = rewrite_game_path(ini, "D:/SteamLibrary/steamapps/common/Cyberpunk 2077");
        assert!(out.contains("1\\binary=D:/SteamLibrary/steamapps/common/Cyberpunk 2077/bin/x64/Cyberpunk2077.exe\r\n"), "{out}");
        assert!(out.contains("1\\workingDirectory=D:/SteamLibrary/steamapps/common/Cyberpunk 2077/bin/x64\r\n"), "{out}");
        assert!(out.contains("3\\workingDirectory=D:\\\\SteamLibrary\\\\steamapps\\\\common\\\\Cyberpunk 2077\\\\tools"), "{out}");
        assert!(out.contains("6\\binary=C:/Windows/System32/cmd.exe"), "{out}");
        assert!(!out.contains("Program Files"), "{out}");
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
    fn picks_the_portable_archive_of_a_release() {
        let json = serde_json::json!({
            "tag_name": "v2.5.2",
            "assets": [
                { "name": "Mod.Organizer-2.5.2-pdbs.7z", "browser_download_url": "https://x/pdbs", "size": 1 },
                { "name": "Mod.Organizer-2.5.2.exe", "browser_download_url": "https://x/exe", "size": 2 },
                { "name": "Mod.Organizer-2.5.2.7z", "browser_download_url": "https://x/7z", "size": 3 },
            ]
        });
        assert_eq!(pick_release(&json), Some(Release { version: "2.5.2".into(), url: "https://x/7z".into(), size: 3 }));
        assert_eq!(pick_release(&serde_json::json!({ "tag_name": "v1", "assets": [] })), None);
    }

    #[test]
    fn creates_a_portable_instance_from_the_archive() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("mo2.zip");
        let mut w = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        for name in ["MO2/ModOrganizer.exe", "MO2/plugins/basic_games/games/game_cyberpunk2077.py", "readme.txt"] {
            w.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"x").unwrap();
        }
        w.finish().unwrap();

        let inst = Instance::new(dir.path().join("inst"));
        create_portable(&inst, &archive, Some(Path::new("C:/Games/Cyberpunk 2077"))).unwrap();
        assert!(inst.is_installed() && inst.is_portable());
        let plugin = std::fs::read_to_string(inst.root().join(CYBERPUNK_PLUGIN_PATH)).unwrap();
        assert!(plugin.contains("Version = \"3.0.0\""));
        assert!(!inst.root().join("readme.txt").exists());
        assert!(inst.modlist_path(DEFAULT_PROFILE).is_file());
        assert_eq!(inst.selected_profile().as_deref(), Some(DEFAULT_PROFILE));
        let ini = std::fs::read_to_string(inst.ini_path()).unwrap();
        assert!(ini.contains("gameName=Cyberpunk 2077") && ini.contains("gamePath=@ByteArray(C:/Games/Cyberpunk 2077)"), "{ini}");
        // Never over an existing folder.
        assert!(create_portable(&inst, &archive, None).is_err());
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

//! Locating the Cyberpunk 2077 installation (Steam, GOG, Epic).

use std::path::{Path, PathBuf};

pub const STEAM_APP_ID: &str = "1091500";
pub const GOG_GAME_ID: &str = "1423049311";
const EXE: &[&str] = &["bin", "x64", "Cyberpunk2077.exe"];

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameInstall {
    pub path: PathBuf,
    pub store: &'static str,
}

pub fn exe_path(game_dir: &Path) -> PathBuf {
    EXE.iter().fold(game_dir.to_path_buf(), |p, c| p.join(c))
}

pub fn is_game_dir(dir: &Path) -> bool {
    exe_path(dir).is_file()
}

/// All installs found on this machine, Steam first.
pub fn detect() -> Vec<GameInstall> {
    let mut found = Vec::new();
    for steam in platform::steam_roots() {
        let vdf = steam.join("steamapps").join("libraryfolders.vdf");
        let libraries = std::fs::read_to_string(&vdf).map(|t| library_paths(&t)).unwrap_or_default();
        for lib in std::iter::once(steam.clone()).chain(libraries) {
            let dir = lib.join("steamapps").join("common").join("Cyberpunk 2077");
            push(&mut found, dir, "Steam");
        }
    }
    for dir in platform::gog_paths() {
        push(&mut found, dir, "GOG");
    }
    for dir in epic_paths(&platform::epic_manifest_dir()) {
        push(&mut found, dir, "Epic");
    }
    found
}

fn push(found: &mut Vec<GameInstall>, path: PathBuf, store: &'static str) {
    if is_game_dir(&path) && !found.iter().any(|g| g.path == path) {
        found.push(GameInstall { path, store });
    }
}

/// `"path"` values from Steam's `libraryfolders.vdf`.
fn library_paths(vdf: &str) -> Vec<PathBuf> {
    vdf.lines()
        .filter_map(|l| {
            let l = l.trim();
            let rest = l.strip_prefix("\"path\"")?.trim();
            let v = rest.strip_prefix('"')?.strip_suffix('"')?;
            Some(PathBuf::from(v.replace("\\\\", "\\")))
        })
        .collect()
}

/// Install locations from Epic's `*.item` manifests.
fn epic_paths(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|x| x == "item"))
        .filter_map(|e| std::fs::read_to_string(e.path()).ok())
        .filter_map(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .filter(|v| v["DisplayName"].as_str().is_some_and(|n| n.starts_with("Cyberpunk 2077")))
        .filter_map(|v| v["InstallLocation"].as_str().map(PathBuf::from))
        .collect()
}

#[cfg(windows)]
mod platform {
    use std::path::PathBuf;
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    fn reg(hive: winreg::HKEY, key: &str, value: &str) -> Option<String> {
        RegKey::predef(hive).open_subkey(key).ok()?.get_value(value).ok()
    }

    pub fn steam_roots() -> Vec<PathBuf> {
        [
            reg(HKEY_CURRENT_USER, r"Software\Valve\Steam", "SteamPath"),
            reg(HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam", "InstallPath"),
        ]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .collect()
    }

    pub fn gog_paths() -> Vec<PathBuf> {
        reg(HKEY_LOCAL_MACHINE, &format!(r"SOFTWARE\WOW6432Node\GOG.com\Games\{}", super::GOG_GAME_ID), "path")
            .map(PathBuf::from)
            .into_iter()
            .collect()
    }

    pub fn epic_manifest_dir() -> PathBuf {
        let data = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| r"C:\ProgramData".into());
        data.join(r"Epic\EpicGamesLauncher\Data\Manifests")
    }
}

#[cfg(not(windows))]
mod platform {
    use std::path::PathBuf;

    pub fn steam_roots() -> Vec<PathBuf> {
        Vec::new()
    }
    pub fn gog_paths() -> Vec<PathBuf> {
        Vec::new()
    }
    pub fn epic_manifest_dir() -> PathBuf {
        PathBuf::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_libraryfolders() {
        let vdf = r#""libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps" { "228980" "1" }
	}
	"1"
	{
		"path"		"D:\\SteamLibrary"
		"apps" { "1091500" "1" }
	}
}"#;
        assert_eq!(
            library_paths(vdf),
            [PathBuf::from(r"C:\Program Files (x86)\Steam"), PathBuf::from(r"D:\SteamLibrary")]
        );
    }

    #[test]
    fn reads_epic_manifests() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("A.item"),
            r#"{"DisplayName":"Cyberpunk 2077","InstallLocation":"E:\\Epic\\Cyberpunk 2077"}"#,
        )
        .unwrap();
        std::fs::write(dir.path().join("B.item"), r#"{"DisplayName":"Fortnite","InstallLocation":"E:\\F"}"#).unwrap();
        assert_eq!(epic_paths(dir.path()), [PathBuf::from(r"E:\Epic\Cyberpunk 2077")]);
    }

    #[test]
    fn recognizes_game_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!is_game_dir(dir.path()));
        std::fs::create_dir_all(dir.path().join("bin/x64")).unwrap();
        std::fs::write(exe_path(dir.path()), "").unwrap();
        assert!(is_game_dir(dir.path()));
    }
}

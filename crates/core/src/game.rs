//! Locating the Cyberpunk 2077 installation (Steam, GOG, Epic) and telling its patch.

use std::cmp::Ordering;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
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

/// REDmod is a free DLC; without it REDmod mods can't be deployed.
pub fn has_redmod(game_dir: &Path) -> bool {
    game_dir.join("tools").join("redmod").join("bin").join("redMod.exe").is_file()
}

/// The version resource of `Cyberpunk2077.exe`. The file version is the engine's
/// (`3.0.x`, what RED4ext checks), the product version the patch players know.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExeVersion {
    pub product: [u16; 4],
    pub file: [u16; 4],
}

impl ExeVersion {
    /// The patch as CD Projekt names it: product `2.2.1` is "2.21", `2.3.0` is "2.3",
    /// `2.0.1` is "2.01". A product version written as `2.31.0` reads the same.
    pub fn patch(&self) -> String {
        let [major, minor, patch, _] = self.product;
        if patch == 0 {
            format!("{major}.{minor}")
        } else {
            format!("{major}.{minor}{patch}")
        }
    }
}

/// Reads the version of the game's exe; `None` when it is missing or has no version resource.
/// Only the headers and the resource section are read, not the 100 MB around them.
pub fn exe_version(game_dir: &Path) -> Option<ExeVersion> {
    let mut file = File::open(exe_path(game_dir)).ok()?;
    pe_version(&mut file)
}

/// Orders two patch names ("2.3" < "2.31" < "2.4"; "2.12a" is "2.12"): digits after the dot
/// compare as a decimal fraction, which is how CD Projekt numbers patches. `None` when the
/// majors differ, which is no patch of the same game generation but a misread version.
pub fn compare_patch(a: &str, b: &str) -> Option<Ordering> {
    let split = |v: &str| {
        let (major, rest) = v.trim().split_once('.').unwrap_or((v.trim(), ""));
        let major: u32 = major.parse().ok()?;
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        Some((major, digits.trim_end_matches('0').to_owned()))
    };
    let (a, b) = (split(a)?, split(b)?);
    (a.0 == b.0).then(|| a.1.cmp(&b.1))
}

/// The player's game is another patch than the build's.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameMismatch {
    /// The build's patch, as its author named it.
    pub expected: String,
    /// The player's patch; `None` when it doesn't read as one (only the exe differs).
    pub found: Option<String>,
    /// The game is newer than the build: frameworks wait for their update. `None`: unknown.
    pub newer: Option<bool>,
}

/// Compares the player's exe with the build's. The author's exe patch (`game_exe`) is
/// compared like for like; a manifest without it only by the typed `game_version`, and
/// then not at all when the exe doesn't read as a patch of the same major: a false
/// "wrong version" on every launch is worse than no check.
pub fn check(game_version: &str, game_exe: Option<&str>, exe: &ExeVersion) -> Option<GameMismatch> {
    let found = exe.patch();
    let ord = match game_exe {
        Some(built) if found == built => return None,
        Some(built) => compare_patch(&found, built),
        None => Some(compare_patch(&found, game_version)?),
    };
    if ord == Some(Ordering::Equal) {
        return None;
    }
    let readable = compare_patch(&found, game_version).is_some();
    Some(GameMismatch {
        expected: game_version.to_owned(),
        found: readable.then_some(found),
        newer: ord.map(|o| o == Ordering::Greater),
    })
}

/// The fixed part of `VS_VERSION_INFO` (`VS_FIXEDFILEINFO`): found by its signature in the
/// `RT_VERSION` resource, which is all the walk through the PE headers is for.
fn pe_version(f: &mut (impl Read + Seek)) -> Option<ExeVersion> {
    const RT_VERSION: u32 = 16;
    const FIXED_SIGNATURE: [u8; 4] = 0xFEEF04BDu32.to_le_bytes();
    let u16_at = |b: &[u8], at: usize| Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?));
    let u32_at = |b: &[u8], at: usize| Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?));
    fn read(f: &mut (impl Read + Seek), at: u64, len: usize) -> Option<Vec<u8>> {
        let mut buf = vec![0; len];
        f.seek(SeekFrom::Start(at)).ok()?;
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    let dos = read(f, 0, 64)?;
    (dos.get(..2)? == b"MZ").then_some(())?;
    let pe = u32_at(&dos, 0x3c)? as u64;
    let coff = read(f, pe, 24)?;
    (coff.get(..4)? == b"PE\0\0").then_some(())?;
    let sections = u16_at(&coff, 6)? as usize;
    let optional_size = u16_at(&coff, 20)? as usize;
    let optional = read(f, pe + 24, optional_size)?;
    let dirs = match u16_at(&optional, 0)? {
        0x20b => 112, // PE32+
        0x10b => 96,
        _ => return None,
    };
    let rsrc_rva = u32_at(&optional, dirs + 2 * 8)?;
    let table = read(f, pe + 24 + optional_size as u64, sections * 40)?;
    let (rsrc_va, raw_size, raw_at) = (0..sections).find_map(|i| {
        let s = &table[i * 40..i * 40 + 40];
        let (va, raw_size, raw_at) = (u32_at(s, 12)?, u32_at(s, 16)?, u32_at(s, 20)?);
        (va <= rsrc_rva && rsrc_rva < va + raw_size).then_some((va, raw_size, raw_at))
    })?;
    // The resource section holds icons too; a few MB at most in a real exe.
    if raw_size > 64 << 20 {
        return None;
    }
    let sec = read(f, raw_at as u64, raw_size as usize)?;
    let dir_at = (rsrc_rva - rsrc_va) as usize;

    // Type → name → language: the first entry of each but the type, as Windows does.
    let entry = |dir: usize, id: Option<u32>| -> Option<u32> {
        let count = u16_at(&sec, dir + 12)? as usize + u16_at(&sec, dir + 14)? as usize;
        (0..count).find_map(|i| {
            let e = dir + 16 + i * 8;
            id.is_none_or(|id| u32_at(&sec, e) == Some(id)).then(|| u32_at(&sec, e + 4)).flatten()
        })
    };
    let sub = |off: u32| (off & 0x8000_0000 != 0).then_some(dir_at + (off & 0x7fff_ffff) as usize);
    let names = sub(entry(dir_at, Some(RT_VERSION))?)?;
    let langs = sub(entry(names, None)?)?;
    let data = entry(langs, None)?;
    (data & 0x8000_0000 == 0).then_some(())?;
    let data = dir_at + data as usize;
    let (rva, size) = (u32_at(&sec, data)?, u32_at(&sec, data + 4)? as usize);
    let start = rva.checked_sub(rsrc_va)? as usize;
    let info = sec.get(start..start.checked_add(size)?)?;

    let fixed = info.windows(4).position(|w| w == FIXED_SIGNATURE)?;
    let word = |at: usize| u32_at(info, fixed + at);
    let parts = |ms: u32, ls: u32| [(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16];
    Some(ExeVersion { file: parts(word(8)?, word(12)?), product: parts(word(16)?, word(20)?) })
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

    /// A minimal PE32+ with one `.rsrc` section holding a version resource, laid out as the linker does.
    fn exe_with_version(file: [u16; 4], product: [u16; 4]) -> Vec<u8> {
        let le16 = |v: u16| v.to_le_bytes().to_vec();
        let le32 = |v: u32| v.to_le_bytes().to_vec();
        let words = |v: [u16; 4]| [le32((v[0] as u32) << 16 | v[1] as u32), le32((v[2] as u32) << 16 | v[3] as u32)].concat();
        let mut blob = vec![0; 6];
        blob.extend("VS_VERSION_INFO\0".encode_utf16().flat_map(u16::to_le_bytes));
        blob.resize(blob.len().next_multiple_of(4), 0);
        blob.extend(le32(0xFEEF04BD));
        blob.extend(le32(0x10000));
        blob.extend(words(file));
        blob.extend(words(product));
        blob.extend([0; 28]);

        let dir = |id: u32, off: u32| [vec![0; 14], le16(1), le32(id), le32(off)].concat();
        let mut rsrc = [dir(16, 0x8000_0018), dir(1, 0x8000_0030), dir(1033, 0x48)].concat();
        rsrc.extend([le32(0x1000 + 0x58), le32(blob.len() as u32), vec![0; 8]].concat());
        rsrc.extend(blob);

        let mut exe = vec![0; 64];
        exe[..2].copy_from_slice(b"MZ");
        exe[0x3c..0x40].copy_from_slice(&le32(64));
        exe.extend(b"PE\0\0");
        exe.extend([le16(0x8664), le16(1), vec![0; 12], le16(240), le16(0x22)].concat());
        let mut optional = le16(0x20b);
        optional.resize(128, 0);
        optional.extend([le32(0x1000), le32(rsrc.len() as u32)].concat());
        optional.resize(240, 0);
        exe.extend(optional);
        exe.extend([b".rsrc\0\0\0".to_vec(), le32(rsrc.len() as u32), le32(0x1000), le32(rsrc.len() as u32), le32(512), vec![0; 16]].concat());
        exe.resize(512, 0);
        exe.extend(rsrc);
        exe
    }

    #[test]
    fn reads_exe_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("bin/x64")).unwrap();
        std::fs::write(exe_path(dir.path()), exe_with_version([3, 0, 80, 1234], [2, 3, 1, 0])).unwrap();
        let v = exe_version(dir.path()).unwrap();
        assert_eq!(v, ExeVersion { file: [3, 0, 80, 1234], product: [2, 3, 1, 0] });
        assert_eq!(v.patch(), "2.31");

        std::fs::write(exe_path(dir.path()), b"MZ not really an exe").unwrap();
        assert_eq!(exe_version(dir.path()), None);
    }

    #[test]
    fn names_and_orders_patches() {
        let patch = |p: [u16; 4]| ExeVersion { file: [0; 4], product: p }.patch();
        assert_eq!(patch([2, 3, 0, 0]), "2.3");
        assert_eq!(patch([2, 0, 1, 0]), "2.01");
        assert_eq!(patch([2, 2, 1, 0]), "2.21");
        assert_eq!(patch([2, 31, 0, 0]), "2.31");

        assert_eq!(compare_patch("2.31", "2.31"), Some(Ordering::Equal));
        assert_eq!(compare_patch("2.3", "2.30"), Some(Ordering::Equal));
        assert_eq!(compare_patch("2.12a", "2.12"), Some(Ordering::Equal));
        assert_eq!(compare_patch("2.3", "2.31"), Some(Ordering::Less));
        assert_eq!(compare_patch("2.12", "2.2"), Some(Ordering::Less));
        assert_eq!(compare_patch("2.4", "2.31"), Some(Ordering::Greater));
        assert_eq!(compare_patch("3.080", "2.31"), None, "an engine version is no patch");
        assert_eq!(compare_patch("", "2.31"), None);
    }

    #[test]
    fn checks_the_players_patch() {
        let exe = |p: [u16; 4]| ExeVersion { file: [3, 0, 80, 0], product: p };
        assert_eq!(check("2.31", Some("2.31"), &exe([2, 3, 1, 0])), None);
        assert_eq!(check("2.31", None, &exe([2, 3, 1, 0])), None);
        assert_eq!(
            check("2.31", Some("2.31"), &exe([2, 4, 0, 0])),
            Some(GameMismatch { expected: "2.31".into(), found: Some("2.4".into()), newer: Some(true) })
        );
        assert_eq!(
            check("2.31", None, &exe([2, 2, 1, 0])),
            Some(GameMismatch { expected: "2.31".into(), found: Some("2.21".into()), newer: Some(false) })
        );
        // The author's exe is the reference even if the typed name differs from it.
        assert_eq!(check("2.31 hotfix", Some("2.31"), &exe([2, 3, 1, 0])), None);
        // A product version that isn't a patch: checked only against the author's exe.
        assert_eq!(check("2.31", None, &exe([3, 0, 80, 0])), None);
        assert_eq!(
            check("2.31", Some("3.080"), &exe([3, 0, 81, 0])),
            Some(GameMismatch { expected: "2.31".into(), found: None, newer: Some(true) })
        );
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

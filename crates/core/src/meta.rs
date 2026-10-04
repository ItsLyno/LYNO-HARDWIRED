//! `mods/<mod>/meta.ini` — the per-mod metadata MO2 keeps.
//!
//! The launcher also stamps its own `[LYNO]` section so it can tell the mods
//! it manages from mods the user added by hand.

use std::path::Path;

use ini::{EscapePolicy, Ini, ParseOption, WriteOption};

use crate::{Error, Result};

const GENERAL: &str = "General";
const INSTALLED: &str = "installedFiles";
const LYNO: &str = "LYNO";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModMeta {
    pub game_name: Option<String>,
    pub mod_id: Option<u64>,
    pub file_id: Option<u64>,
    pub version: Option<String>,
    pub installation_file: Option<String>,
    pub repository: Option<String>,
    /// Manifest id of the mod, when the launcher installed it.
    pub lyno_id: Option<String>,
}

impl ModMeta {
    pub fn is_managed(&self) -> bool {
        self.lyno_id.is_some()
    }

    pub fn parse(text: &str, path: &Path) -> Result<Self> {
        let ini = Ini::load_from_str_opt(text, parse_opt()).map_err(|e| Error::Parse {
            path: path.to_owned(),
            message: e.to_string(),
        })?;

        let general = |key: &str| {
            ini.section(Some(GENERAL))
                .and_then(|s| s.get(key))
                .map(unquote)
                .filter(|v| !v.is_empty())
        };
        let num = |v: Option<String>| v.and_then(|v| v.parse::<u64>().ok()).filter(|&n| n > 0);

        // MO2 records the source file under `[installedFiles]` as `1\fileid`.
        let installed = ini.section(Some(INSTALLED));
        let file_id = num(installed.and_then(|s| s.get("1\\fileid")).map(unquote));
        let mod_id = num(general("modid"))
            .or_else(|| num(installed.and_then(|s| s.get("1\\modid")).map(unquote)));

        Ok(Self {
            game_name: general("gameName"),
            mod_id,
            file_id,
            version: general("version"),
            installation_file: general("installationFile"),
            repository: general("repository"),
            lyno_id: ini
                .section(Some(LYNO))
                .and_then(|s| s.get("id"))
                .map(unquote)
                .filter(|v| !v.is_empty()),
        })
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        Self::parse(&text, path)
    }

    /// Writes the fields MO2 needs to show Nexus info and check for updates,
    /// preserving any other keys already in the file.
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut ini = if path.exists() {
            let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
            Ini::load_from_str_opt(&text, parse_opt()).map_err(|e| Error::Parse {
                path: path.to_owned(),
                message: e.to_string(),
            })?
        } else {
            Ini::new()
        };

        let mut set = |section: &str, key: &str, value: Option<String>| {
            if let Some(v) = value {
                ini.with_section(Some(section)).set(key, v);
            }
        };
        set(GENERAL, "gameName", self.game_name.clone());
        set(GENERAL, "modid", self.mod_id.map(|v| v.to_string()));
        set(GENERAL, "version", self.version.clone());
        set(GENERAL, "installationFile", self.installation_file.clone());
        set(GENERAL, "repository", self.repository.clone());
        if self.file_id.is_some() {
            set(INSTALLED, "1\\modid", self.mod_id.map(|v| v.to_string()));
            set(INSTALLED, "1\\fileid", self.file_id.map(|v| v.to_string()));
            set(INSTALLED, "size", Some("1".into()));
        }
        set(LYNO, "id", self.lyno_id.clone());

        let opt = WriteOption { escape_policy: EscapePolicy::Nothing, ..Default::default() };
        ini.write_to_file_opt(path, opt).map_err(|e| Error::io(path, e))
    }
}

fn parse_opt() -> ParseOption {
    // MO2 writes Qt-style keys like `1\fileid`; escapes must stay literal.
    ParseOption { enabled_quote: false, enabled_escape: false, ..Default::default() }
}

fn unquote(v: &str) -> String {
    v.trim().trim_matches('"').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[General]
gameName=cyberpunk2077
modid=107
version=1.35.0
newestVersion=
category="2,"
installationFile=Cyber Engine Tweaks-107-1-35-0-1735000000.zip
repository=Nexus

[installedFiles]
1\modid=107
1\fileid=91234
size=1
"#;

    #[test]
    fn parses_mo2_meta() {
        let m = ModMeta::parse(SAMPLE, Path::new("meta.ini")).unwrap();
        assert_eq!(m.game_name.as_deref(), Some("cyberpunk2077"));
        assert_eq!(m.mod_id, Some(107));
        assert_eq!(m.file_id, Some(91234));
        assert_eq!(m.version.as_deref(), Some("1.35.0"));
        assert_eq!(m.repository.as_deref(), Some("Nexus"));
        assert!(!m.is_managed());
    }

    #[test]
    fn handles_missing_ids() {
        let m = ModMeta::parse("[General]\nmodid=0\nversion=\n", Path::new("meta.ini")).unwrap();
        assert_eq!(m.mod_id, None);
        assert_eq!(m.version, None);
        assert_eq!(m.file_id, None);
    }

    #[test]
    fn save_preserves_unknown_keys_and_stamps_marker() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("meta.ini");
        std::fs::write(&p, SAMPLE).unwrap();

        let mut m = ModMeta::load(&p).unwrap();
        m.lyno_id = Some("cet".into());
        m.save(&p).unwrap();

        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("category=\"2,\""), "unknown keys kept: {text}");
        assert!(text.contains("1\\fileid=91234"));
        let back = ModMeta::load(&p).unwrap();
        assert_eq!(back.lyno_id.as_deref(), Some("cet"));
        assert_eq!(back.file_id, Some(91234));
    }
}

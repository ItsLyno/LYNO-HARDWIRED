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
    /// `[LYNO] optional=true`: players may switch the mod off even inside a core section.
    pub lyno_optional: bool,
    /// `[LYNO] core=true`: players can't switch the mod off. On a separator,
    /// covers every mod below it up to the next separator.
    pub lyno_core: bool,
    /// `[LYNO] fomod`: the options picked in the mod's FOMOD installer
    /// ([`crate::fomod::encode_saved`]).
    pub lyno_fomod: Option<String>,
    /// `[General] color`: the color the author gave a separator in MO2, as `#rrggbb`.
    pub color: Option<String>,
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

        let lyno = |key: &str| ini.section(Some(LYNO)).and_then(|s| s.get(key)).map(unquote).filter(|v| !v.is_empty());
        let flag = |key: &str| lyno(key).is_some_and(|v| matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes"));
        Ok(Self {
            game_name: general("gameName"),
            mod_id,
            file_id,
            version: general("version").map(|v| display_version(&v)),
            installation_file: general("installationFile"),
            repository: general("repository"),
            lyno_id: lyno("id"),
            lyno_optional: flag("optional"),
            lyno_core: flag("core"),
            lyno_fomod: lyno("fomod"),
            color: general("color").and_then(|v| qt_color(&v)),
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

/// Sets (`Some`) or removes `[LYNO] fomod`, keeping every other key.
pub fn save_fomod(path: &Path, value: Option<&str>) -> Result<()> {
    let mut ini = if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        Ini::load_from_str_opt(&text, parse_opt()).map_err(|e| Error::Parse { path: path.to_owned(), message: e.to_string() })?
    } else {
        Ini::new()
    };
    match value {
        Some(v) => {
            ini.with_section(Some(LYNO)).set("fomod", v);
        }
        None => {
            if let Some(section) = ini.section_mut(Some(LYNO)) {
                section.remove("fomod");
            }
        }
    }
    let opt = WriteOption { escape_policy: EscapePolicy::Nothing, ..Default::default() };
    ini.write_to_file_opt(path, opt).map_err(|e| Error::io(path, e))
}

/// Sets `[General] color` (MO2 shows it on the separator), keeping every other key.
pub fn save_color(path: &Path, hex: &str) -> Result<()> {
    let Some(value) = qt_color_value(hex) else {
        return Err(Error::Manifest(format!("bad color {hex:?}")));
    };
    let mut ini = if path.exists() {
        let text = std::fs::read_to_string(path).map_err(|e| Error::io(path, e))?;
        Ini::load_from_str_opt(&text, parse_opt())
            .map_err(|e| Error::Parse { path: path.to_owned(), message: e.to_string() })?
    } else {
        Ini::new()
    };
    ini.with_section(Some(GENERAL)).set("color", value);
    let opt = WriteOption { escape_policy: EscapePolicy::Nothing, ..Default::default() };
    ini.write_to_file_opt(path, opt).map_err(|e| Error::io(path, e))
}

// MO2 stores a QColor through QSettings: `@Variant(<bytes>)`, the bytes being QDataStream (Qt 4.0 format)
// of a QVariant: quint32 type id (0x43 = QColor), then qint8 spec, quint16 alpha, red, green, blue, pad.
// QSettings escapes the bytes C-style (`\0`, `\xff`, `\t`…) and leaves printable ASCII as is.
const QCOLOR_TYPE: [u8; 4] = [0, 0, 0, 0x43];
const QCOLOR_SPEC_RGB: u8 = 1;

/// `@Variant(...)` → `#rrggbb`; `None` for anything that is not a valid RGB QColor.
pub fn qt_color(value: &str) -> Option<String> {
    let bytes = unescape_qt(value.strip_prefix("@Variant(")?.strip_suffix(')')?)?;
    let rest = bytes.strip_prefix(&QCOLOR_TYPE)?;
    // Newer stream versions put an is-null flag before the value; the color itself is the last 11 bytes.
    let c = rest.get(rest.len().checked_sub(11)?..)?;
    if c[0] != QCOLOR_SPEC_RGB {
        return None;
    }
    Some(format!("#{:02x}{:02x}{:02x}", c[3], c[5], c[7]))
}

/// `#rrggbb` → the value MO2 writes. Every byte is `\xNN`: unambiguous for Qt's greedy hex reader.
fn qt_color_value(hex: &str) -> Option<String> {
    let rgb = parse_hex_color(hex)?;
    let mut bytes = QCOLOR_TYPE.to_vec();
    bytes.extend([QCOLOR_SPEC_RGB, 0xff, 0xff]);
    for c in rgb {
        bytes.extend([c, c]);
    }
    bytes.extend([0, 0]);
    let escaped: String = bytes.iter().map(|b| format!("\\x{b:x}")).collect();
    Some(format!("@Variant({escaped})"))
}

pub fn parse_hex_color(hex: &str) -> Option<[u8; 3]> {
    let h = hex.strip_prefix('#').filter(|h| h.len() == 6 && h.bytes().all(|b| b.is_ascii_hexdigit()))?;
    let byte = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
    Some([byte(0)?, byte(2)?, byte(4)?])
}

fn unescape_qt(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(u8::try_from(u32::from(c)).ok()?);
            continue;
        }
        let byte = match chars.next()? {
            'x' => {
                let mut v: u32 = 0;
                let mut digits = 0;
                while let Some(d) = chars.peek().and_then(|d| d.to_digit(16)) {
                    v = v.checked_mul(16)? + d;
                    digits += 1;
                    chars.next();
                }
                if digits == 0 {
                    return None;
                }
                u8::try_from(v).ok()?
            }
            '0' => 0,
            'a' => 7,
            'b' => 8,
            't' => 9,
            'n' => 10,
            'v' => 11,
            'f' => 12,
            'r' => 13,
            other => u8::try_from(u32::from(other)).ok()?,
        };
        out.push(byte);
    }
    Some(out)
}

fn parse_opt() -> ParseOption {
    // MO2 writes Qt-style keys like `1\fileid`; escapes must stay literal.
    ParseOption { enabled_quote: false, enabled_escape: false, ..Default::default() }
}

/// A version the way MO2 shows it. MO2 rewrites `version` in `meta.ini` in its
/// canonical form: at least three segments (`1.35` → `1.35.0`), a `d` prefix for
/// the decimal-mark scheme (`d1.35`), `f` for literal ones; its own UI strips that
/// back to two segments. Shown or compared raw, every version gets an extra `.0`.
pub fn display_version(v: &str) -> String {
    let v = v.trim();
    let v = match v.strip_prefix(['d', 'f']) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => v,
    };
    // The leading `N.N.N` run; whatever follows (`b`, `-hotfix`) stays as is.
    let mut end = 0;
    let bytes = v.as_bytes();
    while end < bytes.len() {
        let c = bytes[end];
        if c.is_ascii_digit() || (c == b'.' && end > 0 && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)) {
            end += 1;
        } else {
            break;
        }
    }
    let (numbers, rest) = v.split_at(end);
    let mut segments: Vec<&str> = numbers.split('.').collect();
    while segments.len() > 2 && segments.last().is_some_and(|s| s.bytes().all(|b| b == b'0')) {
        segments.pop();
    }
    format!("{}{rest}", segments.join("."))
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
        assert_eq!(m.version.as_deref(), Some("1.35"));
        assert_eq!(m.repository.as_deref(), Some("Nexus"));
        assert!(!m.is_managed());
    }

    #[test]
    fn versions_as_mo2_shows_them() {
        for (raw, shown) in [
            ("1.35.0", "1.35"),
            ("1.0.0", "1.0"),
            ("2.1.0.0", "2.1"),
            ("2.5.2", "2.5.2"),
            ("1.2.0.3", "1.2.0.3"),
            ("1.35", "1.35"),
            ("2", "2"),
            ("d1.35", "1.35"),
            ("f1.0.0", "1.0"),
            ("1.10.0b", "1.10b"),
            ("v1.2", "v1.2"),
            ("final", "final"),
            ("1.0.", "1.0."),
        ] {
            assert_eq!(display_version(raw), shown, "{raw}");
        }
    }

    #[test]
    fn handles_missing_ids() {
        let m = ModMeta::parse("[General]\nmodid=0\nversion=\n", Path::new("meta.ini")).unwrap();
        assert_eq!(m.mod_id, None);
        assert_eq!(m.version, None);
        assert_eq!(m.file_id, None);
        assert!(!m.lyno_optional);
    }

    #[test]
    fn parses_optional_and_core_flags() {
        let parse = |t: &str| ModMeta::parse(t, Path::new("meta.ini")).unwrap();
        assert!(parse("[LYNO]\nid=hd\noptional=true\n").lyno_optional);
        assert!(parse("[LYNO]\noptional=1\n").lyno_optional);
        assert!(!parse("[LYNO]\noptional=false\n").lyno_optional);
        assert!(!parse("[General]\noptional=true\n").lyno_optional);
        assert!(parse("[LYNO]\ncore=true\n").lyno_core);
        assert!(!parse("[LYNO]\ncore=no\n").lyno_core);
        assert!(!parse("[LYNO]\noptional=true\n").lyno_core);
    }

    #[test]
    fn parses_separator_color() {
        let parse = |v: &str| qt_color(v);
        assert_eq!(parse(r"@Variant(\0\0\0\x43\x1\xff\xff\x3\x3\x93\x93\xe1\xe1\0\0)").as_deref(), Some("#0393e1"));
        // Printable bytes stay literal (`{`, `A`), control bytes get C escapes (`\t`); a hex digit right after
        // `\xNN` is escaped too, or the reader would take it as part of the number.
        assert_eq!(parse(r"@Variant(\0\0\0\x43\x1\xff\xff{{AA\t\t\0\0)").as_deref(), Some("#7b4109"));
        assert_eq!(parse(r"@Variant(\0\0\0\x43\x1\xff\xff\x41\x41{{\t\t\0\0)").as_deref(), Some("#417b09"));
        // Not RGB (spec 2 = HSV), not a QColor, garbage.
        assert_eq!(parse(r"@Variant(\0\0\0\x43\x2\xff\xff\x3\x3\x93\x93\xe1\xe1\0\0)"), None);
        assert_eq!(parse(r"@Variant(\0\0\0\x2\x1)"), None);
        assert_eq!(parse("red"), None);

        let m = ModMeta::parse("[General]\ncolor=@Variant(\\0\\0\\0\\x43\\x1\\xff\\xff\\xff\\xff\\0\\0\\0\\0\\0\\0)\n", Path::new("meta.ini")).unwrap();
        assert_eq!(m.color.as_deref(), Some("#ff0000"));
    }

    #[test]
    fn color_round_trips_through_meta_ini() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("meta.ini");
        std::fs::write(&p, "[General]\nmodid=0\n").unwrap();
        save_color(&p, "#3cf2a0").unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("modid=0"), "other keys kept: {text}");
        assert_eq!(ModMeta::load(&p).unwrap().color.as_deref(), Some("#3cf2a0"));
        assert!(save_color(&p, "red").is_err());
    }

    #[test]
    fn fomod_choice_is_set_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("meta.ini");
        std::fs::write(&p, SAMPLE).unwrap();
        save_fomod(&p, Some("eyJhIjoxfQ")).unwrap();
        assert_eq!(ModMeta::load(&p).unwrap().lyno_fomod.as_deref(), Some("eyJhIjoxfQ"));
        save_fomod(&p, None).unwrap();
        let back = ModMeta::load(&p).unwrap();
        assert_eq!(back.lyno_fomod, None);
        assert_eq!(back.file_id, Some(91234));
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

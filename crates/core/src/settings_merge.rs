//! Add newly shipped settings without replacing the player's existing values.
//! Arrays and unknown/binary formats are kept as a whole; guessing their schema
//! could corrupt bindings or persistent game data.

use std::path::Path;

use ini::{EscapePolicy, Ini, ParseOption, WriteOption};

use crate::{Error, Result};

pub(crate) fn merge(player: &Path, author: &Path) -> Result<()> {
    let mine = std::fs::read(player).map_err(|e| Error::io(player, e))?;
    let theirs = std::fs::read(author).map_err(|e| Error::io(author, e))?;
    let merged = merge_bytes(author.extension().and_then(|e| e.to_str()).unwrap_or(""), &mine, &theirs);
    std::fs::write(author, merged).map_err(|e| Error::io(author, e))
}

pub(crate) fn merge_bytes(extension: &str, player: &[u8], author: &[u8]) -> Vec<u8> {
    if extension.eq_ignore_ascii_case("json") {
        if let (Ok(mut mine), Ok(theirs)) = (serde_json::from_slice(player), serde_json::from_slice(author)) {
            if fill_json(&mut mine, &theirs) {
                if let Ok(bytes) = serde_json::to_vec_pretty(&mine) {
                    return bytes;
                }
            }
        }
    } else if extension.eq_ignore_ascii_case("ini") {
        if let (Ok(mine), Ok(theirs)) = (std::str::from_utf8(player), std::str::from_utf8(author)) {
            let options = || ParseOption { enabled_escape: false, enabled_quote: false, ..Default::default() };
            if let (Ok(mut mine), Ok(theirs)) = (Ini::load_from_str_opt(mine, options()), Ini::load_from_str_opt(theirs, options())) {
                let mut changed = false;
                for (section, values) in &theirs {
                    for (key, value) in values {
                        if mine.section(section).and_then(|s| s.get(key)).is_none() {
                            mine.with_section(section).set(key, value);
                            changed = true;
                        }
                    }
                }
                if changed {
                    let mut bytes = Vec::new();
                    let options = WriteOption { escape_policy: EscapePolicy::Nothing, ..Default::default() };
                    if mine.write_to_opt(&mut bytes, options).is_ok() {
                        return bytes;
                    }
                }
            }
        }
    }
    player.to_vec()
}

fn fill_json(player: &mut serde_json::Value, author: &serde_json::Value) -> bool {
    let (Some(mine), Some(theirs)) = (player.as_object_mut(), author.as_object()) else { return false };
    let mut changed = false;
    for (key, value) in theirs {
        if let Some(existing) = mine.get_mut(key) {
            changed |= fill_json(existing, value);
        } else {
            mine.insert(key.clone(), value.clone());
            changed = true;
        }
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_adds_new_mods_and_nested_options_but_keeps_player_values() {
        let mine = br#"{"old":{"enabled":false,"bindings":[1]},"own":7,"null":null}"#;
        let author = br#"{"old":{"enabled":true,"bindings":[2,3],"newOption":4},"newMod":{"enabled":true},"null":2}"#;
        let merged: serde_json::Value = serde_json::from_slice(&merge_bytes("JSON", mine, author)).unwrap();
        assert_eq!(merged, serde_json::json!({"old":{"enabled":false,"bindings":[1],"newOption":4},"own":7,"newMod":{"enabled":true},"null":null}));
        assert_eq!(merge_bytes("json", &serde_json::to_vec(&merged).unwrap(), author), serde_json::to_vec(&merged).unwrap());
    }

    #[test]
    fn ini_adds_sections_and_keys_and_preserves_literal_paths() {
        let merged = merge_bytes("ini", b"[Old]\nvalue=player\npath=C:\\new\\test\n", b"[Old]\nvalue=author\nadded=4\n[NewMod]\nenabled=true\n");
        let text = String::from_utf8(merged).unwrap();
        assert!(text.contains("value=player") && text.contains("added=4") && text.contains("[NewMod]") && text.contains("path=C:\\new\\test"), "{text}");
    }

    #[test]
    fn opaque_invalid_and_unchanged_files_are_preserved_byte_for_byte() {
        for (ext, mine, author) in [("sqlite3", &b"binary\0"[..], &b"other"[..]), ("json", b"broken", b"{}"), ("json", b"{\"a\": 2}\n", b"{\"a\":1}"), ("ini", b"[a]\nx=2\n", b"[a]\nx=1\n")] {
            assert_eq!(merge_bytes(ext, mine, author), mine);
        }
    }
}

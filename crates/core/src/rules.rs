//! Which files of an MO2 instance are part of the build.
//!
//! Cyberpunk frameworks write files while the game runs. Through MO2's
//! virtual file system new files land in `overwrite/`, files that already
//! exist in a mod are changed in place. Some of them are generated per
//! machine and game version and must never be shipped:
//!
//! - `r6/cache/*`: redscript bundle (`final.redscripts`, `.bk`), REDmod
//!   deployment (`r6/cache/modded/`) and the plugin's copies of game caches.
//!   Built against the author's game version; a stale copy breaks the game.
//! - `archive/pc/mod/modlist.txt`: archive load order the MO2 Cyberpunk
//!   plugin regenerates before every launch from the MO2 order.
//! - logs: `*.log` (CET, CET mods, TweakXL, ArchiveXL), `r6/logs/` (redscript),
//!   `red4ext/logs/`.
//!
//! Mod settings (CET `config.json`/`bindings.json`, per-mod `db.sqlite3` and
//! json files, Mod Settings `red4ext/plugins/mod_settings/user.ini`) are
//! shipped: they are what makes the build configured.

/// True for files the game, its frameworks or MO2 regenerate at runtime.
/// `path` is relative to a mod folder (= the game folder), with `/`.
pub fn is_generated(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    const PREFIXES: &[&str] = &["r6/cache/", "r6/logs/", "red4ext/logs/"];
    PREFIXES.iter().any(|x| p.starts_with(x))
        || p == "archive/pc/mod/modlist.txt"
        || p.ends_with(".log")
}

/// True for files whose content identifies a mod package.
///
/// MO2 rewrites `meta.ini` on its own (update checks store
/// `lastNexusQuery`, `nexusLastModified`, …). Hashing it would repack and
/// re-upload a multi-gigabyte mod whenever MO2 checked Nexus. It is still
/// shipped, just not part of the package identity.
pub fn is_hashed(path: &str) -> bool {
    !path.eq_ignore_ascii_case("meta.ini")
}

/// True for files of `profiles/<profile>/` that are the author's own and
/// stay out of the base package (`rel` is relative to the profile folder).
///
/// - `modlist.txt`: the manifest defines the load order.
/// - `saves/`: profile-local saves.
/// - `UserSettings.json`: profile-local game settings (graphics, keybinds),
///   tied to the author's hardware.
pub fn is_private_profile_file(rel: &str) -> bool {
    let p = rel.to_ascii_lowercase();
    p == "modlist.txt" || p == "usersettings.json" || p.starts_with("saves/") || p.ends_with(".log")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_files() {
        for p in [
            "r6/cache/final.redscripts",
            "r6/cache/final.redscripts.bk",
            "R6/Cache/modded/MO_REDmod_load_order.txt",
            "r6/logs/redscript_rCURRENT.log",
            "red4ext/logs/red4ext-2026-01-01.log",
            "red4ext/plugins/TweakXL/TweakXL.log",
            "bin/x64/plugins/cyber_engine_tweaks/cyber_engine_tweaks.log",
            "bin/x64/plugins/cyber_engine_tweaks/mods/AppearanceMenuMod/AppearanceMenuMod.log",
            "archive/pc/mod/modlist.txt",
        ] {
            assert!(is_generated(p), "{p}");
        }
        for p in [
            "archive/pc/mod/basegame_x.archive",
            "r6/scripts/mod/a.reds",
            "r6/tweaks/mod.yaml",
            "red4ext/plugins/mod_settings/user.ini",
            "bin/x64/plugins/cyber_engine_tweaks/bindings.json",
            "bin/x64/plugins/cyber_engine_tweaks/mods/AppearanceMenuMod/db.sqlite3",
            "meta.ini",
        ] {
            assert!(!is_generated(p), "{p}");
        }
    }

    #[test]
    fn meta_ini_is_not_hashed() {
        assert!(!is_hashed("meta.ini"));
        assert!(is_hashed("archive/pc/mod/meta.ini"));
        assert!(is_hashed("archive/pc/mod/x.archive"));
    }

    #[test]
    fn private_profile_files() {
        for p in ["modlist.txt", "UserSettings.json", "saves/ManualSave-1/sav.dat"] {
            assert!(is_private_profile_file(p), "{p}");
        }
        for p in ["settings.ini", "plugins.txt", "archives.txt"] {
            assert!(!is_private_profile_file(p), "{p}");
        }
    }
}

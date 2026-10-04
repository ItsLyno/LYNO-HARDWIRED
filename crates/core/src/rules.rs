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
//!   `red4ext/logs/`, `tools/redmod/bin/REDmodLog.txt` (REDmod deploy).
//! - Advanced Crash Reporter (`bin/x64/plugins/AdvancedCrashReporter/`):
//!   crash reports and the `reports/watch/` snapshots, the inventory of
//!   installed mods and caches of engine symbols, rewritten on every launch.
//!
//! Mod settings (CET `config.json`/`bindings.json`, per-mod `db.sqlite3` and
//! json files, Mod Settings `red4ext/plugins/mod_settings/user.ini`) are
//! shipped: they are what makes the build configured.

/// True for files the game, its frameworks or MO2 regenerate at runtime.
/// `path` is relative to a mod folder (= the game folder), with `/`.
pub fn is_generated(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    const PREFIXES: &[&str] = &["r6/cache/", "r6/logs/", "red4ext/logs/", "bin/x64/plugins/advancedcrashreporter/reports/"];
    const CRASH_REPORTER: &str = "bin/x64/plugins/advancedcrashreporter/";
    PREFIXES.iter().any(|x| p.starts_with(x))
        || p == "archive/pc/mod/modlist.txt"
        || p == "tools/redmod/bin/redmodlog.txt"
        || p.ends_with(".log")
        || p.strip_prefix(CRASH_REPORTER).is_some_and(|f| !f.contains('/') && (f.ends_with(".cache") || f == "mod-inventory.txt"))
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

/// True for files that hold settings: the ones a player changes, in game or
/// by hand, inside a build mod. Through MO2's virtual file system the game
/// writes new files to `overwrite/`, but rewrites files that already exist in
/// a mod in place, so a shipped config the player changed (CET `bindings.json`,
/// a CET mod's `db.sqlite3`, Mod Settings `user.ini`) differs from the package
/// without the mod being damaged. [`crate::verify`] reports such files apart
/// from damage, and a repair keeps them.
///
/// By extension, so it is a guess: a truncated config also counts as a
/// setting. TweakXL / ArchiveXL `.yaml` and `.xl` are content, never
/// rewritten by the game, and don't count. Codeware keeps the persistent
/// fields of mods' scriptable services (their settings, among others) in
/// `red4ext/plugins/Codeware/Persistent/` and rewrites it on every launch.
pub fn is_settings(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    let ext = p.rsplit_once('.').map_or("", |(_, e)| e);
    is_hashed(&p)
        && (matches!(ext, "json" | "ini" | "toml" | "cfg" | "conf" | "xml" | "sqlite" | "sqlite3" | "db")
            || p.starts_with("red4ext/plugins/codeware/persistent/"))
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
            "tools/redmod/bin/REDmodLog.txt",
            "bin/x64/plugins/AdvancedCrashReporter/engine-names.cache",
            "bin/x64/plugins/AdvancedCrashReporter/engine-functions.cache",
            "bin/x64/plugins/AdvancedCrashReporter/mod-inventory.txt",
            "bin/x64/plugins/AdvancedCrashReporter/reports/20261004-155731-crash-tid9188-01.txt",
            "bin/x64/plugins/AdvancedCrashReporter/reports/watch/watch-00.txt",
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
            "bin/x64/plugins/AdvancedCrashReporter.asi",
            "bin/x64/plugins/AdvancedCrashReporter/config.ini",
            "red4ext/plugins/Codeware/Persistent/ScriptableServiceContainer.dat",
        ] {
            assert!(!is_generated(p), "{p}");
        }
    }

    #[test]
    fn settings_files() {
        for p in [
            "bin/x64/plugins/cyber_engine_tweaks/bindings.json",
            "bin/x64/plugins/cyber_engine_tweaks/mods/AppearanceMenuMod/db.sqlite3",
            "red4ext/plugins/mod_settings/user.ini",
            "engine/config/platform/pc/ultra.INI",
            "red4ext/plugins/Codeware/Persistent/ScriptableServiceContainer.dat",
        ] {
            assert!(is_settings(p), "{p}");
        }
        for p in ["archive/pc/mod/a.archive", "r6/tweaks/mod.yaml", "archive/pc/mod/a.archive.xl", "bin/x64/x.dll", "meta.ini", "init.lua"] {
            assert!(!is_settings(p), "{p}");
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

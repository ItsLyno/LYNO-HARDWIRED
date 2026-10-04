# Cyberpunk 2077 under Mod Organizer 2

Reference for working on the launcher: how MO2 runs Cyberpunk, which files the
game and its frameworks write at runtime, and what that means for the build.

Sources (checked 2026-10):

- MO2 Cyberpunk plugin v3.0.1 — copy in [`reference/mo2-basic-games/`](reference/mo2-basic-games/),
  wiki: <https://github.com/ModOrganizer2/modorganizer-basic_games/wiki/Game:-Cyberpunk-2077>
- REDmodding wiki, MO2 page — <https://wiki.redmodding.org/cyberpunk-2077-modding/for-mod-users/users-modding-cyberpunk-2077/getting-started/mo2-mod-organizer-2>
  (source: <https://github.com/CDPR-Modding-Documentation/Cyberpunk-Modding-Docs>, `for-mod-users/`)
- Framework sources: CET (`maximegmd/CyberEngineTweaks`), RED4ext (`WopsS/RED4ext`),
  redscript (`jac3km4/redscript`), Mod Settings (`jackhumbert/mod_settings`),
  TweakXL / ArchiveXL / Codeware (`psiberx/*`)
- MO2 source (`ModOrganizer2/modorganizer`, `src/profile.cpp`, `src/settings.cpp`)

## Requirements

- MO2 2.5.3+ with the Cyberpunk plugin 3.0+. MO2 must **not** be inside the game folder.
- The game folder should be clean (no manually installed mods): MO2 loads mods from
  both its VFS and the game folder.
- CET 1.27+ (older versions don't work through USVFS).
- RootBuilder is obsolete: since game ~2.12 the plugin loads CET and RED4ext through
  **forced load libraries** (`bin/x64/version.dll` for CET, `bin/x64/winmm.dll` for
  RED4ext). On first start the plugin offers to unfold old `root/` folders.

## Executables the plugin registers

| Title | Command | Use |
|---|---|---|
| `Cyberpunk 2077 (REDmod)` | `Cyberpunk2077.exe --launcher-skip -modded` | Default. Deploys REDmods before launch (`auto_deploy_redmod`). |
| `Cyberpunk 2077` | `Cyberpunk2077.exe --launcher-skip` | No REDmod. |
| `REDmod` | `tools/redmod/bin/redMod.exe deploy -reportProgress -force %modlist%` | Manual deployment. |
| `REDprelauncher` | `REDprelauncher.exe` | Vanilla launcher. |

Titles are stored in `ModOrganizer.ini` (`[customExecutables]`) when the instance is
created, so the build author's ini decides what exists. The launcher starts
`ModOrganizer.exe -p <profile> run -e <title>` and picks the REDmod entry when the
manifest has `redmod: true` (see `crates/core/src/mo2.rs`).

REDmod needs the free REDmod DLC (`<game>/tools/redmod/bin/redMod.exe`). Without it
the plugin aborts the launch when REDmods are active.

## Plugin settings (`ModOrganizer.ini`, `[Plugins]`)

| Setting | Default | Meaning |
|---|---|---|
| `enforce_archive_load_order` | false | Write `archive/pc/mod/modlist.txt` from MO2 order before launch. Off: archives load alphabetically like vanilla. |
| `reverse_archive_load_order` | false | In that file, highest MO2 priority loads first (wins). |
| `enforce_redmod_load_order` | true | Pass MO2 order to REDmod deploy (`-modlist=`). |
| `reverse_redmod_load_order` | false | Same reversal for REDmod. |
| `auto_deploy_redmod` | true | Deploy REDmods when launching with `-modded`. |
| `clear_cache_after_game_update` | true | Clear `overwrite/r6/cache/*` when game cache files changed (game update). |
| `disable_crashreporter` | true | Create mod `disable CrashReporter (MO CET fix)` with an empty `bin/x64/CrashReporter/CrashReporter.exe` (CrashReporter crashes with VFS `version.dll`). Sets itself to false after creating it. |
| `crash_message` | true | Message box after a crash (exit code > 0). |
| `show_rootbuilder_conversion` | true | Offer to unfold RootBuilder `root/` folders. |
| `skipStartScreen` | false | Adds `-skipStartScreen`. |

These ship with the base package (they live in `ModOrganizer.ini`).

## Where files go at runtime

USVFS rule: a **new** file goes to `overwrite/`; an **existing** file (found in some
mod) is changed in place in that mod's folder.

| File | Written by | Ship? |
|---|---|---|
| `r6/cache/final.redscripts`, `final.redscripts.bk` | redscript compiler | **Never** — compiled for the author's game version |
| `r6/cache/modded/*`, `r6/cache/modded/MO_REDmod_load_order.txt` | REDmod deploy, plugin | **Never** |
| `r6/cache/*` (copies of game caches, e.g. `tweakdb.bin`) | plugin (`_map_cache_files`) before each launch | **Never** |
| `archive/pc/mod/modlist.txt` | plugin, if `enforce_archive_load_order` | **Never** — regenerated before each launch |
| `r6/logs/redscript_rCURRENT.log` | redscript | Never (log) |
| `red4ext/logs/*.log` (`red4ext-<ts>.log`, plugin logs) | RED4ext and its plugins | Never (log) |
| `red4ext/plugins/TweakXL/TweakXL.log`, `.../ArchiveXL/ArchiveXL.log` | TweakXL, ArchiveXL | Never (log) |
| `bin/x64/plugins/cyber_engine_tweaks/{cyber_engine_tweaks,scripting,gamelog}.log` | CET | Never (log) |
| `bin/x64/plugins/cyber_engine_tweaks/mods/<mod>/<mod>.log` | CET mods | Never (log) |
| `tools/redmod/bin/REDmodLog.txt` | REDmod deploy | Never (log) |
| `bin/x64/plugins/AdvancedCrashReporter/{reports/**,*.cache,mod-inventory.txt}` | Advanced Crash Reporter: crash reports, `watch-NN.txt` snapshots, engine symbol caches, list of installed mods | Never — rewritten on every launch |
| `bin/x64/plugins/cyber_engine_tweaks/{config,persistent,bindings}.json`, `layout.ini` | CET: settings, overlay hotkey, window layout | Yes — build settings |
| `bin/x64/plugins/cyber_engine_tweaks/mods/<mod>/db.sqlite3`, `*.json` | CET mods (AMM favorites, mod settings) | Yes |
| `red4ext/plugins/mod_settings/user.ini` | Mod Settings | Yes |
| `red4ext/config.ini` | RED4ext config (when present) | Yes |
| `red4ext/plugins/Codeware/Persistent/ScriptableServiceContainer.dat` | Codeware: persistent fields of mods' scriptable services, rewritten on every launch | Yes — settings (`is_settings`) |

`crates/core/src/rules.rs` encodes the "never" rows (`is_generated`). When a new
framework or a new generated file shows up, add it there with a test.

Game settings are outside MO2: `%LOCALAPPDATA%\CD Projekt Red\Cyberpunk 2077\UserSettings.json`
(graphics, keybinds; the plugin declares it as the game's ini file). Saves:
`%USERPROFILE%\Saved Games\CD Projekt Red\Cyberpunk 2077`. With profile-local
settings/saves enabled, MO2 keeps copies in `profiles/<profile>/` — the build never
ships them (`is_private_profile_file`).

## Mod types the plugin recognizes

Valid top-level folders in a mod: `archive`, `bin`, `engine`, `r6`, `red4ext`, `mods`.

- `.archive` / ArchiveXL `.xl` → `archive/pc/mod/` (loose ones are moved there on install)
- redscript → `r6/scripts/**/*.reds`
- TweakXL → `r6/tweaks/**`
- RED4ext plugins → `red4ext/plugins/*`
- CET mods → `bin/x64/plugins/cyber_engine_tweaks/mods/*`
- REDmod → `mods/<name>/info.json`

## Game updates

- Install game updates with MO2 closed, then restart MO2.
- The plugin clears `overwrite/r6/cache/*` if game cache files changed.
- CET, RED4ext, redscript, TweakXL, ArchiveXL, Codeware usually need updates for a
  new game version; until then the build breaks. `Manifest.game_version` records the
  version the build was made for; the launcher does not check it yet.

## Troubleshooting order (from the plugin's crash message and the wiki)

1. Delete `overwrite/r6/cache` (keeps mod settings) or clear `overwrite/`.
2. Check CET / redscript / RED4ext logs (in `overwrite/`).
3. Disable mods (new profile) to bisect.
4. Verify game files; make sure no manual mod installs are left in the game folder.

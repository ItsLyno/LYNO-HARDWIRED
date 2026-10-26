//! What went wrong in the last game run, read from the frameworks' logs, so a
//! player sees "redscript failed on mod X" instead of a game that quietly
//! closed.
//!
//! Under MO2 the logs land in `overwrite/` (see [`crate::report`]). Only logs
//! written since the launch count: RED4ext starts a new file per run, the
//! others are rewritten or appended at start. Errors name game paths
//! (`...\r6\scripts\Mod\file.reds:12:3`); the mod they belong to is the
//! enabled one with that file, highest priority first, as MO2's VFS resolves it.

use std::collections::HashMap;
use std::time::SystemTime;

use serde::Serialize;

use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::report;

/// Folders under which a path in a log points at a mod's file.
const ROOTS: &[&str] = &["r6/scripts/", "r6/tweaks/", "archive/pc/mod/", "bin/x64/plugins/cyber_engine_tweaks/mods/", "red4ext/plugins/"];
const CET: &str = "bin/x64/plugins/cyber_engine_tweaks/";
const SAMPLES: usize = 3;
const MAX_PROBLEMS: usize = 20;
/// An entry's own lines after the first: redscript puts the source line and the message there.
const ENTRY_LINES: usize = 6;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunReport {
    /// redscript didn't compile: the game ran without any script mod, or didn't start.
    pub scripts_failed: bool,
    pub problems: Vec<Problem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Warning,
    Error,
}

/// Errors of one log about one mod.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    pub severity: Severity,
    /// The first entry without timestamps and the game folder: what a player can read at a glance.
    pub title: String,
    /// What it usually means and what to do, when the message is a known one (Russian, shown as is).
    pub hint: Option<String>,
    /// Who wrote the log: "redscript", "RED4ext", "CET", a RED4ext plugin (TweakXL, ArchiveXL…) or a CET mod.
    pub source: String,
    /// Relative to `overwrite/`, as [`report::read_overwrite_log`] takes it.
    pub log: String,
    /// The mod (folder under `mods/`) the errors point at.
    pub mod_folder: Option<String>,
    /// The first error entries.
    pub samples: Vec<String>,
    pub count: usize,
    /// Same for the same error in another run (no timestamps): what the launcher remembers as "seen, don't show".
    pub key: String,
}

impl RunReport {
    pub fn is_empty(&self) -> bool {
        !self.scripts_failed && self.problems.is_empty()
    }
}

/// Errors in logs written since `since` (the launch).
pub fn last_run(inst: &Instance, profile: &str, since: SystemTime) -> RunReport {
    let since_ms = since.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
    let list = ModList::load(&inst.modlist_path(profile)).unwrap_or_default();
    // Highest priority first: the first mod with a file is the one the game loaded it from.
    let enabled: Vec<String> = list.entries.iter().rev().filter(|e| e.state == EntryState::Enabled && !e.is_separator()).map(|e| e.name.clone()).collect();
    let mut owner = Owners { inst, enabled: &enabled, cache: HashMap::new() };

    let mut scripts_failed = false;
    let mut problems: Vec<Problem> = Vec::new();
    for log in report::overwrite_logs(inst).into_iter().filter(|l| l.modified >= since_ms) {
        let Ok(Some(text)) = report::read_overwrite_log(inst, &log.path) else { continue };
        let source = source_of(&log.path);
        let redscript = source == "redscript";
        let log_owner = owner.of(&log.path);
        for (severity, entry) in entries(&text) {
            let lower = entry.to_ascii_lowercase();
            // Warnings only where they are known to mean a mod didn't work (see `is_notable_warning`).
            if severity == Severity::Warning && !is_notable_warning(&source, &lower) {
                continue;
            }
            if redscript && (lower.contains("compilation has failed") || lower.contains("compilation failed")) {
                scripts_failed = true;
            }
            let mod_folder = game_paths(&entry)
                .iter()
                .find_map(|p| owner.of(p))
                .or_else(|| red4ext_plugin(&source, &entry).and_then(|n| owner.of(&format!("red4ext/plugins/{n}"))))
                .or_else(|| log_owner.clone());
            match problems.iter_mut().find(|p| p.log == log.path && p.mod_folder == mod_folder && p.severity == severity) {
                Some(p) => {
                    p.count += 1;
                    if p.samples.len() < SAMPLES {
                        p.samples.push(entry);
                    }
                }
                None => problems.push(Problem {
                    severity,
                    title: title_of(&entry),
                    hint: hint(&lower),
                    key: format!("{source}|{}|{}", mod_folder.as_deref().unwrap_or(""), strip_stamps(&entry)),
                    source: source.clone(),
                    log: log.path.clone(),
                    mod_folder,
                    samples: vec![entry],
                    count: 1,
                }),
            }
        }
    }
    // The failed compile first: it explains the rest; then the noisiest.
    problems.sort_by(|a, b| {
        let key = |p: &Problem| (!(scripts_failed && p.source == "redscript"), p.severity == Severity::Warning, std::cmp::Reverse(p.count));
        key(a).cmp(&key(b)).then_with(|| a.log.cmp(&b.log))
    });
    problems.truncate(MAX_PROBLEMS);
    RunReport { scripts_failed, problems }
}

/// The framework a log belongs to, by where it lies.
fn source_of(log: &str) -> String {
    let lower = log.to_ascii_lowercase();
    let first = |rest: &str| rest.split('/').next().unwrap_or(rest).to_owned();
    if lower.starts_with("r6/logs/redscript") {
        "redscript".into()
    } else if lower.starts_with("red4ext/logs/") {
        "RED4ext".into()
    } else if let Some(rest) = lower.strip_prefix("red4ext/plugins/") {
        first(&log[log.len() - rest.len()..])
    } else if let Some(rest) = lower.strip_prefix(&format!("{CET}mods/")) {
        format!("CET: {}", first(&log[log.len() - rest.len()..]))
    } else if lower.starts_with(CET) {
        "CET".into()
    } else {
        log.rsplit('/').next().unwrap_or(log).trim_end_matches(".log").to_owned()
    }
}

/// Entries of a log with an error level (spdlog's `[error]` / `[critical]`, redscript's `[ERROR - …]`) or a
/// warning level, and the lines after one up to the next `[`-line. RED4ext pads the level (`[error   ]`) and
/// puts a thread id after it, so the level is looked for in every leading bracket, not at a fixed place.
fn entries(text: &str) -> Vec<(Severity, String)> {
    let mut out = Vec::new();
    let mut current: Option<(Severity, Vec<&str>)> = None;
    for line in text.lines() {
        if line.starts_with('[') {
            out.extend(current.take().map(|(s, l)| (s, l.join("\n"))));
            current = level(line).map(|s| (s, vec![line]));
        } else if let Some((_, lines)) = current.as_mut().filter(|(_, l)| l.len() < ENTRY_LINES && !line.trim().is_empty()) {
            lines.push(line);
        }
    }
    out.extend(current.map(|(s, l)| (s, l.join("\n"))));
    out
}

fn level(line: &str) -> Option<Severity> {
    let mut rest = line;
    while let Some((group, after)) = rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
        let word = group.trim().split([' ', '-']).next().unwrap_or("").to_ascii_lowercase();
        match word.as_str() {
            "error" | "critical" | "fatal" => return Some(Severity::Error),
            "warn" | "warning" => return Some(Severity::Warning),
            _ => {}
        }
        rest = after.trim_start();
    }
    None
}

/// Warnings that mean a mod didn't work, not noise: RED4ext failing to load or start a plugin, redscript
/// finding the same definition twice (a mod installed twice). Texts are from RED4ext's `PluginSystem.cpp`.
fn is_notable_warning(source: &str, lower: &str) -> bool {
    match source {
        "RED4ext" => [
            "could not load plugin",
            "incompatible with the current patch",
            "is not supported by red4ext",
            "did not initialize properly",
            "unsupported api version",
            "exception",
        ]
        .iter()
        .any(|m| lower.contains(m)),
        "redscript" => lower.contains("already defined"),
        _ => false,
    }
}

/// The plugin a RED4ext message talks about, when it has no path: `Name (version: …) is incompatible…`,
/// `Could not load plugin 'Name'…`.
fn red4ext_plugin(source: &str, entry: &str) -> Option<String> {
    if source != "RED4ext" {
        return None;
    }
    let msg = entry.lines().next()?.rsplit_once("] ").map_or(entry, |(_, m)| m);
    let name = match msg.split_once("plugin '") {
        Some((_, rest)) => rest.split('\'').next()?,
        None => msg.split(" (version").next().filter(|_| msg.contains(" (version"))?,
    };
    Some(name.trim().to_owned()).filter(|n| !n.is_empty() && !n.contains('/'))
}

/// What a known message means for the player.
fn hint(lower: &str) -> Option<String> {
    let rules: &[(&[&str], &str)] = &[
        (&["could not load plugin"], "Плагин RED4ext не загрузился. Обычно не хватает Microsoft Visual C++ Redistributable или файл мода повреждён."),
        (&["incompatible with the current patch"], "Плагин не поддерживает эту версию игры. Нужна новая версия мода, пока его можно выключить."),
        (&["is not supported by red4ext", "unsupported api version"], "Плагин собран для другой версии RED4ext. Обновите мод или RED4ext."),
        (&["did not initialize properly"], "Плагин не запустился и выгружен: его функции в игре не работают."),
        (&["already defined"], "Одно и то же определено дважды: мод установлен два раза или два мода правят одно и то же."),
        (&["compilation has failed", "compilation failed"], "Итог сборки скриптов, причина — в ошибках выше."),
        (&["unresolved", "cannot find", "can't find"], "Скрипт обращается к тому, чего нет. Чаще всего не хватает мода-зависимости или мод устарел после обновления игры."),
        (&["attempt to"], "Ошибка в Lua-скрипте мода CET. Игру не ломает, но мод может не работать."),
    ];
    rules.iter().find(|(m, _)| m.iter().any(|m| lower.contains(m))).map(|(_, h)| (*h).to_owned())
}

/// Game-relative paths under [`ROOTS`] in an entry, with `/`. A path ends where a line number
/// (`:12`), a quote or a bracket starts; mod folders have spaces, so a space doesn't end it.
fn game_paths(entry: &str) -> Vec<String> {
    let norm = entry.replace('\\', "/");
    let lower = norm.to_ascii_lowercase();
    let mut out = Vec::new();
    for root in ROOTS {
        let mut from = 0;
        while let Some(at) = lower[from..].find(root).map(|i| i + from) {
            let rest = &norm[at..];
            let end = rest.find([':', '"', '\'', '<', '>', '|', '\n', '(', ')', '[', ']']).unwrap_or(rest.len());
            let path = rest[..end].trim_end_matches([' ', '.', ',']);
            if path.len() > root.len() && !path[root.len()..].starts_with(' ') {
                out.push(path.to_owned());
            }
            from = at + root.len();
        }
    }
    out
}

/// An entry without its leading `[time] [name] [level]` brackets: the same error in another run.
fn strip_stamps(entry: &str) -> String {
    let first = entry.lines().next().unwrap_or("");
    let mut rest = first.trim_start();
    while let Some(after) = rest.strip_prefix('[').and_then(|r| r.split_once(']')).map(|(_, a)| a.trim_start()) {
        rest = after;
    }
    // A path's line numbers stay; the absolute game folder before it doesn't.
    let norm = rest.replace('\\', "/");
    match ROOTS.iter().find_map(|r| norm.to_ascii_lowercase().find(r)) {
        Some(at) => norm[at..].to_owned(),
        None => norm,
    }
}

/// The entry as a player reads it: no timestamps, the lines after the first kept (redscript's source line and message).
fn title_of(entry: &str) -> String {
    let rest: Vec<&str> = entry.lines().skip(1).map(str::trim_end).collect();
    std::iter::once(strip_stamps(entry)).chain(rest.into_iter().map(str::to_owned)).collect::<Vec<_>>().join("\n")
}

/// Which enabled mod a game path comes from: the path itself, or its folders up to the one
/// right under its root (a CET mod's or a RED4ext plugin's folder, a script mod's folder).
struct Owners<'a> {
    inst: &'a Instance,
    enabled: &'a [String],
    cache: HashMap<String, Option<String>>,
}

impl Owners<'_> {
    fn of(&mut self, path: &str) -> Option<String> {
        if let Some(hit) = self.cache.get(path) {
            return hit.clone();
        }
        let hit = self.find(path);
        self.cache.insert(path.to_owned(), hit.clone());
        hit
    }

    fn find(&self, path: &str) -> Option<String> {
        let lower = path.to_ascii_lowercase();
        let root = ROOTS.iter().find(|r| lower.starts_with(*r))?;
        let mods = self.inst.mods_dir();
        let mut candidate = path;
        while candidate.len() > root.len() {
            if let Some(m) = self.enabled.iter().find(|m| crate::tree::from_slash(&mods.join(m), candidate).exists()) {
                return Some(m.clone());
            }
            candidate = &candidate[..candidate.rfind('/')?];
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modlist::Entry;
    use std::path::Path;

    fn write(root: &Path, rel: &str, data: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[test]
    fn finds_errors_and_their_mods() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let inst = Instance::new(root);
        write(root, "mods/Old Scripts/r6/scripts/Shared/a.reds", "");
        write(root, "mods/Better Scripts/r6/scripts/Shared/a.reds", "");
        write(root, "mods/Off Mod/r6/scripts/Off/b.reds", "");
        write(root, "mods/ArchiveXL/red4ext/plugins/ArchiveXL/ArchiveXL.dll", "");
        write(root, "mods/Clothes/archive/pc/mod/clothes.archive", "");
        write(root, "mods/Hud Mod/bin/x64/plugins/cyber_engine_tweaks/mods/hud/init.lua", "");
        let list = ModList {
            entries: vec![
                Entry::enabled("Old Scripts"),
                Entry::enabled("Better Scripts"),
                Entry::disabled("Off Mod"),
                Entry::enabled("ArchiveXL"),
                Entry::enabled("Clothes"),
                Entry::enabled("Hud Mod"),
            ],
        };
        std::fs::create_dir_all(inst.profile_dir("LYNO")).unwrap();
        list.save(&inst.modlist_path("LYNO")).unwrap();

        write(
            root,
            "overwrite/r6/logs/redscript_rCURRENT.log",
            "[INFO - Mon, 1 Jan 2026 10:00:00 +0000] Compiling...\n\
             [ERROR - Mon, 1 Jan 2026 10:00:01 +0000] At C:\\Games\\Cyberpunk 2077\\r6\\scripts\\Shared\\a.reds:12:5:\n\
             \x20   let x = Foo();\n\
             \x20 unresolved function Foo\n\
             [ERROR - Mon, 1 Jan 2026 10:00:01 +0000] REDscript compilation has failed.\n",
        );
        write(
            root,
            "overwrite/red4ext/plugins/ArchiveXL/ArchiveXL.log",
            "[2026-01-01 10:00:02.000] [ArchiveXL] [info] ok\n\
             [2026-01-01 10:00:02.100] [ArchiveXL] [error] Can't load C:\\Games\\Cyberpunk 2077\\archive\\pc\\mod\\clothes.archive (bad header)\n\
             [2026-01-01 10:00:02.200] [ArchiveXL] [error] Something without a path\n",
        );
        write(root, "overwrite/bin/x64/plugins/cyber_engine_tweaks/mods/hud/hud.log", "[10:00:03] [hud] [error] init.lua:4: attempt to index nil\n");
        write(root, "overwrite/red4ext/logs/red4ext.log", "[2026-01-01] [RED4ext] [warning] just a warning\n");

        let report = last_run(&inst, "LYNO", SystemTime::UNIX_EPOCH);
        assert!(report.scripts_failed);
        let got: Vec<_> = report.problems.iter().map(|p| (p.source.as_str(), p.mod_folder.as_deref(), p.count)).collect();
        assert_eq!(
            got,
            [
                ("redscript", Some("Better Scripts"), 1),
                ("redscript", None, 1),
                ("CET: hud", Some("Hud Mod"), 1),
                ("ArchiveXL", Some("Clothes"), 1),
                ("ArchiveXL", Some("ArchiveXL"), 1),
            ]
        );
        assert!(report.problems[0].samples[0].contains("unresolved function Foo"), "{:?}", report.problems[0].samples);
        assert_eq!(report.problems[0].key, "redscript|Better Scripts|r6/scripts/Shared/a.reds:12:5:");

        // Logs from before the launch are another run's.
        let later = SystemTime::now() + std::time::Duration::from_secs(60);
        assert!(last_run(&inst, "LYNO", later).is_empty());
    }

    #[test]
    fn reads_red4ext_padded_levels_and_plugin_warnings() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let inst = Instance::new(root);
        write(root, "mods/Cool Plugin/red4ext/plugins/CoolPlugin/CoolPlugin.dll", "");
        let list = ModList { entries: vec![Entry::enabled("Cool Plugin")] };
        std::fs::create_dir_all(inst.profile_dir("LYNO")).unwrap();
        list.save(&inst.modlist_path("LYNO")).unwrap();
        write(
            root,
            "overwrite/red4ext/logs/red4ext.log",
            "[2026-01-01 10:00:00.000] [info    ] [  1234] [RED4ext] fine\n\
             [2026-01-01 10:00:00.100] [warning ] [  1234] [RED4ext] CoolPlugin (version: 1.0.0) is incompatible with the current patch. The requested runtime of the plugin is 2.1\n\
             [2026-01-01 10:00:00.200] [warning ] [  1234] [RED4ext] Other note\n\
             [2026-01-01 10:00:00.300] [error   ] [  1234] [RED4ext] boom\n",
        );
        let report = last_run(&inst, "LYNO", SystemTime::UNIX_EPOCH);
        let got: Vec<_> = report.problems.iter().map(|p| (p.severity, p.mod_folder.as_deref(), p.hint.is_some())).collect();
        assert_eq!(got, [(Severity::Error, None, false), (Severity::Warning, Some("Cool Plugin"), true)]);
        assert!(report.problems[1].title.starts_with("CoolPlugin (version"), "{}", report.problems[1].title);
    }

    #[test]
    fn names_sources() {
        assert_eq!(source_of("r6/logs/redscript_rCURRENT.log"), "redscript");
        assert_eq!(source_of("red4ext/logs/red4ext-2026.log"), "RED4ext");
        assert_eq!(source_of("red4ext/plugins/TweakXL/TweakXL.log"), "TweakXL");
        assert_eq!(source_of("bin/x64/plugins/cyber_engine_tweaks/cyber_engine_tweaks.log"), "CET");
        assert_eq!(source_of("bin/x64/plugins/cyber_engine_tweaks/mods/Hud/Hud.log"), "CET: Hud");
        assert_eq!(source_of("other/thing.log"), "thing");
    }

    #[test]
    fn pulls_game_paths_from_entries() {
        assert_eq!(
            game_paths("At D:\\Cyberpunk 2077\\r6\\scripts\\My Mod\\x.reds:3:1: oops"),
            ["r6/scripts/My Mod/x.reds"]
        );
        assert_eq!(game_paths("loading \"archive/pc/mod/a.archive\" and r6\\tweaks\\b.yaml."), ["r6/tweaks/b.yaml", "archive/pc/mod/a.archive"]);
        assert!(game_paths("r6/scripts/ alone").is_empty());
    }
}

//! `lyno-pack build` turns the author's MO2 instance into a release:
//! `out/manifest.json` plus `tar.zst` parts to upload to GitHub Releases.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use lyno_core::manifest::{ChangelogEntry, Manifest};
use lyno_core::meta::ModMeta;
use lyno_core::package::PackOptions;
use lyno_core::publish::{build, BuildOptions, ModInfo};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Pack an MO2 instance into a build release.
    Build {
        /// MO2 instance root (folder with ModOrganizer.exe).
        instance: PathBuf,
        #[arg(short, long, default_value = "LYNO")]
        profile: String,
        /// Build version, e.g. 1.4.0. The release tag is `build-<version>`.
        #[arg(long = "version", id = "build_version")]
        build_version: String,
        /// Required Cyberpunk 2077 version, e.g. 2.31.
        #[arg(long)]
        game_version: String,
        #[arg(long, default_value = "2.5.2")]
        mo2_version: String,
        /// GitHub repository that hosts the releases.
        #[arg(long, default_value = "ItsLyno/LYNO-HARDWIRED")]
        repo: String,
        /// Previous manifest (usually build/manifest.json): unchanged mods are not re-uploaded.
        #[arg(long)]
        previous: Option<PathBuf>,
        /// Changelog line for this version; repeat for several.
        #[arg(short, long)]
        note: Vec<String>,
        /// Nexus API key to fill in mod authors (or NEXUS_API_KEY).
        #[arg(long, env = "NEXUS_API_KEY", hide_env_values = true)]
        nexus_key: Option<String>,
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        #[arg(long, default_value_t = 6)]
        zstd_level: i32,
    },
}

fn main() -> ExitCode {
    let Command::Build {
        instance,
        profile,
        build_version,
        game_version,
        mo2_version,
        repo,
        previous,
        note,
        nexus_key,
        out,
        zstd_level,
    } = Cli::parse().command;

    let previous = match previous.map(|p| std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))) {
        Some(Ok(text)) => match Manifest::from_json(&text) {
            Ok(m) => Some(m),
            Err(e) => return fail(format!("previous manifest: {e}")),
        },
        Some(Err(e)) => return fail(e),
        None => None,
    };

    let mut changelog = Vec::new();
    if !note.is_empty() {
        changelog.push(ChangelogEntry { version: build_version.clone(), date: Some(today()), notes: note });
    }
    if let Some(prev) = &previous {
        changelog.extend(prev.changelog.iter().filter(|e| e.version != build_version).cloned());
    }

    let tag = format!("build-{build_version}");
    let opts = BuildOptions {
        name: "LYNO//HARDWIRED".into(),
        profile,
        build_version: build_version.clone(),
        game_version,
        mo2_version,
        base_url: format!("https://github.com/{repo}/releases/download/{tag}"),
        out_dir: out.clone(),
        previous,
        changelog,
        pack: PackOptions { zstd_level, ..Default::default() },
    };

    let mut nexus = Nexus::new(nexus_key);
    let result = build(&instance, &opts, &mut |meta| nexus.info(meta), &mut |msg| eprintln!("  {msg}"));
    let output = match result {
        Ok(o) => o,
        Err(e) => return fail(e.to_string()),
    };
    for w in &output.warnings {
        eprintln!("warning: {w}");
    }

    let manifest_path = out.join("manifest.json");
    let json = serde_json::to_string_pretty(&output.manifest).expect("manifest serializes");
    if let Err(e) = std::fs::write(&manifest_path, json + "\n") {
        return fail(format!("{}: {e}", manifest_path.display()));
    }

    let upload: u64 = output.assets.iter().map(|a| a.size).sum();
    eprintln!();
    eprintln!(
        "{} mods, {} new assets ({:.1} GB to upload)",
        output.manifest.mod_specs().count(),
        output.assets.len(),
        upload as f64 / 1e9
    );
    eprintln!();
    eprintln!("Next steps:");
    if output.assets.is_empty() {
        eprintln!("  no new assets: only the manifest changed");
    } else {
        eprintln!("  gh release create {tag} --repo {repo} --title \"Build {build_version}\" --notes \"\"");
        eprintln!("  gh release upload {tag} --repo {repo} {}/*.tar.zst.*", out.display());
    }
    eprintln!("  copy {} to build/manifest.json, commit and push to main", manifest_path.display());
    ExitCode::SUCCESS
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::FAILURE
}

/// Looks up mod authors and titles on Nexus (`/v1/games/{game}/mods/{id}.json`).
struct Nexus {
    key: Option<String>,
    agent: ureq::Agent,
    cache: HashMap<(String, u64), ModInfo>,
}

impl Nexus {
    fn new(key: Option<String>) -> Self {
        if key.is_none() {
            eprintln!("note: no Nexus API key, mod authors will be empty (pass --nexus-key or NEXUS_API_KEY)");
        }
        Self { key, agent: ureq::Agent::new_with_defaults(), cache: HashMap::new() }
    }

    fn info(&mut self, meta: &ModMeta) -> ModInfo {
        let (Some(key), Some(mod_id)) = (&self.key, meta.mod_id) else { return ModInfo::default() };
        let game = meta.game_name.clone().unwrap_or_else(|| "cyberpunk2077".into()).to_lowercase();
        if let Some(hit) = self.cache.get(&(game.clone(), mod_id)) {
            return hit.clone();
        }
        let url = format!("https://api.nexusmods.com/v1/games/{game}/mods/{mod_id}.json");
        let info = match self.agent.get(&url).header("apikey", key).call() {
            Ok(mut resp) => match resp.body_mut().read_json::<serde_json::Value>() {
                Ok(v) => ModInfo {
                    author: v["author"].as_str().filter(|s| !s.is_empty()).map(Into::into),
                    title: v["name"].as_str().filter(|s| !s.is_empty()).map(Into::into),
                },
                Err(e) => {
                    eprintln!("warning: Nexus {mod_id}: {e}");
                    ModInfo::default()
                }
            },
            Err(e) => {
                eprintln!("warning: Nexus {mod_id}: {e}");
                ModInfo::default()
            }
        };
        self.cache.insert((game, mod_id), info.clone());
        info
    }
}

/// UTC date as YYYY-MM-DD (days-to-civil, Howard Hinnant).
fn today() -> String {
    let days = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

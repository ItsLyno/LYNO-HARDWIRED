//! `lyno-pack build` turns the author's MO2 instance into a release:
//! `out/manifest.json` plus `tar.zst` parts to upload to GitHub Releases.
//! `lyno-pack publish` uploads them and pushes the manifest.

mod publish;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use clap::{Parser, Subcommand};
use lyno_core::manifest::{ChangelogEntry, Manifest};
use lyno_core::meta::ModMeta;
use lyno_core::package::PackOptions;
use lyno_core::publish::{build, BuildOptions, ModInfo};
use lyno_core::release::{github_download_root, release_tag, PUBLISHED_MANIFEST};

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
        /// Previous manifest: unchanged mods are not re-uploaded. By default the
        /// manifest published in the repository (`build/manifest.json` on `main`).
        #[arg(long, conflicts_with = "fresh")]
        previous: Option<PathBuf>,
        /// Ignore the published manifest and pack and upload everything again.
        #[arg(long)]
        fresh: bool,
        /// Read every file instead of trusting cached hashes of unchanged files
        /// (`<instance>/.lyno/pack-cache.json`).
        #[arg(long)]
        no_cache: bool,
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
    /// Upload the assets from `build` and push the manifest to players.
    ///
    /// Run inside the repository clone, on an up-to-date main. Safe to re-run
    /// after a failure: uploaded assets are skipped.
    Publish {
        /// Output folder of `lyno-pack build`.
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        #[arg(long, default_value = "ItsLyno/LYNO-HARDWIRED")]
        repo: String,
        /// Don't ask before pushing the manifest.
        #[arg(short, long)]
        yes: bool,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Publish { out, repo, yes } => match publish::run(&publish::PublishArgs { out, repo, yes }) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => fail(e),
        },
        build @ Command::Build { .. } => build_cmd(build),
    }
}

fn build_cmd(command: Command) -> ExitCode {
    let Command::Build {
        instance,
        profile,
        build_version,
        game_version,
        mo2_version,
        repo,
        previous,
        fresh,
        no_cache,
        note,
        nexus_key,
        out,
        zstd_level,
    } = command
    else {
        unreachable!()
    };

    let previous = match load_previous(previous, fresh, &repo) {
        Ok(p) => p,
        Err(e) => return fail(e),
    };
    if previous.as_ref().is_some_and(|m| m.build_version == build_version) {
        return fail(format!("version {build_version} is already published, pass a new --version"));
    }
    let started = std::time::Instant::now();

    let mut changelog = Vec::new();
    if !note.is_empty() {
        changelog.push(ChangelogEntry { version: build_version.clone(), date: Some(today()), notes: note });
    }
    if let Some(prev) = &previous {
        changelog.extend(prev.changelog.iter().filter(|e| e.version != build_version).cloned());
    }

    let opts = BuildOptions {
        name: "LYNO//HARDWIRED".into(),
        profile,
        build_version: build_version.clone(),
        game_version,
        mo2_version,
        base_url: format!("{}/{}", github_download_root(&repo), release_tag(&build_version)),
        out_dir: out.clone(),
        previous,
        changelog,
        pack: PackOptions { zstd_level, ..Default::default() },
        hash_cache: !no_cache,
    };

    let mut nexus = Nexus::new(nexus_key);
    // Timestamps show where a slow build spends its time (disk, antivirus, Nexus).
    let result =
        build(&instance, &opts, &mut |meta| nexus.info(meta), &mut |msg| eprintln!("  {} {msg}", elapsed(started.elapsed())));
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
    if !output.repacked.is_empty() {
        eprintln!();
        eprintln!("To upload:");
        for r in &output.repacked {
            let size: u64 = output
                .assets
                .iter()
                .filter(|a| a.file_name.starts_with(&asset_prefix(&output.manifest, &r.name)))
                .map(|a| a.size)
                .sum();
            let why = if r.changed { "changed" } else { "new" };
            eprintln!("  {:<40} {why:<8} {:>8.1} MB", r.name, size as f64 / 1e6);
        }
    }
    eprintln!();
    eprintln!(
        "{} mods ({} core), {} new assets ({:.1} GB to upload), took {}",
        output.manifest.mod_specs().count(),
        output.manifest.mod_specs().filter(|m| !m.optional).count(),
        output.assets.len(),
        upload as f64 / 1e9,
        elapsed(started.elapsed())
    );
    eprintln!();
    eprintln!("Next step: upload and push the manifest (in the repository clone, on an up-to-date main):");
    eprintln!("  lyno-pack publish --out {} --repo {repo}", out.display());
    ExitCode::SUCCESS
}

/// `--previous` file, else the manifest published on `main`, else nothing
/// (first release, or `--fresh`).
fn load_previous(path: Option<PathBuf>, fresh: bool, repo: &str) -> Result<Option<Manifest>, String> {
    if fresh {
        eprintln!("note: --fresh, everything is packed and uploaded again");
        return Ok(None);
    }
    let (text, source) = match path {
        Some(p) => (std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?, p.display().to_string()),
        None => {
            let url = format!("https://raw.githubusercontent.com/{repo}/main/{PUBLISHED_MANIFEST}");
            match ureq::get(&url).call() {
                Ok(mut resp) => (resp.body_mut().read_to_string().map_err(|e| format!("{url}: {e}"))?, url),
                Err(ureq::Error::StatusCode(404)) => {
                    eprintln!("note: no published manifest yet ({url}), packing everything");
                    return Ok(None);
                }
                // Never fall back to a full re-upload silently.
                Err(e) => return Err(format!("{url}: {e} (pass --previous <file>, or --fresh to upload everything)")),
            }
        }
    };
    let m = Manifest::from_json(&text).map_err(|e| format!("previous manifest {source}: {e}"))?;
    eprintln!("previous: build {} ({source})", m.build_version);
    Ok(Some(m))
}

fn elapsed(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// Asset file names are `<package id>-<first 16 hash chars>.tar.zst.NNN`.
fn asset_prefix(manifest: &Manifest, name: &str) -> String {
    let (id, pkg) = manifest
        .mod_specs()
        .find(|m| m.name == name)
        .map_or(("base", &manifest.base), |m| (m.id.as_str(), &m.package));
    format!("{id}-{}.", &pkg.hash[..16])
}

fn fail(msg: impl std::fmt::Display) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::FAILURE
}

/// Looks up mod authors and titles on Nexus (`/v1/games/{game}/mods/{id}.json`).
struct Nexus {
    key: Option<String>,
    /// Lookups that failed in a row; the build gives up on Nexus after a few
    /// instead of waiting on an unreachable API for every mod.
    failures: u32,
    agent: ureq::Agent,
    cache: HashMap<(String, u64), ModInfo>,
}

impl Nexus {
    fn new(key: Option<String>) -> Self {
        if key.is_none() {
            eprintln!("note: no Nexus API key, mod authors will be empty (pass --nexus-key or NEXUS_API_KEY)");
        }
        // ureq has no overall timeout by default: one stalled request would hold the whole build.
        let agent = ureq::Agent::config_builder().timeout_global(Some(NEXUS_TIMEOUT)).build().new_agent();
        Self { key, failures: 0, agent, cache: HashMap::new() }
    }

    fn info(&mut self, meta: &ModMeta) -> ModInfo {
        if self.failures >= NEXUS_MAX_FAILURES {
            return ModInfo::default();
        }
        let (Some(key), Some(mod_id)) = (&self.key, meta.mod_id) else { return ModInfo::default() };
        let game = meta.game_name.clone().unwrap_or_else(|| "cyberpunk2077".into()).to_lowercase();
        if let Some(hit) = self.cache.get(&(game.clone(), mod_id)) {
            return hit.clone();
        }
        let url = format!("https://api.nexusmods.com/v1/games/{game}/mods/{mod_id}.json");
        let result = self
            .agent
            .get(&url)
            .header("apikey", key)
            .call()
            .and_then(|mut resp| resp.body_mut().read_json::<serde_json::Value>());
        let info = match result {
            Ok(v) => {
                self.failures = 0;
                ModInfo {
                    author: v["author"].as_str().filter(|s| !s.is_empty()).map(Into::into),
                    title: v["name"].as_str().filter(|s| !s.is_empty()).map(Into::into),
                }
            }
            Err(e) => {
                eprintln!("warning: Nexus {mod_id}: {e}");
                // A hidden or deleted mod answers 404: Nexus itself is fine.
                let reachable = matches!(e, ureq::Error::StatusCode(code) if code != 429);
                self.failures = if reachable { 0 } else { self.failures + 1 };
                if self.failures == NEXUS_MAX_FAILURES {
                    eprintln!("warning: Nexus failed {NEXUS_MAX_FAILURES} times in a row, authors of the remaining mods stay empty");
                }
                ModInfo::default()
            }
        };
        self.cache.insert((game, mod_id), info.clone());
        info
    }
}

const NEXUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);
const NEXUS_MAX_FAILURES: u32 = 3;

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

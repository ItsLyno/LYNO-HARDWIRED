//! `lyno-pack scan` reads the author's MO2 instance and prints a draft
//! manifest. Recipe generation (matching mod files to archive contents)
//! comes in a later stage; for now every Nexus mod is listed with its ids
//! and anything without Nexus ids is reported.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use lyno_core::manifest::{Manifest, ModEntry, ModSpec, NexusSource, SCHEMA_VERSION};
use lyno_core::mo2::Instance;
use lyno_core::modlist::{EntryState, ModList};

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a draft manifest for a profile of an MO2 instance.
    Scan {
        /// MO2 instance root (folder with ModOrganizer.exe or ModOrganizer.ini).
        instance: PathBuf,
        #[arg(short, long, default_value = "Default")]
        profile: String,
        #[arg(long, default_value = "0.1.0")]
        build_version: String,
        /// Required Cyberpunk2077.exe version.
        #[arg(long, default_value = "")]
        game_version: String,
        #[arg(short, long)]
        out: Option<PathBuf>,
    },
}

fn main() -> ExitCode {
    let Command::Scan { instance, profile, build_version, game_version, out } = Cli::parse().command;
    match scan(&instance, &profile, build_version, game_version) {
        Ok((manifest, problems)) => {
            for p in &problems {
                eprintln!("warning: {p}");
            }
            let json = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
            match out {
                Some(path) => {
                    if let Err(e) = std::fs::write(&path, json) {
                        eprintln!("error: {}: {e}", path.display());
                        return ExitCode::FAILURE;
                    }
                    eprintln!("wrote {} ({} mods, {} warnings)", path.display(), manifest.mod_specs().count(), problems.len());
                }
                None => println!("{json}"),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn scan(
    root: &std::path::Path,
    profile: &str,
    build_version: String,
    game_version: String,
) -> lyno_core::Result<(Manifest, Vec<String>)> {
    let inst = Instance::new(root);
    let list = ModList::load(&inst.modlist_path(profile))?;
    let metas = inst.scan_mods()?;
    let mut problems = Vec::new();
    let mut mods = Vec::new();

    for entry in &list.entries {
        if entry.state == EntryState::Unmanaged {
            continue;
        }
        if let Some(title) = entry.separator_title() {
            mods.push(ModEntry::Separator { title: title.to_owned() });
            continue;
        }
        let meta = metas.get(&entry.name).cloned().unwrap_or_default();
        let (Some(mod_id), Some(file_id)) = (meta.mod_id, meta.file_id) else {
            problems.push(format!("{:?}: no Nexus mod/file id in meta.ini, skipped", entry.name));
            continue;
        };
        let file_name = meta.installation_file.clone().unwrap_or_default();
        let size = std::fs::metadata(inst.downloads_dir().join(&file_name)).map(|m| m.len()).unwrap_or(0);
        if size == 0 {
            problems.push(format!("{:?}: archive {file_name:?} not found in downloads/", entry.name));
        }
        mods.push(ModEntry::Mod(ModSpec {
            id: slug(&entry.name),
            name: entry.name.clone(),
            enabled: entry.state == EntryState::Enabled,
            version: meta.version.clone(),
            author: None,
            nexus: NexusSource {
                game: meta.game_name.clone().unwrap_or_else(|| "cyberpunk2077".into()),
                mod_id,
                file_id,
                file_name,
                size,
                md5: None,
            },
            recipe: vec![],
        }));
    }

    let manifest = Manifest {
        schema: SCHEMA_VERSION,
        name: "LYNO//HARDWIRED".into(),
        build_version,
        game_version,
        mo2_version: "2.5.3".into(),
        profile: profile.to_owned(),
        changelog: vec![],
        mods,
        own_files: vec![],
    };
    manifest.validate()?;
    Ok((manifest, problems))
}

fn slug(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_owned()
}

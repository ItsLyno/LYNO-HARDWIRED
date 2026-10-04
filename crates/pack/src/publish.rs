//! `lyno-pack publish`: uploads the parts written by `build` and pushes the
//! manifest, in the only safe order. Launchers start downloading as soon as
//! `build/manifest.json` lands on `main`, so the manifest goes last and only
//! after every part it points at, old releases included, answers over HTTP.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lyno_core::download::Downloader;
use lyno_core::manifest::Manifest;
use lyno_core::publish::check_published;

use crate::PUBLISHED_MANIFEST;

pub struct PublishArgs {
    pub out: PathBuf,
    pub repo: String,
    pub yes: bool,
}

const BRANCH: &str = "main";

pub fn run(args: &PublishArgs) -> Result<(), String> {
    let manifest_path = args.out.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path)
        .map_err(|e| format!("{}: {e} (run `lyno-pack build` first)", manifest_path.display()))?;
    let manifest = Manifest::from_json(&text).map_err(|e| format!("{}: {e}", manifest_path.display()))?;
    let version = &manifest.build_version;
    let tag = format!("build-{version}");

    let assets = new_assets(&manifest, &args.out, &args.repo, &tag)?;
    let root = git_preflight(&args.repo, version, text.as_bytes())?;
    tool_check("gh", &["--version"])?;

    if assets.is_empty() {
        eprintln!("no new assets: only the manifest changes");
    } else {
        upload(&args.repo, &tag, &manifest, &args.out, &assets)?;
    }

    eprintln!("checking every part of build {version} over HTTP...");
    let errors = check_published(&manifest, &Downloader::new(), &|done, total| {
        if done == total || done % 20 == 0 {
            eprintln!("  {done}/{total}");
        }
    });
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("  broken: {e}");
        }
        return Err(format!(
            "{} part(s) are not downloadable, the manifest was not pushed. Re-run `lyno-pack publish` \
             to upload missing assets; a part in an older release means that release was changed or deleted",
            errors.len()
        ));
    }

    if !args.yes && !confirm(&format!("Publish build {version} to players (push {PUBLISHED_MANIFEST} to {BRANCH})?"))? {
        return Err("cancelled, nothing was pushed".into());
    }

    let dest = root.join(PUBLISHED_MANIFEST);
    if let Some(dir) = dest.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    std::fs::copy(&manifest_path, &dest).map_err(|e| format!("{}: {e}", dest.display()))?;
    git(&root, &["add", "--", PUBLISHED_MANIFEST])?;
    git(&root, &["commit", "-m", &format!("Build {version}"), "--", PUBLISHED_MANIFEST])?;
    if let Err(e) = git(&root, &["push", "origin", BRANCH]) {
        return Err(format!("{e}\nThe commit is local; push it with `git push origin {BRANCH}`"));
    }
    eprintln!();
    eprintln!("build {version} is live; raw.githubusercontent.com may serve the old manifest for a few minutes");
    Ok(())
}

/// Parts that point at this version's release, i.e. the files `build` wrote.
/// Reused packages point at older releases and are only checked.
fn new_assets(manifest: &Manifest, out: &Path, repo: &str, tag: &str) -> Result<Vec<(String, u64)>, String> {
    let prefix = format!("https://github.com/{repo}/releases/download/{tag}/");
    let mut assets = Vec::new();
    for part in std::iter::once(&manifest.base).chain(manifest.mod_specs().map(|m| &m.package)).flat_map(|p| &p.parts) {
        if !part.url.starts_with("https://github.com/") {
            return Err(format!("{}: not a GitHub release asset", part.url));
        }
        let Some(name) = part.url.strip_prefix(&prefix) else { continue };
        let local = out.join(name);
        let size = std::fs::metadata(&local).map(|m| m.len()).map_err(|e| format!("{}: {e}", local.display()))?;
        if size != part.size {
            return Err(format!("{}: {size} bytes, the manifest says {} (rebuild)", local.display(), part.size));
        }
        assets.push((name.to_owned(), size));
    }
    Ok(assets)
}

/// The commit must go straight on top of what players see now: on `main`,
/// in sync with `origin/main`, with a pushable origin for `repo`.
fn git_preflight(repo: &str, version: &str, ours: &[u8]) -> Result<PathBuf, String> {
    tool_check("git", &["--version"])?;
    let root = PathBuf::from(git(Path::new("."), &["rev-parse", "--show-toplevel"]).map_err(|e| {
        format!("{e}\nRun `lyno-pack publish` inside the LYNO-HARDWIRED repository clone")
    })?);
    let origin = git(&root, &["remote", "get-url", "origin"])?;
    if !origin.to_lowercase().trim_end_matches(".git").ends_with(&repo.to_lowercase()) {
        return Err(format!("origin is {origin}, expected the {repo} repository"));
    }
    let branch = git(&root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    if branch != BRANCH {
        return Err(format!("on branch {branch}: switch to {BRANCH} (`git switch {BRANCH}`)"));
    }
    git(&root, &["fetch", "origin", BRANCH])?;
    let head = git(&root, &["rev-parse", "HEAD"])?;
    let remote = git(&root, &["rev-parse", &format!("origin/{BRANCH}")])?;
    if head != remote {
        return Err(format!("local {BRANCH} differs from origin/{BRANCH}: run `git pull`, push or drop local commits"));
    }
    // Checked up front: on a fresh machine `git commit` fails only after the
    // upload, leaving a copy of the manifest behind.
    for key in ["user.name", "user.email"] {
        if git(&root, &["config", key]).map_or(true, |v| v.is_empty()) {
            return Err(format!("git {key} is not set, the manifest commit would fail: `git config --global {key} \"...\"`"));
        }
    }
    let status = git(&root, &["status", "--porcelain", "--", PUBLISHED_MANIFEST])?;
    // A copy left by an earlier run that stopped at `git commit` is this very manifest.
    let leftover_is_ours = std::fs::read(root.join(PUBLISHED_MANIFEST)).is_ok_and(|d| d == ours);
    if !status.is_empty() && !leftover_is_ours {
        let fix = if status.starts_with("??") {
            format!("it is not in git yet, delete it (`git clean -f -- {PUBLISHED_MANIFEST}`)")
        } else {
            format!("revert them (`git checkout -- {PUBLISHED_MANIFEST}`)")
        };
        return Err(format!("{PUBLISHED_MANIFEST} has local changes: {fix}"));
    }
    // Read from git, not raw.githubusercontent.com, which serves a cached copy.
    if let Ok(text) = git(&root, &["show", &format!("origin/{BRANCH}:{PUBLISHED_MANIFEST}")]) {
        if let Ok(published) = Manifest::from_json(&text) {
            if published.build_version == version {
                return Err(format!("build {version} is already published"));
            }
            eprintln!("published now: build {}", published.build_version);
        }
    }
    Ok(root)
}

fn upload(repo: &str, tag: &str, manifest: &Manifest, out: &Path, assets: &[(String, u64)]) -> Result<(), String> {
    let existing = match gh(&["release", "view", tag, "--repo", repo, "--json", "assets"]) {
        Ok(json) => release_assets(&json)?,
        Err(e) if e.contains("release not found") => {
            let notes = manifest
                .changelog
                .iter()
                .find(|c| c.version == manifest.build_version)
                .map(|c| c.notes.iter().map(|n| format!("- {n}\n")).collect::<String>())
                .unwrap_or_default();
            let title = format!("Build {}", manifest.build_version);
            // "Latest release" stays the launcher installer, the page players download from.
            gh(&["release", "create", tag, "--repo", repo, "--title", &title, "--notes", &notes, "--latest=false"])?;
            eprintln!("created release {tag}");
            HashMap::new()
        }
        Err(e) => return Err(e),
    };

    let todo: Vec<_> = assets.iter().filter(|(name, size)| existing.get(name) != Some(size)).collect();
    let total: u64 = todo.iter().map(|(_, s)| s).sum();
    eprintln!(
        "{} of {} assets already uploaded, {} to upload ({:.1} GB)",
        assets.len() - todo.len(),
        assets.len(),
        todo.len(),
        total as f64 / 1e9
    );
    for (i, (name, size)) in todo.iter().enumerate() {
        eprintln!("  [{}/{}] {name} ({:.1} MB)", i + 1, todo.len(), *size as f64 / 1e6);
        // --clobber replaces an asset left half-uploaded by an earlier run.
        let path = out.join(name).to_string_lossy().into_owned();
        gh(&["release", "upload", tag, &path, "--repo", repo, "--clobber"])?;
    }
    Ok(())
}

/// Asset name → size of fully uploaded assets (`gh release view --json assets`).
fn release_assets(json: &str) -> Result<HashMap<String, u64>, String> {
    let v: serde_json::Value = serde_json::from_str(json).map_err(|e| format!("gh release view: {e}"))?;
    Ok(v["assets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a["state"].as_str().is_none_or(|s| s == "uploaded"))
        .filter_map(|a| Some((a["name"].as_str()?.to_owned(), a["size"].as_u64()?)))
        .collect())
}

fn confirm(question: &str) -> Result<bool, String> {
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "д" | "да"))
}

fn tool_check(tool: &str, args: &[&str]) -> Result<(), String> {
    let ok = Command::new(tool).args(args).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if ok {
        Ok(())
    } else {
        Err(format!("`{tool}` not found: install it and make sure it is in PATH"))
    }
}

/// Runs `git` in `dir`; trimmed stdout, or stderr as the error.
fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    capture(Command::new("git").current_dir(dir), "git", args)
}

fn gh(args: &[&str]) -> Result<String, String> {
    capture(&mut Command::new("gh"), "gh", args)
}

fn capture(cmd: &mut Command, tool: &str, args: &[&str]) -> Result<String, String> {
    let output = cmd.args(args).output().map_err(|e| format!("{tool}: {e}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    } else {
        Err(format!("{tool} {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

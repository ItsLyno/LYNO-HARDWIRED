//! `lyno-pack publish`: [`lyno_core::release::publish`] with GitHub Releases
//! through `gh` and the manifest pushed to `main` through `git`, from a clone
//! of the repository.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;

use lyno_core::download::Downloader;
use lyno_core::release::{self, Event, Host, PublishOptions, PUBLISHED_MANIFEST};
use lyno_core::Error;

pub struct PublishArgs {
    pub out: PathBuf,
    pub repo: String,
    pub yes: bool,
}

const BRANCH: &str = "main";

pub fn run(args: &PublishArgs) -> Result<(), String> {
    let mut host = GitHost::open(&args.repo)?;
    tool_check("gh", &["--version"])?;
    let root = release::github_download_root(&args.repo);
    let opts = PublishOptions { out: &args.out, download_root: &root };
    let mut ask = |m: &lyno_core::manifest::Manifest| {
        args.yes
            || confirm(&format!("Publish build {} to players (push {PUBLISHED_MANIFEST} to {BRANCH})?", m.build_version))
    };
    let result = release::publish(&mut host, &opts, &Downloader::new(), &AtomicBool::new(false), &mut ask, &print);
    match result {
        Ok(_) => Ok(()),
        Err(Error::Cancelled) => Err("cancelled, nothing was pushed".into()),
        Err(Error::Io { path, source }) if path == args.out.join("manifest.json") => {
            Err(format!("{}: {source} (run `lyno-pack build` first)", path.display()))
        }
        Err(e) => Err(e.to_string()),
    }
}

fn print(event: Event) {
    match event {
        Event::Current(Some(v)) => eprintln!("published now: build {v}"),
        Event::Current(None) => eprintln!("nothing published yet"),
        Event::ReleaseCreated { tag } => eprintln!("created release {tag}"),
        Event::Uploads { total: 0, .. } => eprintln!("no new assets: only the manifest changes"),
        Event::Uploads { uploaded, total, bytes } => eprintln!(
            "{uploaded} of {total} assets already uploaded, {} to upload ({:.1} GB)",
            total - uploaded,
            bytes as f64 / 1e9
        ),
        Event::Uploading { index, count, name, size } => eprintln!("  [{index}/{count}] {name} ({:.1} MB)", size as f64 / 1e6),
        Event::Checking { done: 0, .. } => eprintln!("checking every part over HTTP..."),
        Event::Checking { done, total } if done == total || done % 20 == 0 => eprintln!("  {done}/{total}"),
        Event::Checking { .. } | Event::UploadBytes { .. } => {}
        Event::Live { version } => {
            eprintln!();
            eprintln!("build {version} is live; raw.githubusercontent.com may serve the old manifest for a few minutes");
        }
    }
}

/// Releases through `gh`, the manifest as a commit on `main` of the clone the
/// tool runs in.
struct GitHost {
    repo: String,
    root: PathBuf,
}

impl GitHost {
    /// The commit must go straight on top of what players see now: on `main`,
    /// in sync with `origin/main`, with a pushable origin for `repo`.
    fn open(repo: &str) -> Result<Self, String> {
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
        if !status.is_empty() {
            let published = git(&root, &["cat-file", "-e", &format!("HEAD:{PUBLISHED_MANIFEST}")]).is_ok();
            if published {
                // `checkout HEAD` also resets the index: a plain `checkout --` would
                // restore a change that an earlier run had already staged.
                return Err(format!(
                    "{PUBLISHED_MANIFEST} has local changes: revert them (`git checkout HEAD -- {PUBLISHED_MANIFEST}`)"
                ));
            }
            // Not published yet, so only an earlier run that stopped at `git commit`
            // (untracked or already staged) or a manual copy put it there: ours replaces it.
            eprintln!("note: replacing the unpublished {PUBLISHED_MANIFEST} left by an earlier run");
        }
        Ok(Self { repo: repo.to_owned(), root })
    }
}

impl Host for GitHost {
    fn published_manifest(&mut self) -> lyno_core::Result<Option<String>> {
        // Read from git, not raw.githubusercontent.com, which serves a cached copy.
        Ok(git(&self.root, &["show", &format!("origin/{BRANCH}:{PUBLISHED_MANIFEST}")]).ok())
    }

    fn release_assets(&mut self, tag: &str) -> lyno_core::Result<Option<HashMap<String, u64>>> {
        match gh(&["release", "view", tag, "--repo", &self.repo, "--json", "assets"]) {
            Ok(json) => release_assets(&json).map(Some).map_err(Error::Release),
            Err(e) if e.contains("release not found") => Ok(None),
            Err(e) => Err(Error::Release(e)),
        }
    }

    fn create_release(&mut self, tag: &str, title: &str, notes: &str) -> lyno_core::Result<()> {
        // "Latest release" stays the launcher installer, the page players download from.
        gh(&["release", "create", tag, "--repo", &self.repo, "--title", title, "--notes", notes, "--latest=false"])
            .map(drop)
            .map_err(Error::Release)
    }

    fn upload_asset(&mut self, tag: &str, path: &Path, _progress: &dyn Fn(u64)) -> lyno_core::Result<()> {
        // --clobber replaces an asset left half-uploaded by an earlier run.
        let path = path.to_string_lossy();
        gh(&["release", "upload", tag, &path, "--repo", &self.repo, "--clobber"]).map(drop).map_err(Error::Release)
    }

    fn push_manifest(&mut self, version: &str, path: &Path) -> lyno_core::Result<()> {
        let dest = self.root.join(PUBLISHED_MANIFEST);
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::Release(format!("{}: {e}", dir.display())))?;
        }
        std::fs::copy(path, &dest).map_err(|e| Error::Release(format!("{}: {e}", dest.display())))?;
        let root = &self.root;
        git(root, &["add", "--", PUBLISHED_MANIFEST]).map_err(Error::Release)?;
        git(root, &["commit", "-m", &format!("Build {version}"), "--", PUBLISHED_MANIFEST]).map_err(Error::Release)?;
        git(root, &["push", "origin", BRANCH])
            .map(drop)
            .map_err(|e| Error::Release(format!("{e}\nThe commit is local; push it with `git push origin {BRANCH}`")))
    }
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

fn confirm(question: &str) -> bool {
    eprint!("{question} [y/N] ");
    std::io::stderr().flush().ok();
    let mut line = String::new();
    // An unreadable stdin answers "no": nothing is pushed.
    std::io::stdin().lock().read_line(&mut line).is_ok()
        && matches!(line.trim().to_lowercase().as_str(), "y" | "yes" | "д" | "да")
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

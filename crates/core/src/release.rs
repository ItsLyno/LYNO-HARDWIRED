//! Publishing a build made by [`crate::publish::build`]: uploads its parts and
//! makes the manifest live, in the only safe order. Launchers start downloading
//! as soon as the published manifest changes, so it goes last and only after
//! every part it points at, older releases included, answers over HTTP.
//!
//! Where releases and the manifest live is a [`Host`]; `lyno-pack` drives `gh`
//! and `git`. The order and the checks stay here, so no front end can skip them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::download::Downloader;
use crate::manifest::Manifest;
use crate::publish::check_published;
use crate::{Error, Result};

/// Where launchers read the build from, on the repository's `main`.
pub const PUBLISHED_MANIFEST: &str = "build/manifest.json";

/// GitHub release that holds the parts packed for `version`.
pub fn release_tag(version: &str) -> String {
    format!("build-{version}")
}

/// Root of release download URLs: `<root>/<tag>/<asset>`.
pub fn github_download_root(repo: &str) -> String {
    format!("https://github.com/{repo}/releases/download")
}

/// Where releases and the published manifest live.
pub trait Host {
    /// The manifest players see now, read past any cache; `None` before the
    /// first release.
    fn published_manifest(&mut self) -> Result<Option<String>>;
    /// Names and sizes of the fully uploaded assets of `tag`; `None` when the
    /// release does not exist yet.
    fn release_assets(&mut self, tag: &str) -> Result<Option<HashMap<String, u64>>>;
    fn create_release(&mut self, tag: &str, title: &str, notes: &str) -> Result<()>;
    /// Must replace an asset of the same name: an interrupted run may have left
    /// it half-uploaded. `progress` gets the bytes of this file sent so far.
    fn upload_asset(&mut self, tag: &str, path: &Path, progress: &dyn Fn(u64)) -> Result<()>;
    /// Makes the manifest at `path` the published one.
    fn push_manifest(&mut self, version: &str, path: &Path) -> Result<()>;
}

pub struct PublishOptions<'a> {
    /// Output folder of `build`: `manifest.json` and the new parts.
    pub out: &'a Path,
    /// See [`github_download_root`].
    pub download_root: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// Build players have now.
    Current(Option<String>),
    ReleaseCreated { tag: String },
    /// Before uploading: `uploaded` of `total` new assets are already on the
    /// release (an earlier, interrupted run), `bytes` remain.
    Uploads { uploaded: usize, total: usize, bytes: u64 },
    Uploading { index: usize, count: usize, name: String, size: u64 },
    /// Bytes of all uploads of this run; hosts that can't tell send none.
    UploadBytes { done: u64, total: u64 },
    /// HTTP check of every part; `done: 0` when it starts.
    Checking { done: usize, total: usize },
    Live { version: String },
}

/// Uploads what is missing, checks every part over HTTP, asks `confirm`, then
/// pushes the manifest. Safe to re-run after any failure: uploaded assets are
/// skipped. `cancel` is honored between uploads and before the push.
pub fn publish(
    host: &mut dyn Host,
    opts: &PublishOptions,
    downloader: &Downloader,
    cancel: &AtomicBool,
    confirm: &mut dyn FnMut(&Manifest) -> bool,
    log: &(dyn Fn(Event) + Sync),
) -> Result<Manifest> {
    let manifest_path = opts.out.join("manifest.json");
    let text = std::fs::read_to_string(&manifest_path).map_err(|e| Error::io(&manifest_path, e))?;
    let manifest = Manifest::from_json(&text).map_err(|e| Error::Parse { path: manifest_path.clone(), message: e.to_string() })?;
    let version = &manifest.build_version;
    let tag = release_tag(version);
    let assets = new_assets(&manifest, opts.out, opts.download_root, &tag)?;

    let current = match host.published_manifest()? {
        // An unreadable published manifest is replaced like a missing one.
        Some(text) => Manifest::from_json(&text).ok().map(|m| m.build_version),
        None => None,
    };
    if current.as_ref() == Some(version) {
        return Err(Error::Release(format!("build {version} is already published")));
    }
    log(Event::Current(current));

    if assets.is_empty() {
        // Only the manifest changes (mods reordered or switched, changelog).
        log(Event::Uploads { uploaded: 0, total: 0, bytes: 0 });
    } else {
        upload(host, &tag, &manifest, &assets, cancel, log)?;
    }

    let parts = std::iter::once(&manifest.base).chain(manifest.mod_specs().map(|m| &m.package)).map(|p| p.parts.len()).sum();
    log(Event::Checking { done: 0, total: parts });
    let broken = check_published(&manifest, downloader, &|done, total| log(Event::Checking { done, total }));
    if !broken.is_empty() {
        return Err(Error::Release(format!(
            "{} part(s) are not downloadable, the manifest was not published:\n  {}\n\
             Publish again: uploaded assets are skipped. A timeout or connection error is the network, \
             not the file; HTTP 404 or a wrong size in an older release means that release was changed or deleted",
            broken.len(),
            broken.join("\n  ")
        )));
    }

    if cancel.load(Ordering::Relaxed) || !confirm(&manifest) {
        return Err(Error::Cancelled);
    }
    host.push_manifest(version, &manifest_path)?;
    log(Event::Live { version: version.clone() });
    Ok(manifest)
}

/// A new part of this build, written by `build` into `out`.
struct Asset {
    name: String,
    path: PathBuf,
    size: u64,
}

/// Parts that point at this version's release, i.e. the files `build` wrote.
/// Reused packages point at older releases and are only checked.
fn new_assets(manifest: &Manifest, out: &Path, root: &str, tag: &str) -> Result<Vec<Asset>> {
    let ours = format!("{root}/{tag}/");
    let mut assets = Vec::new();
    for part in std::iter::once(&manifest.base).chain(manifest.mod_specs().map(|m| &m.package)).flat_map(|p| &p.parts) {
        if !part.url.starts_with(&format!("{root}/")) {
            return Err(Error::Release(format!("{}: not a release asset under {root}", part.url)));
        }
        let Some(name) = part.url.strip_prefix(&ours) else { continue };
        let path = out.join(name);
        let size = std::fs::metadata(&path).map_err(|e| Error::io(&path, e))?.len();
        if size != part.size {
            return Err(Error::Release(format!("{}: {size} bytes, the manifest says {} (rebuild)", path.display(), part.size)));
        }
        assets.push(Asset { name: name.to_owned(), path, size });
    }
    Ok(assets)
}

fn upload(
    host: &mut dyn Host,
    tag: &str,
    manifest: &Manifest,
    assets: &[Asset],
    cancel: &AtomicBool,
    log: &(dyn Fn(Event) + Sync),
) -> Result<()> {
    let existing = match host.release_assets(tag)? {
        Some(existing) => existing,
        None => {
            let notes = manifest
                .changelog
                .iter()
                .find(|c| c.version == manifest.build_version)
                .map(|c| c.notes.iter().map(|n| format!("- {n}\n")).collect::<String>())
                .unwrap_or_default();
            host.create_release(tag, &format!("Build {}", manifest.build_version), &notes)?;
            log(Event::ReleaseCreated { tag: tag.to_owned() });
            HashMap::new()
        }
    };
    let todo: Vec<&Asset> = assets.iter().filter(|a| existing.get(&a.name) != Some(&a.size)).collect();
    let total = todo.iter().map(|a| a.size).sum();
    log(Event::Uploads { uploaded: assets.len() - todo.len(), total: assets.len(), bytes: total });
    let mut done = 0;
    for (i, asset) in todo.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(Error::Cancelled);
        }
        log(Event::Uploading { index: i + 1, count: todo.len(), name: asset.name.clone(), size: asset.size });
        host.upload_asset(tag, &asset.path, &|n| log(Event::UploadBytes { done: done + n, total }))?;
        done += asset.size;
    }
    Ok(())
}

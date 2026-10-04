//! Version tracking of mods installed from Nexus.
//!
//! MO2 records the Nexus page and file of a mod installed from Nexus in its
//! `meta.ini` (`modid`, `[installedFiles] 1\fileid`, `version`). A newer
//! version is whatever the author chained to the installed file on the files
//! tab (`file_updates`: "this file replaces that one"). Without a file id
//! (installed by hand, from an archive) only the version string of the page
//! is left to compare, the same fallback MO2 uses.
//!
//! The API allows a limited number of requests per day, and a build has
//! hundreds of mods, so answers are cached in `.lyno/nexus.json` and only
//! mods whose files changed since their last check (`/mods/updated.json`, one
//! request per game) are asked again.
//!
//! Who updates what: build mods of a player come from the build (an update
//! from Nexus would be damage to [`crate::verify`] and be undone by the next
//! build update), so a player updates only mods under `LYNO USER MODS`. The
//! author updates build mods too: that is how the next release gets them.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mo2::Instance;
use crate::modlist::{EntryState, ModList};
use crate::nexus::{FileInfo, ModFiles, ModPage, NexusApi, Period};
use crate::publish::split_user_section;
use crate::state::State;
use crate::{Error, Result};

const DEFAULT_GAME: &str = "cyberpunk2077";

/// A mod folder that came from a Nexus page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tracked {
    pub folder: String,
    pub game: String,
    pub mod_id: u64,
    pub file_id: Option<u64>,
    /// Installed version from `meta.ini`.
    pub version: Option<String>,
    /// Under `LYNO USER MODS`: the player's own mod.
    pub personal: bool,
    /// Installed by the launcher from the build (in `state.json`).
    pub managed: bool,
}

impl Tracked {
    fn key(&self) -> String {
        cache_key(&self.game, self.mod_id)
    }
}

fn cache_key(game: &str, mod_id: u64) -> String {
    format!("{game}/{mod_id}")
}

/// Mods of the profile's list with a Nexus page, in MO2's order, and the
/// folders without one (hand-made mods, other sites).
pub fn tracked_mods(inst: &Instance, profile: &str) -> Result<(Vec<Tracked>, Vec<String>)> {
    let list = ModList::load(&inst.modlist_path(profile))?;
    let metas = inst.scan_mods()?;
    let state = State::load(&crate::install::state_path(inst))?;
    let managed: HashSet<&str> = state.mods.values().map(|m| m.folder.as_str()).collect();
    let (build, personal) = split_user_section(&list);
    let mut tracked = Vec::new();
    let mut untracked = Vec::new();
    let sections = [(build, false), (personal, true)];
    for (entries, personal) in sections {
        for e in entries.iter().filter(|e| e.state != EntryState::Unmanaged && !e.is_separator()) {
            let meta = metas.get(&e.name).cloned().unwrap_or_default();
            let Some(mod_id) = meta.mod_id else {
                untracked.push(e.name.clone());
                continue;
            };
            tracked.push(Tracked {
                folder: e.name.clone(),
                game: meta.game_name.map_or_else(|| DEFAULT_GAME.to_owned(), |g| g.to_lowercase()),
                mod_id,
                file_id: meta.file_id,
                version: meta.version,
                personal,
                managed: managed.contains(e.name.as_str()),
            });
        }
    }
    Ok((tracked, untracked))
}

/// What the API said about a mod page, and when.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Checked {
    /// Unix seconds.
    pub at: u64,
    #[serde(default)]
    pub files: ModFiles,
    /// Only fetched for mods without a file id: their version is compared with the page's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<ModPage>,
    /// The page is hidden, removed or moderated.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unavailable: bool,
}

/// `.lyno/nexus.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    /// `game/mod_id`.
    pub mods: BTreeMap<String, Checked>,
    #[serde(skip)]
    path: PathBuf,
}

impl Cache {
    pub fn path(inst: &Instance) -> PathBuf {
        inst.root().join(".lyno").join("nexus.json")
    }

    /// A missing or unreadable cache only costs requests.
    pub fn load(path: &Path) -> Self {
        let cache: Option<Self> = std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok());
        Self { path: path.to_owned(), ..cache.unwrap_or_default() }
    }

    pub fn save(&self) -> Result<()> {
        let path = &self.path;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self)?).map_err(|e| Error::io(&tmp, e))?;
        std::fs::rename(&tmp, path).map_err(|e| Error::io(path, e))
    }

    pub fn get(&self, game: &str, mod_id: u64) -> Option<&Checked> {
        self.mods.get(&cache_key(game, mod_id))
    }

    /// Oldest check among `mods`; `None` when one was never checked.
    pub fn oldest(&self, mods: &[Tracked]) -> Option<u64> {
        mods.iter().map(|t| self.mods.get(&t.key()).map(|c| c.at)).collect::<Option<Vec<_>>>()?.into_iter().min()
    }

    /// Records the files of a page just fetched for an nxm download, so the
    /// mod shows as current without another request.
    pub fn record(&mut self, game: &str, mod_id: u64, files: ModFiles, now: u64) {
        let entry = self.mods.entry(cache_key(game, mod_id)).or_default();
        entry.at = now;
        entry.files = files;
        entry.unavailable = false;
    }
}

/// Which pages to ask again: never checked, checked over a month ago (out
/// of reach of `/mods/updated.json`), or with files changed since.
fn stale(api: &NexusApi, cache: &Cache, mods: &[Tracked], now: u64, force: bool) -> Result<BTreeSet<(String, u64)>> {
    let mut out = BTreeSet::new();
    let mut by_game: BTreeMap<&str, Vec<&Tracked>> = BTreeMap::new();
    for t in mods {
        by_game.entry(t.game.as_str()).or_default().push(t);
    }
    for (game, mods) in by_game {
        let mut recent = Vec::new();
        for t in mods {
            match cache.get(game, t.mod_id) {
                Some(c) if !force && now.saturating_sub(c.at) < Period::Month.secs() => recent.push((t.mod_id, c.at)),
                _ => {
                    out.insert((game.to_owned(), t.mod_id));
                }
            }
        }
        let Some(oldest) = recent.iter().map(|(_, at)| *at).min() else { continue };
        let Some(period) = Period::covering(now.saturating_sub(oldest)) else { continue };
        let updated: HashMap<u64, u64> =
            api.updated(game, period)?.into_iter().map(|u| (u.mod_id, u.latest_file_update)).collect();
        for (mod_id, at) in recent {
            if updated.get(&mod_id).is_some_and(|&changed| changed >= at) {
                out.insert((game.to_owned(), mod_id));
            }
        }
    }
    Ok(out)
}

/// Brings `cache` up to date for `mods`. `on(done, total)` per page asked.
/// The cache is saved as it goes, so a stopped check (cancel, rate limit)
/// keeps what it already learned.
pub fn check(
    api: &NexusApi,
    cache: &mut Cache,
    mods: &[Tracked],
    now: u64,
    force: bool,
    cancel: &dyn Fn() -> bool,
    on: &mut dyn FnMut(usize, usize),
) -> Result<()> {
    let todo = stale(api, cache, mods, now, force)?;
    let needs_page: HashSet<(String, u64)> =
        mods.iter().filter(|t| t.file_id.is_none()).map(|t| (t.game.clone(), t.mod_id)).collect();
    let total = todo.len();
    on(0, total);
    for (i, (game, mod_id)) in todo.into_iter().enumerate() {
        if cancel() {
            cache.save()?;
            return Err(Error::Cancelled);
        }
        let fetch = || -> Result<Checked> {
            let files = api.files(&game, mod_id)?;
            let page = match needs_page.contains(&(game.clone(), mod_id)) {
                true => Some(api.mod_page(&game, mod_id)?),
                false => None,
            };
            let unavailable = page.as_ref().is_some_and(|p| !p.available);
            Ok(Checked { at: now, files, page, unavailable })
        };
        let checked = match fetch() {
            Ok(c) => c,
            // A hidden or removed page answers 403/404/410: a result, not a failure.
            Err(Error::Nexus { status: 403 | 404 | 410, .. }) => Checked { at: now, unavailable: true, ..Default::default() },
            Err(e) => {
                cache.save()?;
                return Err(e);
            }
        };
        cache.mods.insert(cache_key(&game, mod_id), checked);
        if i % 20 == 19 {
            cache.save()?;
        }
        on(i + 1, total);
    }
    cache.save()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Status {
    /// Not checked yet.
    Unknown,
    UpToDate,
    /// A newer version is out. `file`: the file to install; `None` when Nexus
    /// doesn't say which file replaces the installed one, and the player picks
    /// on the files tab.
    #[serde(rename_all = "camelCase")]
    Update { file: Option<NewFile>, version: Option<String> },
    /// The page is hidden or removed.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NewFile {
    pub file_id: u64,
    pub name: String,
    pub file_name: String,
    pub size: Option<u64>,
}

impl From<&FileInfo> for NewFile {
    fn from(f: &FileInfo) -> Self {
        Self { file_id: f.file_id, name: f.name.clone(), file_name: f.file_name.clone(), size: f.size() }
    }
}

/// The newest file the author chained to `file_id`. Several replacements of
/// one file (the author split it) resolve to the latest upload.
pub fn newest(files: &ModFiles, file_id: u64) -> u64 {
    let uploaded = |id: u64| files.files.iter().find(|f| f.file_id == id).map_or(0, |f| f.uploaded_timestamp);
    let mut seen = HashSet::from([file_id]);
    let mut current = file_id;
    loop {
        let next = files
            .file_updates
            .iter()
            .filter(|u| u.old_file_id == current && !seen.contains(&u.new_file_id))
            .map(|u| u.new_file_id)
            .max_by_key(|&id| (uploaded(id), id));
        match next {
            Some(n) => {
                seen.insert(n);
                current = n;
            }
            None => return current,
        }
    }
}

/// `file_id` and every file the chain says it replaced.
pub fn ancestors(files: &ModFiles, file_id: u64) -> HashSet<u64> {
    let mut out = HashSet::from([file_id]);
    let mut todo = vec![file_id];
    while let Some(id) = todo.pop() {
        for u in files.file_updates.iter().filter(|u| u.new_file_id == id) {
            if out.insert(u.old_file_id) {
                todo.push(u.old_file_id);
            }
        }
    }
    out
}

/// Versions as authors write them: `v1.2` and `1.2` are the same, and so are
/// `1.2.0` on the page and `1.2` from `meta.ini` (MO2 pads versions, see `meta::display_version`).
fn same_version(a: &str, b: &str) -> bool {
    let norm = |s: &str| crate::meta::display_version(s.trim().trim_start_matches(['v', 'V']));
    norm(a) == norm(b)
}

pub fn status(t: &Tracked, checked: Option<&Checked>) -> Status {
    let Some(c) = checked else { return Status::Unknown };
    if c.unavailable {
        return Status::Unavailable;
    }
    let file = |id: u64| c.files.files.iter().find(|f| f.file_id == id);
    let Some(installed) = t.file_id else {
        return match (c.page.as_ref().and_then(|p| p.version.as_deref()), t.version.as_deref()) {
            (Some(latest), Some(have)) if !same_version(latest, have) => {
                Status::Update { file: None, version: Some(latest.to_owned()) }
            }
            (Some(_), Some(_)) => Status::UpToDate,
            _ => Status::Unknown,
        };
    };
    let next = newest(&c.files, installed);
    if next != installed {
        if let Some(f) = file(next).filter(|f| f.is_current()) {
            return Status::Update { file: Some(f.into()), version: f.version.clone() };
        }
    }
    if file(installed).is_some_and(FileInfo::is_current) {
        return Status::UpToDate;
    }
    // The installed file was moved to old versions or deleted without a
    // chain. With a single main file left, that is the update.
    let mains: Vec<&FileInfo> =
        c.files.files.iter().filter(|f| f.is_current() && f.category_name.as_deref() == Some("MAIN")).collect();
    match mains.as_slice() {
        [main] => Status::Update { file: Some((*main).into()), version: main.version.clone() },
        _ => Status::Update { file: None, version: None },
    }
}

/// Where a downloaded file of `mod_id` goes: over the installed mod it
/// replaces (same file again, or one the chain says it replaces), or into a
/// new folder (another file of the page, e.g. an optional addon).
pub fn target<'a>(mods: &'a [Tracked], files: &ModFiles, game: &str, mod_id: u64, file_id: u64) -> Option<&'a Tracked> {
    let same_page: Vec<&Tracked> = mods.iter().filter(|t| t.game == game && t.mod_id == mod_id).collect();
    let replaced = ancestors(files, file_id);
    if let Some(t) = same_page.iter().find(|t| t.file_id.is_some_and(|f| replaced.contains(&f))) {
        return Some(t);
    }
    // Installed without a file record: the only folder of the page is the one to update.
    match same_page.as_slice() {
        [only] if only.file_id.is_none() => Some(only),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nexus::FileUpdate;

    fn file(id: u64, category: &str, version: &str, uploaded: u64) -> FileInfo {
        FileInfo {
            file_id: id,
            name: format!("File {id}"),
            version: Some(version.into()),
            category_name: Some(category.into()),
            file_name: format!("mod-{id}.zip"),
            size_in_bytes: Some(1000),
            size_kb: None,
            uploaded_timestamp: uploaded,
        }
    }

    fn tracked(file_id: Option<u64>, version: &str) -> Tracked {
        Tracked {
            folder: "Mod".into(),
            game: "cyberpunk2077".into(),
            mod_id: 1,
            file_id,
            version: Some(version.into()),
            personal: true,
            managed: false,
        }
    }

    fn checked(files: Vec<FileInfo>, updates: &[(u64, u64)]) -> Checked {
        let file_updates = updates.iter().map(|&(old_file_id, new_file_id)| FileUpdate { old_file_id, new_file_id }).collect();
        Checked { at: 1, files: ModFiles { files, file_updates }, page: None, unavailable: false }
    }

    #[test]
    fn follows_the_update_chain() {
        let c = checked(
            vec![file(1, "OLD_VERSION", "1.0", 10), file(2, "OLD_VERSION", "1.1", 20), file(3, "MAIN", "1.2", 30)],
            &[(1, 2), (2, 3)],
        );
        let s = status(&tracked(Some(1), "1.0"), Some(&c));
        assert!(matches!(&s, Status::Update { file: Some(f), version: Some(v) } if f.file_id == 3 && v == "1.2"), "{s:?}");
        assert_eq!(status(&tracked(Some(3), "1.2"), Some(&c)), Status::UpToDate);
        assert_eq!(ancestors(&c.files, 3), HashSet::from([1, 2, 3]));
    }

    #[test]
    fn optional_files_without_a_chain_are_current() {
        let c = checked(vec![file(1, "MAIN", "2.0", 10), file(5, "OPTIONAL", "1.0", 5)], &[]);
        assert_eq!(status(&tracked(Some(5), "1.0"), Some(&c)), Status::UpToDate);
    }

    #[test]
    fn old_file_without_a_chain_points_at_the_single_main_file() {
        let c = checked(vec![file(1, "OLD_VERSION", "1.0", 10), file(2, "MAIN", "2.0", 20)], &[]);
        let s = status(&tracked(Some(1), "1.0"), Some(&c));
        assert!(matches!(&s, Status::Update { file: Some(f), .. } if f.file_id == 2), "{s:?}");

        let two_mains = checked(vec![file(1, "OLD_VERSION", "1.0", 10), file(2, "MAIN", "2.0", 20), file(3, "MAIN", "2.0", 20)], &[]);
        assert_eq!(status(&tracked(Some(1), "1.0"), Some(&two_mains)), Status::Update { file: None, version: None });
    }

    #[test]
    fn without_file_id_compares_page_version() {
        let mut c = checked(vec![], &[]);
        c.page = Some(ModPage { version: Some("v1.3".into()), ..Default::default() });
        assert_eq!(status(&tracked(None, "1.3"), Some(&c)), Status::UpToDate);
        c.page = Some(ModPage { version: Some("1.3.0".into()), ..Default::default() });
        assert_eq!(status(&tracked(None, "1.3"), Some(&c)), Status::UpToDate);
        c.page = Some(ModPage { version: Some("v1.3".into()), ..Default::default() });
        assert_eq!(
            status(&tracked(None, "1.2"), Some(&c)),
            Status::Update { file: None, version: Some("v1.3".into()) }
        );
        assert_eq!(status(&tracked(None, "1.2"), None), Status::Unknown);
    }

    #[test]
    fn split_file_resolves_to_latest_upload() {
        let c = checked(vec![file(1, "OLD_VERSION", "1", 1), file(2, "MAIN", "2", 20), file(3, "MAIN", "2", 30)], &[(1, 2), (1, 3)]);
        assert_eq!(newest(&c.files, 1), 3);
    }

    #[test]
    fn chain_loops_terminate() {
        let c = checked(vec![file(1, "MAIN", "1", 1), file(2, "MAIN", "2", 2)], &[(1, 2), (2, 1)]);
        assert_eq!(newest(&c.files, 1), 2);
    }

    #[test]
    fn picks_the_folder_a_download_replaces() {
        let c = checked(vec![file(1, "OLD_VERSION", "1", 1), file(2, "MAIN", "2", 2), file(7, "OPTIONAL", "1", 1)], &[(1, 2)]);
        let main = Tracked { folder: "Main".into(), ..tracked(Some(1), "1") };
        let addon = Tracked { folder: "Addon".into(), ..tracked(Some(7), "1") };
        let mods = [main, addon];
        assert_eq!(target(&mods, &c.files, "cyberpunk2077", 1, 2).map(|t| t.folder.as_str()), Some("Main"));
        assert_eq!(target(&mods, &c.files, "cyberpunk2077", 1, 7).map(|t| t.folder.as_str()), Some("Addon"));
        assert_eq!(target(&mods, &c.files, "cyberpunk2077", 1, 9), None);
        assert_eq!(target(&mods, &c.files, "cyberpunk2077", 2, 2), None);

        let unknown = [Tracked { folder: "Hand".into(), ..tracked(None, "1") }];
        assert_eq!(target(&unknown, &c.files, "cyberpunk2077", 1, 2).map(|t| t.folder.as_str()), Some("Hand"));
    }

    #[test]
    fn cache_roundtrips_and_finds_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(".lyno/nexus.json");
        let mut cache = Cache::load(&path);
        cache.record("cyberpunk2077", 1, ModFiles::default(), 100);
        cache.save().unwrap();
        let back = Cache::load(&path);
        assert_eq!(back, cache);
        assert_eq!(back.oldest(&[tracked(Some(1), "1")]), Some(100));
        let other = Tracked { mod_id: 2, ..tracked(Some(1), "1") };
        assert_eq!(back.oldest(&[tracked(Some(1), "1"), other]), None);
    }
}

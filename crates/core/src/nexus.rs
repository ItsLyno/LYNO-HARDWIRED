//! The Nexus Mods API.
//!
//! [`Nexus`]: mod authors and titles (`/v1/games/{game}/mods/{id}.json`) for
//! [`crate::publish::build`]. Optional: without a key the build keeps what the
//! previous manifest had for the same Nexus page.
//!
//! [`NexusApi`]: the player's account, for version tracking
//! ([`crate::tracking`]) and downloads from nxm links ([`crate::nxm`]).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::meta::ModMeta;
use crate::publish::ModInfo;
use crate::{Error, Result};

pub const API_URL: &str = "https://api.nexusmods.com";

/// ureq has no overall timeout by default: one stalled request would hold the whole build.
const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_FAILURES: u32 = 3;

pub struct Nexus {
    key: Option<String>,
    /// Lookups that failed in a row; the build gives up on Nexus after a few
    /// instead of waiting on an unreachable API for every mod.
    failures: u32,
    agent: ureq::Agent,
    cache: HashMap<(String, u64), ModInfo>,
    /// Failed lookups, for the author to read after the build.
    pub warnings: Vec<String>,
}

impl Nexus {
    pub fn new(key: Option<String>) -> Self {
        let agent = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).build().new_agent();
        Self { key: key.filter(|k| !k.trim().is_empty()), failures: 0, agent, cache: HashMap::new(), warnings: Vec::new() }
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    pub fn info(&mut self, meta: &ModMeta) -> ModInfo {
        if self.failures >= MAX_FAILURES {
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
                self.warnings.push(format!("Nexus {mod_id}: {e}"));
                // A hidden or deleted mod answers 404: Nexus itself is fine.
                let reachable = matches!(e, ureq::Error::StatusCode(code) if code != 429);
                self.failures = if reachable { 0 } else { self.failures + 1 };
                if self.failures == MAX_FAILURES {
                    self.warnings.push(format!("Nexus failed {MAX_FAILURES} times in a row, authors of the remaining mods are kept from the previous build"));
                }
                ModInfo::default()
            }
        };
        self.cache.insert((game, mod_id), info.clone());
        info
    }
}

/// The account behind an API key (`/v1/users/validate.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub name: String,
    /// Premium accounts get download links from the API; free ones only for
    /// a file the player clicked on the site (an nxm link with a key).
    pub premium: bool,
}

/// One file of a mod page (`/files.json`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileInfo {
    pub file_id: u64,
    /// Display name on the files tab ("Main File").
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
    /// MAIN, UPDATE, OPTIONAL, OLD_VERSION, MISCELLANEOUS, ARCHIVED; null on
    /// some removed files.
    #[serde(default)]
    pub category_name: Option<String>,
    /// Archive name, which MO2 records as `installationFile`.
    #[serde(default)]
    pub file_name: String,
    #[serde(default)]
    pub size_in_bytes: Option<u64>,
    /// Kilobytes; the only size older files have.
    #[serde(default)]
    pub size_kb: Option<u64>,
    #[serde(default)]
    pub uploaded_timestamp: u64,
}

impl FileInfo {
    /// Old versions and archived files stay on the page but are no update target.
    pub fn is_current(&self) -> bool {
        !matches!(self.category_name.as_deref(), None | Some("OLD_VERSION" | "ARCHIVED" | "DELETED" | "REMOVED"))
    }

    pub fn size(&self) -> Option<u64> {
        self.size_in_bytes.or(self.size_kb.map(|kb| kb * 1024))
    }
}

/// The author marked `new_file_id` as the next version of `old_file_id`:
/// how Nexus tells which file replaces which when a page has several.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileUpdate {
    pub old_file_id: u64,
    pub new_file_id: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModFiles {
    #[serde(default)]
    pub files: Vec<FileInfo>,
    #[serde(default)]
    pub file_updates: Vec<FileUpdate>,
}

/// The parts of `/mods/{id}.json` version tracking needs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModPage {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub author: Option<String>,
    /// False for hidden, removed or under-moderation pages.
    #[serde(default = "yes")]
    pub available: bool,
}

fn yes() -> bool {
    true
}

/// `/mods/updated.json`: a mod whose files changed within the period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct Updated {
    pub mod_id: u64,
    pub latest_file_update: u64,
}

/// Periods `/mods/updated.json` accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Period {
    Day,
    Week,
    Month,
}

impl Period {
    pub const fn secs(self) -> u64 {
        match self {
            Period::Day => 86_400,
            Period::Week => 7 * 86_400,
            Period::Month => 28 * 86_400,
        }
    }

    fn param(self) -> &'static str {
        match self {
            Period::Day => "1d",
            Period::Week => "1w",
            Period::Month => "1m",
        }
    }

    /// The shortest period that covers `secs`, if any does.
    pub fn covering(secs: u64) -> Option<Self> {
        [Period::Day, Period::Week, Period::Month].into_iter().find(|p| secs <= p.secs())
    }
}

/// Requests left, from the `x-rl-*` headers of the last answer. Nexus counts
/// per account: a daily allowance, then an hourly one once it is spent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimit {
    pub daily: Option<u64>,
    pub hourly: Option<u64>,
    /// Allowances (`x-rl-*-limit`): 20 000 a day, 500 an hour at the time of writing.
    #[serde(default)]
    pub daily_limit: Option<u64>,
    #[serde(default)]
    pub hourly_limit: Option<u64>,
}

/// The Nexus Mods API with the player's key.
pub struct NexusApi {
    base: String,
    key: String,
    agent: ureq::Agent,
    limit: Mutex<RateLimit>,
}

impl NexusApi {
    pub fn new(key: &str) -> Self {
        Self::with_base(API_URL, key)
    }

    /// Against another server (tests).
    pub fn with_base(base: &str, key: &str) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(TIMEOUT))
            .user_agent(concat!("LYNO-HARDWIRED/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Self { base: base.trim_end_matches('/').to_owned(), key: key.trim().to_owned(), agent, limit: Mutex::default() }
    }

    pub fn rate_limit(&self) -> RateLimit {
        *self.limit.lock().unwrap()
    }

    fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base);
        let mut resp = self
            .agent
            .get(&url)
            .header("apikey", &self.key)
            // Nexus asks API clients to name themselves.
            .header("Application-Name", "LYNO-HARDWIRED")
            .header("Application-Version", env!("CARGO_PKG_VERSION"))
            .call()
            .map_err(|e| Error::Download(format!("Nexus: {e}")))?;
        let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let limit = RateLimit {
            daily: header("x-rl-daily-remaining"),
            hourly: header("x-rl-hourly-remaining"),
            daily_limit: header("x-rl-daily-limit"),
            hourly_limit: header("x-rl-hourly-limit"),
        };
        if limit != RateLimit::default() {
            *self.limit.lock().unwrap() = limit;
        }
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            // Nexus explains refusals in `{"message": …}`.
            let message = resp
                .body_mut()
                .read_json::<serde_json::Value>()
                .ok()
                .and_then(|v| v["message"].as_str().map(str::to_owned))
                .unwrap_or_default();
            return Err(Error::Nexus { status, message });
        }
        resp.body_mut().read_json::<T>().map_err(|e| Error::Download(format!("Nexus {path}: {e}")))
    }

    pub fn validate(&self) -> Result<User> {
        let v: serde_json::Value = self.get("/v1/users/validate.json")?;
        // Older answers spell it `is_premium?`.
        let flag = |k: &str| v[k].as_bool().unwrap_or(false);
        Ok(User {
            name: v["name"].as_str().unwrap_or_default().to_owned(),
            premium: flag("is_premium") || flag("is_premium?"),
        })
    }

    pub fn mod_page(&self, game: &str, mod_id: u64) -> Result<ModPage> {
        self.get(&format!("/v1/games/{game}/mods/{mod_id}.json"))
    }

    pub fn files(&self, game: &str, mod_id: u64) -> Result<ModFiles> {
        self.get(&format!("/v1/games/{game}/mods/{mod_id}/files.json"))
    }

    pub fn updated(&self, game: &str, period: Period) -> Result<Vec<Updated>> {
        self.get(&format!("/v1/games/{game}/mods/updated.json?period={}", period.param()))
    }

    /// CDN addresses of a file, best first. `permit` is the key and expiry of
    /// an nxm link; without one only a Premium account gets an answer.
    pub fn download_links(&self, game: &str, mod_id: u64, file_id: u64, permit: Option<(&str, u64)>) -> Result<Vec<String>> {
        let mut path = format!("/v1/games/{game}/mods/{mod_id}/files/{file_id}/download_link.json");
        if let Some((key, expires)) = permit {
            path.push_str(&format!("?key={key}&expires={expires}"));
        }
        let links: Vec<serde_json::Value> = self.get(&path)?;
        Ok(links.iter().filter_map(|l| l["URI"].as_str().map(str::to_owned)).collect())
    }
}

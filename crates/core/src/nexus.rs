//! Mod authors and titles from Nexus (`/v1/games/{game}/mods/{id}.json`) for
//! [`crate::publish::build`]. Optional: without a key the build keeps what the
//! previous manifest had for the same Nexus page.

use std::collections::HashMap;
use std::time::Duration;

use crate::meta::ModMeta;
use crate::publish::ModInfo;

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

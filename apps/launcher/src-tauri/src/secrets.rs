//! Author-mode secrets (GitHub token, Nexus API key) live in Windows Credential
//! Manager, not in `settings.json`: the settings file ends up in diagnostic
//! zips and backups, and a token in it could publish a build.
//!
//! Elsewhere (Linux, for development) keyring falls back to its mock store,
//! which keeps a credential per `Entry` object; entries are kept here for the
//! life of the process so the mock behaves like a store.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use keyring::Entry;

const SERVICE: &str = "LYNO//HARDWIRED";

#[derive(Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Secret {
    GithubToken,
    NexusKey,
}

impl Secret {
    fn key(self) -> &'static str {
        match self {
            Secret::GithubToken => "github-token",
            Secret::NexusKey => "nexus-api-key",
        }
    }
}

fn with_entry<T>(secret: Secret, f: impl FnOnce(&Entry) -> keyring::Result<T>) -> keyring::Result<T> {
    static ENTRIES: OnceLock<Mutex<HashMap<&'static str, Entry>>> = OnceLock::new();
    let mut entries = ENTRIES.get_or_init(Default::default).lock().unwrap();
    let entry = match entries.entry(secret.key()) {
        std::collections::hash_map::Entry::Occupied(e) => e.into_mut(),
        std::collections::hash_map::Entry::Vacant(v) => v.insert(Entry::new(SERVICE, secret.key())?),
    };
    f(entry)
}

pub fn get(secret: Secret) -> Option<String> {
    match with_entry(secret, |e| e.get_password()) {
        Ok(v) => Some(v).filter(|v| !v.trim().is_empty()),
        Err(keyring::Error::NoEntry) => None,
        Err(e) => {
            log::warn!("credential {}: {e}", secret.key());
            None
        }
    }
}

pub fn set(secret: Secret, value: &str) -> Result<(), String> {
    with_entry(secret, |e| e.set_password(value.trim())).map_err(|e| e.to_string())
}

pub fn delete(secret: Secret) -> Result<(), String> {
    match with_entry(secret, |e| e.delete_credential()) {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

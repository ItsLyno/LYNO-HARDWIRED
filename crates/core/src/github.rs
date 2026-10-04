//! [`Host`] over the GitHub REST API with a personal access token, so the
//! launcher can publish without `git` or `gh`. The manifest is committed to
//! `main` through the contents API with the blob `sha` read at the start of
//! the publish: if `build/manifest.json` changed in between (published from
//! somewhere else), GitHub refuses the commit instead of overwriting it.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Value};

use crate::release::{Host, PUBLISHED_MANIFEST};
use crate::{Error, Result};

const API: &str = "https://api.github.com";
const UPLOADS: &str = "https://uploads.github.com";
const BRANCH: &str = "main";

pub struct GitHub {
    repo: String,
    token: String,
    api: String,
    uploads: String,
    agent: ureq::Agent,
    /// No overall timeout: a part is up to 1.9 GB on a home uplink.
    upload_agent: ureq::Agent,
    releases: HashMap<String, Release>,
    /// Blob sha of the published manifest, from [`Host::published_manifest`].
    manifest_sha: Option<String>,
}

struct Release {
    id: u64,
    /// Name → (asset id, size, fully uploaded).
    assets: HashMap<String, (u64, u64, bool)>,
}

impl GitHub {
    pub fn new(repo: &str, token: &str) -> Self {
        Self::with_roots(repo, token, API, UPLOADS)
    }

    /// For tests against a local server.
    pub fn with_roots(repo: &str, token: &str, api: &str, uploads: &str) -> Self {
        let ua = concat!("lyno-hardwired/", env!("CARGO_PKG_VERSION"));
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .user_agent(ua)
            .build()
            .new_agent();
        let upload_agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            // GitHub answers once it has stored the whole file.
            .timeout_recv_response(Some(Duration::from_secs(600)))
            .user_agent(ua)
            .build()
            .new_agent();
        Self {
            repo: repo.to_owned(),
            token: token.trim().to_owned(),
            api: api.trim_end_matches('/').to_owned(),
            uploads: uploads.trim_end_matches('/').to_owned(),
            agent,
            upload_agent,
            releases: HashMap::new(),
            manifest_sha: None,
        }
    }

    /// The token works and may push to the repository: publishing needs
    /// Contents read and write (releases are part of it).
    pub fn check_access(&self) -> Result<()> {
        let (status, body) = self.call("GET", &format!("/repos/{}", self.repo), None)?;
        let v = expect(status, &body, &[200], &format!("repository {}", self.repo))?;
        if v["permissions"]["push"].as_bool() != Some(true) {
            return Err(Error::Release(format!(
                "the token can read {} but not write to it: give it Contents: Read and write",
                self.repo
            )));
        }
        Ok(())
    }

    fn request(&self, method: &str, url: &str) -> ureq::RequestBuilder<ureq::typestate::WithoutBody> {
        let req = match method {
            "GET" => self.agent.get(url),
            "DELETE" => self.agent.delete(url),
            _ => unreachable!("{method} has a body"),
        };
        req.header("Authorization", &format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
    }

    /// API call relative to the API root; status and body text.
    fn call(&self, method: &str, path: &str, body: Option<Value>) -> Result<(u16, String)> {
        let url = format!("{}{path}", self.api);
        let result = match body {
            None => self.request(method, &url).call(),
            Some(body) => {
                let req = match method {
                    "POST" => self.agent.post(&url),
                    "PUT" => self.agent.put(&url),
                    _ => unreachable!("{method} has no body"),
                };
                req.header("Authorization", &format!("Bearer {}", self.token))
                    .header("Accept", "application/vnd.github+json")
                    .header("X-GitHub-Api-Version", "2022-11-28")
                    .send_json(body)
            }
        };
        let mut resp = result.map_err(|e| Error::Release(format!("GitHub {method} {path}: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(|e| Error::Release(format!("GitHub {method} {path}: {e}")))?;
        Ok((status, text))
    }

    fn release(&mut self, tag: &str) -> Result<Option<&mut Release>> {
        if !self.releases.contains_key(tag) {
            let (status, body) = self.call("GET", &format!("/repos/{}/releases/tags/{}", self.repo, encode(tag)), None)?;
            if status == 404 {
                return Ok(None);
            }
            let id = expect(status, &body, &[200], &format!("release {tag}"))?["id"]
                .as_u64()
                .ok_or_else(|| Error::Release(format!("release {tag}: no id in GitHub's answer")))?;
            let mut assets = HashMap::new();
            for page in 1.. {
                let path = format!("/repos/{}/releases/{id}/assets?per_page=100&page={page}", self.repo);
                let (status, body) = self.call("GET", &path, None)?;
                let list = expect(status, &body, &[200], &format!("assets of release {tag}"))?;
                let items = list.as_array().cloned().unwrap_or_default();
                for a in &items {
                    if let (Some(name), Some(aid), Some(size)) = (a["name"].as_str(), a["id"].as_u64(), a["size"].as_u64()) {
                        // "starter": an upload that never finished.
                        assets.insert(name.to_owned(), (aid, size, a["state"].as_str().is_none_or(|s| s == "uploaded")));
                    }
                }
                if items.len() < 100 {
                    break;
                }
            }
            self.releases.insert(tag.to_owned(), Release { id, assets });
        }
        Ok(self.releases.get_mut(tag))
    }
}

impl Host for GitHub {
    fn published_manifest(&mut self) -> Result<Option<String>> {
        let path = format!("/repos/{}/contents/{PUBLISHED_MANIFEST}?ref={BRANCH}", self.repo);
        let (status, body) = self.call("GET", &path, None)?;
        if status == 404 {
            self.manifest_sha = None;
            return Ok(None);
        }
        let meta = expect(status, &body, &[200], PUBLISHED_MANIFEST)?;
        self.manifest_sha = meta["sha"].as_str().map(Into::into);
        // The JSON answer carries the content only up to 1 MB; the raw one always.
        let url = format!("{}{path}", self.api);
        let mut resp = self
            .request("GET", &url)
            .header("Accept", "application/vnd.github.raw")
            .call()
            .map_err(|e| Error::Release(format!("GitHub {PUBLISHED_MANIFEST}: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(|e| Error::Release(format!("GitHub {PUBLISHED_MANIFEST}: {e}")))?;
        expect_status(status, &text, &[200], PUBLISHED_MANIFEST)?;
        Ok(Some(text))
    }

    fn release_assets(&mut self, tag: &str) -> Result<Option<HashMap<String, u64>>> {
        Ok(self
            .release(tag)?
            .map(|r| r.assets.iter().filter(|(_, (_, _, done))| *done).map(|(name, (_, size, _))| (name.clone(), *size)).collect()))
    }

    fn create_release(&mut self, tag: &str, title: &str, notes: &str) -> Result<()> {
        // "Latest release" stays the launcher installer, the page players download from.
        let body = json!({ "tag_name": tag, "target_commitish": BRANCH, "name": title, "body": notes, "make_latest": "false" });
        let (status, text) = self.call("POST", &format!("/repos/{}/releases", self.repo), Some(body))?;
        let v = expect(status, &text, &[201], &format!("creating release {tag}"))?;
        let id = v["id"].as_u64().ok_or_else(|| Error::Release(format!("release {tag}: no id in GitHub's answer")))?;
        self.releases.insert(tag.to_owned(), Release { id, assets: HashMap::new() });
        Ok(())
    }

    fn upload_asset(&mut self, tag: &str, path: &Path, progress: &dyn Fn(u64)) -> Result<()> {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let release = self.release(tag)?.ok_or_else(|| Error::Release(format!("release {tag} does not exist")))?;
        let (id, stale) = (release.id, release.assets.get(&name).map(|a| a.0));
        if let Some(asset) = stale {
            let (status, text) = self.call("DELETE", &format!("/repos/{}/releases/assets/{asset}", self.repo), None)?;
            expect_status(status, &text, &[204, 404], &format!("replacing {name}"))?;
        }

        let file = File::open(path).map_err(|e| Error::io(path, e))?;
        let size = file.metadata().map_err(|e| Error::io(path, e))?.len();
        let url = format!("{}/repos/{}/releases/{id}/assets?name={}", self.uploads, self.repo, encode(&name));
        let mut reader = Progress { inner: file, done: 0, report: progress };
        let resp = self
            .upload_agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.token))
            .header("Accept", "application/vnd.github+json")
            .header("Content-Type", "application/octet-stream")
            .header("Content-Length", &size.to_string())
            .send(ureq::SendBody::from_reader(&mut reader));
        let mut resp = resp.map_err(|e| Error::Release(format!("uploading {name}: {e}")))?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().unwrap_or_default();
        let v = expect(status, &text, &[201], &format!("uploading {name}"))?;
        if v["size"].as_u64() != Some(size) {
            return Err(Error::Release(format!("uploading {name}: GitHub stored {} bytes of {size}", v["size"])));
        }
        let asset = v["id"].as_u64().unwrap_or_default();
        if let Some(r) = self.releases.get_mut(tag) {
            r.assets.insert(name, (asset, size, true));
        }
        Ok(())
    }

    fn push_manifest(&mut self, version: &str, path: &Path) -> Result<()> {
        let bytes = std::fs::read(path).map_err(|e| Error::io(path, e))?;
        let mut body = json!({
            "message": format!("Build {version}"),
            "content": base64::engine::general_purpose::STANDARD.encode(bytes),
            "branch": BRANCH,
        });
        if let Some(sha) = &self.manifest_sha {
            body["sha"] = json!(sha);
        }
        let (status, text) = self.call("PUT", &format!("/repos/{}/contents/{PUBLISHED_MANIFEST}", self.repo), Some(body))?;
        if status == 409 || (status == 422 && text.contains("sha")) {
            return Err(Error::Release(format!(
                "{PUBLISHED_MANIFEST} changed on GitHub while publishing (published from somewhere else?): \
                 nothing was overwritten, publish again"
            )));
        }
        expect_status(status, &text, &[200, 201], &format!("committing {PUBLISHED_MANIFEST}"))
    }
}

/// Counts what the upload has read from the file.
struct Progress<'a> {
    inner: File,
    done: u64,
    report: &'a dyn Fn(u64),
}

impl Read for Progress<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        (self.report)(self.done);
        Ok(n)
    }
}

fn expect(status: u16, body: &str, ok: &[u16], what: &str) -> Result<Value> {
    expect_status(status, body, ok, what)?;
    serde_json::from_str(body).map_err(|e| Error::Release(format!("{what}: unexpected answer from GitHub: {e}")))
}

fn expect_status(status: u16, body: &str, ok: &[u16], what: &str) -> Result<()> {
    if ok.contains(&status) {
        return Ok(());
    }
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v["message"].as_str().map(str::to_owned))
        .unwrap_or_else(|| body.chars().take(200).collect());
    let hint = match status {
        401 => " (the token is wrong or expired)",
        403 | 404 => " (the token has no access: it needs Contents: Read and write on the repository)",
        _ => "",
    };
    Err(Error::Release(format!("{what}: GitHub answered {status}: {message}{hint}")))
}

/// Percent-encodes everything but unreserved characters (RFC 3986).
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_names() {
        assert_eq!(encode("cet-0123.tar.zst.001"), "cet-0123.tar.zst.001");
        assert_eq!(encode("a b/с"), "a%20b%2F%D1%81");
    }
}

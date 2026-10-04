//! HTTP downloads of package parts with resume and hash verification.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::hash::blake3_file;
use crate::manifest::Part;
use crate::{Error, Result};

pub struct Downloader {
    agent: ureq::Agent,
}

impl Default for Downloader {
    fn default() -> Self {
        Self::new()
    }
}

impl Downloader {
    pub fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(20)))
            .user_agent(concat!("lyno-hardwired/", env!("CARGO_PKG_VERSION")))
            .build()
            .new_agent();
        Self { agent }
    }

    /// Small text resources (the manifest).
    pub fn get_text(&self, url: &str) -> Result<String> {
        let mut resp = self.agent.get(url).call().map_err(|e| Error::Download(format!("{url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Download(format!("{url}: HTTP {}", resp.status())));
        }
        resp.body_mut()
            .read_to_string()
            .map_err(|e| Error::Download(format!("{url}: {e}")))
    }

    /// Checks that `part` is downloadable and has the expected size without
    /// fetching it: a HEAD request follows the GitHub redirect to the storage
    /// host, which reports the asset's Content-Length.
    pub fn check_part(&self, part: &Part) -> Result<()> {
        let resp = self.agent.head(&part.url).call().map_err(|e| Error::Download(format!("{}: {e}", part.url)))?;
        if !resp.status().is_success() {
            return Err(Error::Download(format!("{}: HTTP {}", part.url, resp.status())));
        }
        let len = resp
            .headers()
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        match len {
            Some(n) if n == part.size => Ok(()),
            Some(n) => Err(Error::Download(format!("{}: {n} bytes, expected {}", part.url, part.size))),
            None => Err(Error::Download(format!("{}: no Content-Length", part.url))),
        }
    }

    /// Downloads `part` to `dest`, resuming from `<dest>.partial` and
    /// skipping the transfer when `dest` is already complete.
    /// `progress` receives the number of new bytes as they arrive.
    pub fn fetch_part(
        &self,
        part: &Part,
        dest: &Path,
        cancel: &AtomicBool,
        progress: &mut dyn FnMut(u64),
    ) -> Result<()> {
        if dest.is_file() && file_len(dest) == part.size && blake3_file(dest)? == part.blake3 {
            progress(part.size);
            return Ok(());
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let partial = dest.with_extension(format!(
            "{}.partial",
            dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
        ));

        let mut have = file_len(&partial);
        if have > part.size {
            std::fs::remove_file(&partial).map_err(|e| Error::io(&partial, e))?;
            have = 0;
        }

        if have < part.size {
            let mut req = self.agent.get(&part.url);
            if have > 0 {
                req = req.header("Range", format!("bytes={have}-"));
            }
            let resp = req.call().map_err(|e| Error::Download(format!("{}: {e}", part.url)))?;
            let status = resp.status().as_u16();
            let mut file = match status {
                206 if have > 0 => OpenOptions::new().append(true).open(&partial),
                200 => {
                    // Server ignored the range: start over.
                    have = 0;
                    File::create(&partial)
                }
                _ => return Err(Error::Download(format!("{}: HTTP {status}", part.url))),
            }
            .map_err(|e| Error::io(&partial, e))?;
            progress(have);

            let mut body = resp.into_body().into_reader();
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                if cancel.load(Ordering::Relaxed) {
                    return Err(Error::Cancelled);
                }
                let n = body.read(&mut buf).map_err(|e| Error::Download(format!("{}: {e}", part.url)))?;
                if n == 0 {
                    break;
                }
                file.write_all(&buf[..n]).map_err(|e| Error::io(&partial, e))?;
                progress(n as u64);
            }
            file.flush().map_err(|e| Error::io(&partial, e))?;
        } else {
            progress(have);
        }

        let got = file_len(&partial);
        if got != part.size {
            return Err(Error::Download(format!("{}: got {got} bytes, expected {}", part.url, part.size)));
        }
        let hash = blake3_file(&partial)?;
        if hash != part.blake3 {
            let _ = std::fs::remove_file(&partial);
            return Err(Error::Integrity(format!("{}: hash mismatch, file removed", part.url)));
        }
        std::fs::rename(&partial, dest).map_err(|e| Error::io(dest, e))
    }
}

fn file_len(p: &Path) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    /// Minimal HTTP server: serves `data` for every request, honoring
    /// `Range: bytes=N-`. Returns the base URL.
    fn serve(data: Vec<u8>, requests: usize) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let mut stream = stream.unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut start = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if line == "\r\n" || line.is_empty() {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        start = v.trim().trim_end_matches('-').parse().unwrap();
                    }
                }
                let body = &data[start..];
                let status = if start > 0 { "206 Partial Content" } else { "200 OK" };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        format!("http://{addr}/part")
    }

    fn part_for(data: &[u8], url: String) -> Part {
        Part { url, size: data.len() as u64, blake3: blake3::hash(data).to_hex().to_string() }
    }

    #[test]
    fn downloads_and_resumes() {
        let data: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("mod.tar.zst.001");
        let part = part_for(&data, serve(data.clone(), 1));

        // Simulate an interrupted earlier download.
        std::fs::write(dir.path().join("mod.tar.zst.001.partial"), &data[..30_000]).unwrap();

        let mut seen = 0;
        Downloader::new().fetch_part(&part, &dest, &AtomicBool::new(false), &mut |n| seen += n).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(seen, data.len() as u64);

        // Already complete: no request needed (server accepted only one).
        Downloader::new().fetch_part(&part, &dest, &AtomicBool::new(false), &mut |_| {}).unwrap();
    }

    #[test]
    fn rejects_corrupt_data() {
        let data = vec![7u8; 1000];
        let dir = tempfile::tempdir().unwrap();
        let mut part = part_for(&data, serve(data.clone(), 1));
        part.blake3 = "0".repeat(64);
        let err = Downloader::new()
            .fetch_part(&part, &dir.path().join("x.001"), &AtomicBool::new(false), &mut |_| {})
            .unwrap_err();
        assert!(matches!(err, Error::Integrity(_)), "{err}");
        assert!(!dir.path().join("x.001.partial").exists());
    }
}

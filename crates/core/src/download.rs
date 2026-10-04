//! HTTP downloads of package parts with resume, retries and hash verification.

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::hash::blake3_file;
use crate::manifest::Part;
use crate::{Error, Result};

const CHECK_ATTEMPTS: u32 = 4;
const CHECK_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Attempts in a row without a single new byte before a part download gives
/// up. With [`retry_delay`] that is a few minutes of lost connection; the
/// `.partial` file stays, so the next update continues from it.
const FETCH_ATTEMPTS: u32 = 10;

/// ureq has no idle timeout for reading a body, only a total one, and a
/// connection that died without a RST (Wi-Fi dropped, router rebooted) blocks
/// a read forever. So every GET may run this long and is then reopened with a
/// Range from where it stopped: costs a request every couple of minutes,
/// bounds a stall to the same time.
const REQUEST_WINDOW: Duration = Duration::from_secs(120);

/// What [`Downloader::fetch_part`] reports while it works.
#[derive(Debug, Clone, PartialEq)]
pub enum PartEvent {
    /// Bytes of the part on disk now. Can go down when the server ignores a
    /// Range and the part starts over.
    Bytes(u64),
    /// The connection failed; attempt `attempt` follows after `delay`.
    Retry { attempt: u32, delay: Duration, error: String },
}

/// One failed GET: worth repeating (network, timeout, server overload) or not.
enum Attempt {
    Transient(Error),
    Fatal(Error),
}

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
            .timeout_recv_response(Some(Duration::from_secs(30)))
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
    ///
    /// Connection failures are retried: hundreds of HEAD requests in a row
    /// regularly hit one connect timeout, and a single one must not stop a
    /// publish. An HTTP status or a wrong size is an answer and is not retried.
    pub fn check_part(&self, part: &Part) -> Result<()> {
        let mut attempt = 0;
        let resp = loop {
            match self.agent.head(&part.url).call() {
                Ok(resp) => break resp,
                Err(_) if attempt + 1 < CHECK_ATTEMPTS => {
                    attempt += 1;
                    std::thread::sleep(CHECK_RETRY_DELAY * attempt);
                }
                Err(e) => {
                    return Err(Error::Download(format!("{}: {e} (after {CHECK_ATTEMPTS} attempts)", part.url)));
                }
            }
        };
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
    ///
    /// A lost connection is retried with growing pauses for as long as each
    /// attempt brings new bytes, and up to [`FETCH_ATTEMPTS`] attempts when
    /// none do. Downloaded bytes are never thrown away except on a hash
    /// mismatch, so closing the launcher or losing the network costs nothing.
    pub fn fetch_part(
        &self,
        part: &Part,
        dest: &Path,
        cancel: &dyn Fn() -> bool,
        on: &mut dyn FnMut(PartEvent),
    ) -> Result<()> {
        if dest.is_file() && file_len(dest) == part.size && blake3_file(dest)? == part.blake3 {
            on(PartEvent::Bytes(part.size));
            return Ok(());
        }
        if let Some(dir) = dest.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        let partial = partial_path(dest);

        let mut failures = 0;
        loop {
            let before = file_len(&partial);
            match self.fetch_range(part, &partial, cancel, on) {
                Ok(()) => break,
                Err(Attempt::Fatal(e)) => return Err(e),
                // A window ran out or the connection broke after some data:
                // continue at once, it is not a failure of the network.
                Err(Attempt::Transient(_)) if file_len(&partial) > before => failures = 0,
                Err(Attempt::Transient(e)) => {
                    failures += 1;
                    if failures >= FETCH_ATTEMPTS {
                        return Err(Error::Download(format!("{e} (after {FETCH_ATTEMPTS} attempts)")));
                    }
                    let delay = retry_delay(failures);
                    on(PartEvent::Retry { attempt: failures + 1, delay, error: e.to_string() });
                    sleep(delay, cancel)?;
                }
            }
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

    /// One GET appending to `partial` until the part is complete, the
    /// connection fails or [`REQUEST_WINDOW`] runs out.
    fn fetch_range(
        &self,
        part: &Part,
        partial: &Path,
        cancel: &dyn Fn() -> bool,
        on: &mut dyn FnMut(PartEvent),
    ) -> Result<(), Attempt> {
        let transient = |e: &dyn std::fmt::Display| Attempt::Transient(Error::Download(format!("{}: {e}", part.url)));
        let mut have = file_len(partial);
        if have > part.size {
            std::fs::remove_file(partial).map_err(|e| Attempt::Fatal(Error::io(partial, e)))?;
            have = 0;
        }
        on(PartEvent::Bytes(have));
        if have == part.size {
            return Ok(());
        }
        if cancel() {
            return Err(Attempt::Fatal(Error::Cancelled));
        }

        let mut req = self.agent.get(&part.url);
        if have > 0 {
            req = req.header("Range", format!("bytes={have}-"));
        }
        let resp = req.config().timeout_recv_body(Some(REQUEST_WINDOW)).build().call().map_err(|e| transient(&e))?;
        let status = resp.status().as_u16();
        let resumed = status == 206 && have > 0;
        if resumed {
            // A range other than the one asked for would corrupt the part.
            let range = resp.headers().get("content-range").and_then(|v| v.to_str().ok());
            if range.is_some_and(|r| !r.trim().starts_with(&format!("bytes {have}-"))) {
                let _ = std::fs::remove_file(partial);
                return Err(transient(&format!("unexpected Content-Range {range:?}")));
            }
        }
        let file = match status {
            _ if resumed => OpenOptions::new().append(true).open(partial),
            200 => {
                // Server ignored the range: start over.
                have = 0;
                on(PartEvent::Bytes(0));
                File::create(partial)
            }
            408 | 429 | 500..=599 => return Err(transient(&format!("HTTP {status}"))),
            _ => return Err(Attempt::Fatal(Error::Download(format!("{}: HTTP {status}", part.url)))),
        };
        let mut file = file.map_err(|e| Attempt::Fatal(Error::io(partial, e)))?;

        let mut body = resp.into_body().into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        let result = loop {
            if cancel() {
                break Err(Attempt::Fatal(Error::Cancelled));
            }
            match body.read(&mut buf) {
                Ok(0) => break Ok(()),
                Ok(n) => {
                    if let Err(e) = file.write_all(&buf[..n]) {
                        break Err(Attempt::Fatal(Error::io(partial, e)));
                    }
                    have += n as u64;
                    on(PartEvent::Bytes(have));
                }
                Err(e) => break Err(transient(&e)),
            }
        };
        file.flush().map_err(|e| Attempt::Fatal(Error::io(partial, e)))?;
        result?;
        if have < part.size {
            // The server closed the body early.
            return Err(transient(&format!("connection closed at {have} of {} bytes", part.size)));
        }
        Ok(())
    }
}

/// `<dest>.partial`: where [`Downloader::fetch_part`] collects `dest`.
pub fn partial_path(dest: &Path) -> PathBuf {
    dest.with_extension(format!(
        "{}.partial",
        dest.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default()
    ))
}

/// 2, 4, 8, 16, then 30 seconds.
fn retry_delay(failures: u32) -> Duration {
    Duration::from_secs((1u64 << failures.min(5)).min(30))
}

fn sleep(d: Duration, cancel: &dyn Fn() -> bool) -> Result<()> {
    let until = std::time::Instant::now() + d;
    while std::time::Instant::now() < until {
        if cancel() {
            return Err(Error::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub(crate) fn file_len(p: &Path) -> u64 {
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

        let mut seen = Vec::new();
        Downloader::new().fetch_part(&part, &dest, &|| false, &mut |e| seen.push(e)).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(seen.first(), Some(&PartEvent::Bytes(30_000)));
        assert_eq!(seen.last(), Some(&PartEvent::Bytes(data.len() as u64)));

        // Already complete: no request needed (server accepted only one).
        Downloader::new().fetch_part(&part, &dest, &|| false, &mut |_| {}).unwrap();
    }

    /// The connection breaks twice mid-body, then the server is unreachable
    /// for one attempt: the part still completes, nothing is downloaded twice.
    #[test]
    fn survives_lost_connection() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let served = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = served.clone();
        let body = data.clone();
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().enumerate() {
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
                log.lock().unwrap().push(start);
                if i == 2 {
                    continue; // no answer at all
                }
                let rest = &body[start..];
                let status = if start > 0 { "206 Partial Content" } else { "200 OK" };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", rest.len()).unwrap();
                // The first two answers stop after 100 KB without closing cleanly.
                let n = if i < 2 { 100_000 } else { rest.len() };
                stream.write_all(&rest[..n]).unwrap();
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("p.001");
        let part = part_for(&data, format!("http://{addr}/part"));
        let mut retries = 0;
        Downloader::new()
            .fetch_part(&part, &dest, &|| false, &mut |e| {
                if matches!(e, PartEvent::Retry { .. }) {
                    retries += 1;
                }
            })
            .unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), data);
        assert_eq!(*served.lock().unwrap(), [0, 100_000, 200_000, 200_000]);
        assert_eq!(retries, 1, "only the attempt without data waits");
    }

    #[test]
    fn check_retries_dropped_connections_but_not_answers() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for (i, stream) in listener.incoming().enumerate() {
                let mut stream = stream.unwrap();
                if i < 2 {
                    continue; // dropped without an answer
                }
                let mut line = String::new();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                while reader.read_line(&mut line).unwrap() > 2 {
                    line.clear();
                }
                let len = if i == 2 { 1000 } else { 999 };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n").unwrap();
            }
        });
        let part = Part { url: format!("http://{addr}/part"), size: 1000, blake3: String::new() };
        Downloader::new().check_part(&part).unwrap();
        // A wrong size is an answer: reported at once, not retried into success.
        let err = Downloader::new().check_part(&part).unwrap_err();
        assert!(err.to_string().contains("999 bytes, expected 1000"), "{err}");
    }

    #[test]
    fn rejects_corrupt_data() {
        let data = vec![7u8; 1000];
        let dir = tempfile::tempdir().unwrap();
        let mut part = part_for(&data, serve(data.clone(), 1));
        part.blake3 = "0".repeat(64);
        let err = Downloader::new()
            .fetch_part(&part, &dir.path().join("x.001"), &|| false, &mut |_| {})
            .unwrap_err();
        assert!(matches!(err, Error::Integrity(_)), "{err}");
        assert!(!dir.path().join("x.001.partial").exists());
    }
}

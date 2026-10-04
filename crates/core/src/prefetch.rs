//! Parallel download of the packages an update needs.
//!
//! Every mod is its own release asset, and one HTTP stream from GitHub's
//! storage rarely uses the whole line, especially on the many small mods. So
//! [`PARALLEL_DOWNLOADS`] workers fetch parts in plan order while the installer
//! unpacks packages one by one, in the same order, as soon as each is complete.
//! Install order and `state.json` handling stay exactly as without prefetching.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::Duration;

use crate::download::{self, Downloader, PartEvent};
use crate::manifest::{Package, Part};
use crate::{Error, Result};

pub const PARALLEL_DOWNLOADS: usize = 4;

/// How far downloads may run ahead of the package being installed. Bounds the
/// extra disk space the cache takes (parts are deleted once unpacked); larger
/// than a part, so a package is never starved of its own parts.
const LOOKAHEAD: u64 = 4 << 30;

/// The downloads of one package, in plan order.
pub(crate) struct Job {
    pub parts: Vec<(Part, PathBuf)>,
}

impl Job {
    /// Parts of `pkg` cached under `name` (a mod id or `base`). Names are stable
    /// across launches and builds, so an interrupted download is found again.
    pub fn new(pkg: &Package, name: &str, cache: &Path) -> Self {
        let parts = pkg
            .parts
            .iter()
            .enumerate()
            .map(|(i, part)| {
                let path = cache.join(format!("{name}-{}.{:03}", &pkg.hash[..pkg.hash.len().min(16)], i + 1));
                (part.clone(), path)
            })
            .collect();
        Self { parts }
    }

    pub fn paths(&self) -> Vec<PathBuf> {
        self.parts.iter().map(|(_, p)| p.clone()).collect()
    }
}

/// Bytes of a part already on disk, finished or not. Not hash-checked: an
/// estimate for progress; [`Downloader::fetch_part`] verifies.
pub(crate) fn cached_len(part: &Part, path: &Path) -> u64 {
    if download::file_len(path) == part.size {
        return part.size;
    }
    download::file_len(&download::partial_path(path)).min(part.size)
}

/// Download progress for the UI, from all workers.
pub(crate) enum Report {
    Bytes { done: u64, total: u64 },
    Retry { attempt: u32, delay: Duration, error: String },
}

#[derive(Clone, Copy, PartialEq)]
enum Status {
    Pending,
    Done,
    Failed,
}

/// One part, in plan order.
struct Slot {
    job: usize,
    /// Index in the job's parts.
    index: usize,
    /// Bytes of all parts before this one.
    offset: u64,
}

struct Shared {
    /// Next slot a worker takes.
    next: usize,
    /// The job the installer waits for or unpacks.
    current: usize,
    /// Per slot; slots from `next` on are not started.
    status: Vec<Status>,
    /// A download failed: workers take nothing new.
    failed: bool,
    /// The first failure, until the installer picks it up.
    error: Option<Error>,
    done_bytes: u64,
}

pub(crate) struct Prefetch<'a> {
    jobs: &'a [Job],
    slots: Vec<Slot>,
    job_offsets: Vec<u64>,
    total: u64,
    shared: Mutex<Shared>,
    changed: Condvar,
    /// Set when the installer is finished with the downloads, successfully or not.
    stop: AtomicBool,
    cancel: &'a AtomicBool,
}

impl<'a> Prefetch<'a> {
    pub fn new(jobs: &'a [Job], total: u64, cancel: &'a AtomicBool) -> Self {
        let mut slots = Vec::new();
        let mut job_offsets = Vec::with_capacity(jobs.len());
        let mut offset = 0;
        for (j, job) in jobs.iter().enumerate() {
            job_offsets.push(offset);
            for (index, (part, _)) in job.parts.iter().enumerate() {
                slots.push(Slot { job: j, index, offset });
                offset += part.size;
            }
        }
        let done_bytes = jobs.iter().flat_map(|j| &j.parts).map(|(part, path)| cached_len(part, path)).sum();
        let status = vec![Status::Pending; slots.len()];
        let shared = Mutex::new(Shared { next: 0, current: 0, status, failed: false, error: None, done_bytes });
        Self { jobs, slots, job_offsets, total, shared, changed: Condvar::new(), stop: AtomicBool::new(false), cancel }
    }

    fn part(&self, slot: usize) -> &(Part, PathBuf) {
        let s = &self.slots[slot];
        &self.jobs[s.job].parts[s.index]
    }

    fn cancelled(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || self.cancel.load(Ordering::Relaxed)
    }

    /// Download progress as it stands, for the first event of an update.
    pub fn bytes(&self) -> Report {
        Report::Bytes { done: self.shared.lock().unwrap().done_bytes, total: self.total }
    }

    /// Takes parts in order and downloads them until none are left, one
    /// fails, or [`Self::finish`] is called.
    pub fn worker(&self, downloader: &Downloader, report: &(dyn Fn(Report) + Sync)) {
        while let Some(slot) = self.take() {
            let (part, path) = self.part(slot);
            let mut last = cached_len(part, path);
            let result = downloader.fetch_part(part, path, &|| self.cancelled(), &mut |e| match e {
                PartEvent::Bytes(n) => {
                    let done = {
                        let mut s = self.shared.lock().unwrap();
                        s.done_bytes = (s.done_bytes + n).saturating_sub(last);
                        s.done_bytes
                    };
                    last = n;
                    report(Report::Bytes { done, total: self.total });
                }
                PartEvent::Retry { attempt, delay, error } => report(Report::Retry { attempt, delay, error }),
            });
            let mut s = self.shared.lock().unwrap();
            match result {
                Ok(()) => s.status[slot] = Status::Done,
                // The other workers finish what they have, the installer
                // installs what is complete and then reports the error.
                Err(e) => {
                    s.status[slot] = Status::Failed;
                    if !s.failed && !self.stop.load(Ordering::Relaxed) {
                        s.failed = true;
                        s.error = Some(e);
                    }
                }
            }
            drop(s);
            self.changed.notify_all();
        }
    }

    fn take(&self) -> Option<usize> {
        let mut s = self.shared.lock().unwrap();
        loop {
            if s.failed || self.cancelled() || s.next >= self.slots.len() {
                return None;
            }
            let slot = &self.slots[s.next];
            if slot.job <= s.current || slot.offset < self.job_offsets[s.current] + LOOKAHEAD {
                s.next += 1;
                return Some(s.next - 1);
            }
            s = self.changed.wait_timeout(s, Duration::from_millis(200)).unwrap().0;
        }
    }

    /// Blocks until every part of `job` is downloaded. Jobs must be awaited in order.
    pub fn wait(&self, job: usize) -> Result<()> {
        let mut s = self.shared.lock().unwrap();
        s.current = job;
        self.changed.notify_all();
        loop {
            let mine = || (0..self.slots.len()).filter(|&i| self.slots[i].job == job);
            if mine().all(|i| s.status[i] == Status::Done) {
                return Ok(());
            }
            if self.cancel.load(Ordering::Relaxed) {
                return Err(Error::Cancelled);
            }
            // Wait for parts still downloading, but not for ones nobody will start.
            if mine().any(|i| s.status[i] == Status::Failed || (s.failed && i >= s.next)) {
                return Err(s.error.take().unwrap_or(Error::Cancelled));
            }
            s = self.changed.wait_timeout(s, Duration::from_millis(100)).unwrap().0;
        }
    }

    /// Stops the workers; downloads in progress keep their `.partial` files.
    pub fn finish(&self) {
        self.stop.store(true, Ordering::Relaxed);
        self.changed.notify_all();
    }
}

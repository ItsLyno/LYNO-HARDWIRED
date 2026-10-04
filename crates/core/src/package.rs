//! Packages: a folder as a `tar.zst` stream split into release-sized parts.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use crate::tree::{self, FileEntry};
use crate::{Error, Result};

/// GitHub rejects release assets of 2 GiB and more; stay well below.
pub const DEFAULT_PART_SIZE: u64 = 1900 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedPart {
    pub file_name: String,
    pub path: PathBuf,
    pub size: u64,
    pub blake3: String,
}

pub struct PackOptions {
    pub part_size: u64,
    pub zstd_level: i32,
}

impl Default for PackOptions {
    fn default() -> Self {
        // Cyberpunk .archive files are already Oodle-compressed; high zstd
        // levels cost a lot of time for little gain.
        Self { part_size: DEFAULT_PART_SIZE, zstd_level: 6 }
    }
}

/// Packs `files` (relative to `root`) into `<out_dir>/<name>.tar.zst.001`, `.002`, …
pub fn pack(root: &Path, files: &[FileEntry], out_dir: &Path, name: &str, opts: &PackOptions) -> Result<Vec<PackedPart>> {
    std::fs::create_dir_all(out_dir).map_err(|e| Error::io(out_dir, e))?;
    let writer = SplitWriter::new(out_dir, name, opts.part_size);
    let mut encoder = zstd::Encoder::new(writer, opts.zstd_level).map_err(|e| Error::io(out_dir, e))?;
    // Multithreaded compression when the platform supports it.
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get() as u32);
    let _ = encoder.multithread(threads);

    let mut builder = tar::Builder::new(encoder);
    builder.mode(tar::HeaderMode::Deterministic);
    for f in files {
        let src = tree::from_slash(root, &f.path);
        builder.append_path_with_name(&src, &f.path).map_err(|e| Error::io(&src, e))?;
    }
    let encoder = builder.into_inner().map_err(|e| Error::io(out_dir, e))?;
    let writer = encoder.finish().map_err(|e| Error::io(out_dir, e))?;
    writer.finish()
}

/// Unpacks a package from its downloaded parts into `dest` (created fresh)
/// and checks the result against the expected tree hash.
pub fn unpack(parts: &[PathBuf], dest: &Path, expected_hash: &str) -> Result<()> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| Error::io(dest, e))?;
    }
    std::fs::create_dir_all(dest).map_err(|e| Error::io(dest, e))?;

    let reader = ChainReader::open(parts)?;
    let decoder = zstd::Decoder::new(reader).map_err(|e| Error::io(dest, e))?;
    // `unpack` refuses entries that escape `dest` (absolute paths, `..`).
    tar::Archive::new(decoder).unpack(dest).map_err(|e| Error::io(dest, e))?;

    let got = tree::tree_hash(dest)?;
    if got.hash != expected_hash {
        return Err(Error::Integrity(format!(
            "{}: unpacked content hash {} != expected {expected_hash}",
            dest.display(),
            got.hash
        )));
    }
    Ok(())
}

/// Replaces `target` with `staging` so a failure never leaves a half-updated mod.
pub fn swap_folder(staging: &Path, target: &Path) -> Result<()> {
    let old = target.with_extension("lyno-old");
    if old.exists() {
        std::fs::remove_dir_all(&old).map_err(|e| Error::io(&old, e))?;
    }
    if target.exists() {
        std::fs::rename(target, &old).map_err(|e| Error::io(target, e))?;
    }
    if let Err(e) = std::fs::rename(staging, target) {
        // Put the previous version back.
        if old.exists() {
            let _ = std::fs::rename(&old, target);
        }
        return Err(Error::io(target, e));
    }
    if old.exists() {
        std::fs::remove_dir_all(&old).map_err(|e| Error::io(&old, e))?;
    }
    Ok(())
}

/// Moves every file of `staging` into `target`, overwriting existing files.
/// Used for the base package, which shares the instance root with mods.
pub fn merge_into(staging: &Path, target: &Path) -> Result<()> {
    for f in tree::list_files(staging)? {
        let from = tree::from_slash(staging, &f.path);
        let to = tree::from_slash(target, &f.path);
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::io(dir, e))?;
        }
        std::fs::rename(&from, &to).map_err(|e| Error::io(&to, e))?;
    }
    std::fs::remove_dir_all(staging).map_err(|e| Error::io(staging, e))
}

struct SplitWriter {
    dir: PathBuf,
    name: String,
    part_size: u64,
    current: Option<(BufWriter<File>, blake3::Hasher, u64)>,
    done: Vec<PackedPart>,
}

impl SplitWriter {
    fn new(dir: &Path, name: &str, part_size: u64) -> Self {
        Self { dir: dir.to_owned(), name: name.to_owned(), part_size, current: None, done: Vec::new() }
    }

    fn part_name(&self, index: usize) -> String {
        format!("{}.tar.zst.{:03}", self.name, index + 1)
    }

    fn close_current(&mut self) -> io::Result<()> {
        if let Some((mut w, hasher, size)) = self.current.take() {
            w.flush()?;
            let file_name = self.part_name(self.done.len());
            self.done.push(PackedPart {
                path: self.dir.join(&file_name),
                file_name,
                size,
                blake3: hasher.finalize().to_hex().to_string(),
            });
        }
        Ok(())
    }

    fn finish(mut self) -> Result<Vec<PackedPart>> {
        // An empty mod still produces one (tiny) part.
        if self.current.is_none() && self.done.is_empty() {
            self.open_next().map_err(|e| Error::io(&self.dir, e))?;
        }
        self.close_current().map_err(|e| Error::io(&self.dir, e))?;
        Ok(self.done)
    }

    fn open_next(&mut self) -> io::Result<()> {
        let path = self.dir.join(self.part_name(self.done.len()));
        self.current = Some((BufWriter::new(File::create(path)?), blake3::Hasher::new(), 0));
        Ok(())
    }
}

impl Write for SplitWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        if self.current.as_ref().is_some_and(|(_, _, size)| *size >= self.part_size) {
            self.close_current()?;
        }
        if self.current.is_none() {
            self.open_next()?;
        }
        let (w, hasher, size) = self.current.as_mut().expect("part is open");
        let room = (self.part_size - *size) as usize;
        let n = w.write(&buf[..buf.len().min(room)])?;
        hasher.update(&buf[..n]);
        *size += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match &mut self.current {
            Some((w, _, _)) => w.flush(),
            None => Ok(()),
        }
    }
}

/// Reads several files back to back as one stream.
struct ChainReader {
    files: std::vec::IntoIter<PathBuf>,
    current: Option<BufReader<File>>,
}

impl ChainReader {
    fn open(parts: &[PathBuf]) -> Result<Self> {
        for p in parts {
            if !p.is_file() {
                return Err(Error::io(p, io::Error::new(io::ErrorKind::NotFound, "package part missing")));
            }
        }
        Ok(Self { files: parts.to_vec().into_iter(), current: None })
    }
}

impl Read for ChainReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        loop {
            if self.current.is_none() {
                match self.files.next() {
                    Some(p) => self.current = Some(BufReader::with_capacity(1 << 20, File::open(p)?)),
                    None => return Ok(0),
                }
            }
            let n = self.current.as_mut().expect("file is open").read(buf)?;
            if n > 0 || buf.is_empty() {
                return Ok(n);
            }
            self.current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::{list_files, tree_hash};

    fn sample_mod(root: &Path) {
        let big: Vec<u8> = (0..200_000u32).flat_map(|i| i.wrapping_mul(2654435761).to_le_bytes()).collect();
        std::fs::create_dir_all(root.join("archive/pc/mod")).unwrap();
        std::fs::write(root.join("archive/pc/mod/big.archive"), &big).unwrap();
        std::fs::write(root.join("meta.ini"), "[General]\nmodid=1\n").unwrap();
    }

    #[test]
    fn pack_split_unpack_roundtrip() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        sample_mod(src.path());
        let expected = tree_hash(src.path()).unwrap();

        let opts = PackOptions { part_size: 64 * 1024, zstd_level: 1 };
        let parts = pack(src.path(), &list_files(src.path()).unwrap(), out.path(), "mod-abc", &opts).unwrap();
        assert!(parts.len() > 1, "expected a split, got {parts:?}");
        assert_eq!(parts[0].file_name, "mod-abc.tar.zst.001");
        for p in &parts {
            assert!(p.size <= opts.part_size);
            assert_eq!(std::fs::metadata(&p.path).unwrap().len(), p.size);
            assert_eq!(crate::hash::blake3_file(&p.path).unwrap(), p.blake3);
        }

        let dest = out.path().join("unpacked");
        let paths: Vec<_> = parts.iter().map(|p| p.path.clone()).collect();
        unpack(&paths, &dest, &expected.hash).unwrap();
        assert_eq!(tree_hash(&dest).unwrap(), expected);

        // A wrong expected hash is caught.
        assert!(matches!(unpack(&paths, &dest, "nope"), Err(Error::Integrity(_))));
    }

    #[test]
    fn swap_replaces_folder() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Mod");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("old.txt"), "old").unwrap();
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("new.txt"), "new").unwrap();

        swap_folder(&staging, &target).unwrap();
        assert!(target.join("new.txt").exists());
        assert!(!target.join("old.txt").exists());
        assert!(!staging.exists());
        assert!(!dir.path().join("Mod.lyno-old").exists());
    }

    #[test]
    fn merge_overwrites_and_keeps_other_files() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("instance");
        let staging = dir.path().join("staging");
        std::fs::create_dir_all(root.join("mods/Keep")).unwrap();
        std::fs::write(root.join("ModOrganizer.ini"), "old").unwrap();
        std::fs::create_dir_all(staging.join("profiles/LYNO")).unwrap();
        std::fs::write(staging.join("ModOrganizer.ini"), "new").unwrap();
        std::fs::write(staging.join("profiles/LYNO/modlist.txt"), "+A").unwrap();

        merge_into(&staging, &root).unwrap();
        assert_eq!(std::fs::read_to_string(root.join("ModOrganizer.ini")).unwrap(), "new");
        assert!(root.join("profiles/LYNO/modlist.txt").exists());
        assert!(root.join("mods/Keep").exists());
    }
}

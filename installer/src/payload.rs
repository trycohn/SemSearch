//! The files to install. A release setup exe carries them appended at its end, before its Authenticode
//! signature if it is signed (layout and locator: trailer.rs):
//!
//!     [exe][zip][u64 zip length, little-endian]["SSPAYLD1"]
//!
//! The zip holds `app/<repository-relative path>` for every program file (by installer/payload.txt),
//! `python.tar.gz` (python-build-standalone install_only, stored), `uninstall.exe` (the bare setup exe, signed
//! on its own when the build signs) and `payload.json`
//! (`{"python": "<archive name>", "python_sha256": "<hex>", "files": N}`).
//!
//! For development, SEMSEARCH_PAYLOAD_DIR=<repository root> makes the installer read the program files straight
//! from the repository (by the same payload.txt) and Python from installer\.cache\ (by installer/python.txt).

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::manifest::{self, PythonPin};

pub const APP_PREFIX: &str = "app/";
pub const RUNTIME_ENTRY: &str = "python.tar.gz";
pub const INFO_ENTRY: &str = "payload.json";
pub const UNINSTALL_ENTRY: &str = "uninstall.exe";

pub enum Payload {
    /// Appended to the exe: the zip spans [offset, offset + len).
    Embedded { exe: PathBuf, offset: u64, len: u64 },
    /// SEMSEARCH_PAYLOAD_DIR: a repository checkout.
    Repo { dir: PathBuf },
    /// A bare build without a payload: can uninstall, cannot install.
    None { exe: PathBuf },
}

/// Read side of the embedded zip: a window into the exe file.
struct Slice {
    f: File,
    start: u64,
    len: u64,
    pos: u64,
}

impl Read for Slice {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let left = self.len.saturating_sub(self.pos);
        if left == 0 {
            return Ok(0);
        }
        let n = (buf.len() as u64).min(left) as usize;
        self.f.seek(SeekFrom::Start(self.start + self.pos))?;
        let n = self.f.read(&mut buf[..n])?;
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for Slice {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let p = match to {
            SeekFrom::Start(p) => p as i128,
            SeekFrom::End(d) => self.len as i128 + d as i128,
            SeekFrom::Current(d) => self.pos as i128 + d as i128,
        };
        if p < 0 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "seek before start"));
        }
        self.pos = p as u64;
        Ok(self.pos)
    }
}

/// Counts the bytes read through it (progress of the compressed Python archive).
struct Counting<'a, R> {
    inner: R,
    count: &'a mut dyn FnMut(u64),
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        (self.count)(n as u64);
        Ok(n)
    }
}

type Zip = zip::ZipArchive<Slice>;

/// `rel` as a safe relative path (no root, drive or `..`).
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let p = Path::new(rel);
    let ok = p.components().all(|c| matches!(c, Component::Normal(_)));
    (ok && !rel.is_empty()).then(|| p.to_path_buf())
}

/// Write a file, retrying a few times (antivirus scans and just-closed processes hold files briefly).
fn write_retry(dest: &Path, mut fill: impl FnMut(&mut File) -> io::Result<()>) -> io::Result<()> {
    if let Some(d) = dest.parent() {
        fs::create_dir_all(d)?;
    }
    let mut last = None;
    for _ in 0..6 {
        let r = File::create(dest).and_then(|mut f| {
            fill(&mut f)?;
            f.flush()
        });
        match r {
            Ok(()) => return Ok(()),
            // disk full: no point in retrying
            Err(e) if matches!(e.raw_os_error(), Some(39) | Some(112)) => return Err(e),
            Err(e) => last = Some(e),
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    Err(last.unwrap())
}

impl Payload {
    pub fn detect() -> Payload {
        let exe = std::env::current_exe().unwrap_or_default();
        if let Some(dir) = std::env::var_os("SEMSEARCH_PAYLOAD_DIR").filter(|d| !d.is_empty()) {
            return Payload::Repo { dir: PathBuf::from(dir) };
        }
        match crate::trailer::locate(&exe) {
            Some((offset, len)) => Payload::Embedded { exe, offset, len },
            None => Payload::None { exe },
        }
    }

    pub fn describe(&self) -> String {
        match self {
            Payload::Embedded { exe, offset, len } => format!("embedded in {} at {offset} ({len} bytes)", exe.display()),
            Payload::Repo { dir } => format!("repository {} (SEMSEARCH_PAYLOAD_DIR)", dir.display()),
            Payload::None { exe } => format!("none ({} has no payload)", exe.display()),
        }
    }

    pub fn available(&self) -> bool {
        !matches!(self, Payload::None { .. })
    }

    fn zip(&self) -> io::Result<Zip> {
        let Payload::Embedded { exe, offset, len } = self else {
            return Err(io::Error::other("no embedded payload"));
        };
        let slice = Slice { f: File::open(exe)?, start: *offset, len: *len, pos: 0 };
        zip::ZipArchive::new(slice).map_err(io::Error::other)
    }

    /// `<root>\uninstall.exe`: the payload's `uninstall.exe` (keeps its own signature); without one (development,
    /// SEMSEARCH_PAYLOAD_DIR, or an older payload) this exe without its payload.
    pub fn write_uninstaller(&self, dest: &Path) -> io::Result<()> {
        if let Payload::Embedded { .. } = self {
            let mut z = self.zip()?;
            if let Ok(mut f) = z.by_name(UNINSTALL_ENTRY) {
                let mut data = Vec::with_capacity(f.size() as usize);
                f.read_to_end(&mut data)?;
                return write_retry(dest, |out| out.write_all(&data));
            };
        }
        self.write_bare_exe(dest)
    }

    /// The setup exe without its payload.
    fn write_bare_exe(&self, dest: &Path) -> io::Result<()> {
        let (exe, keep) = match self {
            Payload::Embedded { exe, offset, .. } => (exe.clone(), Some(*offset)),
            Payload::Repo { .. } | Payload::None { .. } => (std::env::current_exe()?, None),
        };
        let mut src = File::open(&exe)?;
        write_retry(dest, |f| {
            src.seek(SeekFrom::Start(0))?;
            match keep {
                Some(n) => io::copy(&mut (&mut src).take(n), f)?,
                None => io::copy(&mut src, f)?,
            };
            Ok(())
        })
    }

    /// A text file of the program (e.g. requirements-torch.txt).
    pub fn read_text(&self, rel: &str) -> io::Result<String> {
        match self {
            Payload::Repo { dir } => fs::read_to_string(dir.join(rel)),
            Payload::Embedded { .. } => {
                let mut z = self.zip()?;
                let mut f = z.by_name(&format!("{APP_PREFIX}{rel}")).map_err(|e| io::Error::other(format!("{rel}: {e}")))?;
                let mut s = String::new();
                f.read_to_string(&mut s)?;
                Ok(s)
            }
            Payload::None { .. } => Err(io::Error::other("no payload")),
        }
    }

    /// (relative path, size) of every program file.
    pub fn app_files(&self) -> io::Result<Vec<(String, u64)>> {
        match self {
            Payload::Repo { dir } => {
                let list = manifest::payload_files(dir).map_err(io::Error::other)?;
                Ok(list
                    .into_iter()
                    .map(|rel| {
                        let n = fs::metadata(dir.join(&rel)).map(|m| m.len()).unwrap_or(0);
                        (rel, n)
                    })
                    .collect())
            }
            Payload::Embedded { .. } => {
                let mut z = self.zip()?;
                let mut out = Vec::new();
                for i in 0..z.len() {
                    let f = z.by_index_raw(i).map_err(io::Error::other)?;
                    if let Some(rel) = f.name().strip_prefix(APP_PREFIX) {
                        if !f.is_dir() {
                            out.push((rel.to_string(), f.size()));
                        }
                    }
                }
                Ok(out)
            }
            Payload::None { .. } => Err(io::Error::other("no payload")),
        }
    }

    /// Copy every program file into `root`. `progress(bytes)` gets the bytes written since its last call.
    pub fn extract_app(&self, root: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> io::Result<()> {
        let cancelled = || io::Error::new(io::ErrorKind::Interrupted, "cancelled");
        match self {
            Payload::Repo { dir } => {
                for (rel, _) in self.app_files()? {
                    if cancel.load(Ordering::SeqCst) {
                        return Err(cancelled());
                    }
                    let rp = safe_rel(&rel).ok_or_else(|| io::Error::other(format!("bad path {rel}")))?;
                    let dest = root.join(&rp);
                    let src = dir.join(&rp);
                    write_retry(&dest, |f| {
                        let mut s = File::open(&src)?;
                        let n = io::copy(&mut s, f)?;
                        progress(n);
                        Ok(())
                    })
                    .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", dest.display())))?;
                }
                Ok(())
            }
            Payload::Embedded { .. } => {
                let mut z = self.zip()?;
                for i in 0..z.len() {
                    if cancel.load(Ordering::SeqCst) {
                        return Err(cancelled());
                    }
                    let mut f = z.by_index(i).map_err(io::Error::other)?;
                    let Some(rel) = f.name().strip_prefix(APP_PREFIX).map(str::to_string) else { continue };
                    if f.is_dir() {
                        continue;
                    }
                    let rp = safe_rel(&rel).ok_or_else(|| io::Error::other(format!("bad path {rel}")))?;
                    let dest = root.join(&rp);
                    // the zip entry can be read once only: buffer it for the retries
                    let mut data = Vec::with_capacity(f.size() as usize);
                    f.read_to_end(&mut data)?;
                    write_retry(&dest, |out| out.write_all(&data))
                        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", dest.display())))?;
                    progress(data.len() as u64);
                }
                Ok(())
            }
            Payload::None { .. } => Err(io::Error::other("no payload")),
        }
    }

    /// (sha256, compressed size) of the Python archive: identifies the runtime in `runtime\.payload-sha256`.
    pub fn runtime_info(&self) -> io::Result<(String, u64)> {
        match self {
            Payload::Repo { dir } => {
                let pin = PythonPin::read(dir).map_err(io::Error::other)?;
                let path = pin.cache_path(dir);
                let n = fs::metadata(&path)
                    .map_err(|e| io::Error::new(e.kind(), format!("{}: {e} (run build-installer.cmd once to fetch Python)", path.display())))?
                    .len();
                Ok((pin.sha256, n))
            }
            Payload::Embedded { .. } => {
                let mut z = self.zip()?;
                let mut s = String::new();
                z.by_name(INFO_ENTRY).map_err(io::Error::other)?.read_to_string(&mut s)?;
                let v: serde_json::Value = serde_json::from_str(&s).map_err(io::Error::other)?;
                let sha = v.get("python_sha256").and_then(|x| x.as_str()).unwrap_or_default().to_string();
                let n = z.by_name(RUNTIME_ENTRY).map_err(io::Error::other)?.size();
                Ok((sha, n))
            }
            Payload::None { .. } => Err(io::Error::other("no payload")),
        }
    }

    /// Unpack the Python archive (`python/...`) into `dest` (`<root>\runtime`). `progress(bytes)` gets the
    /// compressed bytes consumed since its last call.
    pub fn extract_runtime(&self, dest: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> io::Result<()> {
        match self {
            Payload::Repo { dir } => {
                let pin = PythonPin::read(dir).map_err(io::Error::other)?;
                let path = pin.cache_path(dir);
                let got = crate::net::sha256_file(&path)?;
                if got != pin.sha256 {
                    return Err(io::Error::other(format!("{}: sha256 {got}, expected {}", path.display(), pin.sha256)));
                }
                untar(File::open(&path)?, dest, cancel, progress)
            }
            Payload::Embedded { .. } => {
                let mut z = self.zip()?;
                let f = z.by_name(RUNTIME_ENTRY).map_err(io::Error::other)?;
                untar(f, dest, cancel, progress)
            }
            Payload::None { .. } => Err(io::Error::other("no payload")),
        }
    }
}

/// Unpack a python-build-standalone install_only .tar.gz: entries `python/<rel>` go to `dest\<rel>`.
fn untar(src: impl Read, dest: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(u64)) -> io::Result<()> {
    let counting = Counting { inner: src, count: progress };
    let gz = flate2::read::GzDecoder::new(io::BufReader::with_capacity(1 << 20, counting));
    let mut ar = tar::Archive::new(gz);
    fs::create_dir_all(dest)?;
    for entry in ar.entries()? {
        if cancel.load(Ordering::SeqCst) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "cancelled"));
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        let mut comps = path.components();
        // the top folder "python"
        comps.next();
        let rel: PathBuf = comps.collect();
        if rel.as_os_str().is_empty() {
            continue;
        }
        if !rel.components().all(|c| matches!(c, Component::Normal(_))) {
            return Err(io::Error::other(format!("bad path in the Python archive: {}", path.display())));
        }
        let out = dest.join(&rel);
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            fs::create_dir_all(&out)?;
        } else if kind.is_file() {
            if let Some(d) = out.parent() {
                fs::create_dir_all(d)?;
            }
            let mut f = File::create(&out).map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", out.display())))?;
            io::copy(&mut entry, &mut f)?;
        }
        // links and other entry types do not occur in the Windows builds
    }
    Ok(())
}

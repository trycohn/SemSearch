//! Build tool for build-installer.cmd (docs/INSTALLER.md, "Сборка"):
//!
//!     pack python <repo>                         download the Python of installer/python.txt into
//!                                                installer\.cache\ (resumes; checks the sha256)
//!     pack list   <repo>                         print the program files that installer/payload.txt selects
//!     pack zip    <repo> <out.zip> [<uninstall.exe>]
//!                                                build the payload zip (with uninstall.exe: the bare setup exe,
//!                                                already signed when the build signs)
//!     pack append <setup.exe> <zip> <out.exe>    [exe][zip][u64 zip length LE]["SSPAYLD1"] (sign <out.exe> after)
//!     pack info   <setup.exe>                    find the payload (also behind a signature) and list it
//!
//! installer/payload.txt: one glob per line relative to the repository root, `#` comments, `!glob` excludes,
//! `dir/` = everything under dir; every include line must match a file (details in src/manifest.rs).

#[path = "../manifest.rs"]
mod manifest;
#[path = "../net.rs"]
#[allow(dead_code)]
mod net;
#[path = "../trailer.rs"]
mod trailer;

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use manifest::PythonPin;
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

use trailer::MAGIC;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let r = match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["python", repo] => python(Path::new(repo)),
        ["list", repo] => list(Path::new(repo)),
        ["zip", repo, out] => make_zip(Path::new(repo), Path::new(out), None),
        ["zip", repo, out, un] => make_zip(Path::new(repo), Path::new(out), Some(Path::new(un))),
        ["append", exe, zip, out] => append(Path::new(exe), Path::new(zip), Path::new(out)),
        ["info", exe] => info(Path::new(exe)),
        _ => Err("usage: pack python <repo> | list <repo> | zip <repo> <out.zip> [<uninstall.exe>] \
                  | append <setup.exe> <zip> <out.exe> | info <setup.exe>"
            .into()),
    };
    if let Err(e) = r {
        eprintln!("pack: {e}");
        std::process::exit(1);
    }
}

fn python(repo: &Path) -> Result<(), String> {
    let pin = PythonPin::read(repo)?;
    let dest = pin.cache_path(repo);
    println!("Python: {}", pin.file_name());
    let never = AtomicBool::new(false);
    let mut last = Instant::now();
    net::download(
        &pin.url,
        &dest,
        Some(&pin.sha256),
        &never,
        &mut |got, total| {
            if last.elapsed().as_secs() >= 2 {
                last = Instant::now();
                match total {
                    Some(t) => println!("  {} / {} MB", got >> 20, t >> 20),
                    None => println!("  {} MB", got >> 20),
                }
            }
        },
        &mut || {},
    )
    .map_err(|e| format!("{}: {e}", pin.url))?;
    println!("  {} (sha256 ok)", dest.display());
    Ok(())
}

fn list(repo: &Path) -> Result<(), String> {
    for f in manifest::payload_files(repo)? {
        println!("{f}");
    }
    Ok(())
}

fn make_zip(repo: &Path, out: &Path, uninstaller: Option<&Path>) -> Result<(), String> {
    let files = manifest::payload_files(repo)?;
    let pin = PythonPin::read(repo)?;
    let py = pin.cache_path(repo);
    let sha = net::sha256_file(&py).map_err(|e| format!("{}: {e} (run `pack python` first)", py.display()))?;
    if sha != pin.sha256 {
        return Err(format!("{}: sha256 {sha}, python.txt says {}", py.display(), pin.sha256));
    }
    if let Some(d) = out.parent() {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    let err = |e: &dyn std::fmt::Display| format!("{}: {e}", out.display());
    let mut z = zip::ZipWriter::new(BufWriter::new(File::create(out).map_err(|e| err(&e))?));
    let deflated = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated).large_file(true);
    let mut bytes = 0u64;
    for rel in &files {
        z.start_file(format!("app/{rel}"), deflated).map_err(|e| err(&e))?;
        let mut f = File::open(repo.join(rel)).map_err(|e| format!("{rel}: {e}"))?;
        bytes += io::copy(&mut f, &mut z).map_err(|e| format!("{rel}: {e}"))?;
    }
    let stored = SimpleFileOptions::default().compression_method(CompressionMethod::Stored).large_file(true);
    z.start_file("python.tar.gz", stored).map_err(|e| err(&e))?;
    io::copy(&mut File::open(&py).map_err(|e| err(&e))?, &mut z).map_err(|e| err(&e))?;
    if let Some(un) = uninstaller {
        // a payload inside it would double the size; a signed one keeps its signature as it is
        if trailer::locate(un).is_some() {
            return Err(format!("{}: has a payload — give the bare setup exe", un.display()));
        }
        z.start_file("uninstall.exe", deflated).map_err(|e| err(&e))?;
        io::copy(&mut File::open(un).map_err(|e| format!("{}: {e}", un.display()))?, &mut z).map_err(|e| err(&e))?;
        let signed = if trailer::is_signed(un) { "signed" } else { "unsigned" };
        println!("uninstall.exe: {} ({signed})", un.display());
    }
    let info = serde_json::json!({ "python": pin.file_name(), "python_sha256": pin.sha256, "files": files.len() });
    z.start_file("payload.json", deflated).map_err(|e| err(&e))?;
    z.write_all(info.to_string().as_bytes()).map_err(|e| err(&e))?;
    z.finish().map_err(|e| err(&e))?.flush().map_err(|e| err(&e))?;
    let size = std::fs::metadata(out).map(|m| m.len()).unwrap_or(0);
    println!("payload: {} files ({} MB) + Python -> {} ({} MB)", files.len(), bytes >> 20, out.display(), size >> 20);
    Ok(())
}

fn append(exe: &Path, zip: &Path, out: &Path) -> Result<(), String> {
    let tmp = PathBuf::from(format!("{}.tmp", out.display()));
    let r = (|| -> io::Result<u64> {
        let mut w = BufWriter::new(File::create(&tmp)?);
        io::copy(&mut File::open(exe)?, &mut w)?;
        let len = io::copy(&mut File::open(zip)?, &mut w)?;
        w.write_all(&len.to_le_bytes())?;
        w.write_all(MAGIC)?;
        w.flush()?;
        Ok(len)
    })();
    let len = r.map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, out).map_err(|e| format!("{}: {e}", out.display()))?;
    println!("{} (+{} MB payload)", out.display(), len >> 20);
    Ok(())
}

/// Find the payload as the installer does (also behind a signature) and list the zip.
fn info(exe: &Path) -> Result<(), String> {
    let (offset, len) = trailer::locate(exe).ok_or_else(|| format!("{}: no payload found", exe.display()))?;
    let size = std::fs::metadata(exe).map(|m| m.len()).unwrap_or(0);
    let signed = if trailer::is_signed(exe) { "signed" } else { "unsigned" };
    println!("{}: {size} bytes, {signed}; payload zip at {offset}, {len} bytes", exe.display());
    let mut f = File::open(exe).map_err(|e| e.to_string())?;
    f.seek(SeekFrom::Start(offset)).map_err(|e| e.to_string())?;
    let mut data = Vec::with_capacity(len as usize);
    f.take(len).read_to_end(&mut data).map_err(|e| e.to_string())?;
    let z = zip::ZipArchive::new(io::Cursor::new(data)).map_err(|e| format!("payload zip: {e}"))?;
    let app = z.file_names().filter(|n| n.starts_with("app/")).count();
    let mut other: Vec<&str> = z.file_names().filter(|n| !n.starts_with("app/")).collect();
    other.sort();
    println!("  {app} program files; {}", other.join(", "));
    Ok(())
}

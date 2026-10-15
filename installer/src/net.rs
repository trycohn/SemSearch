//! HTTP for the installer and the `pack` build tool: one blocking ureq agent (rustls; webpki roots plus the
//! Windows certificate store; proxy from HTTPS_PROXY / HTTP_PROXY / ALL_PROXY or the Windows proxy setting),
//! text pages and resumable downloads (HTTP Range) checked with sha256.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use sha2::{Digest, Sha256};

#[derive(Debug)]
pub enum NetError {
    /// No connection, timeout, dropped transfer, 5xx: worth retrying.
    Network(String),
    /// The server answered with an error status that retrying will not fix (404 and the like).
    Http(u16, String),
    /// Writing the file failed because the disk is full.
    Space(String),
    /// Writing the file failed for another reason.
    Disk(String),
    Cancelled,
    Checksum { expected: String, got: String },
}

impl std::fmt::Display for NetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NetError::Network(m) => write!(f, "network error: {m}"),
            NetError::Http(code, url) => write!(f, "HTTP {code} for {url}"),
            NetError::Space(m) => write!(f, "disk full: {m}"),
            NetError::Disk(m) => write!(f, "write error: {m}"),
            NetError::Cancelled => write!(f, "cancelled"),
            NetError::Checksum { expected, got } => write!(f, "sha256 mismatch: expected {expected}, got {got}"),
        }
    }
}

/// The proxy in use (for the log): "none", "env HTTPS_PROXY=…" or "system …".
pub fn proxy_description() -> String {
    match proxy_setting() {
        Some((src, p)) => format!("{src} {p}"),
        None => "none".into(),
    }
}

fn proxy_setting() -> Option<(&'static str, String)> {
    for k in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(v) = std::env::var(k) {
            let v = v.trim();
            if !v.is_empty() {
                return Some(("env", v.to_string()));
            }
        }
    }
    system_proxy().map(|p| ("system", p))
}

/// The WinINet proxy of the user (Settings > Network > Proxy > Manual), as pip and Python's urllib use it:
/// `host:port`, or the `https=` / `http=` entry of a per-protocol list.
fn system_proxy() -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD, RRF_RT_REG_SZ};
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings");
    let (enable, server) = (wide("ProxyEnable"), wide("ProxyServer"));
    let mut on: u32 = 0;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), enable.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut on as *mut u32 as *mut _, &mut size)
    };
    if rc != 0 || on == 0 {
        return None;
    }
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), server.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size)
    };
    if rc != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    let s = String::from_utf16_lossy(&buf[..n]);
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if !s.contains('=') {
        return Some(s.to_string());
    }
    let entry = |proto: &str| {
        s.split(';').find_map(|part| {
            let (k, v) = part.split_once('=')?;
            (k.trim().eq_ignore_ascii_case(proto) && !v.trim().is_empty()).then(|| v.trim().to_string())
        })
    };
    entry("https").or_else(|| entry("http"))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The shared agent: 20 s to connect, 60 s without data on a read aborts the transfer.
pub fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        let mut roots = rustls::RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
        if let Ok(native) = rustls_native_certs::load_native_certs() {
            roots.add_parsable_certificates(native);
        }
        let tls = rustls::ClientConfig::builder_with_provider(Arc::new(rustls::crypto::ring::default_provider()))
            .with_safe_default_protocol_versions()
            .expect("rustls protocol versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        let mut b = ureq::AgentBuilder::new()
            .tls_config(Arc::new(tls))
            .timeout_connect(Duration::from_secs(20))
            .timeout_read(Duration::from_secs(60))
            .timeout_write(Duration::from_secs(60))
            .user_agent(concat!("SemSearch-Setup/", env!("CARGO_PKG_VERSION")));
        if let Some(p) = proxy_setting().and_then(|(_, p)| ureq::Proxy::new(p).ok()) {
            b = b.proxy(p);
        }
        b.build()
    })
}

fn classify(e: ureq::Error, url: &str) -> NetError {
    match e {
        ureq::Error::Status(code, _) if code >= 500 || code == 408 || code == 429 => {
            NetError::Network(format!("HTTP {code} for {url}"))
        }
        ureq::Error::Status(code, _) => NetError::Http(code, url.to_string()),
        ureq::Error::Transport(t) => NetError::Network(t.to_string()),
    }
}

/// GET a text page (e.g. a PEP 503 index page).
pub fn get_text(url: &str) -> Result<String, NetError> {
    let resp = agent().get(url).call().map_err(|e| classify(e, url))?;
    let mut s = String::new();
    resp.into_reader().read_to_string(&mut s).map_err(|e| NetError::Network(e.to_string()))?;
    Ok(s)
}

/// `<dest>.part`: the partial download.
pub fn part_path(dest: &Path) -> PathBuf {
    let mut s = dest.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn write_error(e: io::Error) -> NetError {
    // ERROR_HANDLE_DISK_FULL (39), ERROR_DISK_FULL (112)
    if matches!(e.raw_os_error(), Some(39) | Some(112)) {
        NetError::Space(e.to_string())
    } else {
        NetError::Disk(e.to_string())
    }
}

/// Download `url` to `dest`, resuming `<dest>.part` with an HTTP Range request. `sha256` (lower-case hex), when
/// given, is checked once the file is complete; a mismatch deletes the partial file. A `dest` that already exists
/// with the right sha256 is not downloaded again. `progress(got, total)` is called after every chunk (the caller
/// throttles). `cancel` stops the transfer between chunks and keeps the partial file.
pub fn download(
    url: &str,
    dest: &Path,
    sha256: Option<&str>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u64, Option<u64>),
    on_verify: &mut dyn FnMut(),
) -> Result<(), NetError> {
    if dest.is_file() {
        match sha256 {
            Some(want) => {
                on_verify();
                if sha256_file(dest).map(|h| h == want).unwrap_or(false) {
                    let n = fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
                    progress(n, Some(n));
                    return Ok(());
                }
                let _ = fs::remove_file(dest);
            }
            None => return Ok(()),
        }
    }
    if let Some(dir) = dest.parent() {
        fs::create_dir_all(dir).map_err(write_error)?;
    }
    let part = part_path(dest);
    let mut restarted = false;
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(NetError::Cancelled);
        }
        let have = fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        let mut req = agent().get(url);
        if have > 0 {
            req = req.set("Range", &format!("bytes={have}-"));
        }
        let resp = match req.call() {
            Ok(r) => r,
            Err(ureq::Error::Status(416, r)) if have > 0 => {
                // nothing left to send: the partial file is complete (or the file changed on the server)
                let total = content_range_total(r.header("Content-Range"));
                if total == Some(have) {
                    break;
                }
                if restarted {
                    return Err(NetError::Http(416, url.to_string()));
                }
                let _ = fs::remove_file(&part);
                restarted = true;
                continue;
            }
            Err(e) => return Err(classify(e, url)),
        };
        let (mut file, mut got, total) = if resp.status() == 206 && have > 0 {
            let start = resp.header("Content-Range").and_then(content_range_start);
            if start != Some(have) {
                return Err(NetError::Network(format!("unexpected Content-Range {:?}", resp.header("Content-Range"))));
            }
            let total = content_range_total(resp.header("Content-Range"));
            let f = OpenOptions::new().append(true).open(&part).map_err(write_error)?;
            (f, have, total)
        } else {
            // 200: the server ignored the range (or there was none) — start over
            let total = resp.header("Content-Length").and_then(|v| v.trim().parse::<u64>().ok());
            let f = File::create(&part).map_err(write_error)?;
            (f, 0, total)
        };
        progress(got, total);
        let mut reader = resp.into_reader();
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = file.flush();
                return Err(NetError::Cancelled);
            }
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = file.flush();
                    return Err(NetError::Network(format!("{e} (after {got} bytes)")));
                }
            };
            file.write_all(&buf[..n]).map_err(write_error)?;
            got += n as u64;
            progress(got, total);
        }
        file.flush().map_err(write_error)?;
        drop(file);
        if let Some(t) = total {
            if got < t {
                return Err(NetError::Network(format!("connection closed after {got} of {t} bytes")));
            }
        }
        break;
    }
    if let Some(want) = sha256 {
        on_verify();
        let got = sha256_file(&part).map_err(|e| NetError::Disk(e.to_string()))?;
        if got != want {
            let _ = fs::remove_file(&part);
            return Err(NetError::Checksum { expected: want.to_string(), got });
        }
    }
    let _ = fs::remove_file(dest);
    fs::rename(&part, dest).map_err(write_error)?;
    Ok(())
}

/// "bytes 100-199/1000" -> 100
fn content_range_start(v: &str) -> Option<u64> {
    let v = v.trim().strip_prefix("bytes")?.trim();
    v.split('-').next()?.trim().parse().ok()
}

/// "bytes 100-199/1000" or "bytes */1000" -> 1000
fn content_range_total(v: Option<&str>) -> Option<u64> {
    v?.rsplit('/').next()?.trim().parse().ok()
}

/// sha256 of a file, lower-case hex.
pub fn sha256_file(path: &Path) -> io::Result<String> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

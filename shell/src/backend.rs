//! Backend process management: attach or port choice, readiness probe, spawn (inside a kill-on-close job object),
//! restart policy, shutdown. Contract: docs/API.md §3.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::Url;

use crate::util::{local_url, log, Config};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_SUSPENDED: u32 = 0x0000_0004;
/// At most this many spawns per rolling minute.
const MAX_SPAWNS_PER_MIN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// GET /api/ping says "semsearch" and GET /api/status with the token answered 200.
    Ready,
    /// The port answers HTTP, but not as our backend (another program, another user's SemSearch, a wrong token).
    Http(u16),
    /// Nothing listening / no answer.
    Down,
}

/// Minimal HTTP/1.1 request to the backend; returns the status code.
pub fn http(base: &Url, method: &str, path: &str, token: Option<&str>, body: Option<&str>, timeout: Duration) -> io::Result<u16> {
    request(base, method, path, token, body, timeout, false).map(|(code, _)| code)
}

/// The same request; with `want_body` it also reads the body (up to 64 KB, until the server closes the connection).
fn request(
    base: &Url,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&str>,
    timeout: Duration,
    want_body: bool,
) -> io::Result<(u16, String)> {
    let host = base.host_str().unwrap_or("127.0.0.1").to_string();
    let port = base.port_or_known_default().unwrap_or(80);
    let addrs: Vec<SocketAddr> = (host.as_str(), port).to_socket_addrs()?.collect();
    let mut last = io::Error::new(io::ErrorKind::NotFound, "no address");
    for addr in addrs {
        // A refused loopback connect takes ~1 s on Windows (SYN retries); a listening one connects instantly.
        let ct = timeout.min(Duration::from_millis(if addr.ip().is_loopback() { 250 } else { 1500 }));
        let mut s = match TcpStream::connect_timeout(&addr, ct) {
            Ok(s) => s,
            Err(e) => {
                last = io::Error::new(io::ErrorKind::ConnectionRefused, e); // nothing listening
                continue;
            }
        };
        s.set_read_timeout(Some(timeout))?;
        s.set_write_timeout(Some(timeout))?;
        let mut req = format!("{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nUser-Agent: SemSearch-Shell\r\nConnection: close\r\n");
        if let Some(t) = token {
            req.push_str(&format!("X-SemSearch-Token: {t}\r\n"));
        }
        let body = body.unwrap_or("");
        if method != "GET" {
            req.push_str(&format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()));
        }
        req.push_str("\r\n");
        req.push_str(body);
        s.write_all(req.as_bytes())?;
        let limit = if want_body { 64 << 10 } else { 12 };
        let mut buf = Vec::with_capacity(256);
        let mut chunk = [0u8; 1024];
        while buf.len() < limit {
            match s.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => buf.extend_from_slice(&chunk[..n]),
                Err(e) if buf.is_empty() => return Err(e),
                Err(_) => break, // keeps the connection open: use what has arrived
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let code = text
            .strip_prefix("HTTP/1.")
            .and_then(|r| r.get(2..5))
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or_else(|| {
                let head: String = text.chars().take(64).collect();
                io::Error::new(io::ErrorKind::InvalidData, format!("bad HTTP answer: {head:?}"))
            })?;
        let body = text.split_once("\r\n\r\n").map(|(_, b)| b.to_string()).unwrap_or_default();
        return Ok((code, body));
    }
    Err(last)
}

/// GET /api/ping (no token): the pid a SemSearch backend reports, or what answered instead (Http / Down).
fn ping(base: &Url, timeout: Duration) -> Result<u64, Probe> {
    match request(base, "GET", "/api/ping", None, None, timeout, true) {
        Ok((200, body)) => ping_pid(&body).ok_or(Probe::Http(200)),
        Ok((code, _)) => Err(Probe::Http(code)),
        Err(_) => Err(Probe::Down),
    }
}

/// `{"app": "semsearch", "pid": 123}` -> 123 (0 without "pid"); None for anything else.
fn ping_pid(body: &str) -> Option<u64> {
    let json = body.get(body.find('{')?..=body.rfind('}')?)?;
    let v: serde_json::Value = serde_json::from_str(json).ok()?;
    (v.get("app")?.as_str()? == "semsearch").then(|| v.get("pid").and_then(|p| p.as_u64()).unwrap_or(0))
}

fn status(base: &Url, token: Option<&str>, timeout: Duration) -> Probe {
    match http(base, "GET", "/api/status", token, None, timeout) {
        Ok(200) => Probe::Ready,
        Ok(code) => Probe::Http(code),
        Err(_) => Probe::Down,
    }
}

/// Cheap liveness check for the steady state: something accepts TCP connections on the backend port
/// (no HTTP request, so nothing shows up in the backend's access log).
pub fn listening(cfg: &Config) -> bool {
    let base = cfg.base();
    let host = base.host_str().unwrap_or("127.0.0.1");
    let port = base.port_or_known_default().unwrap_or(80);
    match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs.into_iter().any(|a| {
            let ct = Duration::from_millis(if a.ip().is_loopback() { 250 } else { 1500 });
            TcpStream::connect_timeout(&a, ct).is_ok()
        }),
        Err(_) => false,
    }
}

/// Readiness of the backend in use: /api/ping says "semsearch" and GET /api/status with its token answers 200.
/// With SEMSEARCH_DEV_URL only GET /api/status is asked (the supervisor takes any HTTP answer there).
pub fn probe(cfg: &Config, timeout: Duration) -> Probe {
    let base = cfg.base();
    if !cfg.dev {
        if let Err(p) = ping(&base, timeout) {
            return p;
        }
    }
    status(&base, cfg.token().as_deref(), timeout)
}

/// No backend of ours: use the one on the preferred port if /api/ping says "semsearch" and GET /api/status
/// with the token from `<data>\token` answers 200. Ready = it is now the backend in use.
pub fn attach(cfg: &Config, timeout: Duration) -> Probe {
    let base = local_url(cfg.port);
    let pid = match ping(&base, timeout) {
        Ok(pid) => pid,
        Err(p) => return p,
    };
    let token = cfg.file_token();
    let res = status(&base, token.as_deref(), timeout);
    if res == Probe::Ready {
        log(&format!("attached to the backend on port {} (pid {pid})", cfg.port));
        cfg.set_backend(cfg.port, token);
    }
    res
}

/// Port for a backend we start: the preferred one if it is free, else the first free one of
/// preferred+1 … preferred+20, else one the OS assigns.
fn choose_port(preferred: u16) -> u16 {
    let near = (1..=20).filter_map(|k| preferred.checked_add(k));
    if let Some(p) = std::iter::once(preferred).chain(near).find(|&p| port_free(p)) {
        if p != preferred {
            log(&format!("port {preferred} is busy, using {p}"));
        }
        return p;
    }
    let os = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).and_then(|l| l.local_addr()).map(|a| a.port());
    log(&format!("ports {preferred}..+20 are busy, OS-assigned port: {os:?}"));
    os.unwrap_or(preferred)
}

/// Nothing answers on 127.0.0.1:`port` and the port can be bound there.
fn port_free(port: u16) -> bool {
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    // fails at once for a port in use or in a range Windows reserves (Hyper-V, WSL) ...
    match TcpListener::bind(addr) {
        Ok(l) => drop(l),
        Err(_) => return false,
    }
    // ... but a program listening on 0.0.0.0:port does not stop a 127.0.0.1 bind: it would still answer here.
    // A connection to itself (possible for a port in the dynamic range) means that nobody listens.
    match TcpStream::connect_timeout(&addr, Duration::from_millis(250)) {
        Ok(s) => matches!((s.local_addr(), s.peer_addr()), (Ok(a), Ok(b)) if a == b),
        Err(_) => true,
    }
}

/// Session token for a backend we start (passed in SEMSEARCH_TOKEN): 32 random bytes as hex.
/// None if the system RNG fails: the backend then makes its own and we read `<data>\token`.
fn new_token() -> Option<String> {
    use windows_sys::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};
    let mut b = [0u8; 32];
    let st = unsafe { BCryptGenRandom(std::ptr::null_mut(), b.as_mut_ptr(), b.len() as u32, BCRYPT_USE_SYSTEM_PREFERRED_RNG) };
    (st == 0).then(|| b.iter().map(|x| format!("{x:02x}")).collect())
}

// ---------------------------------------------------------------- job object

struct Job(windows_sys::Win32::Foundation::HANDLE);
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    /// Job that kills the backend's whole process tree when the shell exits (or crashes).
    fn new() -> Option<Job> {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_BREAKAWAY_OK;
            let ok = SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                // without KILL_ON_JOB_CLOSE the job is pointless; the backend is still stopped by POST /api/shutdown
                log("job object: cannot set KILL_ON_JOB_CLOSE");
                windows_sys::Win32::Foundation::CloseHandle(h);
                return None;
            }
            Some(Job(h))
        }
    }
    fn assign(&self, child: &Child) -> bool {
        unsafe { windows_sys::Win32::System::JobObjects::AssignProcessToJobObject(self.0, child.as_raw_handle() as _) != 0 }
    }
    fn terminate(&self) {
        unsafe { windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1) };
    }
}

/// Resume the main thread of a process created with CREATE_SUSPENDED (`Child` does not keep the thread handle).
fn resume_main_thread(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snap == INVALID_HANDLE_VALUE {
            return false;
        }
        let mut te: THREADENTRY32 = std::mem::zeroed();
        te.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut resumed = false;
        let mut more = Thread32First(snap, &mut te) != 0;
        while more {
            if te.th32OwnerProcessID == pid {
                let t = OpenThread(THREAD_SUSPEND_RESUME, 0, te.th32ThreadID);
                if !t.is_null() {
                    resumed |= ResumeThread(t) != u32::MAX;
                    CloseHandle(t);
                }
            }
            more = Thread32Next(snap, &mut te) != 0;
        }
        CloseHandle(snap);
        resumed
    }
}

// ---------------------------------------------------------------- python

/// `pythonw.exe` for the backend and whether it is the explicit one: SEMSEARCH_PYTHON (full path), else the venv
/// (`<root>\.venv\Scripts`: a source checkout after setup.bat, and the installed copy), else an embedded runtime
/// (`<root>\runtime`). Err = every path that was tried.
fn find_python(root: &Path) -> Result<(PathBuf, bool), Vec<PathBuf>> {
    let mut tried = Vec::new();
    if let Some(p) = std::env::var_os("SEMSEARCH_PYTHON").map(PathBuf::from).filter(|p| !p.as_os_str().is_empty()) {
        if p.is_file() {
            return Ok((p, true));
        }
        log(&format!("SEMSEARCH_PYTHON points to a missing file: {}", p.display()));
        tried.push(p);
    }
    for p in [root.join(".venv").join("Scripts").join("pythonw.exe"), root.join("runtime").join("pythonw.exe")] {
        if p.is_file() {
            return Ok((p, false));
        }
        tried.push(p);
    }
    Err(tried)
}

/// What the user can do about missing files: an installed copy has uninstall.exe next to server.py and is reinstalled,
/// a source checkout has setup.bat. `python`: the missing file is Python (else server.py).
fn fix_hint(root: &Path, en: bool, python: bool) -> &'static str {
    if root.join("uninstall.exe").is_file() {
        if en { "reinstall SemSearch" } else { "переустановите SemSearch" }
    } else if python && root.join("setup.bat").is_file() {
        if en { "run setup.bat" } else { "запустите setup.bat" }
    } else if python {
        if en { "set SEMSEARCH_PYTHON to the full path of pythonw.exe" } else { "укажите путь к pythonw.exe в переменной SEMSEARCH_PYTHON" }
    } else if en {
        "check SEMSEARCH_ROOT"
    } else {
        "проверьте SEMSEARCH_ROOT"
    }
}

// ---------------------------------------------------------------- backend process

pub struct Backend {
    child: Mutex<Option<Child>>,
    job: Option<Job>,
    spawns: Mutex<VecDeque<Instant>>,
    /// Earliest time of the next spawn attempt (backoff).
    next_spawn: Mutex<Option<Instant>>,
    failures: Mutex<u32>,
    /// Exit code of the last spawned backend that died.
    pub last_exit: Mutex<Option<i32>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ChildState {
    None,
    Running,
    Exited(Option<i32>),
}

impl Backend {
    pub fn new() -> Backend {
        Backend {
            child: Mutex::new(None),
            job: Job::new(),
            spawns: Mutex::new(VecDeque::new()),
            next_spawn: Mutex::new(None),
            failures: Mutex::new(0),
            last_exit: Mutex::new(None),
        }
    }

    /// Whether we have a spawned backend and if it is still running (reaps an exited one).
    pub fn child_state(&self) -> ChildState {
        let mut g = self.child.lock().unwrap();
        let Some(child) = g.as_mut() else { return ChildState::None };
        match child.try_wait() {
            Ok(None) => ChildState::Running,
            Ok(Some(st)) => {
                let code = st.code();
                *g = None;
                *self.last_exit.lock().unwrap() = code;
                let mut f = self.failures.lock().unwrap();
                *f += 1;
                let delay = Duration::from_secs(1u64 << (*f).min(5)); // 2, 4, 8, 16, 32 s
                *self.next_spawn.lock().unwrap() = Some(Instant::now() + delay);
                log(&format!("backend exited (code {code:?}); next start in >= {}s", delay.as_secs()));
                ChildState::Exited(code)
            }
            Err(_) => ChildState::Running,
        }
    }

    pub fn has_child(&self) -> bool {
        self.child.lock().unwrap().is_some()
    }

    /// Seconds until a spawn is allowed (0 = now).
    pub fn spawn_wait(&self) -> u64 {
        let now = Instant::now();
        let mut wait = Duration::ZERO;
        if let Some(t) = *self.next_spawn.lock().unwrap() {
            wait = wait.max(t.saturating_duration_since(now));
        }
        let mut q = self.spawns.lock().unwrap();
        while q.front().is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60)) {
            q.pop_front();
        }
        if q.len() >= MAX_SPAWNS_PER_MIN {
            let oldest = *q.front().unwrap();
            wait = wait.max((oldest + Duration::from_secs(61)).saturating_duration_since(now));
        }
        wait.as_secs() + u64::from(wait.subsec_millis() > 0)
    }

    /// A spawn that cannot even be tried (files missing): the next attempt is in 10 s, in case the user has fixed it.
    fn refuse(&self, msg: String) -> Result<(), String> {
        *self.next_spawn.lock().unwrap() = Some(Instant::now() + Duration::from_secs(10));
        Err(msg)
    }

    /// `<python> <root>\server.py --no-browser` (python: see find_python), cwd root, no window, env SEMSEARCH_SHELL=1,
    /// SEMSEARCH_PORT (preferred port or a free one), SEMSEARCH_TOKEN (new session token) and SEMSEARCH_DATA_DIR
    /// (the data folder the shell uses). `en`: error texts in English (else Russian).
    pub fn spawn(&self, cfg: &Config, en: bool) -> Result<(), String> {
        let server = cfg.root.join("server.py");
        if !server.is_file() {
            let hint = fix_hint(&cfg.root, en, false);
            return self.refuse(if en {
                format!("{} not found — {hint}", server.display())
            } else {
                format!("не найден {} — {hint}", server.display())
            });
        }
        let (py, explicit) = match find_python(&cfg.root) {
            Ok(found) => found,
            Err(tried) => {
                let list = tried.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(", ");
                let hint = fix_hint(&cfg.root, en, true);
                return self.refuse(if en {
                    format!("Python not found (looked for {list}) — {hint}")
                } else {
                    format!("не найден Python (искал {list}) — {hint}")
                });
            }
        };
        self.spawns.lock().unwrap().push_back(Instant::now());
        let port = choose_port(cfg.port);
        let token = new_token();
        let mut cmd = Command::new(&py);
        cmd.arg(&server)
            .arg("--no-browser")
            .current_dir(&cfg.root)
            .env("SEMSEARCH_SHELL", "1")
            .env("SEMSEARCH_PORT", port.to_string())
            .env("SEMSEARCH_DATA_DIR", &cfg.data)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Suspended until it is in the job: the venv's pythonw.exe is only a launcher that starts the real interpreter
            // as its child (and puts itself in a job of its own), and a process created before AssignProcessToJobObject
            // would stay outside our job - or make the assignment fail.
            .creation_flags(CREATE_NO_WINDOW | CREATE_SUSPENDED);
        if !explicit {
            // a PYTHONHOME / PYTHONPATH of some other Python on the user's machine breaks ours
            cmd.env_remove("PYTHONHOME").env_remove("PYTHONPATH");
        }
        if let Some(t) = &token {
            cmd.env("SEMSEARCH_TOKEN", t);
        }
        match cmd.spawn() {
            Ok(mut child) => {
                let in_job = self.job.as_ref().map(|j| j.assign(&child)).unwrap_or(false);
                if !resume_main_thread(child.id()) {
                    let _ = child.kill();
                    let _ = child.wait();
                    *self.next_spawn.lock().unwrap() = Some(Instant::now() + Duration::from_secs(5));
                    return Err(if en {
                        format!("couldn't start {}: the process did not resume", py.display())
                    } else {
                        format!("не удалось запустить {}: процесс не продолжил работу", py.display())
                    });
                }
                log(&format!("backend started: {} pid {}, port {port} (job: {in_job})", py.display(), child.id()));
                cfg.set_backend(port, token);
                *self.child.lock().unwrap() = Some(child);
                Ok(())
            }
            Err(e) => {
                *self.next_spawn.lock().unwrap() = Some(Instant::now() + Duration::from_secs(5));
                Err(if en {
                    format!("couldn't start {}: {e}", py.display())
                } else {
                    format!("не удалось запустить {}: {e}", py.display())
                })
            }
        }
    }

    /// Backend answered "ready" after a spawn: reset the failure counter (the minute window still applies).
    pub fn mark_healthy(&self) {
        *self.failures.lock().unwrap() = 0;
    }

    /// POST /api/shutdown with the token, wait <= `wait`, kill only a backend we spawned.
    pub fn shutdown(&self, cfg: &Config, wait: Duration) {
        let t0 = Instant::now();
        let base = cfg.base();
        let token = cfg.token();
        let res = http(&base, "POST", "/api/shutdown", token.as_deref(), Some("{}"), Duration::from_secs(2));
        log(&format!("POST /api/shutdown -> {res:?}"));
        let ours = self.has_child();
        // our backend is not even listening yet (still starting): nothing to wait for
        let not_listening = matches!(&res, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused);
        let deadline = if ours && not_listening { t0 } else { t0 + wait };
        loop {
            let gone = if ours {
                matches!(self.child_state(), ChildState::Exited(_) | ChildState::None)
            } else {
                http(&base, "GET", "/", None, None, Duration::from_millis(300)).is_err()
            };
            if gone {
                log(&format!("backend stopped in {} ms", t0.elapsed().as_millis()));
                break;
            }
            if Instant::now() >= deadline {
                if ours {
                    log("backend did not stop in time - killing it");
                    if let Some(j) = &self.job {
                        j.terminate();
                    }
                    if let Some(mut c) = self.child.lock().unwrap().take() {
                        let _ = c.kill();
                        let _ = c.wait();
                    }
                } else {
                    log("external backend still answering - leaving it alone");
                }
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

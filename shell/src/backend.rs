//! Backend process management: readiness probe, spawn (inside a kill-on-close job object), restart policy, shutdown.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::Url;

use crate::util::{log, Config};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// At most this many spawns per rolling minute.
const MAX_SPAWNS_PER_MIN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Probe {
    /// GET /api/status with the token answered 200.
    Ready,
    /// The server answers HTTP, but not 200 (e.g. 403 before data\token is written).
    Http(u16),
    /// Nothing listening / no answer.
    Down,
}

/// Minimal HTTP/1.1 request to the backend; returns the status code.
pub fn http(base: &Url, method: &str, path: &str, token: Option<&str>, body: Option<&str>, timeout: Duration) -> io::Result<u16> {
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
        let mut buf = Vec::with_capacity(64);
        let mut chunk = [0u8; 64];
        while buf.len() < 12 {
            let n = s.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        let line = String::from_utf8_lossy(&buf);
        return line
            .strip_prefix("HTTP/1.")
            .and_then(|r| r.get(2..5))
            .and_then(|c| c.parse::<u16>().ok())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, format!("bad HTTP answer: {line:?}")));
    }
    Err(last)
}

/// Cheap liveness check for the steady state: something accepts TCP connections on the backend port
/// (no HTTP request, so nothing shows up in the backend's access log).
pub fn listening(cfg: &Config) -> bool {
    let host = cfg.base.host_str().unwrap_or("127.0.0.1");
    let port = cfg.base.port_or_known_default().unwrap_or(80);
    match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs.into_iter().any(|a| {
            let ct = Duration::from_millis(if a.ip().is_loopback() { 250 } else { 1500 });
            TcpStream::connect_timeout(&a, ct).is_ok()
        }),
        Err(_) => false,
    }
}

pub fn probe(cfg: &Config, timeout: Duration) -> Probe {
    let token = cfg.token();
    match http(&cfg.base, "GET", "/api/status", token.as_deref(), None, timeout) {
        Ok(200) => Probe::Ready,
        Ok(code) => Probe::Http(code),
        Err(_) => Probe::Down,
    }
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
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
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

    /// `<root>\.venv\Scripts\pythonw.exe <root>\server.py --no-browser`, cwd root, SEMSEARCH_SHELL=1, no window.
    pub fn spawn(&self, cfg: &Config) -> Result<(), String> {
        let py = cfg.root.join(".venv").join("Scripts").join("pythonw.exe");
        let server = cfg.root.join("server.py");
        let missing = if !server.is_file() {
            Some(format!("не найден {}", server.display()))
        } else if !py.is_file() {
            Some(format!("не найден {} — запустите setup.bat", py.display()))
        } else {
            None
        };
        if let Some(e) = missing {
            *self.next_spawn.lock().unwrap() = Some(Instant::now() + Duration::from_secs(10));
            return Err(e);
        }
        self.spawns.lock().unwrap().push_back(Instant::now());
        let res = Command::new(&py)
            .arg(&server)
            .arg("--no-browser")
            .current_dir(&cfg.root)
            .env("SEMSEARCH_SHELL", "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn();
        match res {
            Ok(child) => {
                let in_job = self.job.as_ref().map(|j| j.assign(&child)).unwrap_or(false);
                log(&format!("backend started: pid {} (job: {in_job})", child.id()));
                *self.child.lock().unwrap() = Some(child);
                Ok(())
            }
            Err(e) => {
                *self.next_spawn.lock().unwrap() = Some(Instant::now() + Duration::from_secs(5));
                Err(format!("не удалось запустить {}: {e}", py.display()))
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
        let token = cfg.token();
        let res = http(&cfg.base, "POST", "/api/shutdown", token.as_deref(), Some("{}"), Duration::from_secs(2));
        log(&format!("POST /api/shutdown -> {res:?}"));
        let ours = self.has_child();
        // our backend is not even listening yet (still starting): nothing to wait for
        let not_listening = matches!(&res, Err(e) if e.kind() == io::ErrorKind::ConnectionRefused);
        let deadline = if ours && not_listening { t0 } else { t0 + wait };
        loop {
            let gone = if ours {
                matches!(self.child_state(), ChildState::Exited(_) | ChildState::None)
            } else {
                http(&cfg.base, "GET", "/", None, None, Duration::from_millis(300)).is_err()
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

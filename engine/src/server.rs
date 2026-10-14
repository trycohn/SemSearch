//! Именованный канал `\\.\pipe\semsearch-engine-<SessionId>`: построчный JSON, поток на соединение.
//! DACL: SYSTEM, Администраторы, SID текущего пользователя (чтение/запись); метка целостности
//! Medium — процесс пользователя со средним уровнем может подключиться к повышенному движку.

use serde_json::Value;
use std::sync::mpsc::{sync_channel, RecvTimeoutError};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::*;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Pipes::*;

use crate::engine::*;
use crate::ops;
use crate::util::*;

pub const DEFAULT_PIPE: &str = "semsearch-engine";
static PIPE: OnceLock<String> = OnceLock::new();

/// `--pipe <name>` для отладочного экземпляра. Допустимы только [A-Za-z0-9._-], до 100 символов.
pub fn set_pipe_name(name: &str) -> Result<(), String> {
    let ok = !name.is_empty() && name.len() <= 100 && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'.' || c == b'_' || c == b'-');
    if !ok {
        return Err(format!("invalid pipe name {name:?} (allowed: A-Z a-z 0-9 . _ -, up to 100 chars)"));
    }
    let _ = PIPE.set(format!(r"\\.\pipe\{name}"));
    Ok(())
}

/// Имя канала (docs/API.md §4): `--pipe <name>`, иначе свой у каждого сеанса Windows —
/// `\\.\pipe\semsearch-engine-<SessionId>`, SessionId = ProcessIdToSessionId(текущий процесс), десятичное.
/// Так движки разных вошедших пользователей не мешают друг другу.
pub fn pipe_name() -> &'static str {
    PIPE.get_or_init(|| {
        use windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId;
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut session = 0u32;
        unsafe { ProcessIdToSessionId(GetCurrentProcessId(), &mut session) };
        format!(r"\\.\pipe\{DEFAULT_PIPE}-{session}")
    })
}

const MAX_REQ: usize = 1 << 20;
const PIPE_BUF: u32 = 64 * 1024;

pub struct Security {
    sd: PSECURITY_DESCRIPTOR,
    pub sddl: String,
}
unsafe impl Send for Security {}
unsafe impl Sync for Security {}

impl Security {
    fn from_sddl(sddl: &str) -> Option<Security> {
        let w = wide_z(sddl);
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let ok = unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) };
        if ok == 0 || sd.is_null() {
            warn!("SDDL conversion failed (err {}): {sddl}", unsafe { GetLastError() });
            return None;
        }
        Some(Security { sd, sddl: sddl.to_string() })
    }

    /// Полный вариант (с меткой целостности Medium) и запасной без метки.
    pub fn candidates() -> Vec<Security> {
        let user = user_sid().unwrap_or_else(|| {
            warn!("cannot read user SID from token; granting interactive users instead");
            "IU".into()
        });
        let dacl = format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;{user})");
        [format!("{dacl}S:(ML;;NW;;;ME)"), dacl].iter().filter_map(|s| Security::from_sddl(s)).collect()
    }
}

pub enum FirstErr {
    Busy,
    Other(String),
}

fn create_instance(sec: &Security, first: bool) -> Result<Handle, u32> {
    let sa = SECURITY_ATTRIBUTES { nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32, lpSecurityDescriptor: sec.sd, bInheritHandle: 0 };
    let name = wide_z(pipe_name());
    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }
    let h = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            open_mode,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            PIPE_UNLIMITED_INSTANCES,
            PIPE_BUF,
            PIPE_BUF,
            0,
            &sa,
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(unsafe { GetLastError() });
    }
    Ok(Handle(h))
}

fn pipe_exists() -> bool {
    let name = wide_z(pipe_name());
    unsafe { WaitNamedPipeW(name.as_ptr(), 1) != 0 || GetLastError() == ERROR_SEM_TIMEOUT }
}

/// Первый экземпляр канала (FILE_FLAG_FIRST_PIPE_INSTANCE): заодно проверка «движок уже запущен».
pub fn create_first() -> Result<(Handle, Security), FirstErr> {
    let mut last = String::from("no security descriptor");
    for sec in Security::candidates() {
        match create_instance(&sec, true) {
            Ok(h) => {
                info!("pipe {} listening, SDDL {}", pipe_name(), sec.sddl);
                return Ok((h, sec));
            }
            Err(e) => {
                if (e == ERROR_ACCESS_DENIED || e == ERROR_PIPE_BUSY) && pipe_exists() {
                    return Err(FirstErr::Busy);
                }
                warn!("CreateNamedPipe failed (err {e}) with SDDL {}", sec.sddl);
                last = format!("CreateNamedPipe failed (err {e})");
            }
        }
    }
    Err(FirstErr::Other(last))
}

pub fn serve(eng: Arc<Engine>, first: Handle, sec: Security) -> ! {
    let mut pending = Some(first);
    loop {
        let h = match pending.take() {
            Some(h) => h,
            None => match create_instance(&sec, false) {
                Ok(h) => h,
                Err(e) => {
                    warn!("CreateNamedPipe (next instance) failed (err {e})");
                    std::thread::sleep(Duration::from_millis(500));
                    continue;
                }
            },
        };
        let ok = unsafe { ConnectNamedPipe(h.0, std::ptr::null_mut()) } != 0 || unsafe { GetLastError() } == ERROR_PIPE_CONNECTED;
        if !ok {
            continue;
        }
        let e2 = eng.clone();
        if let Err(e) = std::thread::Builder::new().name("conn".into()).spawn(move || conn_main(e2, h)) {
            warn!("cannot spawn connection thread: {e}");
        }
    }
}

fn write_all(h: &Handle, mut data: &[u8]) -> bool {
    while !data.is_empty() {
        let chunk = data.len().min(1 << 20) as u32;
        let mut n = 0u32;
        let ok = unsafe { WriteFile(h.0, data.as_ptr(), chunk, &mut n, std::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            return false;
        }
        data = &data[n as usize..];
    }
    true
}

fn pipe_alive(h: &Handle) -> bool {
    let mut avail = 0u32;
    unsafe { PeekNamedPipe(h.0, std::ptr::null_mut(), 0, std::ptr::null_mut(), &mut avail, std::ptr::null_mut()) != 0 }
}

fn ok_line(id: &Value, body: &str) -> String {
    let b = body.trim();
    let inner = if b.len() >= 2 && b.starts_with('{') && b.ends_with('}') { b[1..b.len() - 1].trim() } else { "" };
    if inner.is_empty() {
        format!("{{\"id\":{id},\"ok\":true}}\n")
    } else {
        format!("{{\"id\":{id},\"ok\":true,{inner}}}\n")
    }
}

fn err_line(id: &Value, e: &str) -> String {
    format!("{{\"id\":{id},\"ok\":false,\"error\":{}}}\n", serde_json::to_string(e).unwrap_or_else(|_| "\"error\"".into()))
}

enum Action {
    Reply(String),
    Subscribe(Value, Vec<String>),
    Shutdown(Value),
}

fn dispatch(eng: &Arc<Engine>, line: &[u8]) -> Action {
    let req: Value = match serde_json::from_slice(line) {
        Ok(v @ Value::Object(_)) => v,
        Ok(_) => return Action::Reply(err_line(&Value::Null, "request must be a JSON object")),
        Err(e) => return Action::Reply(err_line(&Value::Null, &format!("bad request: {e}"))),
    };
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let op = req.get("op").and_then(Value::as_str).unwrap_or("");
    let t0 = Instant::now();
    let r = match op {
        "ping" => ops::ping(eng),
        "status" => ops::status(eng),
        "search" => ops::search(eng, &req),
        "same_name" => ops::same_name(eng, &req),
        "list" => ops::list(eng, &req),
        "exists" => ops::exists_op(eng, &req),
        "reload_config" => ops::reload_config(eng),
        "subscribe" => return Action::Subscribe(id, ops::str_list(req.get("roots"))),
        "shutdown" => return Action::Shutdown(id),
        "" => Err("missing op".to_string()),
        _ => Err(format!("unknown op: {op}")),
    };
    let el = t0.elapsed().as_millis();
    if el > 1000 {
        info!("op {op} took {el} ms");
    }
    match r {
        Ok(body) => Action::Reply(ok_line(&id, &body)),
        Err(e) => Action::Reply(err_line(&id, &e)),
    }
}

fn conn_main(eng: Arc<Engine>, h: Handle) {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut chunk = vec![0u8; 64 * 1024];
    loop {
        while let Some(pos) = memchr::memchr(b'\n', &buf) {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let line = line.trim_ascii();
            if line.is_empty() {
                continue;
            }
            match dispatch(&eng, line) {
                Action::Reply(s) => {
                    if !write_all(&h, s.as_bytes()) {
                        return;
                    }
                }
                Action::Subscribe(id, roots) => {
                    run_subscription(&eng, &h, id, roots);
                    return;
                }
                Action::Shutdown(id) => {
                    info!("shutdown requested over pipe");
                    write_all(&h, ok_line(&id, "{}").as_bytes());
                    unsafe { FlushFileBuffers(h.0) };
                    std::process::exit(0);
                }
            }
        }
        if buf.len() > MAX_REQ {
            write_all(&h, err_line(&Value::Null, "request too large (max 1 MB)").as_bytes());
            return;
        }
        let mut n = 0u32;
        let ok = unsafe { ReadFile(h.0, chunk.as_mut_ptr(), chunk.len() as u32, &mut n, std::ptr::null_mut()) };
        if ok == 0 || n == 0 {
            return;
        }
        buf.extend_from_slice(&chunk[..n as usize]);
    }
}

fn run_subscription(eng: &Arc<Engine>, h: &Handle, id: Value, roots: Vec<String>) {
    let (tx, rx) = sync_channel::<Arc<str>>(65536);
    let sub = Arc::new(Subscriber::new(&roots, tx));
    eng.add_sub(sub.clone());
    info!("subscriber connected, roots {:?}", roots);
    if write_all(h, ok_line(&id, "{}").as_bytes()) {
        let mut out: Vec<u8> = Vec::with_capacity(64 * 1024);
        loop {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(line) => {
                    out.clear();
                    out.extend_from_slice(line.as_bytes());
                    out.push(b'\n');
                    while out.len() < (1 << 20) {
                        match rx.try_recv() {
                            Ok(l) => {
                                out.extend_from_slice(l.as_bytes());
                                out.push(b'\n');
                            }
                            Err(_) => break,
                        }
                    }
                    if !write_all(h, &out) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if !pipe_alive(h) {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
            if sub.overflow.swap(false, std::sync::atomic::Ordering::SeqCst) {
                warn!("subscriber queue overflow → rescan event");
                let mut s = String::new();
                for l in sub.rescan_lines() {
                    s.push_str(&l);
                    s.push('\n');
                }
                if !write_all(h, s.as_bytes()) {
                    break;
                }
            }
        }
    }
    sub.dead.store(true, std::sync::atomic::Ordering::SeqCst);
    eng.remove_sub(&sub);
    info!("subscriber disconnected");
}

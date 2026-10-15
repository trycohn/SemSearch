//! Child processes: run with captured output inside a kill-on-close job (cancel() and the installer's exit end
//! the whole tree), elevated runs through UAC, and closing the processes of an installed copy.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::JobObjects::*;
use windows_sys::Win32::System::Threading::*;

use crate::util::{self, is_under, log, wide};

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A job object that kills its processes when it is closed or killed.
pub struct Job(HANDLE);

unsafe impl Send for Job {}
unsafe impl Sync for Job {}

impl Job {
    pub fn new() -> Option<Job> {
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            SetInformationJobObject(
                h,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const _,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            Some(Job(h))
        }
    }

    fn assign(&self, child: &std::process::Child) {
        use std::os::windows::io::AsRawHandle;
        unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) };
    }

    pub fn kill(&self) {
        unsafe { TerminateJobObject(self.0, 1) };
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

/// Where the running child's job is kept so that another thread can kill it (cancel()).
pub type JobSlot = Mutex<Option<Arc<Job>>>;

/// Result of a captured run.
pub struct Output {
    pub code: i32,
    /// The last lines of stdout + stderr.
    pub tail: Vec<String>,
}

impl Output {
    pub fn tail_text(&self, n: usize) -> String {
        let skip = self.tail.len().saturating_sub(n);
        self.tail[skip..].join("\n")
    }
}

/// Command line for the log.
pub fn describe(cmd: &Command) -> String {
    let mut s = format!("{:?}", cmd.get_program());
    for a in cmd.get_args() {
        s.push(' ');
        s.push_str(&format!("{a:?}"));
    }
    s
}

/// Run `cmd` without a window, stdout and stderr captured line by line (UTF-8 lossy). `on_line` gets the lines
/// of stdout only (e.g. the JSON lines of fetch_model.py); stderr goes to the log with the last lines of stdout.
/// The process tree lives in a job stored in `slot` while it runs. Logs the command, the exit code and the last
/// lines of output.
pub fn run(mut cmd: Command, slot: &JobSlot, mut on_line: impl FnMut(&str)) -> std::io::Result<Output> {
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).creation_flags(CREATE_NO_WINDOW);
    let line = describe(&cmd);
    log(&format!("run: {line}"));
    let mut child = cmd.spawn().map_err(|e| {
        log(&format!("  spawn failed: {e}"));
        e
    })?;
    let job = Job::new().map(Arc::new);
    if let Some(j) = &job {
        j.assign(&child);
    }
    *slot.lock().unwrap() = job.clone();

    let (tx, rx) = mpsc::channel::<(bool, String)>();
    let mut readers = Vec::new();
    let streams = [
        (false, child.stdout.take().map(|s| Box::new(s) as Box<dyn Read + Send>)),
        (true, child.stderr.take().map(|s| Box::new(s) as Box<dyn Read + Send>)),
    ];
    for (is_err, stream) in streams.into_iter().filter_map(|(e, s)| s.map(|s| (e, s))) {
        let tx = tx.clone();
        readers.push(std::thread::spawn(move || {
            let mut r = BufReader::new(stream);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match r.read_until(b'\n', &mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        let s = String::from_utf8_lossy(&buf);
                        let _ = tx.send((is_err, s.trim_end_matches(['\r', '\n']).to_string()));
                    }
                }
            }
        }));
    }
    drop(tx);
    let mut tail = VecDeque::new();
    for (is_err, l) in rx {
        if !is_err {
            on_line(&l);
        }
        if tail.len() == 80 {
            tail.pop_front();
        }
        tail.push_back(l);
    }
    for r in readers {
        let _ = r.join();
    }
    let status = child.wait();
    *slot.lock().unwrap() = None;
    let code = match status {
        Ok(s) => s.code().unwrap_or(-1),
        Err(_) => -1,
    };
    let out = Output { code, tail: tail.into_iter().collect() };
    log(&format!("  exit {code}; last output:\n{}", indent(&out.tail_text(80))));
    Ok(out)
}

fn indent(s: &str) -> String {
    s.lines().map(|l| format!("    {l}")).collect::<Vec<_>>().join("\n")
}

/// How an elevated run ended.
pub enum Elevated {
    Exit(u32),
    /// The user said no to the UAC prompt.
    Denied,
    Failed(String),
}

/// Run `exe params` elevated (ShellExecuteEx "runas", the UAC prompt) and wait for it. `owner` is the window
/// the prompt belongs to (0: none).
pub fn run_elevated(exe: &Path, params: &str, owner: isize) -> Elevated {
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    log(&format!("run elevated: {} {params}", exe.display()));
    let (verb, file, args) = (wide("runas"), util::wide_path(exe), wide(params));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.hwnd = owner as _;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = args.as_ptr();
    info.nShow = SW_HIDE;
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let err = std::io::Error::last_os_error();
        log(&format!("  ShellExecuteEx failed: {err}"));
        // ERROR_CANCELLED: the user declined the UAC prompt
        return if err.raw_os_error() == Some(1223) { Elevated::Denied } else { Elevated::Failed(err.to_string()) };
    }
    if info.hProcess.is_null() {
        return Elevated::Failed("no process handle".into());
    }
    let mut code = 1u32;
    unsafe {
        WaitForSingleObject(info.hProcess, INFINITE);
        GetExitCodeProcess(info.hProcess, &mut code);
        CloseHandle(info.hProcess);
    }
    log(&format!("  exit {code}"));
    Elevated::Exit(code)
}

/// Full image path of a process, if it can be read.
fn image_path(h: HANDLE) -> Option<PathBuf> {
    let mut buf = vec![0u16; 32768];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) };
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

/// Terminate every process whose executable lies under `root` (SemSearch.exe, Python from .venv\ and
/// runtime\, bin\semsearch-engine.exe), never this process. Nothing else is touched: processes started from
/// other folders (an elevated engine from Program Files, the WebView2 runtime) are left alone. Returns the
/// paths that could not be terminated.
pub fn close_under(root: &Path) -> Vec<String> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::*;
    let me = std::process::id();
    let mut stuck = Vec::new();
    // two passes: the shell may restart the backend between the first kills
    for pass in 0..2 {
        stuck.clear();
        let snap = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
        if snap.is_null() || snap as isize == -1 {
            return stuck;
        }
        let mut pids = Vec::new();
        let mut e: PROCESSENTRY32W = unsafe { std::mem::zeroed() };
        e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = unsafe { Process32FirstW(snap, &mut e) };
        while ok != 0 {
            if e.th32ProcessID != me && e.th32ProcessID != 0 {
                pids.push(e.th32ProcessID);
            }
            ok = unsafe { Process32NextW(snap, &mut e) };
        }
        unsafe { CloseHandle(snap) };
        let mut killed = 0;
        for pid in pids {
            let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, 0, pid) };
            if h.is_null() {
                // no rights to terminate: is it ours at all?
                let q = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
                if !q.is_null() {
                    if let Some(p) = image_path(q).filter(|p| is_under(p, root)) {
                        log(&format!("close: no rights to end {} (pid {pid})", p.display()));
                        stuck.push(p.display().to_string());
                    }
                    unsafe { CloseHandle(q) };
                }
                continue;
            }
            if let Some(p) = image_path(h).filter(|p| is_under(p, root)) {
                let ok = unsafe { TerminateProcess(h, 1) } != 0;
                let gone = ok && unsafe { WaitForSingleObject(h, 10_000) } == WAIT_OBJECT_0;
                log(&format!("close: ended {} (pid {pid}): {}", p.display(), if gone { "ok" } else { "still running" }));
                if gone {
                    killed += 1;
                } else {
                    stuck.push(p.display().to_string());
                }
            }
            unsafe { CloseHandle(h) };
        }
        if killed == 0 || pass == 1 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(300));
    }
    stuck
}

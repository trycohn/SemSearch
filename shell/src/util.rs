//! Paths, configuration, logging and small Win32 helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tauri::Url;

pub const DEFAULT_PORT: u16 = 8765;
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+s";

/// Startup configuration (environment + `<root>\config.json`).
pub struct Config {
    /// Folder with server.py (env SEMSEARCH_ROOT overrides the upward search from the exe).
    pub root: PathBuf,
    /// Base URL of the pages, always ending with "/" (http://127.0.0.1:<port>/ or SEMSEARCH_DEV_URL).
    pub base: Url,
    /// SEMSEARCH_DEV_URL is set: any HTTP answer counts as "ready".
    pub dev: bool,
    /// SEMSEARCH_NO_BACKEND=1: never spawn or shut down the backend.
    pub no_backend: bool,
    /// Accelerator from config.json "hotkey" (e.g. "ctrl+alt+s"), may be empty.
    pub hotkey: String,
    /// `--hidden` on the command line (autostart): start in the tray.
    pub hidden: bool,
}

impl Config {
    pub fn load() -> Config {
        let root = find_root();
        let json = read_config_json(&root);
        let port = json
            .as_ref()
            .and_then(|v| v.get("port"))
            .and_then(|v| v.as_u64())
            .filter(|p| *p > 0 && *p < 65536)
            .map(|p| p as u16)
            .unwrap_or(DEFAULT_PORT);
        let hotkey = json
            .as_ref()
            .and_then(|v| v.get("hotkey"))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| DEFAULT_HOTKEY.to_string());
        let dev_url = std::env::var("SEMSEARCH_DEV_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let mut base = None;
        if let Some(d) = &dev_url {
            let d = if d.ends_with('/') { d.clone() } else { format!("{d}/") };
            match Url::parse(&d) {
                Ok(u) => base = Some(u),
                Err(e) => log(&format!("SEMSEARCH_DEV_URL {d:?} is invalid: {e}")),
            }
        }
        let dev = base.is_some();
        let base = base.unwrap_or_else(|| Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap());
        let no_backend = matches!(
            std::env::var("SEMSEARCH_NO_BACKEND").ok().as_deref().map(str::trim),
            Some("1") | Some("true") | Some("yes")
        );
        let hidden = std::env::args().skip(1).any(|a| a == "--hidden");
        Config { root, base, dev, no_backend, hotkey, hidden }
    }

    pub fn page_url(&self, route: &str) -> Url {
        self.base.join(route.trim_start_matches('/')).unwrap_or_else(|_| self.base.clone())
    }

    pub fn token(&self) -> Option<String> {
        fs::read_to_string(self.root.join("data").join("token"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    pub fn is_backend_url(&self, url: &Url) -> bool {
        url.scheme() == self.base.scheme()
            && url.host_str() == self.base.host_str()
            && url.port_or_known_default() == self.base.port_or_known_default()
    }
}

fn find_root() -> PathBuf {
    if let Ok(r) = std::env::var("SEMSEARCH_ROOT") {
        let r = r.trim();
        if !r.is_empty() {
            return PathBuf::from(r);
        }
    }
    let exe = std::env::current_exe().unwrap_or_default();
    let start = exe.parent().map(Path::to_path_buf).unwrap_or_default();
    let mut dir = Some(start.as_path());
    while let Some(d) = dir {
        if d.join("server.py").is_file() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    start
}

fn read_config_json(root: &Path) -> Option<serde_json::Value> {
    let text = fs::read_to_string(root.join("config.json")).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

// ---------------------------------------------------------------- logging

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Log to `<root>\data\shell.log` (rotated at 1 MB).
pub fn init_log(root: &Path) {
    let dir = root.join("data");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("shell.log");
    if fs::metadata(&path).map(|m| m.len() > 1_000_000).unwrap_or(false) {
        let _ = fs::rename(&path, dir.join("shell.log.1"));
    }
    let _ = LOG_PATH.set(path);
}

pub fn log(msg: &str) {
    let line = format!("{} [{}] {}\r\n", now_string(), std::process::id(), msg);
    #[cfg(debug_assertions)]
    eprint!("{line}");
    let _guard = LOG_LOCK.lock();
    if let Some(p) = LOG_PATH.get() {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(p) {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

fn now_string() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let mut st = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut st) };
    let st: windows_sys::Win32::Foundation::SYSTEMTIME = st;
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    )
}

// ---------------------------------------------------------------- Win32 helpers

/// Windows "app mode" is dark (HKCU\...\Themes\Personalize\AppsUseLightTheme == 0).
pub fn system_dark() -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let val = wide("AppsUseLightTheme");
    let mut data: u32 = 1;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    rc == 0 && data == 0
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Lets the already running instance take the foreground when this (second) instance asks it to show itself.
pub fn allow_any_foreground() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
}

/// Hide a window from Alt+Tab (WS_EX_TOOLWINDOW) — used for the tray popup.
pub fn make_tool_window(hwnd: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };
    let h = hwnd as windows_sys::Win32::Foundation::HWND;
    unsafe {
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        SetWindowLongPtrW(h, GWL_EXSTYLE, (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize));
    }
}

/// Small-icon size for the notification area at the current DPI (16 at 100 %, 20 at 125 %, 24 at 150 %, 32 at 200 %).
pub fn small_icon_size() -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSMICON};
    unsafe { GetSystemMetrics(SM_CXSMICON) }
}

/// `hwnd` is the foreground (active) window.
pub fn is_foreground(hwnd: isize) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    unsafe { GetForegroundWindow() as isize == hwnd }
}

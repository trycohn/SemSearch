//! Paths, configuration, logging and small Win32 helpers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tauri::Url;

pub const DEFAULT_PORT: u16 = 8765;
pub const DEFAULT_HOTKEY: &str = "ctrl+alt+s";

/// Startup configuration (environment + `config.json`) and the backend in use (docs/API.md §1, §3).
pub struct Config {
    /// Folder with server.py (env SEMSEARCH_ROOT overrides the upward search from the exe).
    pub root: PathBuf,
    /// Data folder: SEMSEARCH_DATA_DIR or %LOCALAPPDATA%\SemSearch (config.json, token, logs).
    pub data: PathBuf,
    /// Why SEMSEARCH_DATA_DIR was ignored (logged once the log is open).
    pub data_note: Option<String>,
    /// Preferred backend port: config.json "port", else 8765.
    pub port: u16,
    /// SEMSEARCH_DEV_URL is set: pages come from there and any HTTP answer counts as "ready".
    pub dev: bool,
    /// SEMSEARCH_NO_BACKEND=1: never spawn or shut down the backend.
    pub no_backend: bool,
    /// Accelerator from config.json "hotkey" (e.g. "ctrl+alt+s"), may be empty.
    pub hotkey: String,
    /// `--hidden` on the command line (autostart): start in the tray.
    pub hidden: bool,
    /// config.json "lang": "system" | "ru" | "en" (the UI language; "system" follows Windows).
    pub lang: String,
    /// config.json "theme" ("system" | "light" | "dark") and "accent" ("#rrggbb") for the bundled loading page.
    pub theme: String,
    pub accent: String,
    /// SEMSEARCH_DEV_URL, always ending with "/".
    dev_url: Option<Url>,
    /// Backend in use: port and session token. Set when the shell attaches to a running backend or spawns one.
    backend: Mutex<(u16, Option<String>)>,
}

impl Config {
    pub fn load() -> Config {
        let root = find_root();
        let (data, data_note) = data_dir();
        let json = read_config_json(&data, &root);
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
        let dev_env = std::env::var("SEMSEARCH_DEV_URL").ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let mut dev_url = None;
        if let Some(d) = &dev_env {
            let d = if d.ends_with('/') { d.clone() } else { format!("{d}/") };
            match Url::parse(&d) {
                Ok(u) => dev_url = Some(u),
                Err(e) => log(&format!("SEMSEARCH_DEV_URL {d:?} is invalid: {e}")),
            }
        }
        let dev = dev_url.is_some();
        let no_backend = matches!(
            std::env::var("SEMSEARCH_NO_BACKEND").ok().as_deref().map(str::trim),
            Some("1") | Some("true") | Some("yes")
        );
        let hidden = std::env::args().skip(1).any(|a| a == "--hidden");
        let backend = Mutex::new((port, None));
        let text = |key: &str, ok: &dyn Fn(&str) -> bool, default: &str| {
            json.as_ref()
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| ok(s))
                .unwrap_or_else(|| default.to_string())
        };
        let lang = text("lang", &|s| matches!(s, "system" | "ru" | "en"), "system");
        let theme = text("theme", &valid_theme, "system");
        let accent = text("accent", &valid_accent, "#005fb8");
        Config { root, data, data_note, port, dev, no_backend, hotkey, hidden, lang, theme, accent, dev_url, backend }
    }

    /// Base URL of the pages, always ending with "/": SEMSEARCH_DEV_URL or http://127.0.0.1:<port of the backend in use>/.
    pub fn base(&self) -> Url {
        match &self.dev_url {
            Some(u) => u.clone(),
            None => local_url(self.backend.lock().unwrap().0),
        }
    }

    pub fn page_url(&self, route: &str) -> Url {
        let base = self.base();
        base.join(route.trim_start_matches('/')).unwrap_or(base)
    }

    /// Token of the backend in use: the one the shell gave it, else `<data>\token` (always so with SEMSEARCH_DEV_URL).
    pub fn token(&self) -> Option<String> {
        let given = if self.dev { None } else { self.backend.lock().unwrap().1.clone() };
        given.or_else(|| self.file_token())
    }

    /// "theme" and "accent" from `config.json` as they are now (the settings page changes them while the shell runs);
    /// the values read at startup if the file cannot be read.
    pub fn appearance(&self) -> (String, String) {
        let json = read_config_json(&self.data, &self.root);
        let text = |key: &str, ok: &dyn Fn(&str) -> bool, default: &str| {
            json.as_ref()
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| ok(s))
                .unwrap_or_else(|| default.to_string())
        };
        (text("theme", &valid_theme, &self.theme), text("accent", &valid_accent, &self.accent))
    }

    /// `<data>\token`, written by the backend at startup.
    pub fn file_token(&self) -> Option<String> {
        fs::read_to_string(self.data.join("token"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// The backend in use is now the one on `port` (attached or spawned with `token`).
    pub fn set_backend(&self, port: u16, token: Option<String>) {
        *self.backend.lock().unwrap() = (port, token);
    }

    pub fn is_backend_url(&self, url: &Url) -> bool {
        let base = self.base();
        url.scheme() == base.scheme()
            && url.host_str() == base.host_str()
            && url.port_or_known_default() == base.port_or_known_default()
    }
}

fn valid_theme(s: &str) -> bool {
    matches!(s, "system" | "light" | "dark")
}

fn valid_accent(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// http://127.0.0.1:<port>/
pub fn local_url(port: u16) -> Url {
    Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap()
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
        // server.py alone is not enough: SemSearch.exe may sit below an unrelated project that has its own server.py
        if d.join("server.py").is_file() && d.join("app.py").is_file() {
            return d.to_path_buf();
        }
        dir = d.parent();
    }
    start
}

/// Data folder (docs/API.md §1): SEMSEARCH_DATA_DIR (absolute path), else `<LocalAppData>\SemSearch` (known folder
/// FOLDERID_LocalAppData, then %LOCALAPPDATA%, then %USERPROFILE%\AppData\Local). Also returns why
/// SEMSEARCH_DATA_DIR was ignored. The folder is created when the log is opened.
fn data_dir() -> (PathBuf, Option<String>) {
    let mut note = None;
    if let Ok(v) = std::env::var("SEMSEARCH_DATA_DIR") {
        let v = v.trim();
        if Path::new(v).is_absolute() {
            return (PathBuf::from(v), None);
        }
        if !v.is_empty() {
            note = Some(format!("SEMSEARCH_DATA_DIR ignored (not an absolute path): {v:?}"));
        }
    }
    let env_dir = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|d| d.is_absolute());
    let local = known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_LocalAppData)
        .or_else(|| env_dir("LOCALAPPDATA"))
        .or_else(|| env_dir("USERPROFILE").map(|d| d.join("AppData").join("Local")))
        .unwrap_or_else(std::env::temp_dir);
    (local.join("SemSearch"), note)
}

/// Path of a known folder (SHGetKnownFolderPath).
fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::SHGetKnownFolderPath;
    let mut p: *mut u16 = std::ptr::null_mut();
    unsafe {
        let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut p);
        let mut out = None;
        if hr >= 0 && !p.is_null() {
            let mut len = 0;
            while *p.add(len) != 0 {
                len += 1;
            }
            out = Some(PathBuf::from(std::ffi::OsString::from_wide(std::slice::from_raw_parts(p, len))));
        }
        CoTaskMemFree(p as *const _); // freed even when the call fails
        out.filter(|d| d.is_absolute())
    }
}

/// `<data>\config.json`; if it does not exist, `<root>\config.json` (old layout, read-only: the backend moves it).
fn read_config_json(data: &Path, root: &Path) -> Option<serde_json::Value> {
    let path = [data, root].iter().map(|d| d.join("config.json")).find(|p| p.is_file())?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

// ---------------------------------------------------------------- logging

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// Log to `<data>\shell.log` (rotated at 1 MB).
pub fn init_log(dir: &Path) {
    let _ = fs::create_dir_all(dir);
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

/// Language of the Windows UI for SemSearch: "ru" when it is Russian, otherwise "en"
/// (the backend's i18n.py and the pages' ui/i18n.js decide the same way).
pub fn system_lang() -> &'static str {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    const LANG_RUSSIAN: u16 = 0x19;
    if unsafe { GetUserDefaultUILanguage() } & 0x3FF == LANG_RUSSIAN {
        "ru"
    } else {
        "en"
    }
}

/// "system" | "ru" | "en" -> "ru" | "en".
pub fn resolve_lang(pref: &str) -> &'static str {
    match pref {
        "ru" => "ru",
        "en" => "en",
        _ => system_lang(),
    }
}

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

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Value `name` of HKCU\Software\Microsoft\Windows\CurrentVersion\Run (REG_SZ), if present.
pub fn run_entry(name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    let (key, val) = (wide(RUN_KEY), wide(name));
    let mut buf = vec![0u16; 4096];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            val.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr() as *mut _,
            &mut size,
        )
    };
    if rc != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

/// Set the Run value `name` (HKCU) to `command` and, like tauri-plugin-autostart's enable(), mark it enabled
/// for Task Manager (Explorer\StartupApproved\Run).
pub fn set_run_entry(name: &str, command: &str) -> Result<(), String> {
    use windows_sys::Win32::System::Registry::*;
    let (key, val, data) = (wide(RUN_KEY), wide(name), wide(command));
    let rc = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), REG_SZ, data.as_ptr() as *const _, (data.len() * 2) as u32)
    };
    if rc != 0 {
        return Err(format!("RegSetKeyValueW failed (error {rc})"));
    }
    let approved = wide(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");
    let mut hk: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, approved.as_ptr(), 0, KEY_SET_VALUE, &mut hk) } == 0 {
        let enabled = [2u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        unsafe {
            RegSetValueExW(hk, val.as_ptr(), 0, REG_BINARY, enabled.as_ptr(), enabled.len() as u32);
            RegCloseKey(hk);
        }
    }
    Ok(())
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

/// Per-monitor DPI awareness (v2) before anything is drawn: the native dialogs shown before the first window exists
/// (WebView2 missing, ...) are otherwise bitmap-scaled and blurry at 125 % and up. tao asks for the same later, which then
/// fails harmlessly because the awareness cannot be changed twice.
pub fn set_dpi_aware() {
    use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
}

/// Native message box: the shell has no console, and before the first window exists no UI of its own.
/// With `ok_cancel` it has two buttons; returns true when the user pressed OK.
pub fn message_box(title: &str, text: &str, ok_cancel: bool) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, IDOK, MB_ICONERROR, MB_OK, MB_OKCANCEL, MB_SETFOREGROUND};
    let (t, x) = (wide(title), wide(text));
    let flags = MB_ICONERROR | MB_SETFOREGROUND | if ok_cancel { MB_OKCANCEL } else { MB_OK };
    unsafe { MessageBoxW(std::ptr::null_mut(), x.as_ptr(), t.as_ptr(), flags) == IDOK }
}

/// Open a web address in the default browser.
pub fn open_url(url: &str) {
    use windows_sys::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, url) = (wide("open"), wide(url));
    unsafe {
        // ShellExecute may hand the address to shell extensions that need COM (only used right before the shell exits)
        CoInitializeEx(std::ptr::null(), (COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) as u32);
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), url.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL);
    }
}

/// Version of the WebView2 runtime that the windows will use (Evergreen, or WEBVIEW2_BROWSER_EXECUTABLE_FOLDER).
/// Err(None) = not installed (Windows 10 before 21H1, LTSC and Server editions come without it); Err(Some(hresult)) =
/// the loader failed for another reason.
pub fn webview2_version() -> Result<String, Option<i32>> {
    use webview2_com::Microsoft::Web::WebView2::Win32::GetAvailableCoreWebView2BrowserVersionString;
    use windows_core::{PCWSTR, PWSTR};
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    const ERROR_FILE_NOT_FOUND: i32 = 0x8007_0002_u32 as i32; // HRESULT_FROM_WIN32
    const ERROR_PATH_NOT_FOUND: i32 = 0x8007_0003_u32 as i32;
    let mut v = PWSTR::null();
    let res = unsafe { GetAvailableCoreWebView2BrowserVersionString(PCWSTR::null(), &mut v) };
    let text = if !v.is_null() { unsafe { v.to_string() }.ok() } else { None };
    if !v.is_null() {
        unsafe { CoTaskMemFree(v.as_ptr() as *const _) };
    }
    match res {
        Ok(()) => text.filter(|s| !s.is_empty()).ok_or(None),
        Err(e) if e.code().0 == ERROR_FILE_NOT_FOUND || e.code().0 == ERROR_PATH_NOT_FOUND => Err(None),
        Err(e) => Err(Some(e.code().0)),
    }
}

/// Panics of any thread go to shell.log: the release build has no console, and a panicking supervisor thread would
/// otherwise just stop restarting the backend without a trace.
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let payload = info.payload();
        let what = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        log(&format!("PANIC at {at}: {what}"));
    }));
}

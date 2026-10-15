//! Logging, paths, registry and small Win32 helpers.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::windows::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use windows_sys::Win32::System::Registry::*;

// ---------------------------------------------------------------- logging

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();
static LOG_LOCK: Mutex<()> = Mutex::new(());

/// `%TEMP%\SemSearch-setup.log` (rotated at 2 MB).
pub fn log_path() -> PathBuf {
    std::env::temp_dir().join("SemSearch-setup.log")
}

pub fn init_log() {
    let path = log_path();
    if fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = fs::rename(&path, path.with_extension("log.1"));
    }
    let _ = LOG_PATH.set(path);
}

pub fn log(msg: &str) {
    let line = format!("{} [{}] {}\r\n", now_string(), std::process::id(), msg);
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

// ---------------------------------------------------------------- strings, language

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn wide_path(p: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

/// Language of the Windows UI: "ru" when it is Russian, otherwise "en" (as the shell and the backend decide).
pub fn system_lang() -> &'static str {
    use windows_sys::Win32::Globalization::GetUserDefaultUILanguage;
    const LANG_RUSSIAN: u16 = 0x19;
    if unsafe { GetUserDefaultUILanguage() } & 0x3FF == LANG_RUSSIAN {
        "ru"
    } else {
        "en"
    }
}

/// Pick the Russian or the English text.
pub fn tr(lang: &str, ru: &str, en: &str) -> String {
    if lang == "ru" { ru.to_string() } else { en.to_string() }
}

// ---------------------------------------------------------------- paths

/// Absolute path without a trailing separator; `/` becomes `\`.
pub fn clean_path(s: &str) -> Option<PathBuf> {
    let s = s.trim().trim_matches('"').replace('/', "\\");
    if s.is_empty() {
        return None;
    }
    let p = std::path::absolute(Path::new(&s)).ok()?;
    let mut t = p.display().to_string();
    while t.ends_with('\\') && !t.ends_with(":\\") {
        t.pop();
    }
    Some(PathBuf::from(t))
}

fn norm(p: &Path) -> String {
    let mut s = p.display().to_string().replace('/', "\\").to_lowercase();
    while s.ends_with('\\') {
        s.pop();
    }
    s
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}

/// `p` is `dir` or lies inside it (case-insensitive, by path text).
pub fn is_under(p: &Path, dir: &Path) -> bool {
    let (p, d) = (norm(p), norm(dir));
    p == d || p.starts_with(&(d + "\\"))
}

/// A known folder (SHGetKnownFolderPath).
pub fn known_folder(id: &windows_sys::core::GUID) -> Option<PathBuf> {
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
            out = Some(PathBuf::from(OsString::from_wide(std::slice::from_raw_parts(p, len))));
        }
        CoTaskMemFree(p as *const _);
        out.filter(|d| d.is_absolute())
    }
}

/// `<LocalAppData>` (known folder, then %LOCALAPPDATA%, then %USERPROFILE%\AppData\Local), as the shell finds it.
pub fn local_app_data() -> PathBuf {
    let env_dir = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|d| d.is_absolute());
    known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_LocalAppData)
        .or_else(|| env_dir("LOCALAPPDATA"))
        .or_else(|| env_dir("USERPROFILE").map(|d| d.join("AppData").join("Local")))
        .unwrap_or_else(std::env::temp_dir)
}

/// The temp folder without a trailing separator, with the long path (GetTempPath can return 8.3 names).
pub fn temp_dir() -> PathBuf {
    let t = std::env::temp_dir();
    let t = fs::canonicalize(&t).map(strip_verbatim).unwrap_or(t);
    clean_path(&t.display().to_string()).unwrap_or(t)
}

/// `\\?\C:\x` -> `C:\x` (from fs::canonicalize).
pub fn strip_verbatim(p: PathBuf) -> PathBuf {
    let s = p.display().to_string();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        p
    }
}

// ---------------------------------------------------------------- registry (HKCU unless stated)

pub fn reg_get_string(root: HKEY, key: &str, name: &str) -> Option<String> {
    let (k, v) = (wide(key), wide(name));
    let mut size = 0u32;
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ | RRF_NOEXPAND;
    let rc = unsafe { RegGetValueW(root, k.as_ptr(), v.as_ptr(), flags, std::ptr::null_mut(), std::ptr::null_mut(), &mut size) };
    if rc != 0 || size == 0 {
        return None;
    }
    let mut buf = vec![0u16; (size as usize).div_ceil(2) + 1];
    let mut size = (buf.len() * 2) as u32;
    let rc = unsafe {
        RegGetValueW(root, k.as_ptr(), v.as_ptr(), flags, std::ptr::null_mut(), buf.as_mut_ptr() as *mut _, &mut size)
    };
    if rc != 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

pub fn reg_get_dword(root: HKEY, key: &str, name: &str) -> Option<u32> {
    let (k, v) = (wide(key), wide(name));
    let mut data = 0u32;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(root, k.as_ptr(), v.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), &mut data as *mut u32 as *mut _, &mut size)
    };
    (rc == 0).then_some(data)
}


pub fn reg_set_string(key: &str, name: &str, value: &str) -> Result<(), String> {
    let (k, v, d) = (wide(key), wide(name), wide(value));
    let rc = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), REG_SZ, d.as_ptr() as *const _, (d.len() * 2) as u32)
    };
    if rc != 0 {
        return Err(format!("RegSetKeyValueW {key}\\{name} failed (error {rc})"));
    }
    Ok(())
}

pub fn reg_set_dword(key: &str, name: &str, value: u32) -> Result<(), String> {
    let (k, v) = (wide(key), wide(name));
    let rc = unsafe {
        RegSetKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr(), REG_DWORD, &value as *const u32 as *const _, 4)
    };
    if rc != 0 {
        return Err(format!("RegSetKeyValueW {key}\\{name} failed (error {rc})"));
    }
    Ok(())
}

/// Delete a value; a missing value or key is fine.
pub fn reg_delete_value(key: &str, name: &str) -> Result<(), String> {
    let (k, v) = (wide(key), wide(name));
    let rc = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, k.as_ptr(), v.as_ptr()) };
    // ERROR_FILE_NOT_FOUND
    if rc != 0 && rc != 2 {
        return Err(format!("RegDeleteKeyValueW {key}\\{name} failed (error {rc})"));
    }
    Ok(())
}

/// Delete a key with its subkeys; a missing key is fine.
pub fn reg_delete_tree(key: &str) -> Result<(), String> {
    let k = wide(key);
    let rc = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc != 0 && rc != 2 {
        return Err(format!("RegDeleteTreeW {key} failed (error {rc})"));
    }
    let rc = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, k.as_ptr()) };
    if rc != 0 && rc != 2 {
        return Err(format!("RegDeleteKeyW {key} failed (error {rc})"));
    }
    Ok(())
}

// ---------------------------------------------------------------- autostart (as shell/src/util.rs)

pub const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Value `name` of HKCU\...\Run, if present.
pub fn run_entry(name: &str) -> Option<String> {
    reg_get_string(HKEY_CURRENT_USER, RUN_KEY, name)
}

/// Set the Run value `name` (HKCU) to `command` and, like the shell (and tauri-plugin-autostart's enable()),
/// mark it enabled for Task Manager (Explorer\StartupApproved\Run).
pub fn set_run_entry(name: &str, command: &str) -> Result<(), String> {
    reg_set_string(RUN_KEY, name, command)?;
    let approved = wide(APPROVED_KEY);
    let val = wide(name);
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

/// Remove the Run value `name` and its StartupApproved mark.
pub fn delete_run_entry(name: &str) -> Result<(), String> {
    reg_delete_value(RUN_KEY, name)?;
    reg_delete_value(APPROVED_KEY, name)
}

// ---------------------------------------------------------------- misc

/// Size of everything under `dir`, in bytes (links are not followed).
pub fn dir_size(dir: &Path) -> u64 {
    let mut total = 0;
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if let Ok(ft) = e.file_type() {
                if ft.is_dir() {
                    total += dir_size(&e.path());
                } else if ft.is_file() {
                    total += e.metadata().map(|m| m.len()).unwrap_or(0);
                }
            }
        }
    }
    total
}

/// Remove a folder tree, retrying for a while: files of just-killed processes and files under an antivirus
/// scan are released with a delay.
pub fn remove_dir_retry(dir: &Path, tries: u32) -> std::io::Result<()> {
    let mut last = Ok(());
    for i in 0..tries {
        if !dir.exists() {
            return Ok(());
        }
        last = fs::remove_dir_all(dir);
        if last.is_ok() {
            return Ok(());
        }
        if i + 1 < tries {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    if dir.exists() { last } else { Ok(()) }
}

/// Native message box (before the window exists, or in place of it). Returns the button id.
pub fn message_box(text: &str, caption: &str, flags: u32) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW;
    let (t, c) = (wide(text), wide(caption));
    unsafe { MessageBoxW(std::ptr::null_mut(), t.as_ptr(), c.as_ptr(), flags) }
}

/// Open a file or folder with its default program (ShellExecute "open").
pub fn shell_open(path: &Path) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let (verb, file) = (wide("open"), wide_path(path));
    let r = unsafe {
        ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
    };
    r as isize > 32
}

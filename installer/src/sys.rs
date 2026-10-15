//! What the installer checks and changes in Windows: version, VC++ runtime, WebView2, the GPU, free space,
//! shortcuts, the user environment, the engine's scheduled task.

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use windows_sys::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

use crate::proc::CREATE_NO_WINDOW;
use crate::util::{self, log, reg_get_dword, reg_get_string, tr, wide};

/// Windows build number (CurrentBuildNumber), 0 if unknown.
pub fn windows_build() -> u32 {
    reg_get_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Windows 10 1709 (build 16299) or newer. The setup exe is x64, so running at all means 64-bit Windows.
pub fn windows_ok(build: u32) -> bool {
    build >= 16299
}

/// The Visual C++ 2015-2022 x64 runtime is installed.
pub fn vc_redist_installed() -> bool {
    [
        r"SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\X64",
        r"SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\X64",
    ]
    .iter()
    .any(|k| reg_get_dword(HKEY_LOCAL_MACHINE, k, "Installed") == Some(1))
}

// ---------------------------------------------------------------- WebView2

const WEBVIEW2_CLIENT: &str = r"Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}";
const WEBVIEW2_BOOTSTRAPPER: &str = "https://go.microsoft.com/fwlink/p/?LinkId=2124703";

/// Version of the WebView2 Evergreen runtime, if installed (per machine or per user).
pub fn webview2_version() -> Option<String> {
    let places = [
        (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\WOW6432Node\{WEBVIEW2_CLIENT}")),
        (HKEY_LOCAL_MACHINE, format!(r"SOFTWARE\{WEBVIEW2_CLIENT}")),
        (HKEY_CURRENT_USER, format!(r"Software\{WEBVIEW2_CLIENT}")),
    ];
    places
        .iter()
        .filter_map(|(root, key)| reg_get_string(*root, key, "pv"))
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty() && v != "0.0.0.0")
}

/// Before the window: make sure WebView2 is there, offering to install it (native message boxes).
/// Returns false when the installer cannot go on.
pub fn ensure_webview2(lang: &str) -> bool {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IDYES, MB_ICONERROR, MB_ICONQUESTION, MB_OK, MB_YESNO};
    if let Some(v) = webview2_version() {
        log(&format!("WebView2 {v}"));
        return true;
    }
    log("WebView2 is missing");
    let caption = "SemSearch";
    let ask = tr(lang, "Нужен компонент Microsoft Edge WebView2. Установить?", "Microsoft Edge WebView2 is required. Install it?");
    if util::message_box(&ask, caption, MB_YESNO | MB_ICONQUESTION) != IDYES {
        return false;
    }
    let exe = util::temp_dir().join("MicrosoftEdgeWebview2Setup.exe");
    let never = std::sync::atomic::AtomicBool::new(false);
    let r = crate::net::download(WEBVIEW2_BOOTSTRAPPER, &exe, None, &never, &mut |_, _| {}, &mut || {});
    let installed = match r {
        Ok(()) => {
            let mut cmd = Command::new(&exe);
            cmd.args(["/silent", "/install"]);
            log(&format!("run: {}", crate::proc::describe(&cmd)));
            let st = cmd.status();
            log(&format!("  -> {st:?}"));
            let _ = std::fs::remove_file(&exe);
            webview2_version().is_some()
        }
        Err(e) => {
            log(&format!("WebView2 download failed: {e}"));
            false
        }
    };
    if !installed {
        let msg = tr(
            lang,
            "Не удалось установить Microsoft Edge WebView2. Проверьте подключение к интернету и запустите установку ещё раз.",
            "Couldn't install Microsoft Edge WebView2. Check the internet connection and run setup again.",
        );
        util::message_box(&msg, caption, MB_OK | MB_ICONERROR);
    }
    installed
}

// ---------------------------------------------------------------- GPU

#[derive(Clone, Serialize)]
pub struct Gpu {
    pub name: String,
    /// Compute capability, e.g. "8.9" ("" when nvidia-smi gives none).
    pub cap: String,
    /// NVIDIA driver version, e.g. "566.36" ("" when unknown).
    pub driver: String,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Oldest Windows driver for the CUDA 12.x (cu128) wheels, through CUDA minor-version compatibility.
const MIN_DRIVER: &str = "528.33";

/// "566.36" -> [566, 36]
fn version_parts(v: &str) -> Vec<u32> {
    v.trim().split('.').map(|p| p.trim().parse().unwrap_or(0)).collect()
}

/// `a >= b`, comparing the dotted versions number by number.
fn version_at_least(a: &str, b: &str) -> bool {
    let (a, b) = (version_parts(a), version_parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    true
}

/// Name of an NVIDIA display adapter from the device registry (works without a working driver).
fn nvidia_adapter() -> Option<String> {
    const CLASS: &str = r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
    (0..32).find_map(|i| {
        let key = format!(r"{CLASS}\{i:04}");
        let desc = reg_get_string(HKEY_LOCAL_MACHINE, &key, "DriverDesc")?;
        let provider = reg_get_string(HKEY_LOCAL_MACHINE, &key, "ProviderName").unwrap_or_default();
        (desc.to_lowercase().contains("nvidia") || provider.to_lowercase().contains("nvidia")).then_some(desc)
    })
}

/// The GPU for PyTorch cu128: the NVIDIA GPU with the highest compute capability (nvidia-smi). Supported when
/// compute capability >= 7.5 (the wheels are built for sm_75 and newer) and the driver is 528.33 or newer.
/// `reason` otherwise: "cap" (compute capability below 7.5) or "driver" (driver too old, nvidia-smi missing or
/// failing). None when the machine has no NVIDIA adapter at all.
pub fn detect_gpu() -> Option<Gpu> {
    let candidates = [
        PathBuf::from("nvidia-smi"),
        PathBuf::from(std::env::var("ProgramW6432").unwrap_or_else(|_| r"C:\Program Files".into()))
            .join(r"NVIDIA Corporation\NVSMI\nvidia-smi.exe"),
    ];
    let driver_problem = |name: Option<String>, cap: String, driver: String| {
        let name = name.or_else(nvidia_adapter).unwrap_or_else(|| "NVIDIA".into());
        Gpu { name, cap, driver, supported: false, reason: Some("driver".into()) }
    };
    for exe in candidates {
        let out = Command::new(&exe)
            .args(["--query-gpu=name,compute_cap,driver_version", "--format=csv,noheader,nounits"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        let Ok(out) = out else { continue };
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        log(&format!("nvidia-smi ({}): exit {:?}: {}", exe.display(), out.status.code(), text.trim()));
        if !out.status.success() {
            return Some(driver_problem(None, String::new(), String::new()));
        }
        // "NVIDIA GeForce RTX 4070 SUPER, 8.9, 566.36" (the name may contain commas: split from the right)
        let best = text
            .lines()
            .filter_map(|l| {
                let mut parts = l.rsplitn(3, ',');
                let driver = parts.next()?.trim().to_string();
                let cap = parts.next()?.trim().to_string();
                let name = parts.next()?.trim().to_string();
                let num = cap.parse::<f32>().ok()?;
                Some((name, cap, num, driver))
            })
            .max_by(|a, b| a.2.total_cmp(&b.2));
        let Some((name, cap, num, driver)) = best else {
            return Some(driver_problem(None, String::new(), String::new()));
        };
        let reason = if num < 7.5 - 1e-3 {
            Some("cap")
        } else if !version_at_least(&driver, MIN_DRIVER) {
            Some("driver")
        } else {
            None
        };
        return Some(Gpu { name, cap, driver, supported: reason.is_none(), reason: reason.map(String::from) });
    }
    // no nvidia-smi: an NVIDIA card without its driver, or no NVIDIA card
    let adapter = nvidia_adapter();
    log(&format!("nvidia-smi not found; NVIDIA adapter: {adapter:?}"));
    adapter.map(|name| driver_problem(Some(name), String::new(), String::new()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn driver_versions() {
        use super::version_at_least as ge;
        assert!(ge("566.36", "528.33"));
        assert!(ge("528.33", "528.33"));
        assert!(!ge("528.24", "528.33"));
        assert!(!ge("472.12", "528.33"));
        assert!(ge("1000.1", "528.33"));
    }
}

// ---------------------------------------------------------------- disk space

/// Drive of a path for display: "C:" (or the UNC share).
pub fn drive_of(p: &Path) -> String {
    match p.components().next() {
        Some(std::path::Component::Prefix(pre)) => pre.as_os_str().to_string_lossy().into_owned(),
        _ => String::new(),
    }
}

/// Free bytes for the current user on the volume of `p` (its nearest existing ancestor).
pub fn free_bytes(p: &Path) -> Option<u64> {
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut dir = Some(p);
    while let Some(d) = dir {
        if d.exists() {
            let w = util::wide_path(d);
            let mut free = 0u64;
            let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
            return (ok != 0).then_some(free);
        }
        dir = d.parent();
    }
    None
}

// ---------------------------------------------------------------- shortcuts

/// Create (or overwrite) a .lnk shortcut.
pub fn create_shortcut(lnk: &Path, target: &Path, args: &str, workdir: &Path, description: &str) -> Result<(), String> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IPersistFile, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    if let Some(dir) = lnk.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    unsafe {
        let init = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let r = (|| -> windows::core::Result<()> {
            let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;
            link.SetPath(&HSTRING::from(target.as_os_str()))?;
            link.SetArguments(&HSTRING::from(args))?;
            link.SetWorkingDirectory(&HSTRING::from(workdir.as_os_str()))?;
            link.SetIconLocation(&HSTRING::from(target.as_os_str()), 0)?;
            link.SetDescription(&HSTRING::from(description))?;
            let file: IPersistFile = link.cast()?;
            file.Save(&HSTRING::from(lnk.as_os_str()), true)?;
            Ok(())
        })();
        if init.is_ok() {
            CoUninitialize();
        }
        r.map_err(|e| format!("{}: {e}", lnk.display()))
    }
}

/// `<Start Menu>\Programs` of the user.
pub fn start_menu_programs() -> Option<PathBuf> {
    util::known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Programs)
}

pub fn desktop() -> Option<PathBuf> {
    util::known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Desktop)
}

// ---------------------------------------------------------------- environment

pub const ENV_KEY: &str = "Environment";

/// Tell running programs (Explorer first of all) that the user environment changed.
pub fn broadcast_env_change() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageTimeoutW, HWND_BROADCAST, SMTO_ABORTIFHUNG, WM_SETTINGCHANGE};
    let s = wide("Environment");
    let mut res = 0usize;
    unsafe {
        SendMessageTimeoutW(HWND_BROADCAST, WM_SETTINGCHANGE, 0, s.as_ptr() as isize, SMTO_ABORTIFHUNG, 5000, &mut res);
    }
}

// ---------------------------------------------------------------- engine task

/// The scheduled task "SemSearch Engine" exists (read-only query).
pub fn engine_task_exists() -> bool {
    let st = Command::new("schtasks")
        .args(["/query", "/tn", "SemSearch Engine"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    let exists = matches!(st, Ok(s) if s.success());
    log(&format!("scheduled task \"SemSearch Engine\" exists: {exists}"));
    exists
}

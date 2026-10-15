//! SemSearch setup: installs SemSearch for the current user, updates it and removes it (docs/INSTALLER.md).
//!
//!     SemSearch-Setup.exe                         window: install, or update when already installed
//!     SemSearch-Setup.exe --headless [--root P] [--data P] [--cpu|--gpu] [--no-mft] [--no-autostart]
//!                         [--desktop] [--lang ru|en]
//!     uninstall.exe --uninstall [--headless] [--wipe]
//!     … --headless --uninstall --root P [--wipe]
//!
//! SEMSEARCH_SETUP_TEST=1 (or --test): test mode for a machine where SemSearch is installed for real.
//! SEMSEARCH_PAYLOAD_DIR=<repository root>: take the program files from a checkout (development).

#![windows_subsystem = "windows"]

mod app;
mod manifest;
mod net;
mod payload;
mod proc;
mod setup;
mod steps;
mod sys;
mod trailer;
mod util;

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use setup::{Mode, Setup, Sink, StartArgs};
use util::{is_under, log, same_path};

#[derive(Default, Debug)]
struct Args {
    headless: bool,
    uninstall: bool,
    root: Option<String>,
    data: Option<String>,
    gpu: Option<bool>,
    no_mft: bool,
    no_autostart: bool,
    desktop: bool,
    lang: Option<String>,
    wipe: bool,
    test: bool,
}

impl Args {
    fn parse() -> Args {
        let mut a = Args::default();
        let mut it = std::env::args().skip(1);
        while let Some(arg) = it.next() {
            match arg.as_str() {
                "--headless" => a.headless = true,
                "--uninstall" => a.uninstall = true,
                "--root" => a.root = it.next(),
                "--data" => a.data = it.next(),
                "--cpu" => a.gpu = Some(false),
                "--gpu" => a.gpu = Some(true),
                "--no-mft" => a.no_mft = true,
                "--no-autostart" => a.no_autostart = true,
                "--desktop" => a.desktop = true,
                "--lang" => a.lang = it.next(),
                "--wipe" => a.wipe = true,
                "--test" => a.test = true,
                other => log(&format!("unknown argument {other:?}")),
            }
        }
        a
    }

    fn start_args(&self) -> StartArgs {
        StartArgs {
            root: self.root.clone(),
            data_dir: self.data.clone(),
            gpu: self.gpu,
            autostart: Some(!self.no_autostart),
            mft: Some(!self.no_mft),
            desktop: Some(self.desktop),
            lang: self.lang.clone(),
            theme: None,
            wipe: Some(self.wipe),
        }
    }
}

/// --headless: write to the console of the parent (the exe is a GUI program). Redirected handles (a pipe or a
/// file) are inherited as they are.
fn attach_console() {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_WRITE, OPEN_EXISTING};
    use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, SetStdHandle, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE};
    unsafe {
        let out = GetStdHandle(STD_OUTPUT_HANDLE);
        if !out.is_null() && out != INVALID_HANDLE_VALUE {
            return;
        }
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        let name = util::wide("CONOUT$");
        let h = CreateFileW(name.as_ptr(), GENERIC_READ | GENERIC_WRITE, FILE_SHARE_WRITE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut());
        if h != INVALID_HANDLE_VALUE {
            SetStdHandle(STD_OUTPUT_HANDLE, h);
            SetStdHandle(STD_ERROR_HANDLE, h);
        }
    }
}

/// The named mutex "SemSearchSetup"; None when another installer holds it.
fn single_instance() -> Option<isize> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let name = util::wide("SemSearchSetup");
    let h = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if h.is_null() || unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        return None;
    }
    Some(h as isize)
}

/// Bring the other installer's window to the front.
fn focus_other_window() {
    use windows_sys::Win32::UI::WindowsAndMessaging::{FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE};
    for mode in [Mode::Install, Mode::Update, Mode::Uninstall] {
        for lang in ["ru", "en"] {
            let t = util::wide(&app::window_title(mode, lang));
            let h = unsafe { FindWindowW(std::ptr::null(), t.as_ptr()) };
            if !h.is_null() {
                unsafe {
                    ShowWindow(h, SW_RESTORE);
                    SetForegroundWindow(h);
                }
                return;
            }
        }
    }
}

/// `%TEMP%\SemSearch-uninstall-<pid>.exe`: this process is the temporary copy that removes `<root>`.
fn is_uninstall_copy() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let name = exe.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let temp = util::temp_dir();
    name.starts_with("semsearch-uninstall-")
        && exe.parent().is_some_and(|d| same_path(d, &temp) || same_path(&util::strip_verbatim(std::fs::canonicalize(d).unwrap_or_default()), &temp))
}

/// uninstall.exe cannot delete its own folder while it runs: copy it to %TEMP% and run the copy with
/// `--uninstall --root <root>`.
fn hand_off(a: &Args, root: &std::path::Path) -> i32 {
    let exe = std::env::current_exe().unwrap_or_default();
    let copy = util::temp_dir().join(format!("SemSearch-uninstall-{}.exe", std::process::id()));
    if let Err(e) = std::fs::copy(&exe, &copy) {
        log(&format!("hand-off: copy to {} failed: {e}", copy.display()));
        let msg = util::tr(util::system_lang(), "Не удалось начать удаление", "Couldn't start uninstalling");
        if a.headless {
            println!("{}", serde_json::json!({"event": "setup://error", "id": "start", "kind": "other", "message": format!("{msg}: {e}")}));
        } else {
            util::message_box(&format!("{msg}: {e}"), "SemSearch", windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR);
        }
        return 1;
    }
    let mut cmd = Command::new(&copy);
    cmd.arg("--uninstall").arg("--root").arg(root);
    for (on, flag) in [(a.headless, "--headless"), (a.wipe, "--wipe"), (a.test, "--test")] {
        if on {
            cmd.arg(flag);
        }
    }
    if let Some(l) = &a.lang {
        cmd.arg("--lang").arg(l);
    }
    cmd.current_dir(util::temp_dir());
    let r = cmd.spawn();
    log(&format!("hand-off to {}: {:?}", copy.display(), r.as_ref().map(|c| c.id())));
    if r.is_err() { 1 } else { 0 }
}

/// The temporary uninstall copy deletes itself once it has exited.
fn delete_self_later() {
    let Ok(exe) = std::env::current_exe() else { return };
    let line = format!("/c ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"", exe.display());
    let r = Command::new("cmd.exe").raw_arg(&line).creation_flags(proc::CREATE_NO_WINDOW).current_dir(util::temp_dir()).spawn();
    log(&format!("self-delete scheduled: {:?}", r.map(|c| c.id())));
}

fn main() {
    let a = Args::parse();
    if a.headless {
        attach_console();
    }
    util::init_log();
    let env_test = matches!(std::env::var("SEMSEARCH_SETUP_TEST").ok().as_deref().map(str::trim), Some("1") | Some("true") | Some("yes"));
    let mut test = a.test || env_test;
    log(&format!(
        "=== SemSearch setup {} {:?}: {:?}, test={test}",
        setup::VERSION,
        std::env::current_exe().unwrap_or_default(),
        std::env::args().skip(1).collect::<Vec<_>>()
    ));

    // mode and the folder to remove
    let mut uninstall_root: Option<PathBuf> = None;
    let mut installed = None;
    let mode = if a.uninstall {
        let exe = std::env::current_exe().unwrap_or_default();
        let root = a
            .root
            .as_deref()
            .and_then(util::clean_path)
            .or_else(|| exe.parent().map(PathBuf::from))
            .unwrap_or_default();
        if is_under(&exe, &root) {
            std::process::exit(hand_off(&a, &root));
        }
        let info = setup::read_install_json(&root);
        // an installation made in test mode is removed in test mode, whatever the environment says
        if info.as_ref().is_some_and(|i| i.test) && !test {
            log("install.json says test: test mode");
            test = true;
        }
        installed = info.map(|info| setup::Installed { root: root.clone(), info });
        uninstall_root = Some(root);
        Mode::Uninstall
    } else {
        match setup::find_installed(setup::product(test)) {
            Some(i) => {
                installed = Some(i);
                Mode::Update
            }
            None => Mode::Install,
        }
    };

    let Some(_mutex) = single_instance() else {
        log("another setup is running");
        if a.headless {
            println!("{}", serde_json::json!({"event": "setup://error", "id": "start", "kind": "other", "message": "another SemSearch setup is running"}));
        } else {
            focus_other_window();
        }
        std::process::exit(1);
    };

    let sink = if a.headless { Sink::Stdout } else { Sink::Tauri(Default::default()) };
    let s = Arc::new(Setup::new(mode, test, installed, uninstall_root, sink));
    let signed = std::env::current_exe().map(|e| trailer::is_signed(&e)).unwrap_or(false);
    log(&format!("mode {}, payload: {}, exe signed: {signed}", mode.as_str(), s.payload.describe()));

    let code = if a.headless {
        s.run_headless(a.start_args())
    } else if sys::ensure_webview2(s.sys_lang) {
        let code = app::run(s.clone());
        // the window closed, maybe in the middle of a step: end pip / the model script before leaving
        s.cancel();
        code
    } else {
        1
    };
    log(&format!("exit {code}"));
    if is_uninstall_copy() {
        delete_self_later();
    }
    std::process::exit(code);
}

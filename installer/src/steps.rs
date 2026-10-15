//! The steps of install, update and uninstall (docs/INSTALLER.md, "Шаги").

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

use serde_json::json;
use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;

use crate::net::{self, NetError};
use crate::proc::{self, Elevated};
use crate::setup::*;
use crate::sys;
use crate::util::{self, is_under, log, same_path};

const VC_REDIST_URL: &str = "https://aka.ms/vs/17/release/vc_redist.x64.exe";
const PIP_FLAGS: &[&str] = &["--disable-pip-version-check", "--no-input", "--progress-bar", "off", "--no-warn-script-location"];

/// An io::Error of a step: disk full -> `space`, cancel -> `cancelled`, the rest -> `other`.
fn io_err(c: &Ctx, cancelled: bool, what: &str, e: &std::io::Error) -> StepErr {
    if cancelled {
        return StepErr::new("cancelled", c.tr("Остановлено.", "Stopped."));
    }
    if matches!(e.raw_os_error(), Some(39) | Some(112)) {
        return StepErr::new("space", c.tr(&format!("Не хватило места на диске: {e}"), &format!("The disk is full: {e}")));
    }
    StepErr::other(format!("{what}: {e}"))
}

fn net_err(c: &Ctx, what: &str, e: NetError) -> StepErr {
    match e {
        NetError::Network(m) => StepErr::new("network", c.tr(&format!("{what}: нет связи ({m})"), &format!("{what}: no connection ({m})"))),
        NetError::Space(m) => StepErr::new("space", c.tr(&format!("{what}: не хватило места на диске ({m})"), &format!("{what}: the disk is full ({m})"))),
        NetError::Cancelled => StepErr::new("cancelled", c.tr("Остановлено.", "Stopped.")),
        other => StepErr::other(format!("{what}: {other}")),
    }
}

/// Classify a failed pip run by its output.
fn pip_err(c: &Ctx, cancelled: bool, out: &proc::Output) -> StepErr {
    if cancelled {
        return StepErr::new("cancelled", c.tr("Остановлено.", "Stopped."));
    }
    let text = out.tail.join("\n");
    let has = |pats: &[&str]| pats.iter().any(|p| text.contains(p));
    let tail = out.tail_text(12);
    if has(&["No space left on device", "[Errno 28]", "not enough space on the disk", "WinError 112"]) {
        return StepErr::new("space", c.tr(&format!("Не хватило места на диске.\n{tail}"), &format!("The disk is full.\n{tail}")));
    }
    if has(&[
        "NewConnectionError",
        "ConnectTimeoutError",
        "ReadTimeoutError",
        "Max retries exceeded",
        "Connection aborted",
        "ConnectionResetError",
        "ProxyError",
        "getaddrinfo failed",
        "Temporary failure in name resolution",
        "IncompleteRead",
        "Connection broken",
        "SSLError",
    ]) {
        return StepErr::new("network", c.tr(&format!("Нет связи.\n{tail}"), &format!("No connection.\n{tail}")));
    }
    StepErr::other(format!("pip: exit code {}\n{tail}", out.code))
}

/// powershell.exe of the system (not whatever `powershell` PATH finds first).
fn powershell() -> PathBuf {
    let sys_root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let p = sys_root.join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    if p.is_file() { p } else { PathBuf::from("powershell.exe") }
}

/// The torch in `<root>\.venv` is not the one requirements-torch.txt asks for (or there is none).
pub fn torch_differs(c: &Ctx) -> bool {
    let Some(t) = &c.torch else { return true };
    let want = format!("torch-{}+{}.dist-info", t.torch, c.variant()).to_lowercase();
    let site = c.root.join(".venv").join("Lib").join("site-packages");
    let have: Vec<String> = fs::read_dir(&site)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.file_name().to_string_lossy().to_lowercase())
                .filter(|n| n.starts_with("torch-") && n.ends_with(".dist-info"))
                .collect()
        })
        .unwrap_or_default();
    log(&format!("torch in the venv: {have:?}, wanted {want}"));
    !have.contains(&want)
}

/// `href` values of a PEP 503 page, resolved against `page`.
fn links(page: &str, html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("href=") {
        rest = &rest[i + 5..];
        let q = match rest.chars().next() {
            Some(c @ ('"' | '\'')) => c,
            _ => continue,
        };
        rest = &rest[1..];
        let Some(end) = rest.find(q) else { break };
        let href = rest[..end].replace("&amp;", "&");
        rest = &rest[end..];
        out.push(join_url(page, &href));
    }
    out
}

fn join_url(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        return href.to_string();
    }
    let (scheme, after) = base.split_once("://").unwrap_or(("https", base));
    let host = after.split('/').next().unwrap_or("");
    if let Some(abs) = href.strip_prefix('/') {
        return format!("{scheme}://{host}/{abs}");
    }
    let mut dir: Vec<&str> = after.split('/').collect();
    dir.pop(); // the page's own last segment (empty for ".../torch/")
    let mut h = href;
    loop {
        if let Some(r) = h.strip_prefix("./") {
            h = r;
        } else if let Some(r) = h.strip_prefix("../") {
            if dir.len() > 1 {
                dir.pop();
            }
            h = r;
        } else {
            break;
        }
    }
    format!("{scheme}://{}/{h}", dir.join("/"))
}

/// Looks like a SemSearch program folder (a previous or partial install).
fn looks_like_root(p: &Path) -> bool {
    ["install.json", "server.py", ".setup-cache", "runtime", ".venv"].iter().any(|n| p.join(n).exists())
}

/// Looks like a SemSearch data folder.
fn looks_like_data(p: &Path) -> bool {
    ["config.json", "index.db", "names.db", "models", "semsearch.log", "shell.log", "token", "thumbs"]
        .iter()
        .any(|n| p.join(n).exists())
}

fn is_empty_dir(p: &Path) -> bool {
    fs::read_dir(p).map(|mut rd| rd.next().is_none()).unwrap_or(true)
}

fn write_test(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    let f = dir.join(".setup-write-test");
    fs::write(&f, b"ok")?;
    fs::remove_file(&f)
}

impl Setup {
    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    // ------------------------------------------------------------ install

    pub(crate) fn step_check(&self, c: &Ctx) -> StepResult {
        let build = sys::windows_build();
        if !sys::windows_ok(build) {
            return Err(StepErr::other(c.tr(
                &format!("Нужна Windows 10 версии 1709 или новее (сейчас сборка {build})."),
                &format!("Windows 10 version 1709 or newer is required (this is build {build})."),
            )));
        }
        let disk = disk_info(&c.root, &c.data, c.gpu);
        if disk["ok"] != json!(true) {
            let gb = |v: &serde_json::Value| v.as_u64().unwrap_or(0) as f64 / (1u64 << 30) as f64;
            return Err(StepErr::new(
                "space",
                c.tr(
                    &format!("Мало места: на {} нужно {:.1} ГБ, свободно {:.1} ГБ.", disk["drive"].as_str().unwrap_or(""), gb(&disk["need_bytes"]), gb(&disk["free_bytes"])),
                    &format!("Not enough space: {} needs {:.1} GB, {:.1} GB free.", disk["drive"].as_str().unwrap_or(""), gb(&disk["need_bytes"]), gb(&disk["free_bytes"])),
                ),
            ));
        }
        for (dir, ours) in [(&c.root, looks_like_root(&c.root)), (&c.data, looks_like_data(&c.data))] {
            if dir.exists() && !is_empty_dir(dir) && !ours {
                return Err(StepErr::other(c.tr(
                    &format!("Папка «{}» не пуста. Выберите пустую или новую папку.", dir.display()),
                    &format!("The folder “{}” is not empty. Choose an empty or a new folder.", dir.display()),
                )));
            }
            if let Err(e) = write_test(dir) {
                return Err(StepErr::other(c.tr(
                    &format!("Нет доступа на запись в «{}»: {e}", dir.display()),
                    &format!("Can't write to “{}”: {e}", dir.display()),
                )));
            }
        }
        fs::create_dir_all(c.cache()).map_err(|e| io_err(c, false, &c.cache().display().to_string(), &e))?;
        Ok(Outcome::Done)
    }

    pub(crate) fn step_vcredist(&self, c: &Ctx) -> StepResult {
        let exe = c.cache().join("vc_redist.x64.exe");
        let mut th = Throttle::new(4);
        let mut last = (0u64, None);
        net::download(
            VC_REDIST_URL,
            &exe,
            None,
            &self.cancel,
            &mut |got, total| {
                last = (got, total);
                if th.ready() {
                    self.progress("vcredist", got, total)
                }
            },
            &mut || {},
        )
        .map_err(|e| net_err(c, "Visual C++", e).at(last.0, last.1))?;
        self.note("vcredist", "uac");
        match proc::run_elevated(&exe, "/install /quiet /norestart", self.hwnd.load(Ordering::SeqCst)) {
            Elevated::Exit(0 | 1638 | 3010) => Ok(Outcome::Done),
            Elevated::Exit(code) => Err(StepErr::other(format!("vc_redist.x64.exe: exit code {code}"))),
            Elevated::Denied => Err(StepErr::new(
                "uac_denied",
                c.tr(
                    "Windows не дала разрешение на установку Visual C++ — без него SemSearch не запустится.",
                    "Windows didn't grant permission to install Visual C++ — SemSearch can't run without it.",
                ),
            )),
            Elevated::Failed(m) => Err(StepErr::other(format!("vc_redist.x64.exe: {m}"))),
        }
    }

    /// `copy` (install) and `replace` (update): program files, Python runtime, venv, uninstall.exe.
    pub(crate) fn step_copy(&self, c: &Ctx, id: &str) -> StepResult {
        let stuck = proc::close_under(&c.root);
        if !stuck.is_empty() {
            return Err(StepErr::other(c.tr(
                &format!("Не удалось закрыть SemSearch: {}", stuck.join(", ")),
                &format!("Couldn't close SemSearch: {}", stuck.join(", ")),
            )));
        }
        fs::create_dir_all(c.cache()).map_err(|e| io_err(c, false, &c.cache().display().to_string(), &e))?;
        let files = self.payload.app_files().map_err(|e| StepErr::other(format!("payload: {e}")))?;
        let (sha, rt_size) = self.payload.runtime_info().map_err(|e| StepErr::other(format!("Python: {e}")))?;
        let runtime = c.root.join("runtime");
        let marker = runtime.join(".payload-sha256");
        let runtime_ok = runtime.join("python.exe").is_file() && fs::read_to_string(&marker).map(|s| s.trim() == sha).unwrap_or(false);
        let total = files.iter().map(|f| f.1).sum::<u64>() + if runtime_ok { 0 } else { rt_size };
        let mut got = 0u64;
        let mut th = Throttle::new(4);
        self.progress(id, 0, Some(total));
        let mut tick = |n: u64| {
            got += n;
            if th.ready() {
                self.progress(id, got, Some(total));
            }
        };
        self.payload
            .extract_app(&c.root, &self.cancel, &mut tick)
            .map_err(|e| io_err(c, self.cancelled(), "copy", &e))?;
        log(&format!("copied {} program files", files.len()));

        if !runtime_ok {
            self.note(id, "python");
            util::remove_dir_retry(&runtime, 10).map_err(|e| io_err(c, false, &runtime.display().to_string(), &e))?;
            self.payload
                .extract_runtime(&runtime, &self.cancel, &mut tick)
                .map_err(|e| io_err(c, self.cancelled(), "Python", &e))?;
            fs::write(&marker, &sha).map_err(|e| io_err(c, false, &marker.display().to_string(), &e))?;
            log(&format!("Python runtime unpacked to {}", runtime.display()));
        }
        self.progress(id, total, Some(total));

        self.note(id, "venv");
        let venv = c.root.join(".venv");
        let base_py = runtime.join("python.exe");
        let run_py = |args: &[&std::ffi::OsStr]| -> Result<proc::Output, StepErr> {
            if self.cancelled() {
                return Err(StepErr::new("cancelled", c.tr("Остановлено.", "Stopped.")));
            }
            let mut cmd = Command::new(&base_py);
            cmd.args(args).current_dir(c.cache());
            c.child_env(&mut cmd);
            proc::run(cmd, &self.job, |_| {}).map_err(|e| StepErr::other(format!("{}: {e}", base_py.display())))
        };
        if self.venv_ok(c) {
            if !runtime_ok {
                // a new runtime in place: refresh the venv's launchers
                let out = run_py(&["-I".as_ref(), "-m".as_ref(), "venv".as_ref(), "--upgrade".as_ref(), venv.as_os_str()])?;
                if out.code != 0 {
                    return Err(StepErr::other(format!("venv --upgrade: exit code {}\n{}", out.code, out.tail_text(12))));
                }
            }
        } else {
            util::remove_dir_retry(&venv, 10).map_err(|e| io_err(c, false, &venv.display().to_string(), &e))?;
            let out = run_py(&["-I".as_ref(), "-m".as_ref(), "venv".as_ref(), venv.as_os_str()])?;
            if out.code != 0 {
                return Err(io_or_other(c, self.cancelled(), "venv", &out));
            }
        }
        let pythonw = venv.join("Scripts").join("pythonw.exe");
        if !pythonw.is_file() || !c.venv_python().is_file() {
            return Err(StepErr::other(format!("{} is missing after creating the venv", pythonw.display())));
        }

        let un = c.root.join("uninstall.exe");
        self.payload.write_uninstaller(&un).map_err(|e| io_err(c, false, &un.display().to_string(), &e))?;
        Ok(Outcome::Done)
    }

    /// `<root>\.venv` exists, belongs to `<root>\runtime` and starts.
    fn venv_ok(&self, c: &Ctx) -> bool {
        let py = c.venv_python();
        if !py.is_file() {
            return false;
        }
        let home = fs::read_to_string(c.root.join(".venv").join("pyvenv.cfg"))
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| {
                    let (k, v) = l.split_once('=')?;
                    (k.trim() == "home").then(|| PathBuf::from(v.trim()))
                })
            });
        if !home.is_some_and(|h| same_path(&h, &c.root.join("runtime"))) {
            log("venv: missing or made by another Python");
            return false;
        }
        let mut cmd = Command::new(&py);
        cmd.args(["-I", "-c", "import sys"]).current_dir(c.cache());
        c.child_env(&mut cmd);
        matches!(proc::run(cmd, &self.job, |_| {}), Ok(o) if o.code == 0)
    }

    pub(crate) fn step_torch(&self, c: &Ctx) -> StepResult {
        let wheel = c.torch_wheel().ok_or_else(|| StepErr::other("requirements-torch.txt"))?;
        let page = format!("{}/torch/", c.torch_index());
        let dest = c.cache().join(&wheel);
        // what an earlier attempt left: reported with a failure before the download resumes
        let part = fs::metadata(net::part_path(&dest)).map(|m| m.len()).unwrap_or(0);
        log(&format!("torch: {wheel} from {page}, {part} bytes already here, proxy: {}", net::proxy_description()));
        let html = net::get_text(&page).map_err(|e| {
            let err = net_err(c, "PyTorch", e);
            if part > 0 { err.at(part, None) } else { err }
        })?;
        let found = links(&page, &html).into_iter().find_map(|url| {
            let (path, frag) = url.split_once('#').unwrap_or((url.as_str(), ""));
            let name = crate::manifest::percent_decode(path.rsplit('/').next().unwrap_or(""));
            name.eq_ignore_ascii_case(&wheel).then(|| (path.to_string(), frag.strip_prefix("sha256=").map(|s| s.to_ascii_lowercase())))
        });
        let Some((url, sha)) = found else {
            return Err(StepErr::other(format!("{wheel} is not listed on {page}")));
        };
        let Some(sha) = sha.filter(|s| s.len() == 64) else {
            return Err(StepErr::other(format!("{page}: no sha256 for {wheel}")));
        };
        log(&format!("torch: {url} sha256={sha}"));
        let mut th = Throttle::new(4);
        let mut last = (0u64, None);
        net::download(
            &url,
            &dest,
            Some(&sha),
            &self.cancel,
            &mut |got, total| {
                last = (got, total);
                if th.ready() {
                    self.progress("torch", got, total)
                }
            },
            &mut || self.note("torch", "verify"),
        )
        .map_err(|e| {
            log(&format!("torch download stopped at {} of {:?} bytes: {e}", last.0, last.1));
            net_err(c, "PyTorch", e).at(last.0, last.1)
        })?;
        self.progress("torch", last.0, last.1);
        Ok(Outcome::Done)
    }

    pub(crate) fn step_libs(&self, c: &Ctx) -> StepResult {
        let t = c.torch.clone().ok_or_else(|| StepErr::other("requirements-torch.txt"))?;
        let py = c.venv_python();
        let index = c.torch_index();
        let wheel = c.torch_wheel().map(|w| c.cache().join(w)).filter(|p| p.is_file());
        fs::create_dir_all(c.cache()).map_err(|e| io_err(c, false, "cache", &e))?;
        let pip = |args: Vec<std::ffi::OsString>| -> StepResult {
            if self.cancelled() {
                return Err(StepErr::new("cancelled", c.tr("Остановлено.", "Stopped.")));
            }
            let mut cmd = Command::new(&py);
            cmd.args(["-m", "pip", "install"]).args(args).args(PIP_FLAGS).current_dir(c.cache());
            c.child_env(&mut cmd);
            let out = proc::run(cmd, &self.job, |_| {}).map_err(|e| StepErr::other(format!("{}: {e}", py.display())))?;
            if out.code != 0 {
                return Err(pip_err(c, self.cancelled(), &out));
            }
            Ok(Outcome::Done)
        };
        let torch_spec: std::ffi::OsString = match &wheel {
            Some(w) => w.clone().into_os_string(),
            None => format!("torch=={}", t.torch).into(),
        };
        pip(vec![torch_spec, format!("torchvision=={}", t.vision).into(), "--index-url".into(), index.into()])?;
        pip(vec!["-r".into(), c.root.join("requirements.txt").into_os_string()])?;
        if let Some(w) = wheel {
            let _ = fs::remove_file(w);
        }
        Ok(Outcome::Done)
    }

    pub(crate) fn step_model(&self, c: &Ctx) -> StepResult {
        let script = c.root.join("tools").join("fetch_model.py");
        if !script.is_file() {
            return Err(StepErr::other(format!("{} is missing", script.display())));
        }
        let py = c.venv_python();
        let mut cmd = Command::new(&py);
        cmd.arg("-u").arg(&script).current_dir(&c.root);
        c.child_env(&mut cmd);
        let mut th = Throttle::new(4);
        let mut done = false;
        let mut error: Option<(String, String)> = None;
        let mut last = (0u64, None);
        let out = proc::run(cmd, &self.job, |line| {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line.trim()) else { return };
            if let Some(got) = v.get("got").and_then(|g| g.as_u64()) {
                last = (got, v.get("total").and_then(|t| t.as_u64()));
                if th.ready() {
                    self.progress("model", last.0, last.1);
                }
            }
            if v.get("done").and_then(|d| d.as_bool()) == Some(true) {
                done = true;
            }
            if let Some(e) = v.get("error").and_then(|e| e.as_str()) {
                let kind = v.get("kind").and_then(|k| k.as_str()).unwrap_or("other").to_string();
                error = Some((kind, e.to_string()));
            }
        })
        .map_err(|e| StepErr::other(format!("{}: {e}", py.display())))?;
        if self.cancelled() {
            return Err(StepErr::new("cancelled", c.tr("Остановлено.", "Stopped.")).at(last.0, last.1));
        }
        if out.code == 0 && error.is_none() {
            if !done {
                log("fetch_model.py ended without {\"done\": true}");
            }
            return Ok(Outcome::Done);
        }
        match error {
            Some((kind, msg)) => {
                let kind = match kind.as_str() {
                    "network" => "network",
                    "space" => "space",
                    _ => "other",
                };
                Err(StepErr::new(kind, msg).at(last.0, last.1))
            }
            None => Err(StepErr::other(format!("fetch_model.py: exit code {}\n{}", out.code, out.tail_text(12))).at(last.0, last.1)),
        }
    }

    /// Install or update the engine's scheduled task (UAC). Never in test mode: the task is shared.
    pub(crate) fn step_mft(&self, c: &Ctx) -> StepResult {
        if c.test {
            log("test mode: mft skipped");
            return Ok(Outcome::Skipped);
        }
        let script = c.root.join("engine").join("install-task.ps1");
        let mut cmd = Command::new(powershell());
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script).arg("-NoPause").current_dir(&c.root);
        c.child_env(&mut cmd);
        self.note("mft", "uac");
        let out = proc::run(cmd, &self.job, |_| {}).map_err(|e| StepErr::other(format!("powershell: {e}")))?;
        match out.code {
            0 => Ok(Outcome::Done),
            2 => Err(StepErr::new(
                "not_admin",
                c.tr(
                    "Учётная запись не администратор: быстрый поиск по имени не подключён.",
                    "This account isn't an administrator: fast search by name isn't set up.",
                ),
            )),
            _ if out.tail.iter().any(|l| l.contains("Elevation was cancelled")) => Err(StepErr::new(
                "uac_denied",
                c.tr("Windows не дала разрешение.", "Windows didn't grant permission."),
            )),
            code => Err(StepErr::other(format!("install-task.ps1: exit code {code}\n{}", out.tail_text(12)))),
        }
    }

    pub(crate) fn step_links(&self, c: &Ctx) -> StepResult {
        let p = c.product;
        let exe = c.root.join("SemSearch.exe");
        let lnk_name = format!("{}.lnk", p.display);
        let programs = sys::start_menu_programs().ok_or_else(|| StepErr::other("no Start Menu folder"))?;
        sys::create_shortcut(&programs.join(&lnk_name), &exe, "", &c.root, "SemSearch").map_err(StepErr::other)?;
        if let Some(desk) = sys::desktop() {
            let lnk = desk.join(&lnk_name);
            // update: refresh a desktop shortcut the user kept, don't bring back one they removed
            let want = if c.mode == Mode::Install { c.desktop } else { lnk.exists() };
            if want {
                sys::create_shortcut(&lnk, &exe, "", &c.root, "SemSearch").map_err(StepErr::other)?;
            }
        }
        // autostart: on install only; afterwards it belongs to the app's settings
        if c.mode == Mode::Install && c.autostart {
            util::set_run_entry(p.key, &format!("\"{}\" --hidden", exe.display())).map_err(StepErr::other)?;
        }

        // SEMSEARCH_DATA_DIR for a data folder other than the default; never in test mode (it would move the
        // real installation's data)
        let mut data_env = c.installed.as_ref().is_some_and(|i| i.data_dir_env);
        if c.mode == Mode::Install {
            data_env = !c.data_default && !c.test;
            if data_env {
                util::reg_set_string(sys::ENV_KEY, "SEMSEARCH_DATA_DIR", &c.data.display().to_string()).map_err(StepErr::other)?;
                sys::broadcast_env_change();
            } else if c.test && !c.data_default {
                log("test mode: SEMSEARCH_DATA_DIR is not written to HKCU\\Environment");
            }
        }

        let info = InstallJson {
            version: VERSION.to_string(),
            data_dir: c.data.display().to_string(),
            data_dir_env: data_env,
            gpu: c.gpu,
            mft: c.mft && self.mft_ok() && !c.test,
            autostart: match &c.installed {
                Some(i) if c.mode == Mode::Update => i.autostart,
                _ => c.autostart,
            },
            desktop: match &c.installed {
                Some(i) if c.mode == Mode::Update => i.desktop,
                _ => c.desktop,
            },
            test: c.test,
            lang: c.lang.clone(),
        };
        write_install_json(&c.root, &info).map_err(|e| io_err(c, false, "install.json", &e))?;

        let key = format!(r"{UNINSTALL_BASE}\{}", p.key);
        let un = c.root.join("uninstall.exe");
        let test_flag = if c.test { " --test" } else { "" };
        let size_kb = (util::dir_size(&c.root) / 1024).min(u32::MAX as u64) as u32;
        let root = c.root.display().to_string();
        for (name, value) in [
            ("DisplayName", p.display.to_string()),
            ("DisplayVersion", VERSION.to_string()),
            ("Publisher", "SemSearch".to_string()),
            ("DisplayIcon", exe.display().to_string()),
            ("InstallLocation", root),
            ("UninstallString", format!("\"{}\" --uninstall{test_flag}", un.display())),
            ("QuietUninstallString", format!("\"{}\" --uninstall --headless{test_flag}", un.display())),
        ] {
            util::reg_set_string(&key, name, &value).map_err(StepErr::other)?;
        }
        for (name, value) in [("EstimatedSize", size_kb), ("NoModify", 1), ("NoRepair", 1)] {
            util::reg_set_dword(&key, name, value).map_err(StepErr::other)?;
        }

        let cfg = c.data.join("config.json");
        if !cfg.exists() {
            fs::create_dir_all(&c.data).map_err(|e| io_err(c, false, &c.data.display().to_string(), &e))?;
            // Language and theme follow Windows by default (the app's "system"); only a choice the user made by hand
            // in the installer is written down.
            let mut v = json!({});
            if c.lang_explicit {
                v["lang"] = json!(c.lang);
            }
            if c.theme != "system" {
                v["theme"] = json!(c.theme);
            }
            if v.as_object().is_some_and(|o| o.is_empty()) {
                return Ok(Outcome::Done);
            }
            fs::write(&cfg, v.to_string()).map_err(|e| io_err(c, false, "config.json", &e))?;
        }
        Ok(Outcome::Done)
    }

    // ------------------------------------------------------------ update / uninstall

    pub(crate) fn step_close(&self, c: &Ctx) -> StepResult {
        let stuck = proc::close_under(&c.root);
        if stuck.is_empty() {
            Ok(Outcome::Done)
        } else {
            Err(StepErr::other(c.tr(
                &format!("Не удалось закрыть SemSearch: {}", stuck.join(", ")),
                &format!("Couldn't close SemSearch: {}", stuck.join(", ")),
            )))
        }
    }

    /// Remove the engine's scheduled task (UAC). Never in test mode.
    pub(crate) fn step_engine_del(&self, c: &Ctx) -> StepResult {
        if c.test {
            log("test mode: engine_del skipped");
            return Ok(Outcome::Skipped);
        }
        let script = c.root.join("engine").join("uninstall-task.ps1");
        if !script.is_file() {
            return Err(StepErr::other(format!("{} is missing", script.display())));
        }
        let mut cmd = Command::new(powershell());
        cmd.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]).arg(&script).arg("-NoPause").current_dir(&c.root);
        c.child_env(&mut cmd);
        self.note("engine_del", "uac");
        let out = proc::run(cmd, &self.job, |_| {}).map_err(|e| StepErr::other(format!("powershell: {e}")))?;
        match out.code {
            0 => Ok(Outcome::Done),
            _ if out.tail.iter().any(|l| l.contains("Elevation was cancelled")) => Err(StepErr::new(
                "uac_denied",
                c.tr("Windows не дала разрешение.", "Windows didn't grant permission."),
            )),
            code => Err(StepErr::other(format!("uninstall-task.ps1: exit code {code}\n{}", out.tail_text(12)))),
        }
    }

    pub(crate) fn step_prog_del(&self, c: &Ctx) -> StepResult {
        let p = c.product;
        let exe = c.root.join("SemSearch.exe");
        if let Some(cmd) = util::run_entry(p.key) {
            if cmd.to_lowercase().contains(&exe.display().to_string().to_lowercase()) {
                util::delete_run_entry(p.key).map_err(StepErr::other)?;
            } else {
                log(&format!("Run\\{} points elsewhere ({cmd}): kept", p.key));
            }
        }
        let lnk_name = format!("{}.lnk", p.display);
        for dir in [sys::start_menu_programs(), sys::desktop()].into_iter().flatten() {
            let lnk = dir.join(&lnk_name);
            if lnk.exists() {
                if let Err(e) = fs::remove_file(&lnk) {
                    log(&format!("could not delete {}: {e}", lnk.display()));
                }
            }
        }
        let key = format!(r"{UNINSTALL_BASE}\{}", p.key);
        let loc = util::reg_get_string(HKEY_CURRENT_USER, &key, "InstallLocation");
        if loc.as_deref().and_then(util::clean_path).is_none_or(|l| same_path(&l, &c.root)) {
            util::reg_delete_tree(&key).map_err(StepErr::other)?;
        } else {
            log(&format!("{key} belongs to {loc:?}: kept"));
        }
        if !c.test && c.installed.as_ref().is_some_and(|i| i.data_dir_env) {
            let cur = util::reg_get_string(HKEY_CURRENT_USER, sys::ENV_KEY, "SEMSEARCH_DATA_DIR");
            if cur.as_deref().and_then(util::clean_path).is_some_and(|d| same_path(&d, &c.data)) {
                util::reg_delete_value(sys::ENV_KEY, "SEMSEARCH_DATA_DIR").map_err(StepErr::other)?;
                sys::broadcast_env_change();
            }
        }
        if is_dangerous(&c.root) {
            return Err(StepErr::other(format!("refusing to delete {}", c.root.display())));
        }
        if let Ok(me) = std::env::current_exe() {
            if is_under(&me, &c.root) {
                return Err(StepErr::other(format!("{} runs from the folder being removed", me.display())));
            }
        }
        util::remove_dir_retry(&c.root, 20).map_err(|e| {
            StepErr::other(c.tr(
                &format!("Не удалось удалить папку «{}»: {e}", c.root.display()),
                &format!("Couldn't delete “{}”: {e}", c.root.display()),
            ))
        })?;
        // The shell's WebView2 profile (%LOCALAPPDATA%\<shell identifier>). Never in test mode: the test copy shares
        // the identifier with the real installation.
        if !c.test {
            let profile = util::local_app_data().join("com.semsearch.desktop");
            if profile.exists() {
                if let Err(e) = util::remove_dir_retry(&profile, 5) {
                    log(&format!("could not delete {}: {e}", profile.display()));
                }
            }
        }
        Ok(Outcome::Done)
    }

    pub(crate) fn step_data_del(&self, c: &Ctx) -> StepResult {
        if is_dangerous(&c.data) {
            return Err(StepErr::other(format!("refusing to delete {}", c.data.display())));
        }
        util::remove_dir_retry(&c.data, 20).map_err(|e| {
            StepErr::other(c.tr(
                &format!("Не удалось удалить папку «{}»: {e}", c.data.display()),
                &format!("Couldn't delete “{}”: {e}", c.data.display()),
            ))
        })?;
        Ok(Outcome::Done)
    }
}

/// A failed Python run outside pip (venv): disk full or other.
fn io_or_other(c: &Ctx, cancelled: bool, what: &str, out: &proc::Output) -> StepErr {
    if cancelled {
        return StepErr::new("cancelled", c.tr("Остановлено.", "Stopped."));
    }
    let text = out.tail.join("\n");
    if text.contains("No space left on device") || text.contains("[Errno 28]") || text.contains("not enough space") {
        return StepErr::new("space", c.tr("Не хватило места на диске.", "The disk is full."));
    }
    StepErr::other(format!("{what}: exit code {}\n{}", out.code, out.tail_text(12)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pep503_links() {
        let page = "https://download.pytorch.org/whl/cpu/torch/";
        let html = r#"<a href="https://download-r2.pytorch.org/whl/cpu/torch-2.11.0%2Bcpu-cp311-cp311-win_amd64.whl#sha256=ab">x</a>
            <a href="../../torch-1.whl#sha256=cd">y</a><a href='/whl/z.whl'>z</a>"#;
        let l = links(page, html);
        assert_eq!(l[0], "https://download-r2.pytorch.org/whl/cpu/torch-2.11.0%2Bcpu-cp311-cp311-win_amd64.whl#sha256=ab");
        assert_eq!(l[1], "https://download.pytorch.org/whl/torch-1.whl#sha256=cd");
        assert_eq!(l[2], "https://download.pytorch.org/whl/z.whl");
    }

    #[test]
    fn torch_req() {
        let r = parse_torch_req("# x\ntorch==2.11.0\ntorchvision==0.26.0  # y\n").unwrap();
        assert_eq!((r.torch.as_str(), r.vision.as_str()), ("2.11.0", "0.26.0"));
    }
}

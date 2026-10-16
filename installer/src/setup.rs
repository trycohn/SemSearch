//! The setup run (docs/INSTALLER.md): mode, options, plan, step runner, events. The steps themselves are in
//! steps.rs. The same runner serves the window (Tauri events) and --headless (JSON lines on stdout).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;

use crate::payload::Payload;
use crate::proc::JobSlot;
use crate::sys::{self, Gpu};
use crate::util::{self, is_under, log, reg_get_string, same_path, tr};

pub const VERSION: &str = env!("SEMSEARCH_VERSION");
pub const UNINSTALL_BASE: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall";
const GB: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Install,
    Update,
    Uninstall,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Install => "install",
            Mode::Update => "update",
            Mode::Uninstall => "uninstall",
        }
    }
}

/// Names under which the program is registered: the Uninstall key and the Run value (`key`), the DisplayName
/// and the shortcut names (`display`). Test mode uses its own, so a real installation is never touched.
#[derive(Clone, Copy)]
pub struct Product {
    pub key: &'static str,
    pub display: &'static str,
}

pub fn product(test: bool) -> Product {
    if test {
        Product { key: "SemSearch-Test", display: "SemSearch (test)" }
    } else {
        Product { key: "SemSearch", display: "SemSearch" }
    }
}

/// `<root>\install.json`: what the installer did.
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct InstallJson {
    pub version: String,
    pub data_dir: String,
    pub data_dir_env: bool,
    pub gpu: bool,
    pub mft: bool,
    pub autostart: bool,
    pub desktop: bool,
    /// Installed by a test-mode setup: its uninstaller and updates run in test mode too.
    #[serde(default)]
    pub test: bool,
    #[serde(default)]
    pub lang: String,
}

#[derive(Clone, Debug)]
pub struct Installed {
    pub root: PathBuf,
    pub info: InstallJson,
}

pub fn read_install_json(root: &Path) -> Option<InstallJson> {
    let text = std::fs::read_to_string(root.join("install.json")).ok()?;
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

/// The installation registered under `product` (Uninstall key + `<InstallLocation>\install.json`).
pub fn find_installed(p: Product) -> Option<Installed> {
    let key = format!(r"{UNINSTALL_BASE}\{}", p.key);
    let loc = reg_get_string(HKEY_CURRENT_USER, &key, "InstallLocation")?;
    let root = util::clean_path(&loc)?;
    let info = read_install_json(&root)?;
    Some(Installed { root, info })
}

/// Arguments of `start()`; every field is optional (update and uninstall take theirs from install.json).
#[derive(Deserialize, Default, Debug, Clone)]
pub struct StartArgs {
    pub root: Option<String>,
    pub data_dir: Option<String>,
    pub gpu: Option<bool>,
    pub autostart: Option<bool>,
    pub mft: Option<bool>,
    pub desktop: Option<bool>,
    pub lang: Option<String>,
    /// "light" | "dark"; anything else = same as Windows.
    pub theme: Option<String>,
    pub wipe: Option<bool>,
}

/// torch / torchvision versions from requirements-torch.txt.
#[derive(Clone, Debug)]
pub struct TorchReq {
    pub torch: String,
    pub vision: String,
}

pub fn parse_torch_req(text: &str) -> Option<TorchReq> {
    let mut torch = None;
    let mut vision = None;
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if let Some((name, ver)) = line.split_once("==") {
            let ver = ver.trim().to_string();
            match name.trim().to_ascii_lowercase().as_str() {
                "torch" => torch = Some(ver),
                "torchvision" => vision = Some(ver),
                _ => {}
            }
        }
    }
    Some(TorchReq { torch: torch?, vision: vision? })
}

/// Everything a run needs, fixed at start().
pub struct Ctx {
    pub mode: Mode,
    pub test: bool,
    pub product: Product,
    pub root: PathBuf,
    pub data: PathBuf,
    /// `data` is the default data folder (no SEMSEARCH_DATA_DIR needed).
    pub data_default: bool,
    pub gpu: bool,
    pub autostart: bool,
    pub mft: bool,
    pub desktop: bool,
    pub lang: String,
    /// `lang` was chosen by hand (not "same as Windows"): it goes into a new config.json.
    pub lang_explicit: bool,
    /// "system" | "light" | "dark" — written to a new config.json like `lang`.
    pub theme: String,
    pub wipe: bool,
    pub torch: Option<TorchReq>,
    /// install.json of the installation being updated or removed.
    pub installed: Option<InstallJson>,
}

impl Ctx {
    pub fn cache(&self) -> PathBuf {
        self.root.join(".setup-cache")
    }
    pub fn venv_python(&self) -> PathBuf {
        self.root.join(".venv").join("Scripts").join("python.exe")
    }
    pub fn variant(&self) -> &'static str {
        if self.gpu { "cu128" } else { "cpu" }
    }
    pub fn torch_index(&self) -> String {
        format!("https://download.pytorch.org/whl/{}", self.variant())
    }
    pub fn torch_wheel(&self) -> Option<String> {
        let t = self.torch.as_ref()?;
        Some(format!("torch-{}+{}-cp311-cp311-win_amd64.whl", t.torch, self.variant()))
    }
    pub fn tr(&self, ru: &str, en: &str) -> String {
        tr(&self.lang, ru, en)
    }

    /// Environment of every child process: the data folder, UTF-8 Python, no stray Python settings; in test
    /// mode a private, empty Hugging Face home so that core.migrate_model() cannot move the real model.
    pub fn child_env(&self, cmd: &mut Command) {
        cmd.env("SEMSEARCH_DATA_DIR", &self.data)
            .env("PYTHONUTF8", "1")
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONNOUSERSITE", "1")
            .env_remove("PYTHONHOME")
            .env_remove("PYTHONPATH")
            .env_remove("SEMSEARCH_PAYLOAD_DIR");
        if self.test {
            cmd.env("HF_HOME", self.cache().join("hf-home"));
            for k in ["HF_HUB_CACHE", "HUGGINGFACE_HUB_CACHE", "HF_XET_CACHE", "TRANSFORMERS_CACHE", "SENTENCE_TRANSFORMERS_HOME"] {
                cmd.env_remove(k);
            }
        }
    }
}

/// A failed step: `kind` is one of network | space | uac_denied | not_admin | cancelled | other.
#[derive(Debug)]
pub struct StepErr {
    pub kind: &'static str,
    pub message: String,
    /// Bytes downloaded when a download step stopped (shown as "the connection dropped at ...").
    pub got: Option<(u64, Option<u64>)>,
}

impl StepErr {
    pub fn new(kind: &'static str, message: impl Into<String>) -> StepErr {
        StepErr { kind, message: message.into(), got: None }
    }
    pub fn other(message: impl Into<String>) -> StepErr {
        StepErr::new("other", message)
    }
    pub fn at(mut self, got: u64, total: Option<u64>) -> StepErr {
        self.got = Some((got, total));
        self
    }
    /// Payload of the `failed` step event and of setup://error.
    fn payload(&self, id: &str) -> Value {
        let mut v = json!({ "id": id, "kind": self.kind, "message": self.message });
        if let Some((got, total)) = self.got {
            v["got"] = json!(got);
            v["total"] = json!(total);
        }
        v
    }
}

pub enum Outcome {
    Done,
    Skipped,
}

pub type StepResult = Result<Outcome, StepErr>;

/// Steps whose failure does not stop the run.
fn non_fatal(id: &str) -> bool {
    matches!(id, "mft" | "engine_del")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    Running,
    Failed(usize),
    Done,
}

struct Run {
    ctx: Option<Arc<Ctx>>,
    plan: Vec<&'static str>,
    phase: Phase,
    mft_ok: bool,
}

/// Where events go.
pub enum Sink {
    /// --headless: one JSON object per line on stdout, `{"event": "setup://step", ...payload}`.
    Stdout,
    Tauri(OnceLock<tauri::AppHandle>),
}

/// Rate limit for progress events.
pub struct Throttle {
    last: Option<Instant>,
    every: Duration,
}

impl Throttle {
    pub fn new(per_second: u32) -> Throttle {
        Throttle { last: None, every: Duration::from_millis(1000 / per_second as u64) }
    }
    pub fn ready(&mut self) -> bool {
        let now = Instant::now();
        if self.last.is_none_or(|l| now.duration_since(l) >= self.every) {
            self.last = Some(now);
            true
        } else {
            false
        }
    }
}

pub struct Setup {
    pub mode: Mode,
    pub test: bool,
    pub sys_lang: &'static str,
    pub installed: Option<Installed>,
    /// --uninstall: the folder to remove.
    pub uninstall_root: Option<PathBuf>,
    pub payload: Payload,
    sink: Sink,
    gpu: OnceLock<Option<Gpu>>,
    run: Mutex<Run>,
    pub cancel: AtomicBool,
    pub job: JobSlot,
    /// The installer window (owner of UAC prompts), 0 without one.
    pub hwnd: AtomicIsize,
    finished: Mutex<Option<mpsc::Sender<bool>>>,
}

impl Setup {
    pub fn new(mode: Mode, test: bool, installed: Option<Installed>, uninstall_root: Option<PathBuf>, sink: Sink) -> Setup {
        Setup {
            mode,
            test,
            sys_lang: util::system_lang(),
            installed,
            uninstall_root,
            payload: Payload::detect(),
            sink,
            gpu: OnceLock::new(),
            run: Mutex::new(Run { ctx: None, plan: Vec::new(), phase: Phase::Idle, mft_ok: true }),
            cancel: AtomicBool::new(false),
            job: Mutex::new(None),
            hwnd: AtomicIsize::new(0),
            finished: Mutex::new(None),
        }
    }

    pub fn product(&self) -> Product {
        product(self.test)
    }

    pub fn attach(&self, app: tauri::AppHandle) {
        if let Sink::Tauri(h) = &self.sink {
            let _ = h.set(app);
        }
    }

    // ------------------------------------------------------------ events

    pub fn emit(&self, event: &str, payload: Value) {
        match &self.sink {
            Sink::Stdout => {
                let mut obj = serde_json::Map::new();
                obj.insert("event".into(), Value::from(event));
                if let Value::Object(m) = payload {
                    obj.extend(m);
                }
                let mut out = std::io::stdout().lock();
                let _ = writeln!(out, "{}", Value::Object(obj));
                let _ = out.flush();
            }
            Sink::Tauri(h) => {
                if let Some(app) = h.get() {
                    use tauri::Emitter;
                    let _ = app.emit(event, payload);
                }
            }
        }
    }

    fn emit_step(&self, id: &str, state: &str, extra: Value) {
        let mut p = json!({ "id": id, "state": state });
        if let (Value::Object(m), Value::Object(e)) = (&mut p, extra) {
            m.extend(e);
        }
        if state != "active" || p.get("got").is_none() {
            log(&format!("step {p}"));
        }
        self.emit("setup://step", p);
    }

    /// Progress of the running step (bytes).
    pub fn progress(&self, id: &str, got: u64, total: Option<u64>) {
        self.emit_step(id, "active", json!({ "got": got, "total": total }));
    }

    /// A note on the running step: "uac" (waiting for the Windows permission prompt), "verify" (checking a
    /// download), "python" (unpacking Python), "venv" (creating the environment).
    pub fn note(&self, id: &str, note: &str) {
        self.emit_step(id, "active", json!({ "note": note }));
    }

    // ------------------------------------------------------------ state for the UI

    pub fn gpu(&self) -> Option<Gpu> {
        self.gpu.get_or_init(sys::detect_gpu).clone()
    }

    pub fn defaults(&self) -> (PathBuf, PathBuf) {
        default_dirs(self.test)
    }

    pub fn state(&self) -> Value {
        let (root, data) = self.defaults();
        let build = sys::windows_build();
        let installed = self.installed.as_ref().map(|i| {
            json!({
                "version": i.info.version, "root": i.root, "data_dir": i.info.data_dir, "gpu": i.info.gpu,
                "mft": i.info.mft, "autostart": i.info.autostart, "desktop": i.info.desktop,
            })
        });
        json!({
            "mode": self.mode.as_str(),
            "version": VERSION,
            "lang": self.sys_lang,
            "test": self.test,
            "installed": installed,
            "defaults": { "root": root, "data_dir": data },
            "gpu": self.gpu(),
            "windows": { "build": build, "ok": sys::windows_ok(build) },
        })
    }

    // ------------------------------------------------------------ commands

    /// Begin the steps of the current mode.
    /// Allowed before the first run and after a failed or cancelled one (the UI goes back to its first screen,
    /// e.g. to pick a folder with more space); a run still stopping after cancel() is waited for.
    pub fn start(self: &Arc<Self>, a: StartArgs) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut run = self.run.lock().unwrap();
        while run.phase == Phase::Running && self.cancel.load(Ordering::SeqCst) && Instant::now() < deadline {
            drop(run);
            std::thread::sleep(Duration::from_millis(100));
            run = self.run.lock().unwrap();
        }
        if !matches!(run.phase, Phase::Idle | Phase::Failed(_)) {
            return Err("already started".into());
        }
        let ctx = self.make_ctx(a)?;
        log(&format!(
            "start {}: root={} data={} gpu={} autostart={} mft={} desktop={} lang={} wipe={} test={} payload={}",
            ctx.mode.as_str(),
            ctx.root.display(),
            ctx.data.display(),
            ctx.gpu,
            ctx.autostart,
            ctx.mft,
            ctx.desktop,
            ctx.lang,
            ctx.wipe,
            ctx.test,
            self.payload.describe()
        ));
        let plan = self.plan(&ctx);
        run.plan = plan.clone();
        run.ctx = Some(Arc::new(ctx));
        run.phase = Phase::Running;
        run.mft_ok = true;
        drop(run);
        self.cancel.store(false, Ordering::SeqCst);
        self.emit("setup://plan", json!({ "mode": self.mode.as_str(), "steps": plan }));
        let me = self.clone();
        std::thread::spawn(move || me.run_from(0));
        Ok(())
    }

    /// Run the failed step again and go on.
    pub fn retry(self: &Arc<Self>) -> Result<(), String> {
        let mut run = self.run.lock().unwrap();
        let Phase::Failed(i) = run.phase else {
            return Err("nothing to retry".into());
        };
        run.phase = Phase::Running;
        drop(run);
        self.cancel.store(false, Ordering::SeqCst);
        log("retry");
        let me = self.clone();
        std::thread::spawn(move || me.run_from(i));
        Ok(())
    }

    /// Stop the running step: kill pip / the model script, stop a download (the partial file stays).
    pub fn cancel(&self) {
        log("cancel");
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(job) = self.job.lock().unwrap().as_ref() {
            job.kill();
        }
    }

    /// `mft` once more after the run (the user agreed to the UAC prompt this time).
    pub fn retry_mft(self: &Arc<Self>) -> Result<(), String> {
        let run = self.run.lock().unwrap();
        if run.phase != Phase::Done || self.mode == Mode::Uninstall {
            return Err("retry_mft is for a finished install or update".into());
        }
        let ctx = run.ctx.clone().ok_or("no run")?;
        drop(run);
        self.cancel.store(false, Ordering::SeqCst);
        let me = self.clone();
        std::thread::spawn(move || {
            me.emit_step("mft", "active", json!({}));
            match me.step_mft(&ctx) {
                Ok(Outcome::Done) => {
                    me.run.lock().unwrap().mft_ok = true;
                    if let Some(mut info) = read_install_json(&ctx.root) {
                        info.mft = true;
                        let _ = write_install_json(&ctx.root, &info);
                    }
                    me.emit_step("mft", "done", json!({}));
                }
                Ok(Outcome::Skipped) => me.emit_step("mft", "skipped", json!({})),
                Err(e) => me.emit_step("mft", "failed", e.payload("mft")),
            }
        });
        Ok(())
    }

    /// Start the installed SemSearch (separately, with SEMSEARCH_DATA_DIR). Not in test mode: the shell's
    /// autostart repair would point the real Run\SemSearch entry at the test copy.
    pub fn launch(&self) {
        let ctx = self.run.lock().unwrap().ctx.clone();
        let (root, data) = match (&ctx, &self.installed) {
            (Some(c), _) => (c.root.clone(), c.data.clone()),
            (None, Some(i)) => (i.root.clone(), PathBuf::from(&i.info.data_dir)),
            _ => return,
        };
        if self.mode == Mode::Uninstall {
            return;
        }
        if self.test {
            log("test mode: SemSearch is not started");
            return;
        }
        let exe = root.join("SemSearch.exe");
        let mut cmd = Command::new(&exe);
        cmd.current_dir(&root)
            .env("SEMSEARCH_DATA_DIR", &data)
            .env_remove("SEMSEARCH_PAYLOAD_DIR")
            .env_remove("SEMSEARCH_SETUP_TEST");
        log(&format!("launch {}: {:?}", exe.display(), cmd.spawn().map(|c| c.id())));
    }

    /// --headless: start with the command-line options and wait. Exit code 0 when everything went through.
    pub fn run_headless(self: &Arc<Self>, a: StartArgs) -> i32 {
        let (tx, rx) = mpsc::channel();
        *self.finished.lock().unwrap() = Some(tx);
        if let Err(e) = self.start(a) {
            log(&format!("start refused: {e}"));
            self.emit("setup://error", json!({ "id": "start", "kind": "other", "message": e }));
            return 1;
        }
        match rx.recv() {
            Ok(true) => 0,
            _ => 1,
        }
    }

    // ------------------------------------------------------------ the run

    fn make_ctx(&self, a: StartArgs) -> Result<Ctx, String> {
        // "ru" / "en" — chosen by hand (written to the app's config.json); "system" or nothing — same as Windows.
        let lang_explicit = matches!(a.lang.as_deref(), Some("ru" | "en"));
        let lang = match a.lang.as_deref() {
            Some("ru") => "ru".to_string(),
            Some("en") => "en".to_string(),
            _ => self.installed.as_ref().map(|i| i.info.lang.clone()).filter(|l| l == "ru" || l == "en").unwrap_or_else(|| self.sys_lang.to_string()),
        };
        let theme = match a.theme.as_deref() {
            Some(t @ ("light" | "dark")) => t.to_string(),
            _ => "system".to_string(),
        };
        let (def_root, def_data) = self.defaults();
        let path = |v: &Option<String>, def: &Path| match v.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => util::clean_path(s).ok_or_else(|| format!("bad path: {s}")),
            None => Ok(def.to_path_buf()),
        };
        let product = self.product();
        let ctx = match self.mode {
            Mode::Install => {
                let gpu = a.gpu.unwrap_or_else(|| self.gpu().is_some_and(|g| g.supported));
                Ctx {
                    mode: self.mode,
                    test: self.test,
                    product,
                    root: path(&a.root, &def_root)?,
                    data: path(&a.data_dir, &def_data)?,
                    data_default: false,
                    gpu,
                    autostart: a.autostart.unwrap_or(true),
                    mft: a.mft.unwrap_or(true),
                    desktop: a.desktop.unwrap_or(false),
                    lang: lang.clone(),
                    lang_explicit,
                    theme: theme.clone(),
                    wipe: false,
                    torch: None,
                    installed: None,
                }
            }
            Mode::Update => {
                let inst = self.installed.clone().ok_or("no installation to update")?;
                let i = &inst.info;
                let data = util::clean_path(&i.data_dir).unwrap_or(def_data.clone());
                Ctx {
                    mode: self.mode,
                    test: self.test,
                    product,
                    root: inst.root.clone(),
                    data,
                    data_default: false,
                    gpu: i.gpu,
                    autostart: i.autostart,
                    mft: if self.test { i.mft } else { sys::engine_task_exists() },
                    desktop: i.desktop,
                    lang: lang.clone(),
                    lang_explicit,
                    theme: theme.clone(),
                    wipe: false,
                    torch: None,
                    installed: Some(inst.info.clone()),
                }
            }
            Mode::Uninstall => {
                let root = self.uninstall_root.clone().ok_or("--uninstall needs --root")?;
                let info = self.installed.as_ref().map(|i| i.info.clone());
                let data = info.as_ref().and_then(|i| util::clean_path(&i.data_dir)).unwrap_or(def_data.clone());
                Ctx {
                    mode: self.mode,
                    test: self.test,
                    product,
                    root,
                    data,
                    data_default: false,
                    gpu: false,
                    autostart: false,
                    mft: if self.test { info.as_ref().is_some_and(|i| i.mft) } else { sys::engine_task_exists() },
                    desktop: false,
                    lang: lang.clone(),
                    lang_explicit,
                    theme: theme.clone(),
                    wipe: a.wipe.unwrap_or(false),
                    torch: None,
                    installed: info,
                }
            }
        };
        let mut ctx = ctx;
        ctx.data_default = same_path(&ctx.data, &default_dirs(false).1);
        check_paths(&ctx)?;
        if ctx.mode != Mode::Uninstall {
            if !self.payload.available() {
                return Err(ctx.tr(
                    "В установщике нет файлов программы (сборка без полезной нагрузки).",
                    "The installer carries no program files (a build without payload).",
                ));
            }
            let text = self
                .payload
                .read_text("requirements-torch.txt")
                .map_err(|e| format!("requirements-torch.txt: {e}"))?;
            ctx.torch = Some(parse_torch_req(&text).ok_or("requirements-torch.txt: needs torch==… and torchvision==…")?);
        }
        Ok(ctx)
    }

    fn plan(&self, c: &Ctx) -> Vec<&'static str> {
        let mut p = Vec::new();
        match c.mode {
            Mode::Install => {
                p.push("check");
                if !sys::vc_redist_installed() {
                    p.push("vcredist");
                }
                p.extend(["copy", "torch", "libs", "model"]);
                if c.mft {
                    p.push("mft");
                }
                p.push("links");
            }
            Mode::Update => {
                p.extend(["close", "replace"]);
                if crate::steps::torch_differs(c) {
                    p.push("torch");
                }
                p.push("libs");
                if c.mft {
                    p.push("mft");
                }
                p.push("links");
            }
            Mode::Uninstall => {
                p.push("close");
                if c.mft {
                    p.push("engine_del");
                }
                p.push("prog_del");
                if c.wipe {
                    p.push("data_del");
                }
            }
        }
        p
    }

    fn run_from(self: Arc<Self>, start: usize) {
        let (ctx, plan) = {
            let run = self.run.lock().unwrap();
            (run.ctx.clone().expect("ctx"), run.plan.clone())
        };
        for (i, id) in plan.iter().enumerate().skip(start) {
            self.emit_step(id, "active", json!({}));
            let r = if self.cancel.load(Ordering::SeqCst) {
                Err(StepErr::new("cancelled", ctx.tr("Остановлено.", "Stopped.")))
            } else {
                self.run_step(id, &ctx)
            };
            match r {
                Ok(Outcome::Done) => self.emit_step(id, "done", json!({})),
                Ok(Outcome::Skipped) => self.emit_step(id, "skipped", json!({})),
                Err(e) => {
                    log(&format!("step {id} failed ({}): {}", e.kind, e.message));
                    self.emit_step(id, "failed", e.payload(id));
                    if non_fatal(id) {
                        self.run.lock().unwrap().mft_ok = false;
                        continue;
                    }
                    self.run.lock().unwrap().phase = Phase::Failed(i);
                    self.emit("setup://error", e.payload(id));
                    if let Some(tx) = self.finished.lock().unwrap().as_ref() {
                        let _ = tx.send(false);
                    }
                    return;
                }
            }
        }
        if ctx.mode != Mode::Uninstall {
            let cache = ctx.cache();
            if let Err(e) = util::remove_dir_retry(&cache, 4) {
                log(&format!("could not remove {}: {e}", cache.display()));
            }
        }
        let mft_ok = {
            let mut run = self.run.lock().unwrap();
            run.phase = Phase::Done;
            run.mft_ok
        };
        let payload = json!({
            "mode": ctx.mode.as_str(), "ok": true, "mft_ok": mft_ok, "root": ctx.root, "data_dir": ctx.data,
            "hotkey": util::hotkey_label(&ctx.data),
        });
        log(&format!("done {payload}"));
        self.emit("setup://done", payload);
        if let Some(tx) = self.finished.lock().unwrap().as_ref() {
            let _ = tx.send(true);
        }
    }

    fn run_step(&self, id: &str, c: &Ctx) -> StepResult {
        match id {
            "check" => self.step_check(c),
            "vcredist" => self.step_vcredist(c),
            "copy" | "replace" => self.step_copy(c, id),
            "torch" => self.step_torch(c),
            "libs" => self.step_libs(c),
            "model" => self.step_model(c),
            "mft" => self.step_mft(c),
            "links" => self.step_links(c),
            "close" => self.step_close(c),
            "engine_del" => self.step_engine_del(c),
            "prog_del" => self.step_prog_del(c),
            "data_del" => self.step_data_del(c),
            _ => Err(StepErr::other(format!("unknown step {id}"))),
        }
    }

    pub fn mft_ok(&self) -> bool {
        self.run.lock().unwrap().mft_ok
    }
}

/// Default program and data folders: `%LOCALAPPDATA%\Programs\SemSearch` and `%LOCALAPPDATA%\SemSearch`;
/// in test mode `%TEMP%\SemSearch-Test\app` and `…\data`.
pub fn default_dirs(test: bool) -> (PathBuf, PathBuf) {
    if test {
        let t = util::temp_dir().join("SemSearch-Test");
        (t.join("app"), t.join("data"))
    } else {
        let l = util::local_app_data();
        (l.join("Programs").join("SemSearch"), l.join("SemSearch"))
    }
}

/// Folders that must never be the program or data folder (and so never be deleted whole): drive roots and the
/// user's and the system's known folders.
pub fn is_dangerous(p: &Path) -> bool {
    use windows_sys::Win32::UI::Shell::*;
    if p.parent().is_none() || p.components().count() < 3 {
        return true;
    }
    let ids = [
        FOLDERID_Profile,
        FOLDERID_Documents,
        FOLDERID_Desktop,
        FOLDERID_Downloads,
        FOLDERID_Pictures,
        FOLDERID_Videos,
        FOLDERID_Music,
        FOLDERID_RoamingAppData,
        FOLDERID_LocalAppData,
        FOLDERID_UserProgramFiles,
        FOLDERID_ProgramFiles,
        FOLDERID_ProgramFilesX86,
        FOLDERID_Windows,
        FOLDERID_Programs,
        FOLDERID_StartMenu,
    ];
    ids.iter().filter_map(util::known_folder).chain([util::temp_dir()]).any(|k| same_path(p, &k))
}

/// The real installation's folders, which test mode must never touch.
fn real_dirs() -> Vec<PathBuf> {
    let (root, data) = default_dirs(false);
    let mut v = vec![root, data];
    if let Some(i) = reg_get_string(HKEY_CURRENT_USER, &format!(r"{UNINSTALL_BASE}\SemSearch"), "InstallLocation") {
        if let Some(p) = util::clean_path(&i) {
            v.push(p);
        }
    }
    if let Some(home) = std::env::var_os("USERPROFILE") {
        v.push(PathBuf::from(home).join(".cache").join("huggingface"));
    }
    v
}

fn check_paths(c: &Ctx) -> Result<(), String> {
    let (root, data) = (&c.root, &c.data);
    for p in [root, data] {
        if is_dangerous(p) {
            return Err(c.tr(
                &format!("Папка «{}» не подходит: выберите отдельную папку.", p.display()),
                &format!("“{}” can't be used: choose a separate folder.", p.display()),
            ));
        }
    }
    if is_under(root, data) || is_under(data, root) {
        return Err(c.tr(
            "Папка программы и папка данных не должны быть одна в другой.",
            "The program folder and the data folder must not be inside each other.",
        ));
    }
    if c.test {
        for real in real_dirs() {
            for p in [root, data] {
                if is_under(p, &real) || is_under(&real, p) {
                    return Err(format!("test mode: {} overlaps the real installation's {}", p.display(), real.display()));
                }
            }
        }
    }
    Ok(())
}

/// Free space for `disk_info()` and the `check` step: 7 GB with the GPU build, 3.5 GB without, on the drive of
/// the program folder; 2 GB on the data folder's drive when it is another one.
pub fn disk_info(root: &Path, data: &Path, gpu: bool) -> Value {
    let need = if gpu { 7 * GB } else { 7 * GB / 2 };
    let drive = sys::drive_of(root);
    let free = sys::free_bytes(root).unwrap_or(0);
    let mut ok = free >= need;
    let mut v = json!({ "drive": drive, "free_bytes": free, "need_bytes": need });
    let data_drive = sys::drive_of(data);
    if !data_drive.eq_ignore_ascii_case(&drive) {
        let dfree = sys::free_bytes(data).unwrap_or(0);
        ok &= dfree >= 2 * GB;
        v["data_drive"] = json!(data_drive);
        v["data_free_bytes"] = json!(dfree);
    }
    v["ok"] = json!(ok);
    v
}

pub fn write_install_json(root: &Path, info: &InstallJson) -> std::io::Result<()> {
    let text = serde_json::to_string_pretty(info).map_err(std::io::Error::other)?;
    std::fs::write(root.join("install.json"), text)
}

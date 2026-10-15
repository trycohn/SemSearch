//! The installer window: Tauri commands of docs/INSTALLER.md over the shared Setup.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::DialogExt;

use crate::setup::{disk_info as disk, Mode, Setup, StartArgs};
use crate::util::{self, log};

fn setup(app: &AppHandle) -> Arc<Setup> {
    app.state::<Arc<Setup>>().inner().clone()
}

/// Window title (taskbar, Alt+Tab); the page draws its own title bar.
pub fn window_title(mode: Mode, lang: &str) -> String {
    let (ru, en) = match mode {
        Mode::Install => ("Установка SemSearch", "SemSearch Setup"),
        Mode::Update => ("Обновление SemSearch", "SemSearch Update"),
        Mode::Uninstall => ("Удаление SemSearch", "Uninstall SemSearch"),
    };
    util::tr(lang, ru, en)
}

#[tauri::command(rename_all = "snake_case")]
async fn get_state(app: AppHandle) -> Value {
    setup(&app).state()
}

#[tauri::command(rename_all = "snake_case")]
async fn disk_info(root: String, data_dir: String, gpu: bool) -> Value {
    let def = crate::setup::default_dirs(false);
    let root = util::clean_path(&root).unwrap_or(def.0);
    let data = util::clean_path(&data_dir).unwrap_or(def.1);
    disk(&root, &data, gpu)
}

#[tauri::command(rename_all = "snake_case")]
async fn pick_folder(window: tauri::WebviewWindow, initial: Option<String>) -> Option<String> {
    let w = window.clone();
    let lang = setup(window.app_handle()).sys_lang;
    let title = util::tr(lang, "Выберите папку", "Choose a folder");
    tauri::async_runtime::spawn_blocking(move || {
        let mut d = w.dialog().file().set_parent(&w).set_title(title);
        if let Some(dir) = initial.as_deref().and_then(util::clean_path) {
            // the dialog opens at the nearest existing folder
            let mut p: Option<PathBuf> = Some(dir);
            while let Some(x) = p.clone() {
                if x.is_dir() {
                    d = d.set_directory(&x);
                    break;
                }
                p = x.parent().map(PathBuf::from);
            }
        }
        d.blocking_pick_folder()
    })
    .await
    .ok()
    .flatten()
    .and_then(|p| p.into_path().ok())
    .map(|p| p.display().to_string())
}

#[tauri::command(rename_all = "snake_case")]
#[allow(clippy::too_many_arguments)]
async fn start(
    app: AppHandle,
    root: Option<String>,
    data_dir: Option<String>,
    gpu: Option<bool>,
    autostart: Option<bool>,
    mft: Option<bool>,
    desktop: Option<bool>,
    lang: Option<String>,
    theme: Option<String>,
    wipe: Option<bool>,
) -> Result<(), String> {
    let a = StartArgs { root, data_dir, gpu, autostart, mft, desktop, lang, theme, wipe };
    log(&format!("start({a:?})"));
    setup(&app).start(a)
}

#[tauri::command(rename_all = "snake_case")]
async fn retry(app: AppHandle) -> Result<(), String> {
    setup(&app).retry()
}

#[tauri::command(rename_all = "snake_case")]
async fn cancel(app: AppHandle) {
    setup(&app).cancel()
}

#[tauri::command(rename_all = "snake_case")]
async fn retry_mft(app: AppHandle) -> Result<(), String> {
    setup(&app).retry_mft()
}

#[tauri::command(rename_all = "snake_case")]
async fn finish(app: AppHandle, launch: Option<bool>) {
    log(&format!("finish(launch={launch:?})"));
    if launch.unwrap_or(false) {
        setup(&app).launch();
    }
    app.exit(0);
}

#[tauri::command(rename_all = "snake_case")]
async fn open_log() -> bool {
    util::shell_open(&util::log_path())
}

#[tauri::command(rename_all = "snake_case")]
async fn open_path(path: String) -> bool {
    match util::clean_path(&path).filter(|p| p.exists()) {
        Some(p) => util::shell_open(&p),
        None => false,
    }
}

/// Run the window until it closes; returns the exit code.
pub fn run(s: Arc<Setup>) -> i32 {
    let title = window_title(s.mode, s.sys_lang);
    // warm up the GPU check (nvidia-smi) while the page loads
    let warm = s.clone();
    std::thread::spawn(move || {
        warm.gpu();
    });
    let s2 = s.clone();
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(s.clone())
        .invoke_handler(tauri::generate_handler![
            get_state, disk_info, pick_folder, start, retry, cancel, retry_mft, finish, open_log, open_path
        ])
        .setup(move |app| {
            s2.attach(app.handle().clone());
            let w = WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                .title(&title)
                .inner_size(720.0, 700.0)
                .resizable(false)
                .maximizable(false)
                .decorations(false)
                .shadow(true)
                .center()
                // WebView2 profile in %TEMP%, not in %LOCALAPPDATA%\<identifier>: nothing is left behind
                .data_directory(util::temp_dir().join("SemSearch-setup-webview"))
                .build()?;
            if let Ok(h) = w.hwnd() {
                s2.hwnd.store(h.0 as isize, Ordering::SeqCst);
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the installer window");
    app.run_return(|_, _| {})
}

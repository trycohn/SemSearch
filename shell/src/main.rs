//! SemSearch desktop shell (Tauri v2).
//!
//! Hosts the backend's web UI (http://127.0.0.1:<port>/, /settings, /tray) in frameless windows,
//! provides the tray icon + popup, the global hotkey, autostart, single instance and manages the
//! Python backend process. Contract: docs/API.md, section 3.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![allow(non_snake_case)] // binary/crate name is "SemSearch"

mod backend;
mod util;

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::window::Color;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, RunEvent, Url, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt as _, Shortcut, ShortcutState};

use backend::{Backend, ChildState, Probe};
use util::{log, Config};

/// Tray popup window size (logical px). The page draws the 292 px panel anchored bottom-right.
const TRAY_W: f64 = 300.0;
const TRAY_H: f64 = 340.0;
/// Gap between the popup window and the taskbar / screen edge (logical px).
const TRAY_GAP: f64 = 8.0;
/// Origin of the bundled pages (dist/) on Windows.
const LOCAL_ORIGIN: &str = "http://tauri.localhost/";

struct Shell {
    cfg: Config,
    backend: Backend,
    hotkey: Mutex<Hotkey>,
    /// Windows show the backend pages (backend answered).
    ready: AtomicBool,
    quitting: AtomicBool,
    /// Extra status line for the bundled loading page ("" = default text).
    status: Mutex<String>,
    /// When the tray popup was last hidden because it lost focus (to swallow the tray click that caused it).
    tray_blur: Mutex<Option<Instant>>,
    /// Ignore tray single clicks until then (the click that follows a double click).
    ignore_clicks_until: Mutex<Option<Instant>>,
    /// Last navigation per window label (to avoid repeating it while the page commits).
    navs: Mutex<HashMap<String, (Url, Instant)>>,
    /// UI language in effect: "ru" | "en" (config.json "lang" at start, then set_lang from the pages).
    lang: Mutex<&'static str>,
}

#[derive(Default)]
struct Hotkey {
    accel: String,
    shortcut: Option<Shortcut>,
}

fn shell(app: &AppHandle) -> tauri::State<'_, Shell> {
    app.state::<Shell>()
}

/// The shell's own texts (window titles, errors, loading-page status) in the UI language.
fn tr(app: &AppHandle, ru: &str, en: &str) -> String {
    (if *shell(app).lang.lock().unwrap() == "en" { en } else { ru }).to_string()
}

/// Runs before every page of a window: language and theme for the bundled loading page (dist/), so it is drawn
/// in the right language and colours before the backend answers; ui/i18n.js also starts from it.
fn boot_script(app: &AppHandle) -> String {
    let s = shell(app);
    let lang = *s.lang.lock().unwrap();
    let (theme, accent) = s.cfg.appearance();
    let info = serde_json::json!({
        "lang": lang, "sys": util::system_lang(), "theme": theme, "accent": accent,
    });
    format!("window.__SEMSEARCH_BOOT = {info};")
}

/// Window titles that depend on the language (taskbar, Alt+Tab).
fn retitle(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.set_title(&tr(app, "Настройки — SemSearch", "Settings — SemSearch"));
    }
    if let Some(w) = app.get_webview_window("tray") {
        let _ = w.set_title(&tr(app, "SemSearch — меню", "SemSearch — menu"));
    }
}

// ---------------------------------------------------------------- windows & pages

fn route(label: &str) -> &'static str {
    match label {
        "settings" => "/settings",
        "tray" => "/tray",
        _ => "/",
    }
}

fn local_page(label: &str) -> &'static str {
    if label == "tray" {
        "tray.html"
    } else {
        "index.html"
    }
}

fn is_local(url: &Url) -> bool {
    url.host_str() == Some("tauri.localhost")
}

fn initial_url(app: &AppHandle, label: &str) -> WebviewUrl {
    let s = shell(app);
    if s.ready.load(Ordering::SeqCst) {
        WebviewUrl::External(s.cfg.page_url(route(label)))
    } else {
        WebviewUrl::App(local_page(label).into())
    }
}

/// Window background until the page paints: the chosen theme (config.json), else Windows' app mode.
fn page_bg(app: &AppHandle) -> Color {
    let dark = match shell(app).cfg.appearance().0.as_str() {
        "dark" => true,
        "light" => false,
        _ => util::system_dark(),
    };
    if dark {
        Color(32, 32, 32, 255)
    } else {
        Color(243, 243, 243, 255)
    }
}

fn build_main(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, "main", initial_url(app, "main"))
        .title("SemSearch")
        .initialization_script(&boot_script(app))
        .inner_size(1100.0, 760.0)
        // must fit a 1366x768 screen at 150 % (910x512 logical px minus the taskbar): a larger minimum would push the
        // footer off the screen
        .min_inner_size(640.0, 420.0)
        .center()
        .prevent_overflow()
        .decorations(false)
        .shadow(true)
        .background_color(page_bg(app))
        .visible(visible)
        .focused(visible) // --hidden: don't take the focus from the user's current window
        .disable_drag_drop_handler() // let the page get HTML5 file drops (search by image)
        .on_navigation(allow_navigation)
        .build()
        .inspect(disable_browser_keys)
}

fn build_settings(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    WebviewWindowBuilder::new(app, "settings", initial_url(app, "settings"))
        .title(tr(app, "Настройки — SemSearch", "Settings — SemSearch"))
        .initialization_script(&boot_script(app))
        .inner_size(880.0, 900.0)
        .min_inner_size(600.0, 420.0)
        .center()
        .prevent_overflow()
        .decorations(false)
        .shadow(true)
        .background_color(page_bg(app))
        // Скрыто до page_ready: иначе мелькают пустой WebView2 и страница без данных
        .visible(false)
        .disable_drag_drop_handler()
        .on_navigation(allow_navigation)
        .build()
        .inspect(disable_browser_keys)
}

/// Страховка: если страница так и не сказала page_ready (backend не ответил), показать окно всё равно.
const SETTINGS_SHOW_FALLBACK: Duration = Duration::from_millis(1500);

fn show_if_hidden(w: &WebviewWindow) {
    if !w.is_visible().unwrap_or(true) {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let w = WebviewWindowBuilder::new(app, "tray", initial_url(app, "tray"))
        .title(tr(app, "SemSearch — меню", "SemSearch — menu"))
        .initialization_script(&boot_script(app))
        .inner_size(TRAY_W, TRAY_H)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .disable_drag_drop_handler()
        .on_navigation(allow_navigation)
        .build()?;
    disable_browser_keys(&w);
    Ok(w)
}

/// A file dropped outside a drop zone would navigate the window to that file: never leave our pages that way.
fn allow_navigation(url: &Url) -> bool {
    if url.scheme() == "file" {
        log(&format!("blocked navigation to {url}"));
        return false;
    }
    true
}

/// Switch off WebView2's browser accelerator keys: Ctrl+F/F3 find bar, Ctrl+P, Ctrl+R/F5, Ctrl+±/0 zoom,
/// F7 caret browsing, Alt+Left/Right, … Editing keys (Ctrl+C/V/X/A/Z) still work and pages still get every keydown.
/// Also no page zoom (Ctrl+wheel, touchpad or touch-screen pinch: it would scale the page with no way back, since
/// Ctrl+0 is off too) and no swipe navigation. The newer settings need a newer runtime and are best effort.
fn disable_browser_keys(w: &WebviewWindow) {
    let label = w.label().to_string();
    let res = w.with_webview(move |pw| {
        use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2Settings3, ICoreWebView2Settings5, ICoreWebView2Settings6};
        use windows_core::Interface;
        let r = unsafe {
            pw.controller().CoreWebView2().and_then(|core| core.Settings()).and_then(|s| {
                let _ = s.SetIsZoomControlEnabled(false);
                if let Ok(s5) = s.cast::<ICoreWebView2Settings5>() {
                    let _ = s5.SetIsPinchZoomEnabled(false);
                }
                if let Ok(s6) = s.cast::<ICoreWebView2Settings6>() {
                    let _ = s6.SetIsSwipeNavigationEnabled(false);
                }
                s.cast::<ICoreWebView2Settings3>().and_then(|s3| s3.SetAreBrowserAcceleratorKeysEnabled(false))
            })
        };
        if let Err(e) = r {
            log(&format!("{label}: cannot disable browser accelerator keys: {e}"));
        }
    });
    if let Err(e) = res {
        log(&format!("with_webview: {e}"));
    }
}

fn hide_tray(app: &AppHandle) {
    if let Some(t) = app.get_webview_window("tray") {
        let _ = t.hide();
    }
}

/// A window hidden to the tray keeps its position. If the monitor it was on is gone by the time it is shown again
/// (laptop undocked, RDP session with another resolution), Windows leaves it off-screen: bring it back to a monitor,
/// and shrink it to the work area if it no longer fits.
fn ensure_visible(w: &WebviewWindow) {
    if w.is_maximized().unwrap_or(false) {
        return;
    }
    let (Ok(pos), Ok(size), Ok(mons)) = (w.outer_position(), w.outer_size(), w.app_handle().available_monitors()) else {
        return;
    };
    let (l, t, r, b) = (pos.x, pos.y, pos.x + size.width as i32, pos.y + size.height as i32);
    // part of the window that lies on the monitor's work area, in px
    let overlap = |m: &tauri::Monitor| {
        let wa = m.work_area();
        let (wl, wt) = (wa.position.x, wa.position.y);
        let (wr, wb) = (wl + wa.size.width as i32, wt + wa.size.height as i32);
        ((r.min(wr) - l.max(wl)).max(0), (b.min(wb) - t.max(wt)).max(0))
    };
    // reachable: enough of the window (title strip) is on some monitor to grab it
    let reachable = mons.iter().any(|m| {
        let (ow, oh) = overlap(m);
        ow >= 160 && oh >= 80
    });
    let work = w.current_monitor().ok().flatten().or_else(|| w.primary_monitor().ok().flatten()).map(|m| m.work_area().size);
    let too_big = work.is_some_and(|a| size.width > a.width || size.height > a.height);
    if reachable && !too_big {
        return;
    }
    log(&format!("{}: not on any monitor ({pos:?}, {size:?}) - centering", w.label()));
    if let Some(a) = work {
        let _ = w.set_size(PhysicalSize::new(size.width.min(a.width), size.height.min(a.height)));
    }
    let _ = w.center();
}

fn show_main_window(app: &AppHandle) {
    hide_tray(app);
    let Some(w) = app.get_webview_window("main") else {
        log("show_main: main window is missing");
        return;
    };
    let _ = w.unminimize();
    ensure_visible(&w);
    let _ = w.show();
    let _ = w.set_focus();
    let _ = app.emit_to("main", "focus-search", ());
}

/// Global hotkey: visible & focused -> hide; otherwise show, focus, emit focus-search.
fn toggle_main(app: &AppHandle) {
    let Some(w) = app.get_webview_window("main") else { return };
    let visible = w.is_visible().unwrap_or(false);
    let minimized = w.is_minimized().unwrap_or(false);
    // tao's is_focused() tracks the parent HWND, not the WebView2 child that really has the focus
    let focused = w.hwnd().map(|h| util::is_foreground(h.0 as isize)).unwrap_or(false);
    if visible && !minimized && focused {
        let _ = w.hide();
    } else {
        show_main_window(app);
    }
}

fn open_settings(app: &AppHandle) {
    hide_tray(app);
    if let Some(w) = app.get_webview_window("settings") {
        let _ = w.unminimize();
        ensure_visible(&w);
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    match build_settings(app) {
        // окно покажет page_ready, когда страница отрисуется с данными; это — страховка
        Ok(_) => {
            let app = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(SETTINGS_SHOW_FALLBACK);
                if let Some(w) = app.get_webview_window("settings") {
                    show_if_hidden(&w);
                }
            });
        }
        Err(e) => log(&format!("settings window: {e}")),
    }
}

fn toggle_tray_popup(app: &AppHandle, click: PhysicalPosition<f64>, icon: Option<tauri::Rect>) {
    let Some(tray) = app.get_webview_window("tray") else { return };
    if tray.is_visible().unwrap_or(false) {
        let _ = tray.hide();
        return;
    }
    // The click on the tray icon itself took the focus away from the open popup -> keep it closed.
    if shell(app).tray_blur.lock().unwrap().is_some_and(|t| t.elapsed() < Duration::from_millis(400)) {
        return;
    }
    place_tray_popup(app, &tray, click, icon);
    let hwnd = tray.hwnd().map(|h| h.0 as isize).ok();
    if let Some(h) = hwnd {
        util::make_tool_window(h); // tao rewrites the ex-style on state changes; keep it out of Alt+Tab
    }
    let _ = tray.show();
    if let Some(h) = hwnd {
        util::make_tool_window(h);
    }
    let _ = tray.set_focus();
    let _ = app.emit_to("tray", "tray-shown", ());
}

/// Put the popup just above the taskbar, horizontally centred on the click and clamped to the work area
/// (or next to the taskbar when it is docked at the top / left / right).
/// Значок может лежать не на самой панели задач, а в области скрытых значков («^»), которая открывается
/// НАД панелью — тогда меню ставится за пределами прямоугольника значка, чтобы не накрыть его.
fn place_tray_popup(app: &AppHandle, tray: &WebviewWindow, click: PhysicalPosition<f64>, icon: Option<tauri::Rect>) {
    let mon = app
        .monitor_from_point(click.x, click.y)
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(mon) = mon else { return };
    let k = mon.scale_factor();
    let (w, h, gap) = ((TRAY_W * k).round() as i32, (TRAY_H * k).round() as i32, (TRAY_GAP * k).round() as i32);
    let (mp, ms, wa) = (*mon.position(), *mon.size(), *mon.work_area());
    let (ml, mt, mr) = (mp.x, mp.y, mp.x + ms.width as i32);
    let (wl, wt) = (wa.position.x, wa.position.y);
    let (wr, wb) = (wl + wa.size.width as i32, wt + wa.size.height as i32);
    let (cx, cy) = (click.x.round() as i32, click.y.round() as i32);
    let clamp = |v: i32, lo: i32, hi: i32| v.min(hi).max(lo);
    // прямоугольник значка (left, top, right, bottom) в физических пикселях, если Windows его сообщила
    let icon = icon
        .map(|r| {
            let p: PhysicalPosition<i32> = r.position.to_physical(k);
            let s: PhysicalSize<u32> = r.size.to_physical(k);
            (p.x, p.y, p.x + s.width as i32, p.y + s.height as i32)
        })
        .filter(|&(l, t, r, b)| r > l && b > t);
    let (x, y) = if wt > mt {
        let top = icon.map_or(wt, |(_, _, _, b)| b.max(wt));
        (clamp(cx - w / 2, wl + gap, wr - w - gap), top + gap) // taskbar at the top
    } else if wl > ml {
        let left = icon.map_or(wl, |(_, _, r, _)| r.max(wl));
        (left + gap, clamp(cy - h / 2, wt + gap, wb - h - gap)) // left
    } else if wr < mr {
        let right = icon.map_or(wr, |(l, _, _, _)| l.min(wr));
        (right - w - gap, clamp(cy - h / 2, wt + gap, wb - h - gap)) // right
    } else {
        let bottom = icon.map_or(wb, |(_, t, _, _)| t.min(wb));
        (clamp(cx - w / 2, wl + gap, wr - w - gap), bottom - h - gap) // bottom (or auto-hide)
    };
    let _ = tray.set_position(PhysicalPosition::new(x, y));
    let _ = tray.set_size(PhysicalSize::new(w as u32, h as u32));
}

fn on_tray_event(tray: &TrayIcon, event: TrayIconEvent) {
    let app = tray.app_handle();
    if matches!(event, TrayIconEvent::Click { button_state: MouseButtonState::Up, .. } | TrayIconEvent::DoubleClick { .. }) {
        log(&format!("tray: {event:?}"));
    }
    match event {
        TrayIconEvent::Click {
            button: MouseButton::Left | MouseButton::Right,
            button_state: MouseButtonState::Up,
            position,
            rect,
            ..
        } => {
            if shell(app).ignore_clicks_until.lock().unwrap().is_some_and(|t| Instant::now() < t) {
                return;
            }
            toggle_tray_popup(app, position, Some(rect));
        }
        TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
            *shell(app).ignore_clicks_until.lock().unwrap() = Some(Instant::now() + Duration::from_millis(500));
            show_main_window(app);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- hotkey

/// "ctrl+alt+s" / "Win+Shift+F1" -> accelerator understood by the global-shortcut plugin.
fn normalize_accel(accel: &str) -> String {
    accel
        .split('+')
        .map(|p| p.trim().to_lowercase())
        .filter(|p| !p.is_empty())
        .map(|p| {
            match p.as_str() {
                "win" | "windows" | "meta" | "cmd" | "command" | "super" => "super",
                "control" => "ctrl",
                "option" => "alt",
                "return" => "enter",
                "escape" => "esc",
                "del" => "delete",
                "ins" => "insert",
                "pgup" => "pageup",
                "pgdn" => "pagedown",
                other => other,
            }
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("+")
}

fn pretty_accel(accel: &str) -> String {
    accel
        .split('+')
        .map(|p| {
            let p = p.trim();
            let mut c = p.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// (Re)register the global hotkey. On failure the previous one stays registered and an error text is returned.
fn apply_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    let s = shell(app);
    let gs = app.global_shortcut();
    let accel = accel.trim();
    let new = if accel.is_empty() {
        None
    } else {
        Some(
            Shortcut::from_str(&normalize_accel(accel))
                .map_err(|e| {
                    log(&format!("hotkey {accel:?}: {e}"));
                    format!("{}: {}", tr(app, "Неверное сочетание клавиш", "Invalid shortcut"), pretty_accel(accel))
                })?,
        )
    };
    let mut hk = s.hotkey.lock().unwrap();
    if new.is_some() && new == hk.shortcut {
        hk.accel = accel.to_string();
        return Ok(());
    }
    let old = hk.shortcut.take();
    if let Some(o) = old {
        let _ = gs.unregister(o);
    }
    if let Some(n) = new {
        if let Err(e) = gs.register(n) {
            log(&format!("hotkey {accel:?} registration failed: {e}"));
            if let Some(o) = old {
                if gs.register(o).is_ok() {
                    hk.shortcut = Some(o);
                }
            }
            return Err(if *s.lang.lock().unwrap() == "en" {
                format!("Couldn't set {}: another program already uses this shortcut.", pretty_accel(accel))
            } else {
                format!("Не удалось назначить {}: это сочетание уже занято другой программой.", pretty_accel(accel))
            });
        }
        hk.shortcut = Some(n);
    }
    hk.accel = accel.to_string();
    log(&format!("hotkey = {accel:?}"));
    Ok(())
}

// ---------------------------------------------------------------- commands (docs/API.md §3)

#[tauri::command]
async fn win_minimize(window: WebviewWindow) {
    let _ = window.minimize();
}

#[tauri::command]
async fn win_toggle_maximize(window: WebviewWindow) {
    if window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    } else {
        let _ = window.maximize();
    }
}

/// main / tray: hide; settings: close (it is recreated with fresh data next time).
#[tauri::command]
async fn win_hide(window: WebviewWindow) {
    let _ = window.hide();
    if window.label() == "settings" {
        let _ = window.destroy();
    }
}

#[tauri::command]
async fn show_main(app: AppHandle) {
    show_main_window(&app);
}

#[tauri::command]
async fn show_settings(app: AppHandle) {
    open_settings(&app);
}

#[tauri::command]
async fn tray_hide(app: AppHandle) {
    hide_tray(&app);
}

/// Страница отрисовалась с данными — теперь окно можно показать без мелькания (окно настроек создаётся скрытым).
#[tauri::command]
async fn page_ready(window: WebviewWindow) {
    if window.label() == "settings" {
        show_if_hidden(&window);
    }
}

#[tauri::command]
async fn quit_app(app: AppHandle) {
    quit(&app);
}

#[tauri::command]
async fn pick_folder(window: WebviewWindow) -> Option<String> {
    let w = window.clone();
    let title = tr(window.app_handle(), "Выберите папку", "Choose a folder");
    tauri::async_runtime::spawn_blocking(move || w.dialog().file().set_parent(&w).set_title(title).blocking_pick_folder())
    .await
    .ok()
    .flatten()
    .and_then(|p| p.into_path().ok())
    .map(|p| p.display().to_string())
}

#[tauri::command]
async fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
async fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    let m = app.autolaunch();
    if m.is_enabled().unwrap_or(!enabled) == enabled {
        return Ok(());
    }
    let r = if enabled { enable_autostart() } else { m.disable().map_err(|e| e.to_string()) };
    log(&format!("autostart -> {enabled}: {r:?}"));
    r.map_err(|e| format!("{}: {e}", tr(&app, "Не удалось изменить автозапуск", "Couldn't change startup with Windows")))
}

/// The pages report the UI language they show ("ru" | "en"): the shell's own texts and window titles follow it.
#[tauri::command]
async fn set_lang(app: AppHandle, lang: String) {
    let l: &'static str = if lang == "en" { "en" } else { "ru" };
    let s = shell(&app);
    let old = std::mem::replace(&mut *s.lang.lock().unwrap(), l);
    if old != l {
        log(&format!("language = {l}"));
        retitle(&app);
    }
}

// ---------------------------------------------------------------- autostart

/// Name of the HKCU\...\Run entry (also given to the autostart plugin, which reads and removes it).
const AUTOSTART_NAME: &str = "SemSearch";

/// `"<this exe>" --hidden`. The plugin's enable() writes the path without quotes, which breaks for paths with
/// spaces, so the entry is written by util::set_run_entry instead.
fn autostart_command() -> String {
    format!("\"{}\" --hidden", std::env::current_exe().unwrap_or_default().display())
}

fn enable_autostart() -> Result<(), String> {
    util::set_run_entry(AUTOSTART_NAME, &autostart_command())
}

/// Autostart is on but the entry starts another exe (the folder was moved, or an older build wrote it unquoted):
/// point it at this exe.
fn repair_autostart(app: &AppHandle) {
    if !app.autolaunch().is_enabled().unwrap_or(false) {
        return;
    }
    let want = autostart_command();
    // None: only an all-users (HKLM) entry exists — not ours to change
    if let Some(cur) = util::run_entry(AUTOSTART_NAME).filter(|cur| !cur.eq_ignore_ascii_case(&want)) {
        let r = enable_autostart();
        log(&format!("autostart entry {cur:?} -> {want:?}: {r:?}"));
    }
}

/// The shortcut that is registered right now; "" when none is (disabled, or another program holds it).
#[tauri::command]
async fn get_hotkey(app: AppHandle) -> String {
    let s = shell(&app);
    let hk = s.hotkey.lock().unwrap();
    if hk.shortcut.is_some() {
        hk.accel.clone()
    } else {
        String::new()
    }
}

#[tauri::command]
async fn set_hotkey(app: AppHandle, accel: String) -> Option<String> {
    apply_hotkey(&app, &accel).err()
}

/// The shortcut from config.json could not be registered at start (another program holds it, or the previous SemSearch is
/// still shutting down after a restart): try again now and then, so that it starts working as soon as it is free.
fn retry_hotkey(app: &AppHandle) {
    let s = shell(app);
    let accel = {
        let hk = s.hotkey.lock().unwrap();
        if hk.shortcut.is_some() || hk.accel.is_empty() {
            return;
        }
        hk.accel.clone()
    };
    if apply_hotkey(app, &accel).is_ok() {
        log("hotkey registered after a retry");
    }
}

// ---------------------------------------------------------------- quit

fn quit(app: &AppHandle) {
    let s = shell(app);
    if s.quitting.swap(true, Ordering::SeqCst) {
        return;
    }
    log("quit requested");
    for (_, w) in app.webview_windows() {
        let _ = w.hide();
    }
    if let Some(t) = app.tray_by_id("tray") {
        let _ = t.set_visible(false);
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let s = shell(&app);
        if !s.cfg.no_backend {
            s.backend.shutdown(&s.cfg, Duration::from_secs(5));
        }
        log("bye");
        app.exit(0);
    });
}

// ---------------------------------------------------------------- backend supervision

fn set_status(app: &AppHandle, text: &str) {
    let s = shell(app);
    let mut st = s.status.lock().unwrap();
    if *st != text {
        // don't log every tick of a countdown
        let head = |s: &str| s.chars().take(24).collect::<String>();
        if !text.is_empty() && head(&st) != head(text) {
            log(&format!("status: {text}"));
        }
        *st = text.to_string();
    }
}

/// Point every window at the backend pages (ready) or at the bundled placeholder (not ready),
/// and refresh the placeholder's status line.
fn sync_windows(app: &AppHandle) {
    let s = shell(app);
    let ready = s.ready.load(Ordering::SeqCst);
    let status_text = s.status.lock().unwrap().clone();
    // anything but the "waiting for the server" note of SEMSEARCH_NO_BACKEND is a problem: the page shows it as one
    let is_err = !s.cfg.no_backend && !status_text.is_empty();
    let status = serde_json::to_string(&status_text).unwrap_or_default();
    for (label, w) in app.webview_windows() {
        let Ok(cur) = w.url() else { continue };
        if cur.scheme() == "about" {
            continue; // still loading its initial page
        }
        let target = if ready && !s.cfg.is_backend_url(&cur) {
            let mut u = s.cfg.page_url(route(&label));
            // from the bundled loading page: the backend's launch screen continues it without replaying its entrance
            if is_local(&cur) && label == "main" {
                u.set_fragment(Some("boot"));
            }
            Some(u)
        } else if !ready && !is_local(&cur) {
            Url::parse(LOCAL_ORIGIN).and_then(|u| u.join(local_page(&label))).ok()
        } else {
            None
        };
        match target {
            Some(t) => {
                let mut navs = s.navs.lock().unwrap();
                if navs.get(&label).is_some_and(|(u, at)| *u == t && at.elapsed() < Duration::from_secs(5)) {
                    continue;
                }
                log(&format!("{label}: {cur} -> {t}"));
                navs.insert(label.clone(), (t.clone(), Instant::now()));
                let _ = w.navigate(t);
            }
            None if !ready => {
                let _ = w.eval(format!("window.__semsearchStatus && window.__semsearchStatus({status}, {is_err})"));
            }
            None => {}
        }
    }
}

fn set_ready(app: &AppHandle, ready: bool, why: &str) {
    let s = shell(app);
    if s.ready.swap(ready, Ordering::SeqCst) != ready {
        log(&format!("backend {} ({why})", if ready { "ready" } else { "down" }));
    }
    if ready {
        set_status(app, "");
    }
}

/// What the exit codes of the Windows loader mean for a backend that died before it could write its own log: a
/// computer without the Visual C++ runtime (Python and PyTorch need it) is the usual cause.
fn exit_hint(code: i32, en: bool) -> &'static str {
    match code as u32 {
        0xC000_0135 if en => ": a required DLL is missing - install the Microsoft Visual C++ Redistributable (x64)",
        0xC000_0135 => ": не найдена библиотека - установите Microsoft Visual C++ Redistributable (x64)",
        0xC000_007B if en => ": a library does not fit the program (32/64-bit)",
        0xC000_007B => ": библиотека не подходит программе (32/64 бит)",
        _ => "",
    }
}

fn supervisor(app: AppHandle) {
    let s = shell(&app);
    let mut misses = 0u32; // consecutive probes without our backend answering
    let mut ever_ready = false;
    let (mut hotkey_tries, mut hotkey_at) = (0u32, Instant::now() + Duration::from_secs(10));
    while !s.quitting.load(Ordering::SeqCst) {
        if hotkey_tries < 18 && Instant::now() >= hotkey_at {
            // every 10 s for 3 minutes
            hotkey_tries += 1;
            hotkey_at = Instant::now() + Duration::from_secs(10);
            retry_hotkey(&app);
        }
        let ready = s.ready.load(Ordering::SeqCst);
        // until ready: GET /api/ping + /api/status with the token (docs/API.md §3); afterwards only a TCP liveness
        // check. Without a backend of ours, first look for a running one on the preferred port to attach to.
        let probe = if ready {
            if backend::listening(&s.cfg) {
                Probe::Ready
            } else {
                Probe::Down
            }
        } else if s.cfg.dev || s.backend.has_child() {
            backend::probe(&s.cfg, Duration::from_secs(2))
        } else {
            backend::attach(&s.cfg, Duration::from_secs(2))
        };
        match probe {
            Probe::Ready => {
                misses = 0;
                ever_ready = true;
                if !ready {
                    set_ready(&app, true, "GET /api/ping + /api/status -> 200");
                    s.backend.mark_healthy();
                }
            }
            // SEMSEARCH_DEV_URL: any HTTP answer counts
            Probe::Http(code) if s.cfg.dev => {
                misses = 0;
                ever_ready = true;
                if !ready {
                    set_ready(&app, true, &format!("GET /api/status -> {code}"));
                }
            }
            // nothing answers, or not our backend (the port is taken by another program): (re)start ours
            Probe::Http(_) | Probe::Down => {
                misses += 1;
                let child = s.backend.child_state();
                if ready && (matches!(child, ChildState::Exited(_)) || (child == ChildState::None && misses >= 3)) {
                    set_ready(&app, false, &format!("{child:?}"));
                }
                if s.cfg.no_backend {
                    if !ready {
                        let wait = tr(&app, "Жду сервер поиска на", "Waiting for the search server at");
                        set_status(&app, &format!("{wait} {} (SEMSEARCH_NO_BACKEND=1)…", s.cfg.base()));
                    }
                } else if child == ChildState::None && ever_ready && misses < 3 {
                    // a backend we did not start went away: give it a moment (it may be restarting) before we start ours
                } else if child != ChildState::Running {
                    let wait = s.backend.spawn_wait();
                    if wait == 0 {
                        // the language lock must not live through the match: the Err arm calls tr(), which takes it again
                        let en = *s.lang.lock().unwrap() == "en";
                        match s.backend.spawn(&s.cfg, en) {
                            Ok(()) => set_status(&app, ""),
                            Err(e) => set_status(
                                &app,
                                &format!("{}: {e}", tr(&app, "Не удалось запустить сервер поиска", "Couldn't start the search server")),
                            ),
                        }
                    } else {
                        let en = *s.lang.lock().unwrap() == "en";
                        let code = match *s.backend.last_exit.lock().unwrap() {
                            Some(c) if en => format!(" (code {c}{})", exit_hint(c, true)),
                            Some(c) => format!(" (код {c}{})", exit_hint(c, false)),
                            None => String::new(),
                        };
                        let failed = { let st = s.status.lock().unwrap(); st.starts_with("Не удалось") || st.starts_with("Couldn't") };
                        if !matches!(child, ChildState::None) || !failed {
                            let log_file = s.cfg.data.join("semsearch.log");
                            let text = if en {
                                format!("The search server stopped{code}. Restarting in {wait} s… Details: {}", log_file.display())
                            } else {
                                format!("Сервер поиска остановился{code}. Перезапуск через {wait} с… Подробности: {}", log_file.display())
                            };
                            set_status(&app, &text);
                        }
                    }
                }
            }
        }
        sync_windows(&app);
        let ready = s.ready.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(if ready { 2000 } else { 500 }));
    }
}

// ---------------------------------------------------------------- setup

fn setup(app: &AppHandle) {
    let s = shell(app);
    // An already running backend: open the real pages right away (no loading page flash).
    let running = if s.cfg.dev {
        backend::probe(&s.cfg, Duration::from_millis(1000))
    } else {
        backend::attach(&s.cfg, Duration::from_millis(1000))
    };
    match running {
        Probe::Ready => set_ready(app, true, "already running"),
        Probe::Http(_) if s.cfg.dev => set_ready(app, true, "dev server answers"),
        _ => {}
    }
    repair_autostart(app);
    if let Err(e) = build_main(app, !s.cfg.hidden) {
        // WebView2 broken or too old, its data folder (%LOCALAPPDATA%\com.semsearch.desktop) not writable ...:
        // without a window there is nothing to show, and a tray icon that does nothing would only look like a hang
        let lang = *s.lang.lock().unwrap();
        let log_file = s.cfg.data.join("shell.log").display().to_string();
        fatal(
            lang,
            format!("Не удалось открыть окно SemSearch: {e}\n\nПроверьте, что установлен Microsoft Edge WebView2 Runtime и что есть доступ на запись в папку %LOCALAPPDATA%.\n\nПодробности: {log_file}"),
            format!("Couldn't open the SemSearch window: {e}\n\nMake sure Microsoft Edge WebView2 Runtime is installed and that %LOCALAPPDATA% is writable.\n\nDetails: {log_file}"),
        );
    }
    if let Err(e) = build_tray(app) {
        log(&format!("tray window: {e}"));
    }

    let icon = match util::small_icon_size() {
        ..=17 => tauri::include_image!("icons/tray-16.png"),
        18..=21 => tauri::include_image!("icons/tray-20.png"),
        22..=27 => tauri::include_image!("icons/tray-24.png"),
        _ => tauri::include_image!("icons/tray-32.png"),
    };
    if let Err(e) = TrayIconBuilder::with_id("tray")
        .icon(icon)
        .tooltip("SemSearch")
        .on_tray_icon_event(on_tray_event)
        .build(app)
    {
        log(&format!("tray icon: {e}"));
    }

    let accel = s.cfg.hotkey.clone();
    if !accel.is_empty() {
        if let Err(e) = apply_hotkey(app, &accel) {
            log(&e);
            s.hotkey.lock().unwrap().accel = accel; // still report the configured one
        }
    }

    let h = app.clone();
    std::thread::Builder::new()
        .name("backend-supervisor".into())
        .spawn(move || supervisor(h))
        .expect("supervisor thread");
}

/// A failure the shell cannot work around: say it in a dialog (the release build has no console), log it and exit.
fn fatal(lang: &str, ru: String, en: String) -> ! {
    let text = if lang == "en" { en } else { ru };
    log(&format!("fatal: {text}"));
    util::message_box("SemSearch", &text, false);
    std::process::exit(1);
}

/// The windows are WebView2: without its runtime (Windows 10 before 21H1, LTSC, Server) nothing can be shown.
fn check_webview2(lang: &str) {
    const DOWNLOAD: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";
    match util::webview2_version() {
        Ok(v) => log(&format!("WebView2 runtime {v}")),
        // anything but "not installed" (an odd error from the loader): let the windows try and report their own error
        Err(Some(hr)) => log(&format!("WebView2 runtime check failed: {hr:#010x}")),
        Err(None) => {
            log("WebView2 runtime is not installed");
            let (title, text) = if lang == "en" {
                (
                    "SemSearch - WebView2 is missing",
                    format!(
                        "SemSearch needs the Microsoft Edge WebView2 Runtime, and it is not installed on this computer.\n\nPress OK to open the download page ({DOWNLOAD}), install the Evergreen runtime and start SemSearch again."
                    ),
                )
            } else {
                (
                    "SemSearch — нет WebView2",
                    format!(
                        "Для SemSearch нужен Microsoft Edge WebView2 Runtime, но на этом компьютере его нет.\n\nНажмите OK, чтобы открыть страницу загрузки ({DOWNLOAD}), установите Evergreen-версию и запустите SemSearch снова."
                    ),
                )
            };
            if util::message_box(title, &text, true) {
                util::open_url(DOWNLOAD);
            }
            std::process::exit(1);
        }
    }
}

fn main() {
    util::allow_any_foreground();
    util::set_dpi_aware();
    let cfg = Config::load();
    util::init_log(&cfg.data);
    util::install_panic_hook();
    log(&format!(
        "start {:?}: root={} data={} port={} base={} dev={} no_backend={} hidden={}",
        std::env::current_exe().unwrap_or_default(),
        cfg.root.display(),
        cfg.data.display(),
        cfg.port,
        cfg.base(),
        cfg.dev,
        cfg.no_backend,
        cfg.hidden
    ));
    if let Some(note) = &cfg.data_note {
        log(note);
    }

    let lang = util::resolve_lang(&cfg.lang);
    check_webview2(lang);
    let state = Shell {
        cfg,
        backend: Backend::new(),
        hotkey: Mutex::new(Hotkey::default()),
        ready: AtomicBool::new(false),
        quitting: AtomicBool::new(false),
        status: Mutex::new(String::new()),
        tray_blur: Mutex::new(None),
        ignore_clicks_until: Mutex::new(None),
        navs: Mutex::new(HashMap::new()),
        lang: Mutex::new(lang),
    };

    let app = tauri::Builder::default()
        // must be the first plugin
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            log(&format!("second instance started: {argv:?}"));
            if argv.iter().any(|a| a == "--tray") {
                // `SemSearch.exe --tray`: toggle the tray menu at the mouse pointer (e.g. from a shortcut)
                let at = app.cursor_position().unwrap_or_default();
                toggle_tray_popup(app, at, None);
            } else if !argv.iter().any(|a| a == "--hidden") {
                show_main_window(app);
            }
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_main(app);
                    }
                })
                .build(),
        )
        // is_enabled() / disable() only: the entry itself is written by enable_autostart() (quoted path)
        .plugin(tauri_plugin_autostart::Builder::new().app_name(AUTOSTART_NAME).build())
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            win_minimize,
            win_toggle_maximize,
            win_hide,
            show_main,
            show_settings,
            tray_hide,
            quit_app,
            pick_folder,
            get_autostart,
            set_autostart,
            get_hotkey,
            set_hotkey,
            page_ready,
            set_lang
        ])
        .on_window_event(|window, event| match event {
            // closing main (Alt+F4 etc.) only hides it — SemSearch keeps running in the tray
            WindowEvent::CloseRequested { api, .. } if window.label() != "settings" => {
                api.prevent_close();
                let _ = window.hide();
            }
            WindowEvent::Focused(false) if window.label() == "tray" => {
                *shell(window.app_handle()).tray_blur.lock().unwrap() = Some(Instant::now());
                let _ = window.hide();
            }
            _ => {}
        })
        .setup(|app| {
            setup(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .unwrap_or_else(|e| {
            fatal(
                lang,
                format!("Не удалось запустить SemSearch: {e}"),
                format!("Couldn't start SemSearch: {e}"),
            )
        });

    app.run(|app, event| {
        if let RunEvent::ExitRequested { code: None, api, .. } = event {
            // last window closed: keep running in the tray unless we are quitting
            if !shell(app).quitting.load(Ordering::SeqCst) {
                api.prevent_exit();
            }
        }
    });
}

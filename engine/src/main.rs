//! semsearch-engine — индекс имён всех файлов (как Everything) с мгновенным поиском и
//! живой лентой изменений, отдаётся по `\\.\pipe\semsearch-engine-<SessionId>` (docs/API.md §4).
//!
//! Режимы: MFT + USN-журнал (с правами администратора) или параллельный обход +
//! ReadDirectoryChangesExW (без них). Флаги: --walk, --selftest, --console.

#![windows_subsystem = "windows"]

#[macro_use]
mod util;
mod classify;
mod eject;
mod engine;
mod index;
mod mft;
mod ops;
mod search;
mod server;
mod walk;

use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::Instant;

use engine::*;

const USAGE: &str = "semsearch-engine [--root <dir>] [--pipe <name>] [--walk] [--console] [--selftest]
  --root <dir> project root (folder with server.py); default: nearest ancestor
               of the exe folder that contains server.py or config.json
  --pipe <name> pipe name (\\\\.\\pipe\\<name>), default semsearch-engine-<SessionId>;
               use another name for a development instance
  --walk       force walk mode (directory enumeration + ReadDirectoryChangesW)
  --console    log to stdout instead of the log file
  --selftest   build the index, run sample searches and a live-update check,
               print stats JSON and exit
Data folder <data>: %SEMSEARCH_DATA_DIR% (absolute path), else
%LOCALAPPDATA%\\SemSearch.
Settings: <data>\\config.json, else <root>\\config.json (old layout, read-only),
else defaults: all fixed drives (NTFS: MFT/USN when elevated, otherwise and on
other file systems: walk).
Logs: <data>\\engine.log; when elevated <exe dir>\\logs\\engine.log
(an elevated engine never writes into the user-writable data folder).
Pipe: \\\\.\\pipe\\semsearch-engine-<session id>
";

/// Значение флага: `--root <dir>` или `--root=<dir>`.
fn arg_value(args: &[String], flag: &str) -> Option<String> {
    for (i, a) in args.iter().enumerate() {
        if a.eq_ignore_ascii_case(flag) {
            return args.get(i + 1).cloned();
        }
        if let Some((k, v)) = a.split_once('=') {
            if k.eq_ignore_ascii_case(flag) {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn main() {
    util::set_error_mode();
    // args_os: путь с непарным суррогатом (бывает в именах NTFS) не должен ронять процесс до первой строки журнала
    let args: Vec<String> = std::env::args_os().skip(1).map(|a| a.to_string_lossy().into_owned()).collect();
    let has = |f: &str| args.iter().any(|a| a.eq_ignore_ascii_case(f));
    if has("--help") || has("-h") || has("/?") {
        let mut w = util::stdout_writer();
        let _ = w.write_all(USAGE.as_bytes());
        return;
    }
    let force_walk = has("--walk");
    let selftest = has("--selftest");
    let console = has("--console");
    let elevated = util::is_elevated();
    let (root, root_err) = match arg_value(&args, "--root") {
        Some(r) => match util::check_root(&r) {
            Ok(p) => (p, None),
            Err(e) => (util::find_root(), Some(e)),
        },
        None => (util::find_root(), None),
    };
    let (data, data_err) = util::data_dir();
    let exe_dir = util::exe_dir();
    // с повышенными правами — ничего не пишем в папку данных (она доступна пользователю на запись)
    let log_dir = if elevated { exe_dir.join("logs") } else { data.clone() };
    if selftest {
        util::log_init(&log_dir, true, Some("engine-selftest.log"));
    } else if console {
        util::log_init(&log_dir, true, None);
    } else {
        util::log_init(&log_dir, false, Some("engine.log"));
    }
    std::panic::set_hook(Box::new(|pi| {
        util::log_write('E', &format!("PANIC: {pi}"));
    }));
    util::set_below_normal();
    info!(
        "semsearch-engine {} start: pid {}, elevated {elevated}, root {}, data {}, exe dir {}, logs {}, args {:?}",
        ops::VERSION,
        std::process::id(),
        root.display(),
        data.display(),
        exe_dir.display(),
        log_dir.display(),
        args
    );
    if let Some(e) = root_err {
        warn!("--root ignored ({e}); using {}", root.display());
    }
    if let Some(e) = data_err {
        warn!("{e}; using {}", data.display());
    }
    if let Some(p) = arg_value(&args, "--pipe") {
        // неверное имя — выход, а не тихий откат на рабочий канал
        if let Err(e) = server::set_pipe_name(p.trim()) {
            warn!("{e}");
            std::process::exit(2);
        }
    }
    if selftest {
        let code = run_selftest(root, data, log_dir, elevated, force_walk);
        std::process::exit(code);
    }
    let (first, sec) = match server::create_first() {
        Ok(x) => x,
        Err(server::FirstErr::Busy) => {
            info!("another engine instance owns {} — exiting", server::pipe_name());
            std::process::exit(0);
        }
        Err(server::FirstErr::Other(e)) => {
            warn!("cannot create pipe: {e}");
            std::process::exit(1);
        }
    };
    let eng = Engine::new(root.clone(), data.clone(), elevated, force_walk);
    let drives = util::config_drives(&data, &root);
    let cfg_file = util::config_path(&data, &root).map(|p| p.display().to_string());
    info!("name drives: {:?} (config: {})", drives, cfg_file.as_deref().unwrap_or("none, defaults"));
    if drives.is_empty() {
        warn!("no drives to index (no fixed drive with a file system, or config.json name_drives lists none of them)");
    }
    if !elevated || force_walk {
        // для журнала: проверка непривилегированного USN-журнала (имена в нём обнуляются)
        if let Some(d) = drives.first().cloned() {
            std::thread::spawn(move || {
                let p = mft::probe_unprivileged(&d);
                info!("FSCTL_READ_UNPRIVILEGED_USN_JOURNAL probe on {d}: {p}");
            });
        }
    }
    for d in &drives {
        eng.start_volume(d);
    }
    // внешние диски подключают и отключают, BitLocker разблокируют, буквы меняют — без перезапуска движка
    eng.watch_drives(drives.clone());
    let e2 = eng.clone();
    let _ = std::thread::Builder::new().name("stats".into()).spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(1800));
        let vols: Vec<String> = e2
            .slots()
            .iter()
            .map(|s| {
                let i = s.info();
                let v = s.read();
                format!("{} {} files {} dirs {} garbage {} KB rescans {}", i.drive, i.mode, v.files, v.dirs, v.garbage >> 10, i.rescans)
            })
            .collect();
        let subs = e2.subs.lock().map(|s| s.len()).unwrap_or(0);
        info!("stats: {}; subscribers {subs}; {}", vols.join("; "), util::mem_str());
    });
    server::serve(eng, first, sec);
}

use std::io::Write;

fn parse(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Null)
}

/// Ждать условие до 5 с; Some(мс) — выполнено.
fn poll(f: impl Fn() -> bool) -> Option<u64> {
    let t0 = Instant::now();
    while t0.elapsed() < std::time::Duration::from_secs(5) {
        if f() {
            return Some(t0.elapsed().as_millis() as u64);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}

/// Проверка живых обновлений: своя папка `<base>\semsearch-selftest-<pid>` (создаётся заново —
/// уже существующая не используется — и удаляется только этим тестом, без рекурсивного удаления),
/// события подписки и результаты поиска.
fn live_check(eng: &std::sync::Arc<Engine>, base: PathBuf) -> Value {
    use std::sync::mpsc::sync_channel;
    let tag = format!("sstest{}", std::process::id());
    let dir = base.join(format!("semsearch-selftest-{}", std::process::id()));
    let dir_s = dir.to_string_lossy().to_string();
    let (tx, rx) = sync_channel::<std::sync::Arc<str>>(10000);
    let sub = std::sync::Arc::new(Subscriber::new(&[dir_s.clone()], tx));
    eng.add_sub(sub.clone());
    let total = |q: String| -> u64 { parse(&ops::search(eng, &json!({"q": q, "limit": 5})).unwrap_or_default())["total"].as_u64().unwrap_or(u64::MAX) };
    let indexed = |p: &str| -> bool { parse(&ops::exists_op(eng, &json!({"path": p})).unwrap_or_default())["indexed"].as_bool().unwrap_or(false) };
    let f1 = dir.join(format!("{tag}_alpha.txt"));
    let f2 = dir.join(format!("{tag}_beta.txt"));
    let mut steps = Vec::new();
    let mut step = |name: &str, r: Option<u64>| steps.push(json!({"step": name, "ok": r.is_some(), "ms": r}));
    // create_dir (не _all): если по этому пути уже что-то есть — тест не пишет туда ничего
    let created = std::fs::create_dir(&dir).is_ok();
    step("mkdir", if created { poll(|| indexed(&dir_s)) } else { None });
    if created {
        let ok = std::fs::write(&f1, b"semsearch selftest").is_ok();
        step("create file", if ok { poll(|| total(format!("{tag}_alpha")) == 1) } else { None });
        let ok = std::fs::rename(&f1, &f2).is_ok();
        step("rename file", if ok { poll(|| total(format!("{tag}_beta")) == 1 && total(format!("{tag}_alpha")) == 0) } else { None });
        let ok = std::fs::remove_file(&f2).is_ok();
        step("delete file", if ok { poll(|| total(format!("{tag}_beta")) == 0) } else { None });
        let ok = std::fs::remove_dir(&dir).is_ok();
        step("rmdir", if ok { poll(|| !indexed(&dir_s)) } else { None });
        // уборка только своих файлов, без рекурсивного удаления
        let _ = std::fs::remove_file(&f1);
        let _ = std::fs::remove_file(&f2);
        let _ = std::fs::remove_dir(&dir);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    eng.remove_sub(&sub);
    let mut kinds = serde_json::Map::new();
    while let Ok(line) = rx.try_recv() {
        let ev = parse(&line);
        let k = ev["ev"].as_str().unwrap_or("?").to_string();
        let n = kinds.get(&k).and_then(|v| v.as_u64()).unwrap_or(0) + 1;
        kinds.insert(k, json!(n));
    }
    json!({"dir": dir_s, "steps": steps, "events": kinds})
}

/// --selftest: запустить тома как в обычной работе (MFT + USN-журнал или обход + RDCW),
/// дождаться готовности, прогнать поиски / list / same_name / exists, проверить живые
/// обновления и вывести статистику JSON (также в файл out_dir\engine-selftest.json).
fn run_selftest(root: PathBuf, data: PathBuf, out_dir: PathBuf, elevated: bool, force_walk: bool) -> i32 {
    // Отчёт — туда же, где лог: без прав — папка данных, с правами — <папка exe>\logs.
    // Проверка живых обновлений: без прав — в %TEMP%; с правами и exe вне папки проекта
    // (установлен в Program Files) — в <папка exe>\logs; с правами из папки проекта — пропускаем
    // (не пишем от администратора туда, где пользователь может подменить папку ссылкой).
    let exe_dir = util::exe_dir();
    let installed = elevated && !util::path_under(&exe_dir, &root);
    let live_base: Option<PathBuf> = if !elevated {
        std::env::var("TEMP").ok().map(PathBuf::from)
    } else if installed {
        Some(exe_dir.join("logs"))
    } else {
        None
    };
    let drives = util::config_drives(&data, &root);
    let eng = Engine::new(root.clone(), data, elevated, force_walk);
    let t_all = Instant::now();
    for d in &drives {
        eng.start_volume(d);
    }
    let deadline = Instant::now() + std::time::Duration::from_secs(1800);
    loop {
        let done = eng.slots().iter().all(|s| {
            let i = s.info();
            i.ready || (i.fs.is_empty() && !i.note.is_empty())
        });
        if done {
            break;
        }
        if Instant::now() > deadline {
            warn!("selftest: index not ready after 30 min");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let build_total = t_all.elapsed().as_millis() as u64;
    let m_after_build = util::mem();
    let vols: Vec<Value> = eng
        .slots()
        .iter()
        .map(|s| {
            let i = s.info();
            let v = s.read();
            json!({
                "drive": i.drive, "fs": i.fs, "mode": i.mode, "ready": i.ready, "note": i.note,
                "files": v.files, "dirs": v.dirs, "build_ms": i.build_ms, "index_mb": v.mem_bytes() >> 20,
                "journal_id": format!("{:#x}", i.journal_id), "mft_records": i.records,
                "mft_phases_ms": {"enumerate": i.enum_ms, "parents": i.parents_ms, "exclusions_childmap": i.finish_ms, "hardlink_pass": i.links_ms, "enum_threads": i.enum_threads},
                "orphans": i.orphans, "hidden": i.hidden, "hardlink_names_added": i.links,
            })
        })
        .collect();
    info!("selftest: ready in {build_total} ms; {}", util::mem_str());

    // поиски: второй прогон — «тёплый»
    let queries = [
        "notepad.exe",
        "cmd.exe",
        "kernel32.dll",
        "*.pdf",
        "документ",
        "отчет 2024",
        r"re:^readme\.(md|txt)$",
        r"windows\system32 kernel32",
        r"*\documents\*.pdf",
        "setup exe",
        "a",
    ];
    let mut searches = Vec::new();
    for q in queries {
        let req = json!({"q": q, "limit": 200});
        let _ = ops::search(&eng, &req);
        let r = parse(&ops::search(&eng, &req).unwrap_or_else(|e| json!({"error": e}).to_string()));
        let top: Vec<Value> = r["items"].as_array().map(|a| a.iter().take(3).map(|i| i["path"].clone()).collect()).unwrap_or_default();
        searches.push(json!({"q": q, "total": r["total"], "search_ms": r["search_ms"], "ms_with_stats": r["ms"], "error": r.get("error"), "top": top}));
    }
    // имена из System32 (проверка жёстких ссылок WinSxS ↔ System32 в режиме MFT); системная папка и
    // папка пользователя — известные папки, запасные — %SystemRoot%\System32, %USERPROFILE%
    use windows_sys::Win32::UI::Shell::{FOLDERID_Profile, FOLDERID_System};
    let known = |id| util::known_folder(id).map(|p| p.to_string_lossy().to_string());
    let sys_dir = known(&FOLDERID_System)
        .or_else(|| std::env::var("SystemRoot").ok().map(|r| format!(r"{r}\System32")))
        .unwrap_or_default();
    let sys32 = parse(&ops::search(&eng, &json!({"q": "notepad.exe", "roots": [sys_dir], "limit": 5})).unwrap_or_default());
    let home = known(&FOLDERID_Profile).or_else(|| std::env::var("USERPROFILE").ok()).unwrap_or_default();
    let lreq = json!({"roots": [home], "exts": [".pdf", ".docx", ".py", ".jpg"], "exclude_dirs": [], "exclude_paths": [], "skip_dot_dirs": true, "skip_cloud": true});
    let t = Instant::now();
    let lr = parse(&ops::list(&eng, &lreq).unwrap_or_default());
    let list = json!({"root": lreq["roots"][0], "items": lr["total"], "ms": t.elapsed().as_millis() as u64, "index_ms": lr["index_ms"]});
    let sn = parse(&ops::same_name(&eng, &json!({"name": "notepad.exe", "limit": 5})).unwrap_or_default());
    let ex = parse(&ops::exists_op(&eng, &json!({"path": format!(r"{sys_dir}\cmd.exe")})).unwrap_or_default());
    let live = match live_base {
        Some(b) if b.is_dir() => live_check(&eng, b),
        Some(b) => json!({"skipped": format!("folder for the test does not exist: {}", b.display())}),
        None => json!({"skipped": "elevated run from the user-writable project folder; run the installed copy (Program Files) with --root to test live updates"}),
    };
    let m = util::mem();
    let (files, dirs): (i64, i64) = eng
        .slots()
        .iter()
        .map(|s| {
            let v = s.read();
            (v.files, v.dirs)
        })
        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
    let probe = drives.first().map(|d| mft::probe_unprivileged(d)).unwrap_or(Value::Null);
    let (k, u) = util::cpu_times();
    let report = json!({
        "version": ops::VERSION,
        "mode": eng.mode(),
        "elevated": elevated,
        "drives": drives,
        "build_ms": build_total,
        "files": files,
        "dirs": dirs,
        "rss_mb": m.ws >> 20,
        "rss_after_build_mb": m_after_build.ws >> 20,
        "peak_rss_mb": m.peak_ws >> 20,
        "private_mb": m.private >> 20,
        "cpu_s": {"kernel": k, "user": u},
        "volumes": vols,
        "searches": searches,
        "system32_notepad": {"total": sys32["total"], "top": sys32["items"].as_array().map(|a| a.iter().map(|i| i["path"].clone()).collect::<Vec<_>>())},
        "list": list,
        "same_name_notepad": sn["items"].as_array().map(|a| a.iter().map(|i| i["path"].clone()).collect::<Vec<_>>()),
        "exists_cmd": ex,
        "live_updates": live,
        "unprivileged_usn_probe": probe,
    });
    let text = serde_json::to_string_pretty(&report).unwrap_or_default();
    let _ = std::fs::create_dir_all(&out_dir);
    let out_file = out_dir.join("engine-selftest.json");
    match util::write_file(&out_file, text.as_bytes()) {
        Ok(()) => info!("selftest report written to {}", out_file.display()),
        Err(e) => warn!("cannot write {}: {e}", out_file.display()),
    }
    let mut out = util::stdout_writer();
    let _ = out.write_all(text.as_bytes());
    let _ = out.write_all(b"\n");
    let _ = out.flush();
    for s in eng.slots() {
        s.request_stop();
    }
    0
}

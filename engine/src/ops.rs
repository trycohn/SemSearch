//! Обработчики операций протокола (docs/API.md §4). Каждая возвращает JSON-объект строкой.

use rayon::prelude::*;
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::atomic::Ordering::Relaxed;
use std::time::Instant;

use crate::engine::*;
use crate::index::*;
use crate::search::*;
use crate::util::*;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
pub struct SearchItem {
    pub path: String,
    pub name: String,
    pub dir: bool,
    pub size: Option<u64>,
    pub mtime: Option<f64>,
    pub ctime: Option<f64>,
}

#[derive(Serialize)]
pub struct ListItem {
    pub path: String,
    pub size: u64,
    pub mtime: f64,
    pub attrs: u32,
    /// classify: null | "submodule" | "gitignored" | "archive" | "foreign" (без classify поля нет)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<Option<&'static str>>,
    /// classify: корень репозитория или null
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo: Option<Option<Arc<str>>>,
}

pub fn str_list(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(|s| s.to_string()).filter(|s| !s.trim().is_empty()).collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn get_str<'a>(req: &'a Value, k: &str) -> &'a str {
    req.get(k).and_then(Value::as_str).unwrap_or("")
}
fn get_bool(req: &Value, k: &str, d: bool) -> bool {
    req.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn get_u64(req: &Value, k: &str, d: u64) -> u64 {
    req.get(k).and_then(|v| v.as_u64().or_else(|| v.as_f64().map(|f| f.max(0.0) as u64))).unwrap_or(d)
}

fn items_with_stats(rows: Vec<(String, String, bool)>) -> Vec<SearchItem> {
    rows.into_par_iter()
        .map(|(path, name, dir)| {
            let st = stat(&path);
            SearchItem {
                size: if dir { None } else { st.map(|s| s.size) },
                mtime: st.map(|s| s.mtime),
                ctime: st.map(|s| s.ctime),
                path,
                name,
                dir,
            }
        })
        .collect()
}

pub fn ping(eng: &Engine) -> Result<String, String> {
    Ok(json!({"version": VERSION, "mode": eng.mode(), "elevated": eng.elevated}).to_string())
}

pub fn status(eng: &Engine) -> Result<String, String> {
    let slots = eng.slots();
    let mut vols = Vec::new();
    let (mut files, mut dirs, mut build_ms) = (0i64, 0i64, 0u64);
    let mut ready = true; // нет дисков в конфиге — индексировать нечего
    for s in &slots {
        let i = s.info();
        let (f, d) = {
            let v = s.read();
            (v.files, v.dirs)
        };
        if i.ready {
            files += f;
            dirs += d;
        }
        ready &= i.ready;
        build_ms = build_ms.max(i.build_ms);
        vols.push(json!({
            "drive": i.drive, "fs": i.fs, "mode": i.mode, "files": f, "dirs": d, "ready": i.ready,
            "build_ms": i.build_ms, "note": i.note, "rescans": i.rescans, "links": i.links,
            "phases_ms": {"enumerate": i.enum_ms, "parents": i.parents_ms, "exclusions_childmap": i.finish_ms, "hardlink_pass": i.links_ms, "enum_threads": i.enum_threads},
        }));
    }
    let m = mem();
    Ok(json!({
        "mode": eng.mode(), "ready": ready, "files": files, "dirs": dirs, "volumes": vols,
        "updated": eng.updated_ms.load(Relaxed), "build_ms": build_ms,
        "elevated": eng.elevated, "version": VERSION,
        "rss_mb": m.ws >> 20, "private_mb": m.private >> 20,
        "subscribers": eng.subs.lock().map(|s| s.len()).unwrap_or(0),
    })
    .to_string())
}

pub fn search(eng: &Engine, req: &Value) -> Result<String, String> {
    let t0 = Instant::now();
    let q = get_str(req, "q");
    let mode = req.get("mode").and_then(Value::as_str).unwrap_or("auto");
    let limit = get_u64(req, "limit", 200).min(1000) as usize;
    let offset = get_u64(req, "offset", 0).min(100_000) as usize;
    let files_only = get_bool(req, "files_only", true);
    let roots = str_list(req.get("roots"));
    let exts = str_list(req.get("exts"));
    let query = parse_query(q, mode, files_only, &exts)?;
    let k = offset + limit;
    let slots = eng.slots();
    let guards: Vec<_> = slots.iter().map(|s| s.read()).collect();
    let mut total = 0u64;
    let mut cands: Vec<Cand> = Vec::new();
    for (vi, g) in guards.iter().enumerate() {
        let (t, c) = search_volume(g, vi as u32, &query, &roots, k);
        total += t;
        cands.extend(c);
    }
    cands.sort_unstable();
    let rows: Vec<(String, String, bool)> = cands
        .iter()
        .skip(offset)
        .take(limit)
        .map(|c| {
            let v = &guards[c.vol as usize];
            (v.path(c.idx), v.name(c.idx).to_string(), v.is_dir(c.idx))
        })
        .collect();
    drop(guards);
    let search_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let items = items_with_stats(rows);
    let ms = t0.elapsed().as_millis() as u64;
    let items_json = serde_json::to_string(&items).map_err(|e| e.to_string())?;
    Ok(format!(r#"{{"total":{total},"ms":{ms},"search_ms":{search_ms:.2},"items":{items_json}}}"#))
}

pub fn same_name(eng: &Engine, req: &Value) -> Result<String, String> {
    let name = get_str(req, "name").trim();
    if name.is_empty() {
        return Err("name required".into());
    }
    let name = name.rsplit(|c| c == '\\' || c == '/').next().unwrap_or(name);
    let limit = get_u64(req, "limit", 10).min(1000) as usize;
    let files_only = get_bool(req, "files_only", true);
    let lname = fold(name);
    let mut rows: Vec<(u32, String, String, bool)> = Vec::new();
    for s in eng.slots() {
        let v = s.read();
        for i in same_name_volume(&v, &lname, files_only) {
            rows.push((v.path_len(i), v.path(i), v.name(i).to_string(), v.is_dir(i)));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    let total = rows.len();
    rows.truncate(limit);
    let items = items_with_stats(rows.into_iter().map(|(_, p, n, d)| (p, n, d)).collect());
    let items_json = serde_json::to_string(&items).map_err(|e| e.to_string())?;
    Ok(format!(r#"{{"total":{total},"items":{items_json}}}"#))
}

pub fn list(eng: &Engine, req: &Value) -> Result<String, String> {
    let t0 = Instant::now();
    let roots = str_list(req.get("roots"));
    if roots.is_empty() {
        return Err("roots required".into());
    }
    let skip_cloud = get_bool(req, "skip_cloud", true);
    let classify = get_bool(req, "classify", false);
    let spec = ListSpec {
        roots,
        exts: norm_exts(&str_list(req.get("exts"))),
        excl_dirs: str_list(req.get("exclude_dirs")).iter().map(|d| fold(d.trim())).collect(),
        excl_paths: str_list(req.get("exclude_paths")),
        skip_dot: get_bool(req, "skip_dot_dirs", true),
        skip_cloud,
        smart: get_bool(req, "smart_excludes", false),
    };
    let mut groups: Vec<ListGroup> = Vec::new();
    let (mut smart_dirs, mut smart_files) = (0u64, 0u64);
    let mut tags = crate::classify::TagCounts::default();
    let mut classify_ms = 0u64;
    for s in eng.slots() {
        let v = s.read();
        let mut out = list_volume(&v, &spec);
        smart_dirs += out.smart_dirs;
        smart_files += out.smart_files;
        if classify {
            let tc = Instant::now();
            tags.add(&crate::classify::classify_groups(&v, &out.roots, &mut out.groups));
            classify_ms += tc.elapsed().as_millis() as u64;
        }
        groups.extend(out.groups);
    }
    let idx_ms = t0.elapsed().as_millis() as u64;
    let items: Vec<ListItem> = groups.into_par_iter().flat_map_iter(|g| stat_group(g, skip_cloud, classify)).collect();
    let ms = t0.elapsed().as_millis() as u64;
    let n = items.len();
    let mut extra = String::new();
    if spec.smart {
        extra.push_str(&format!(r#","smart_skipped":{{"dirs":{smart_dirs},"files":{smart_files}}}"#));
    }
    if classify {
        // счётчики по найденным в индексе файлам (до проверки на диске)
        extra.push_str(&format!(
            r#","classify_ms":{classify_ms},"repos":{},"tags":{{"submodule":{},"gitignored":{},"archive":{},"foreign":{},"none":{}}}"#,
            tags.repos, tags.submodule, tags.gitignored, tags.archive, tags.foreign, tags.none
        ));
    }
    let items_json = serde_json::to_string(&items).map_err(|e| e.to_string())?;
    Ok(format!(r#"{{"total":{n},"ms":{ms},"index_ms":{idx_ms}{extra},"items":{items_json}}}"#))
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

/// Размер/время/атрибуты для файлов одного каталога: если нужна заметная доля каталога —
/// одно перечисление FindFirstFileExW, иначе GetFileAttributesExW на каждый файл.
fn stat_group(g: ListGroup, skip_cloud: bool, classify: bool) -> Vec<ListItem> {
    let keep = |a: u32| a & A_DIRECTORY == 0 && !(skip_cloud && a & (A_CLOUD | A_REPARSE) != 0);
    // тег k-го файла группы (только с classify)
    let tag_of = |k: usize| -> (Option<Option<&'static str>>, Option<Option<Arc<str>>>) {
        if !classify {
            return (None, None);
        }
        match g.tags.get(k) {
            Some((t, r)) => (Some(*t), Some(r.clone())),
            None => (Some(None), Some(None)),
        }
    };
    let mut out = Vec::with_capacity(g.names.len());
    if g.names.len() >= 4 && g.names.len() * 8 >= g.nchild as usize {
        let want: std::collections::HashMap<&str, usize> = g.names.iter().enumerate().map(|(k, s)| (s.as_str(), k)).collect();
        let ok = for_each_dir_entry(&g.dir, |name, a, size, mtime| {
            if let Some(&k) = want.get(name) {
                if keep(a) {
                    let (tag, repo) = tag_of(k);
                    out.push(ListItem { path: join(&g.dir, name), size, mtime, attrs: a, tag, repo });
                }
            }
        });
        if ok {
            return out;
        }
        out.clear();
    }
    for (k, n) in g.names.iter().enumerate() {
        let p = join(&g.dir, n);
        if let Some(st) = stat(&p) {
            if keep(st.attrs) {
                let (tag, repo) = tag_of(k);
                out.push(ListItem { path: p, size: st.size, mtime: st.mtime, attrs: st.attrs, tag, repo });
            }
        }
    }
    out
}

pub fn exists_op(eng: &Engine, req: &Value) -> Result<String, String> {
    let path = norm_path(get_str(req, "path"));
    if path.is_empty() {
        return Err("path required".into());
    }
    // только локальные пути вида "C:\..." (UNC/устройства не трогаем — движок может быть повышенным)
    if drive_of(&path).is_none() || path.as_bytes().get(2) != Some(&b'\\') {
        return Ok(json!({"exists": false, "indexed": false}).to_string());
    }
    let on_disk = exists(&path);
    let mut indexed = false;
    if let Some(d) = drive_of(&path) {
        for s in eng.slots() {
            if s.drive.eq_ignore_ascii_case(&d) {
                let v = s.read();
                indexed = v.resolve(&path).map(|i| i == 0 || v.visible(i)).unwrap_or(false);
            }
        }
    }
    Ok(json!({"exists": on_disk, "indexed": indexed}).to_string())
}

pub fn reload_config(eng: &Arc<Engine>) -> Result<String, String> {
    let drives = config_drives(&eng.root);
    info!("reload_config: drives {:?}", drives);
    eng.apply_drives(&drives);
    Ok(json!({"drives": drives}).to_string())
}

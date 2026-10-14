//! Режим обхода (без прав администратора / не NTFS / сбой MFT):
//! параллельный FindFirstFileExW по нескольким потокам + ReadDirectoryChangesExW на корень диска.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::*};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::System::IO::*;

use crate::engine::*;
use crate::index::*;
use crate::util::*;

const RDCW_BUF: usize = 1 << 20;
const NOTIFY_FILTER: u32 = FILE_NOTIFY_CHANGE_FILE_NAME
    | FILE_NOTIFY_CHANGE_DIR_NAME
    | FILE_NOTIFY_CHANGE_ATTRIBUTES
    | FILE_NOTIFY_CHANGE_SIZE
    | FILE_NOTIFY_CHANGE_LAST_WRITE;
pub const ATTR_UNKNOWN: u32 = u32::MAX;

pub fn is_cloud_tag(tag: u32) -> bool {
    (tag & 0xFFFF_0FFF) == 0x9000_001A || tag == 0x8000_0021
}

/// Спускаться ли в каталог: не точка повторной обработки (junction/symlink), кроме облачных заглушек.
pub fn descend(attrs: u32, tag: u32) -> bool {
    attrs & A_DIRECTORY != 0 && (attrs & A_REPARSE == 0 || is_cloud_tag(tag))
}

/// Имена, исключаемые на любом уровне.
pub fn excluded_name(lname: &[u8]) -> bool {
    lname == b"$recycle.bin" || lname == b"system volume information"
}

// ---------------------------------------------------------------- directory listing

pub struct DirItem {
    pub name: Vec<u16>,
    pub attrs: u32,
    pub tag: u32,
}

static NO_ON_DISK_FLAG: AtomicBool = AtomicBool::new(false);

/// Список каталога (path — без NUL, в форме `\\?\C:\...`).
pub fn list_dir(path: &[u16], out: &mut Vec<DirItem>) -> Result<(), u32> {
    let mut pat: Vec<u16> = Vec::with_capacity(path.len() + 3);
    pat.extend_from_slice(path);
    if pat.last() != Some(&(b'\\' as u16)) {
        pat.push(b'\\' as u16);
    }
    pat.push(b'*' as u16);
    pat.push(0);
    unsafe {
        let mut fd: WIN32_FIND_DATAW = std::mem::zeroed();
        let mut flags = FIND_FIRST_EX_LARGE_FETCH;
        if !NO_ON_DISK_FLAG.load(Relaxed) {
            flags |= FIND_FIRST_EX_ON_DISK_ENTRIES_ONLY;
        }
        let mut h = FindFirstFileExW(pat.as_ptr(), FindExInfoBasic, &mut fd as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), flags);
        if h == INVALID_HANDLE_VALUE && GetLastError() == ERROR_INVALID_PARAMETER && flags & FIND_FIRST_EX_ON_DISK_ENTRIES_ONLY != 0 {
            NO_ON_DISK_FLAG.store(true, Relaxed);
            h = FindFirstFileExW(pat.as_ptr(), FindExInfoBasic, &mut fd as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), FIND_FIRST_EX_LARGE_FETCH);
        }
        if h == INVALID_HANDLE_VALUE {
            return Err(GetLastError());
        }
        loop {
            let n = fd.cFileName.iter().position(|&c| c == 0).unwrap_or(fd.cFileName.len());
            let name = &fd.cFileName[..n];
            let dot = name == [b'.' as u16] || name == [b'.' as u16, b'.' as u16];
            if !dot && n > 0 {
                let tag = if fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 { fd.dwReserved0 } else { 0 };
                out.push(DirItem { name: name.to_vec(), attrs: fd.dwFileAttributes, tag });
            }
            if FindNextFileW(h, &mut fd) == 0 {
                break;
            }
        }
        FindClose(h);
    }
    Ok(())
}

// ---------------------------------------------------------------- parallel build

struct Job {
    idx: u32,
    path: Vec<u16>,
}

struct WorkQ {
    st: Mutex<(Vec<Job>, usize)>,
    cv: Condvar,
}

impl WorkQ {
    fn push(&self, jobs: &mut Vec<Job>) {
        if jobs.is_empty() {
            return;
        }
        let k = jobs.len();
        {
            let mut g = self.st.lock().unwrap();
            g.1 += k;
            g.0.append(jobs);
        }
        for _ in 0..k.min(32) {
            self.cv.notify_one();
        }
    }
    fn pop(&self) -> Option<Job> {
        let mut g = self.st.lock().unwrap();
        loop {
            if let Some(j) = g.0.pop() {
                return Some(j);
            }
            if g.1 == 0 {
                return None;
            }
            g = self.cv.wait(g).unwrap();
        }
    }
    fn done(&self) {
        let mut g = self.st.lock().unwrap();
        g.1 -= 1;
        if g.1 == 0 {
            drop(g);
            self.cv.notify_all();
        }
    }
}

pub struct BuildStats {
    pub dirs_listed: u64,
    pub errors: u64,
}

/// Полный параллельный обход диска.
pub fn build(drive: &str, stop: &AtomicBool) -> (Volume, BuildStats) {
    let vol = Mutex::new(Volume::new(drive, false));
    vol.lock().unwrap().reserve(1 << 20, 32 << 20);
    let windows_idx = AtomicU32::new(NONE);
    let q = WorkQ { st: Mutex::new((Vec::new(), 0)), cv: Condvar::new() };
    let root: Vec<u16> = format!(r"\\?\{drive}\").encode_utf16().collect();
    q.push(&mut vec![Job { idx: 0, path: root }]);
    let errors = AtomicU64::new(0);
    let listed = AtomicU64::new(0);
    let nthreads = std::env::var("SEMSEARCH_WALK_THREADS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or_else(|| std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4))
        .clamp(1, 64)
        .min(if std::env::var("SEMSEARCH_WALK_THREADS").is_ok() { 64 } else { 16 });
    std::thread::scope(|s| {
        for _ in 0..nthreads {
            s.spawn(|| {
                let mut items: Vec<DirItem> = Vec::new();
                let mut prepared: Vec<(String, Vec<u8>, u16, bool, usize)> = Vec::new();
                let mut sub: Vec<Job> = Vec::new();
                while let Some(job) = q.pop() {
                    if !stop.load(Relaxed) {
                        items.clear();
                        if list_dir(&job.path, &mut items).is_err() {
                            errors.fetch_add(1, Relaxed);
                        }
                        listed.fetch_add(1, Relaxed);
                        let win = windows_idx.load(Relaxed);
                        prepared.clear();
                        for (k, it) in items.iter().enumerate() {
                            let name = String::from_utf16_lossy(&it.name);
                            let lname = fold(&name);
                            if excluded_name(&lname) || (job.idx == win && lname == b"winsxs") {
                                continue;
                            }
                            prepared.push((name, lname, flags_from_attrs(it.attrs), descend(it.attrs, it.tag), k));
                        }
                        {
                            let mut v = vol.lock().unwrap();
                            for (name, lname, fl, desc, k) in prepared.iter() {
                                let i = v.add_nomap(job.idx, name, *fl);
                                if job.idx == 0 && lname == b"windows" && fl & F_DIR != 0 {
                                    windows_idx.store(i, Relaxed);
                                }
                                if *desc {
                                    let mut p = job.path.clone();
                                    if p.last() != Some(&(b'\\' as u16)) {
                                        p.push(b'\\' as u16);
                                    }
                                    p.extend_from_slice(&items[*k].name);
                                    sub.push(Job { idx: i, path: p });
                                }
                            }
                        }
                        q.push(&mut sub);
                    }
                    q.done();
                }
            });
        }
    });
    let mut v = vol.into_inner().unwrap();
    v.finish_build();
    (v, BuildStats { dirs_listed: listed.load(Relaxed), errors: errors.load(Relaxed) })
}

/// Обход поддерева при живых изменениях (новая/восстановленная папка): добавляет недостающее.
fn scan_subtree(v: &mut Volume, idx: u32, path: &str, out: &mut Vec<Event>) {
    let mut stack: Vec<(u32, String)> = vec![(idx, path.to_string())];
    let mut items = Vec::new();
    let mut seen = 0usize;
    while let Some((d, p)) = stack.pop() {
        items.clear();
        let w: Vec<u16> = long_w(&p).into_iter().take_while(|&c| c != 0).collect();
        if list_dir(&w, &mut items).is_err() {
            continue;
        }
        let is_windows = v.ents[d as usize].parent == 0 && v.lname(d) == b"windows";
        for it in items.drain(..) {
            let name = String::from_utf16_lossy(&it.name);
            let lname = fold(&name);
            if excluded_name(&lname) || (is_windows && lname == b"winsxs") {
                continue;
            }
            let fl = flags_from_attrs(it.attrs);
            let i = match v.find_child(d, &lname) {
                Some(i) => {
                    v.set_attr_flags(i, fl);
                    i
                }
                None => {
                    let i = v.add(d, &name, fl);
                    out.push(Event::new("created", v.path(i), None, fl & F_DIR != 0));
                    i
                }
            };
            seen += 1;
            if descend(it.attrs, it.tag) && seen < 5_000_000 {
                let cp = if p.ends_with('\\') { format!("{p}{name}") } else { format!("{p}\\{name}") };
                stack.push((i, cp));
            }
        }
    }
}

// ---------------------------------------------------------------- change notifications

pub struct RawEv {
    pub action: u32,
    pub rel: String,
    pub attrs: u32,
    pub tag: u32,
}

pub enum WatchMsg {
    Batch(Vec<RawEv>),
    Overflow,
}

#[inline]
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

fn parse_notify(b: &[u8], ext: bool, out: &mut Vec<RawEv>) {
    let mut off = 0usize;
    loop {
        let hdr = if ext { 84 } else { 12 };
        if off + hdr > b.len() {
            break;
        }
        let next = rd_u32(b, off) as usize;
        let action = rd_u32(b, off + 4);
        let (attrs, tag, nlen, noff) = if ext {
            let a = rd_u32(b, off + 56);
            let t = if a & A_REPARSE != 0 { rd_u32(b, off + 60) } else { 0 };
            (a, t, rd_u32(b, off + 80) as usize, off + 84)
        } else {
            (ATTR_UNKNOWN, 0, rd_u32(b, off + 8) as usize, off + 12)
        };
        if noff + nlen <= b.len() {
            let units: Vec<u16> = b[noff..noff + nlen].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            out.push(RawEv { action, rel: String::from_utf16_lossy(&units), attrs, tag });
        }
        if next == 0 {
            break;
        }
        off += next;
    }
}

/// Поток чтения ReadDirectoryChangesExW (рекурсивно от корня диска).
fn reader_loop(slot: Arc<VolSlot>, tx: Sender<WatchMsg>) {
    let root = wide_z(&format!("{}\\", slot.drive));
    let mut ext = true;
    let mut buf: Vec<u64> = vec![0u64; RDCW_BUF / 8];
    while !slot.stopped() {
        let h = unsafe {
            CreateFileW(
                root.as_ptr(),
                FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
                std::ptr::null_mut(),
            )
        };
        if h == INVALID_HANDLE_VALUE {
            warn!("volume {}: cannot open root for change notifications (err {})", slot.drive, unsafe { GetLastError() });
            unsafe { WaitForSingleObject(slot.stop_ev.0, 5000) };
            continue;
        }
        let h = Handle(h);
        let ev = new_event(true);
        info!("volume {}: watching changes (ReadDirectoryChanges{}W)", slot.drive, if ext { "Ex" } else { "" });
        loop {
            if slot.stopped() {
                return;
            }
            let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
            ov.hEvent = ev.0;
            unsafe { ResetEvent(ev.0) };
            let ok = unsafe {
                if ext {
                    ReadDirectoryChangesExW(h.0, buf.as_mut_ptr() as *mut _, RDCW_BUF as u32, 1, NOTIFY_FILTER, std::ptr::null_mut(), &mut ov, None, ReadDirectoryNotifyExtendedInformation)
                } else {
                    ReadDirectoryChangesW(h.0, buf.as_mut_ptr() as *mut _, RDCW_BUF as u32, 1, NOTIFY_FILTER, std::ptr::null_mut(), &mut ov, None)
                }
            };
            if ok == 0 {
                let e = unsafe { GetLastError() };
                if ext && (e == ERROR_INVALID_PARAMETER || e == ERROR_INVALID_FUNCTION || e == ERROR_NOT_SUPPORTED) {
                    warn!("volume {}: ReadDirectoryChangesExW unsupported (err {e}), using ReadDirectoryChangesW", slot.drive);
                    ext = false;
                    continue;
                }
                warn!("volume {}: ReadDirectoryChanges failed (err {e})", slot.drive);
                break;
            }
            let handles = [ev.0, slot.stop_ev.0];
            let w = unsafe { WaitForMultipleObjects(2, handles.as_ptr(), 0, INFINITE) };
            if w != WAIT_OBJECT_0 {
                unsafe {
                    CancelIoEx(h.0, &ov);
                    let mut n = 0u32;
                    GetOverlappedResult(h.0, &ov, &mut n, 1);
                }
                return;
            }
            let mut n = 0u32;
            if unsafe { GetOverlappedResult(h.0, &ov, &mut n, 0) } == 0 {
                let e = unsafe { GetLastError() };
                if e == ERROR_NOTIFY_ENUM_DIR {
                    let _ = tx.send(WatchMsg::Overflow);
                    continue;
                }
                warn!("volume {}: change notification error (err {e})", slot.drive);
                break;
            }
            if n == 0 {
                let _ = tx.send(WatchMsg::Overflow);
                continue;
            }
            let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, (n as usize).min(RDCW_BUF)) };
            let mut evs = Vec::new();
            parse_notify(bytes, ext, &mut evs);
            if tx.send(WatchMsg::Batch(evs)).is_err() {
                return;
            }
        }
        // ошибка — переоткрыть через паузу; события за это время потеряны → пересканировать
        let _ = tx.send(WatchMsg::Overflow);
        unsafe { WaitForSingleObject(slot.stop_ev.0, 2000) };
    }
}

// ---------------------------------------------------------------- applying changes

fn rel_excluded(comps: &[&str]) -> bool {
    comps.iter().any(|c| {
        let l = fold(c);
        excluded_name(&l)
    }) || (comps.len() >= 2 && comps[0].eq_ignore_ascii_case("windows") && comps[1].eq_ignore_ascii_case("winsxs"))
}

fn full_path(drive: &str, comps: &[&str]) -> String {
    let mut s = String::with_capacity(64);
    s.push_str(drive);
    for c in comps {
        s.push('\\');
        s.push_str(c);
    }
    if comps.is_empty() {
        s.push('\\');
    }
    s
}

/// Найти запись по относительному пути без «лечения».
fn resolve_rel(v: &Volume, comps: &[&str]) -> Option<u32> {
    let mut cur = 0u32;
    let mut lb = Vec::with_capacity(64);
    for c in comps {
        lb.clear();
        fold_into(c, &mut lb);
        cur = v.find_child(cur, &lb)?;
    }
    Some(cur)
}

/// Найти каталог; недостающие звенья проверить на диске и добавить (с содержимым).
fn resolve_dir_heal(v: &mut Volume, comps: &[&str], out: &mut Vec<Event>) -> Option<u32> {
    let mut cur = 0u32;
    let mut lb = Vec::with_capacity(64);
    for (k, c) in comps.iter().enumerate() {
        lb.clear();
        fold_into(c, &mut lb);
        if let Some(ch) = v.find_child(cur, &lb) {
            cur = ch;
            continue;
        }
        let p = full_path(&v.drive, &comps[..=k]);
        let (a, tag) = find_data(&p)?;
        if a & A_DIRECTORY == 0 {
            return None;
        }
        let i = v.add(cur, c, flags_from_attrs(a));
        out.push(Event::new("created", v.path(i), None, true));
        if descend(a, tag) {
            scan_subtree(v, i, &p, out);
        }
        cur = i;
    }
    Some(cur)
}

struct ApplyState {
    pending_old: Option<String>,
}

/// Результаты обработки пачки событий под блокировкой.
#[derive(Default)]
struct Ctx {
    out: Vec<Event>,
    /// новые каталоги, содержимое которых нужно дочитать с диска (вне блокировки)
    deferred: Vec<(u32, String)>,
    seen: HashSet<u32>,
}

fn handle_added(v: &mut Volume, comps: &[&str], attrs: u32, tag: u32, cx: &mut Ctx) {
    if comps.is_empty() || rel_excluded(comps) {
        return;
    }
    let (leaf, dirs) = comps.split_last().unwrap();
    let Some(p) = resolve_dir_heal(v, dirs, &mut cx.out) else { return };
    let lb = fold(leaf);
    if let Some(i) = v.find_child(p, &lb) {
        if attrs != ATTR_UNKNOWN {
            v.set_attr_flags(i, flags_from_attrs(attrs));
        }
        if v.name(i) != *leaf {
            v.rename(i, p, leaf); // смена регистра
        }
        return;
    }
    let path = full_path(&v.drive, comps);
    let (a, tag) = if attrs != ATTR_UNKNOWN {
        (attrs, tag)
    } else {
        match find_data(&path) {
            Some(x) => x,
            None => return, // уже удалён
        }
    };
    let fl = flags_from_attrs(a);
    let i = v.add(p, leaf, fl);
    cx.out.push(Event::new("created", v.path(i), None, fl & F_DIR != 0));
    if descend(a, tag) {
        cx.deferred.push((i, path));
    }
}

fn handle_removed(v: &mut Volume, comps: &[&str], cx: &mut Ctx) {
    if comps.is_empty() || rel_excluded(comps) {
        return;
    }
    let Some(i) = resolve_rel(v, comps) else { return };
    cx.out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
    v.remove_tree(i);
}

fn handle_modified(v: &mut Volume, comps: &[&str], attrs: u32, tag: u32, cx: &mut Ctx) {
    if comps.is_empty() || rel_excluded(comps) {
        return;
    }
    match resolve_rel(v, comps) {
        Some(i) => {
            if attrs != ATTR_UNKNOWN {
                v.set_attr_flags(i, flags_from_attrs(attrs));
            }
            if !v.is_dir(i) && cx.seen.insert(i) {
                cx.out.push(Event::new("modified", v.path(i), None, false));
            }
        }
        None => handle_added(v, comps, attrs, tag, cx),
    }
}

fn handle_renamed(v: &mut Volume, old: Option<&str>, new: &[&str], attrs: u32, tag: u32, cx: &mut Ctx) {
    let old_i = old.and_then(|o| {
        let oc: Vec<&str> = o.split('\\').filter(|c| !c.is_empty()).collect();
        if oc.is_empty() || rel_excluded(&oc) {
            None
        } else {
            resolve_rel(v, &oc)
        }
    });
    let Some(i) = old_i else {
        // источник неизвестен (например, восстановление из корзины) — как создание, с дочитыванием
        handle_added(v, new, attrs, tag, cx);
        return;
    };
    if new.is_empty() || rel_excluded(new) {
        // перенос в корзину и т. п. — для индекса это удаление
        cx.out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
        v.remove_tree(i);
        return;
    }
    let (leaf, dirs) = new.split_last().unwrap();
    let Some(p) = resolve_dir_heal(v, dirs, &mut cx.out) else {
        cx.out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
        v.remove_tree(i);
        return;
    };
    if v.is_ancestor_or_self(i, p) {
        return;
    }
    let old_path = v.path(i);
    if let Some(t) = v.find_child(p, &fold(leaf)) {
        if t != i {
            cx.out.push(Event::new("deleted", v.path(t), None, v.is_dir(t)));
            v.remove_tree(t);
        }
    }
    v.rename(i, p, leaf);
    if attrs != ATTR_UNKNOWN {
        v.set_attr_flags(i, flags_from_attrs(attrs));
    }
    cx.out.push(Event::new("renamed", v.path(i), Some(old_path), v.is_dir(i)));
}

/// Прочитать поддерево с диска без блокировки индекса: (временный индекс родителя | NONE, имя, атрибуты).
/// Порядок — родитель раньше детей.
fn collect_tree(path: &str, is_windows: bool) -> Vec<(u32, String, u32)> {
    let mut out: Vec<(u32, String, u32)> = Vec::new();
    let mut stack: Vec<(u32, String, bool)> = vec![(NONE, path.to_string(), is_windows)];
    let mut items = Vec::new();
    while let Some((tp, p, win)) = stack.pop() {
        items.clear();
        let w: Vec<u16> = long_w(&p).into_iter().take_while(|&c| c != 0).collect();
        if list_dir(&w, &mut items).is_err() {
            continue;
        }
        for it in items.drain(..) {
            let name = String::from_utf16_lossy(&it.name);
            let lname = fold(&name);
            if excluded_name(&lname) || (win && lname == b"winsxs") {
                continue;
            }
            let ti = out.len() as u32;
            if descend(it.attrs, it.tag) && out.len() < 5_000_000 {
                let cp = if p.ends_with('\\') { format!("{p}{name}") } else { format!("{p}\\{name}") };
                stack.push((ti, cp, false));
            }
            out.push((tp, name, it.attrs));
        }
    }
    out
}

/// Добавить прочитанное поддерево под root (существующие записи не дублируются).
fn apply_tree(v: &mut Volume, root: u32, tree: &[(u32, String, u32)], out: &mut Vec<Event>) {
    let mut map: Vec<u32> = Vec::with_capacity(tree.len());
    let mut lb = Vec::with_capacity(64);
    for (tp, name, attrs) in tree {
        let parent = if *tp == NONE { root } else { map[*tp as usize] };
        if parent == NONE || !v.live(parent) {
            map.push(NONE);
            continue;
        }
        let fl = flags_from_attrs(*attrs);
        lb.clear();
        fold_into(name, &mut lb);
        let i = match v.find_child(parent, &lb) {
            Some(i) => {
                v.set_attr_flags(i, fl);
                i
            }
            None => {
                let i = v.add(parent, name, fl);
                out.push(Event::new("created", v.path(i), None, fl & F_DIR != 0));
                i
            }
        };
        map.push(i);
    }
}

fn apply_batch(eng: &Engine, slot: &VolSlot, st: &mut ApplyState, evs: Vec<RawEv>) {
    let mut cx = Ctx::default();
    let t0 = Instant::now();
    let nev = evs.len();
    {
        let mut v = slot.write();
        for e in evs {
            let comps: Vec<&str> = e.rel.split('\\').filter(|c| !c.is_empty()).collect();
            match e.action {
                FILE_ACTION_ADDED => handle_added(&mut v, &comps, e.attrs, e.tag, &mut cx),
                FILE_ACTION_REMOVED => handle_removed(&mut v, &comps, &mut cx),
                FILE_ACTION_MODIFIED => handle_modified(&mut v, &comps, e.attrs, e.tag, &mut cx),
                FILE_ACTION_RENAMED_OLD_NAME => {
                    st.pending_old = Some(e.rel.clone());
                    continue;
                }
                FILE_ACTION_RENAMED_NEW_NAME => {
                    let old = st.pending_old.take();
                    handle_renamed(&mut v, old.as_deref(), &comps, e.attrs, e.tag, &mut cx);
                }
                _ => {}
            }
        }
        if v.compact_if_needed() {
            info!("volume {}: name buffers compacted", v.drive);
        }
    }
    let mut out = std::mem::take(&mut cx.out);
    // содержимое новых/восстановленных каталогов: чтение диска без блокировки, затем вставка
    let win_path = format!("{}\\Windows", slot.drive);
    for (idx, path) in std::mem::take(&mut cx.deferred) {
        let tree = collect_tree(&path, path.eq_ignore_ascii_case(&win_path));
        if tree.is_empty() {
            continue;
        }
        let mut v = slot.write();
        if v.live(idx) && v.path(idx) == path {
            apply_tree(&mut v, idx, &tree, &mut out);
        }
    }
    if nev > 2000 {
        info!("volume {}: applied {nev} changes in {} ms ({} events out); {}", slot.drive, t0.elapsed().as_millis(), out.len(), mem_str());
    }
    eng.touch();
    eng.emit(out);
}

/// Полная (пере)сборка тома обходом.
pub fn full_build(eng: &Engine, slot: &VolSlot, rescan: bool) {
    eng.bg_begin();
    let t0 = Instant::now();
    info!("volume {}: walk build starting; {}", slot.drive, mem_str());
    let (v, st) = build(&slot.drive, &slot.stop);
    eng.bg_end();
    info!("volume {}: walk build done; {}", slot.drive, mem_str());
    let ms = t0.elapsed().as_millis() as u64;
    if slot.stopped() {
        return;
    }
    let (files, dirs, mem) = (v.files, v.dirs, v.mem_bytes());
    slot.install(v);
    slot.set_info(|i| {
        i.mode = "walk".into();
        i.ready = true;
        i.build_ms = ms;
        if rescan {
            i.rescans += 1;
        }
    });
    eng.touch();
    info!(
        "volume {}: walk index built in {ms} ms: {files} files, {dirs} dirs, {} dirs listed, {} unreadable, index {} MB; {}",
        slot.drive,
        st.dirs_listed,
        st.errors,
        mem >> 20,
        mem_str()
    );
    if rescan {
        eng.emit(vec![Event::new("rescan", format!("{}\\", slot.drive), None, true)]);
    }
}

/// Поток тома в режиме обхода: наблюдение запускается до обхода, чтобы не потерять изменения.
pub fn run(eng: &Arc<Engine>, slot: &Arc<VolSlot>) {
    let (tx, rx): (Sender<WatchMsg>, Receiver<WatchMsg>) = channel();
    let s2 = slot.clone();
    let _ = std::thread::Builder::new().name(format!("rdcw-{}", &slot.drive[..1])).spawn(move || reader_loop(s2, tx));
    slot.set_info(|i| i.mode = "walk".into());
    full_build(eng, slot, false);
    let mut st = ApplyState { pending_old: None };
    let mut last_rescan: Option<Instant> = None;
    while !slot.stopped() {
        let first = match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(m) => m,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => {
                if !slot.stopped() {
                    warn!("volume {}: change reader stopped", slot.drive);
                }
                return;
            }
        };
        let mut msgs = vec![first];
        while let Ok(m) = rx.try_recv() {
            msgs.push(m);
        }
        if let Some(pos) = msgs.iter().rposition(|m| matches!(m, WatchMsg::Overflow)) {
            // не чаще раза в 30 с: копим изменения, потом один полный обход
            if let Some(t) = last_rescan {
                let el = t.elapsed();
                if el < Duration::from_secs(30) {
                    std::thread::sleep(Duration::from_secs(30) - el);
                }
            }
            if slot.stopped() {
                return;
            }
            info!("volume {}: change buffer overflow → full rescan", slot.drive);
            msgs.drain(..=pos);
            while let Ok(m) = rx.try_recv() {
                msgs.push(m);
            }
            if let Some(p2) = msgs.iter().rposition(|m| matches!(m, WatchMsg::Overflow)) {
                msgs.drain(..=p2);
            }
            full_build(eng, slot, true);
            last_rescan = Some(Instant::now());
            st.pending_old = None;
        }
        let mut evs = Vec::new();
        for m in msgs {
            if let WatchMsg::Batch(b) = m {
                evs.extend(b);
            }
        }
        if !evs.is_empty() {
            apply_batch(eng, slot, &mut st, evs);
        }
    }
}

//! Режим MFT (нужны права администратора): перечисление MFT через FSCTL_ENUM_USN_DATA,
//! затем слежение за USN-журналом (FSCTL_READ_USN_JOURNAL, блокирующее ожидание).
//! Журнал не создаётся: если его нет — возврат Err и переход в режим обхода.
//! Том открывается только на чтение (GENERIC_READ); ничего не меняется.

use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering::*};
use std::sync::Arc;
use std::time::Instant;

use rayon::prelude::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Ioctl::*;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::System::IO::*;

use crate::engine::*;
use crate::index::*;
use crate::util::*;

const ENUM_BUF: usize = 4 << 20;
const READ_BUF: usize = 512 << 10;
const DATA_CHANGE: u32 = USN_REASON_DATA_OVERWRITE | USN_REASON_DATA_EXTEND | USN_REASON_DATA_TRUNCATION;
const META_NAMES: [&[u8]; 11] = [
    b"$mft", b"$mftmirr", b"$logfile", b"$volume", b"$attrdef", b"$bitmap", b"$boot", b"$badclus", b"$secure", b"$upcase", b"$extend",
];

#[derive(Clone, Copy, Debug)]
pub struct Journal {
    pub id: u64,
    pub first: i64,
    pub next: i64,
    pub max_size: u64,
}

// ---------------------------------------------------------------- low level

pub fn open_volume(drive: &str) -> Result<Handle, String> {
    let w = wide_z(&format!(r"\\.\{drive}"));
    let h = unsafe {
        CreateFileW(
            w.as_ptr(),
            GENERIC_READ,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        )
    };
    if h == INVALID_HANDLE_VALUE {
        return Err(format!("cannot open volume {drive} for reading (err {})", unsafe { GetLastError() }));
    }
    Ok(Handle(h))
}

/// DeviceIoControl на перекрытом дескрипторе; stop_ev прерывает ожидание (CancelIoEx).
fn ioctl(h: HANDLE, code: u32, inp: *const std::ffi::c_void, inlen: usize, out: &mut [u64], stop_ev: Option<HANDLE>) -> Result<usize, u32> {
    let ev = new_event(true);
    let mut ov: OVERLAPPED = unsafe { std::mem::zeroed() };
    ov.hEvent = ev.0;
    let outlen = (out.len() * 8) as u32;
    let ok = unsafe { DeviceIoControl(h, code, inp, inlen as u32, out.as_mut_ptr() as *mut _, outlen, std::ptr::null_mut(), &mut ov) };
    if ok == 0 {
        let e = unsafe { GetLastError() };
        if e != ERROR_IO_PENDING {
            return Err(e);
        }
    }
    if let Some(stop) = stop_ev {
        let hs = [ev.0, stop];
        let w = unsafe { WaitForMultipleObjects(2, hs.as_ptr(), 0, INFINITE) };
        if w != WAIT_OBJECT_0 {
            unsafe {
                CancelIoEx(h, &ov);
                let mut n = 0u32;
                GetOverlappedResult(h, &ov, &mut n, 1);
            }
            return Err(ERROR_OPERATION_ABORTED);
        }
    }
    let mut n = 0u32;
    if unsafe { GetOverlappedResult(h, &ov, &mut n, 1) } == 0 {
        return Err(unsafe { GetLastError() });
    }
    Ok(n as usize)
}

pub fn query_journal(h: HANDLE) -> Result<Journal, u32> {
    let mut out = vec![0u64; 16];
    let n = ioctl(h, FSCTL_QUERY_USN_JOURNAL, std::ptr::null(), 0, &mut out, None)?;
    if n < std::mem::size_of::<USN_JOURNAL_DATA_V0>() {
        return Err(ERROR_INVALID_DATA);
    }
    let jd = unsafe { &*(out.as_ptr() as *const USN_JOURNAL_DATA_V0) };
    Ok(Journal { id: jd.UsnJournalID, first: jd.FirstUsn, next: jd.NextUsn, max_size: jd.MaximumSize })
}

/// FRN корневого каталога тома.
pub fn root_frn(drive: &str) -> Result<u64, String> {
    let w = wide_z(&format!("{drive}\\"));
    unsafe {
        let h = CreateFileW(
            w.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return Err(format!("cannot open {drive}\\ (err {})", GetLastError()));
        }
        let h = Handle(h);
        let mut bi: BY_HANDLE_FILE_INFORMATION = std::mem::zeroed();
        if GetFileInformationByHandle(h.0, &mut bi) == 0 {
            return Err(format!("GetFileInformationByHandle({drive}\\) failed (err {})", GetLastError()));
        }
        Ok(((bi.nFileIndexHigh as u64) << 32) | bi.nFileIndexLow as u64)
    }
}

#[inline]
fn rd_u16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
#[inline]
fn rd_u32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
#[inline]
fn rd_u64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

pub struct RawRec<'a> {
    pub frn: u64,
    pub parent: u64,
    #[allow(dead_code)]
    pub usn: i64,
    pub reason: u32,
    pub attrs: u32,
    pub name: &'a [u8], // UTF-16LE
}

/// Разбор USN_RECORD_V2 / V3 (у V3 берутся младшие 64 бита FILE_ID_128 — для NTFS это FRN).
pub fn for_each_record<'a>(b: &'a [u8], mut f: impl FnMut(RawRec<'a>)) {
    let mut off = 0usize;
    while off + 8 <= b.len() {
        let len = rd_u32(b, off) as usize;
        if len < 8 || off + len > b.len() {
            break;
        }
        let major = rd_u16(b, off + 4);
        let fields = match major {
            2 if len >= 60 => Some((rd_u64(b, off + 8), rd_u64(b, off + 16), rd_u64(b, off + 24) as i64, rd_u32(b, off + 40), rd_u32(b, off + 52), rd_u16(b, off + 56), rd_u16(b, off + 58))),
            3 if len >= 76 => Some((rd_u64(b, off + 8), rd_u64(b, off + 24), rd_u64(b, off + 40) as i64, rd_u32(b, off + 56), rd_u32(b, off + 68), rd_u16(b, off + 72), rd_u16(b, off + 74))),
            _ => None,
        };
        if let Some((frn, parent, usn, reason, attrs, nlen, noff)) = fields {
            let s = off + noff as usize;
            let e = s + nlen as usize;
            if e <= off + len {
                f(RawRec { frn, parent, usn, reason, attrs, name: &b[s..e] });
            }
        }
        off += len;
    }
}

fn utf16le_to_string(raw: &[u8]) -> String {
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

// ---------------------------------------------------------------- enumeration

pub struct EnumStats {
    pub records: u64,
    pub orphans: u64,
    pub excluded: u64,
    pub ms: u64,
    /// время фаз: ioctl-перечисление, привязка родителей, скрытие исключённых + хеш-таблица
    pub enum_ms: u64,
    pub resolve_ms: u64,
    pub finish_ms: u64,
    pub threads: usize,
    #[allow(dead_code)]
    pub calls: u64,
}

#[derive(Clone, Copy)]
struct RawH(HANDLE);
unsafe impl Send for RawH {}
unsafe impl Sync for RawH {}

/// Общий построитель для потоков перечисления (блокировка берётся раз на буфер ~4 МБ).
struct EnumShared {
    v: Volume,
    parents: Vec<u64>,
    records: u64,
    calls: u64,
}

/// Число записей MFT (MftValidDataLength / BytesPerFileRecordSegment).
fn mft_record_count(h: HANDLE) -> Option<u64> {
    let mut out = vec![0u64; 32];
    let n = ioctl(h, FSCTL_GET_NTFS_VOLUME_DATA, std::ptr::null(), 0, &mut out, None).ok()?;
    if n < std::mem::size_of::<NTFS_VOLUME_DATA_BUFFER>() {
        return None;
    }
    let d = unsafe { &*(out.as_ptr() as *const NTFS_VOLUME_DATA_BUFFER) };
    if d.BytesPerFileRecordSegment == 0 || d.MftValidDataLength <= 0 {
        return None;
    }
    Some(d.MftValidDataLength as u64 / d.BytesPerFileRecordSegment as u64)
}

/// Перечислить записи MFT с номерами [start, end) (end = u64::MAX — до конца).
/// Состояние перечисления целиком во входной структуре, поэтому диапазоны независимы.
fn enum_range(h: RawH, start: u64, end: u64, root_rec: usize, shared: &std::sync::Mutex<EnumShared>, stop: &AtomicBool, use_v1: &AtomicBool) -> Result<(), String> {
    let mut buf: Vec<u64> = vec![0u64; ENUM_BUF / 8];
    let mut next = start;
    loop {
        if stop.load(Relaxed) {
            return Err("stopped".into());
        }
        let v1 = use_v1.load(Relaxed);
        // V0 — префикс V1: одна структура, разный размер входа
        let med = MFT_ENUM_DATA_V1 { StartFileReferenceNumber: next, LowUsn: 0, HighUsn: i64::MAX, MinMajorVersion: 2, MaxMajorVersion: 3 };
        let size = if v1 { std::mem::size_of::<MFT_ENUM_DATA_V1>() } else { std::mem::size_of::<MFT_ENUM_DATA_V0>() };
        let n = match ioctl(h.0, FSCTL_ENUM_USN_DATA, &med as *const _ as *const _, size, &mut buf, None) {
            Ok(n) => n,
            Err(e) if e == ERROR_HANDLE_EOF => return Ok(()),
            Err(e) if e == ERROR_INVALID_PARAMETER && !v1 => {
                if !use_v1.swap(true, SeqCst) {
                    warn!("FSCTL_ENUM_USN_DATA rejected MFT_ENUM_DATA_V0, using V1");
                }
                continue;
            }
            Err(e) => return Err(format!("FSCTL_ENUM_USN_DATA failed (err {e})")),
        };
        if n <= 8 {
            return Ok(());
        }
        let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, n.min(ENUM_BUF)) };
        let nx = rd_u64(bytes, 0);
        let mut past_end = false;
        {
            let mut g = shared.lock().unwrap_or_else(|e| e.into_inner());
            let s = &mut *g;
            s.calls += 1;
            for_each_record(&bytes[8..], |r| {
                let rec = rec_of(r.frn);
                if rec as u64 >= end {
                    past_end = true;
                    return;
                }
                if (rec as u64) < start {
                    return; // диапазоны строго не пересекаются (иначе — дубликаты записей)
                }
                s.records += 1;
                if rec == root_rec {
                    return;
                }
                let (off, len) = s.v.push_name_u16le(r.name);
                let i = s.v.ents.len() as u32;
                s.v.ents.push(Entry { parent: NONE, off, len, flags: flags_from_attrs(r.attrs) });
                s.v.nchild.push(0);
                s.v.ent_rec.push(NONE);
                s.v.set_frn(i, r.frn);
                s.parents.push(r.parent);
            });
        }
        if past_end || nx >= end || nx <= next {
            return Ok(());
        }
        next = nx;
    }
}

/// Полное перечисление MFT в новый индекс. Большой том делится на диапазоны номеров
/// записей, которые читаются параллельно (FSCTL_ENUM_USN_DATA на одном перекрытом
/// дескрипторе); при любой ошибке или подозрительном числе «сирот» — повтор одним потоком.
pub fn enumerate(h: HANDLE, drive: &str, j: &Journal, stop: &AtomicBool, media: Media) -> Result<(Volume, EnumStats), String> {
    let total = mft_record_count(h).unwrap_or(0);
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    // на HDD несколько читателей MFT мешают друг другу (головки прыгают между диапазонами): один поток
    let par = match media {
        Media::Hdd => 1,
        Media::Unknown => cpus.clamp(1, 4),
        Media::Ssd => cpus.clamp(1, 8),
    };
    let threads = if total >= 400_000 { par } else { 1 };
    match enumerate_with(h, drive, j, stop, total, threads) {
        Ok((v, st)) if threads > 1 && st.orphans > 1000 + st.records / 1000 => {
            warn!("volume {drive}: parallel MFT enumeration gave {} orphans of {} records → sequential re-run", st.orphans, st.records);
            drop(v);
            enumerate_with(h, drive, j, stop, total, 1)
        }
        Err(e) if threads > 1 && !stop.load(Relaxed) => {
            warn!("volume {drive}: parallel MFT enumeration failed ({e}) → sequential re-run");
            enumerate_with(h, drive, j, stop, total, 1)
        }
        r => r,
    }
}

fn enumerate_with(h: HANDLE, drive: &str, j: &Journal, stop: &AtomicBool, total: u64, threads: usize) -> Result<(Volume, EnumStats), String> {
    // HighUsn = максимум: файлы, изменённые во время перечисления, тоже попадают в индекс
    // (иначе переименованный в этот момент каталог «осиротил» бы своих детей);
    // их записи журнала после j.next затем применяются повторно — это идемпотентно.
    let _ = j;
    let t0 = Instant::now();
    info!("volume {drive}: MFT enumeration start: ~{total} MFT records, {threads} thread(s); {}", mem_str());
    let rootfrn = root_frn(drive)?;
    let root_rec = rec_of(rootfrn);
    let mut v = Volume::new(drive, true);
    v.set_frn(0, rootfrn);
    let cap = (total as usize).clamp(1 << 20, 1 << 26);
    v.reserve(cap, cap * 24);
    let mut parents: Vec<u64> = Vec::with_capacity(cap);
    parents.push(0);
    let shared = std::sync::Mutex::new(EnumShared { v, parents, records: 0, calls: 0 });
    let use_v1 = AtomicBool::new(false);
    let ranges: Vec<(u64, u64)> = if threads <= 1 || total == 0 {
        vec![(0, u64::MAX)]
    } else {
        let step = total.div_ceil(threads as u64).max(1);
        (0..threads as u64).map(|k| (k * step, if k + 1 == threads as u64 { u64::MAX } else { (k + 1) * step })).collect()
    };
    let hh = RawH(h);
    let results: Vec<Result<(), String>> = std::thread::scope(|s| {
        let hs: Vec<_> = ranges
            .iter()
            .map(|&(a, b)| {
                let (sh, st, v1) = (&shared, stop, &use_v1);
                s.spawn(move || enum_range(hh, a, b, root_rec, sh, st, v1))
            })
            .collect();
        hs.into_iter().map(|t| t.join().unwrap_or_else(|_| Err("enumeration thread panicked".into()))).collect()
    });
    let EnumShared { mut v, parents, records, calls } = shared.into_inner().unwrap_or_else(|e| e.into_inner());
    for r in results {
        r?;
    }
    let enum_ms = t0.elapsed().as_millis() as u64;
    info!("volume {drive}: MFT enumeration ioctl phase {enum_ms} ms: {records} records in {calls} calls; {}", mem_str());
    // родители
    let t1 = Instant::now();
    let mut orphans = 0u64;
    for i in 1..v.ents.len() {
        match v.lookup_frn(parents[i]) {
            Some(p) if p as usize != i => v.ents[i].parent = p,
            _ => {
                v.ents[i].parent = 0;
                v.ents[i].flags |= F_EXCLUDED;
                orphans += 1;
            }
        }
    }
    drop(parents);
    let resolve_ms = t1.elapsed().as_millis() as u64;
    let t2 = Instant::now();
    let excluded = mark_excluded(&mut v);
    v.finish_build();
    let finish_ms = t2.elapsed().as_millis() as u64;
    let ms = t0.elapsed().as_millis() as u64;
    info!("volume {drive}: MFT phases: enumerate {enum_ms} ms, parents {resolve_ms} ms, exclusions + child map {finish_ms} ms; {}", mem_str());
    Ok((v, EnumStats { records, orphans, excluded, ms, enum_ms, resolve_ms, finish_ms, threads, calls }))
}

/// Скрыть $Recycle.Bin, System Volume Information, метафайлы NTFS, Windows\WinSxS (с поддеревьями).
fn mark_excluded(v: &mut Volume) -> u64 {
    let n = v.ents.len();
    let mut roots: Vec<u32> = Vec::new();
    let mut windows = NONE;
    for i in 1..n as u32 {
        let e = v.ents[i as usize];
        if e.flags & F_EXCLUDED != 0 {
            roots.push(i);
            continue;
        }
        if e.parent != 0 {
            continue;
        }
        let l = v.lname(i);
        if crate::walk::excluded_name(l) || META_NAMES.contains(&l) {
            roots.push(i);
        } else if l == b"windows" && e.flags & F_DIR != 0 {
            windows = i;
        }
    }
    if windows != NONE {
        for i in 1..n as u32 {
            if v.ents[i as usize].parent == windows && v.lname(i) == b"winsxs" {
                roots.push(i);
            }
        }
    }
    let mask = v.descendants_mask(&roots);
    let mut cnt = 0;
    for i in 1..n {
        if mask[i] == 2 {
            v.ents[i].flags |= F_EXCLUDED;
            cnt += 1;
        }
    }
    cnt
}

/// Жёсткие ссылки: FSCTL_ENUM_USN_DATA отдаёт одно имя файла. Для файлов, чьё имя попало в
/// скрытую WinSxS, добавляем остальные имена (System32 и т. п.) через FindFirstFileNameW.
pub fn link_pass(slot: &VolSlot) -> u64 {
    let t0 = Instant::now();
    let cands: Vec<(u32, String)> = {
        let v = slot.read();
        let winsxs = v
            .find_child_name(0, "Windows")
            .and_then(|w| v.find_child_name(w, "WinSxS"));
        let Some(wsx) = winsxs else { return 0 };
        let mut memo = vec![0u8; v.ents.len()];
        memo[wsx as usize] = 2;
        let mut out = Vec::new();
        let mut stack = Vec::new();
        for i in 1..v.ents.len() as u32 {
            let e = v.ents[i as usize];
            if e.flags & (F_DELETED | F_DIR) != 0 || e.flags & F_EXCLUDED == 0 {
                continue;
            }
            // под WinSxS?
            let mut cur = e.parent;
            let val;
            loop {
                if cur == NONE || stack.len() > 4096 {
                    val = 1;
                    break;
                }
                let m = memo[cur as usize];
                if m != 0 {
                    val = m;
                    break;
                }
                stack.push(cur);
                cur = v.ents[cur as usize].parent;
            }
            for x in stack.drain(..) {
                memo[x as usize] = val;
            }
            if val == 2 {
                out.push((i, v.path(i)));
            }
        }
        out
    };
    let drive = slot.drive.clone();
    let collect = || -> Vec<(u32, String)> {
        cands
            .par_iter()
            .flat_map_iter(|(i, p)| {
                // том извлекают или останавливают: не дочитывать остаток
                let links = if slot.stopped() { Vec::new() } else { hard_links(&drive, p) };
                links.into_iter().filter(|l| !l.eq_ignore_ascii_case(p)).map(move |l| (*i, l))
            })
            .collect()
    };
    // имена по номерам MFT читаются вразброс: на HDD всеми ядрами это только лишние перемещения головок
    let found = match slot.media() {
        Media::Hdd => match rayon::ThreadPoolBuilder::new().num_threads(2).build() {
            Ok(pool) => pool.install(collect),
            Err(_) => collect(),
        },
        _ => collect(),
    };
    let mut added = 0u64;
    {
        let mut v = slot.write();
        for (i, lp) in found {
            if !v.live(i) {
                continue;
            }
            let Some(pos) = lp.rfind('\\') else { continue };
            let (dir, name) = (&lp[..pos], &lp[pos + 1..]);
            let Some(p) = v.resolve(dir) else { continue };
            if !v.visible(p) && p != 0 {
                continue;
            }
            if v.find_child_name(p, name).is_some() {
                continue;
            }
            let fl = (v.ents[i as usize].flags & !(F_EXCLUDED | F_DELETED | F_ROOT)) | F_LINK;
            v.add(p, name, fl);
            added += 1;
        }
    }
    info!(
        "volume {}: hard-link pass: {} WinSxS files checked, {added} extra names added in {} ms; {}",
        slot.drive,
        cands.len(),
        t0.elapsed().as_millis(),
        mem_str()
    );
    added
}

/// Все имена файла (жёсткие ссылки) в виде "C:\…".
fn hard_links(drive: &str, path: &str) -> Vec<String> {
    let w = long_w(path);
    let mut out = Vec::new();
    unsafe {
        let mut buf: Vec<u16> = vec![0; 1024];
        let mut len = buf.len() as u32;
        let h = FindFirstFileNameW(w.as_ptr(), 0, &mut len, buf.as_mut_ptr());
        if h == INVALID_HANDLE_VALUE {
            return out;
        }
        loop {
            let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
            let s = String::from_utf16_lossy(&buf[..n]);
            if s.starts_with('\\') {
                out.push(format!("{drive}{s}"));
            }
            len = buf.len() as u32;
            if FindNextFileNameW(h, &mut len, buf.as_mut_ptr()) == 0 {
                if GetLastError() == ERROR_MORE_DATA && (len as usize) > buf.len() && len < 65536 {
                    buf.resize(len as usize + 1, 0);
                    len = buf.len() as u32;
                    if FindNextFileNameW(h, &mut len, buf.as_mut_ptr()) == 0 {
                        break;
                    }
                    continue;
                }
                break;
            }
        }
        FindClose(h);
    }
    out
}

// ---------------------------------------------------------------- applying journal records

pub struct Rec {
    pub frn: u64,
    pub parent: u64,
    pub reason: u32,
    pub attrs: u32,
    pub name: String,
}

/// Проверить на диске имя (родитель, имя) после изменения жёсткой ссылки.
fn reconcile_link(v: &mut Volume, p: u32, name: &str, attrs: u32, out: &mut Vec<Event>) {
    let path = v.child_path(p, name);
    let on_disk = exists(&path);
    match (on_disk, v.find_child_name(p, name)) {
        (true, None) => {
            let fl = flags_from_attrs(attrs) | F_LINK | (v.ents[p as usize].flags & F_EXCLUDED);
            let i = v.add(p, name, fl);
            if v.visible(i) {
                out.push(Event::new("created", v.path(i), None, v.is_dir(i)));
            }
        }
        (false, Some(i)) => {
            if v.visible(i) {
                out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
            }
            v.remove_tree(i);
        }
        _ => {}
    }
}

pub fn apply_records(v: &mut Volume, recs: &[Rec], out: &mut Vec<Event>) {
    let mut modified: HashSet<u32> = HashSet::new();
    for r in recs {
        let reason = r.reason;
        let mut idx = v.lookup_frn(r.frn);
        if let (Some(i), true) = (idx, reason & USN_REASON_FILE_CREATE != 0) {
            // номер записи MFT переиспользован, а удаление старого файла мы не видели
            let same = v.lookup_frn(r.parent) == Some(v.ents[i as usize].parent) && v.name(i) == r.name;
            if !same {
                if v.visible(i) {
                    out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
                }
                v.remove_tree(i);
                idx = None;
            }
        }
        if reason & USN_REASON_FILE_DELETE != 0 {
            if let Some(i) = idx {
                if v.visible(i) {
                    out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
                }
                v.remove_tree(i);
            }
            continue;
        }
        if reason & USN_REASON_RENAME_OLD_NAME != 0 && reason & USN_REASON_RENAME_NEW_NAME == 0 {
            continue; // старое имя; ждём запись с новым
        }
        let p = v.lookup_frn(r.parent);
        if reason & USN_REASON_HARD_LINK_CHANGE != 0 {
            if let Some(p) = p {
                reconcile_link(v, p, &r.name, r.attrs, out);
            }
            continue;
        }
        match idx {
            None => {
                let Some(p) = p else { continue };
                let fl = flags_from_attrs(r.attrs) | (v.ents[p as usize].flags & F_EXCLUDED);
                let i = v.add(p, &r.name, fl);
                v.set_frn(i, r.frn);
                if v.visible(i) {
                    out.push(Event::new("created", v.path(i), None, v.is_dir(i)));
                }
            }
            Some(i) => {
                if reason & USN_REASON_RENAME_NEW_NAME != 0 {
                    match p {
                        None => {
                            // перемещён в неизвестное место — убрать
                            if v.visible(i) {
                                out.push(Event::new("deleted", v.path(i), None, v.is_dir(i)));
                            }
                            v.remove_tree(i);
                            continue;
                        }
                        Some(p) => {
                            if (v.ents[i as usize].parent != p || v.name(i) != r.name) && !v.is_ancestor_or_self(i, p) {
                                let old_vis = v.visible(i);
                                let old_path = v.path(i);
                                v.rename(i, p, &r.name);
                                let pex = v.ents[p as usize].flags & F_EXCLUDED != 0;
                                let was_ex = v.ents[i as usize].flags & F_EXCLUDED != 0;
                                let mut uncovered = Vec::new();
                                if pex != was_ex {
                                    let changed = v.set_excluded_tree(i, pex);
                                    if !pex {
                                        uncovered = changed;
                                    }
                                }
                                let new_vis = v.visible(i);
                                match (old_vis, new_vis) {
                                    (true, true) => out.push(Event::new("renamed", v.path(i), Some(old_path), v.is_dir(i))),
                                    (true, false) => out.push(Event::new("deleted", old_path, None, v.is_dir(i))),
                                    (false, true) => {
                                        for c in uncovered {
                                            if v.visible(c) {
                                                out.push(Event::new("created", v.path(c), None, v.is_dir(c)));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
                v.set_attr_flags(i, flags_from_attrs(r.attrs));
            }
        }
        if reason & USN_REASON_CLOSE != 0 && reason & DATA_CHANGE != 0 {
            if let Some(i) = v.lookup_frn(r.frn) {
                if !v.is_dir(i) && v.visible(i) && modified.insert(i) {
                    out.push(Event::new("modified", v.path(i), None, false));
                }
            }
        }
    }
}

// ---------------------------------------------------------------- driver

/// Перечислить MFT и поставить индекс в слот.
fn full_enum(eng: &Engine, slot: &VolSlot, h: HANDLE, rescan: bool) -> Result<Journal, String> {
    let j = match query_journal(h) {
        Ok(j) => j,
        Err(e) if e == ERROR_JOURNAL_NOT_ACTIVE || e == ERROR_JOURNAL_DELETE_IN_PROGRESS => {
            return Err(format!("USN journal is not active on {} (err {e}); not creating one", slot.drive));
        }
        Err(e) => return Err(format!("FSCTL_QUERY_USN_JOURNAL failed (err {e})")),
    };
    eng.bg_begin();
    let r = enumerate(h, &slot.drive, &j, &slot.stop, slot.media());
    eng.bg_end();
    let (v, st) = r?;
    let (files, dirs, mem) = (v.files, v.dirs, v.mem_bytes());
    slot.install(v);
    slot.set_info(|i| {
        i.mode = "mft".into();
        i.build_ms = st.ms;
        i.journal_id = j.id;
        i.records = st.records;
        i.enum_ms = st.enum_ms;
        i.parents_ms = st.resolve_ms;
        i.finish_ms = st.finish_ms;
        i.enum_threads = st.threads as u64;
        i.orphans = st.orphans;
        i.hidden = st.excluded;
        i.note.clear();
        if rescan {
            i.rescans += 1;
        }
    });
    info!(
        "volume {}: MFT index built in {} ms: {} records, {files} files, {dirs} dirs, {} hidden, {} orphans, index {} MB; journal {:#x} next usn {} max size {} MB",
        slot.drive,
        st.ms,
        st.records,
        st.excluded,
        st.orphans,
        mem >> 20,
        j.id,
        j.next,
        j.max_size >> 20
    );
    let tl = Instant::now();
    let links = link_pass(slot);
    let links_ms = tl.elapsed().as_millis() as u64;
    slot.set_info(|i| i.links_ms = links_ms);
    slot.set_info(|i| {
        i.links = links;
        i.ready = true;
    });
    eng.touch();
    if rescan {
        eng.emit(vec![Event::new("rescan", format!("{}\\", slot.drive), None, true)]);
    }
    Ok(j)
}

/// Поток тома в режиме MFT. Ok(()) — остановлен; Err(причина) — перейти в режим обхода.
pub fn run(eng: &Arc<Engine>, slot: &Arc<VolSlot>) -> Result<(), String> {
    let vh = open_volume(&slot.drive)?;
    let h = vh.0;
    let mut j = full_enum(eng, slot, h, false)?;
    let mut next = j.next;
    let mut buf: Vec<u64> = vec![0u64; READ_BUF / 8];
    let mut errors = 0u32;
    let mut use_v1 = false;
    loop {
        if slot.stopped() {
            return Ok(());
        }
        // V0 (записи USN_RECORD_V2); если система отвергнет — V1 с версиями 2..3
        let rd = READ_USN_JOURNAL_DATA_V1 {
            StartUsn: next,
            ReasonMask: 0xFFFF_FFFF,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 1,
            UsnJournalID: j.id,
            MinMajorVersion: 2,
            MaxMajorVersion: 3,
        };
        let size = if use_v1 { std::mem::size_of::<READ_USN_JOURNAL_DATA_V1>() } else { std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() };
        match ioctl(h, FSCTL_READ_USN_JOURNAL, &rd as *const _ as *const _, size, &mut buf, Some(slot.stop_ev.0)) {
            Err(e) if e == ERROR_INVALID_PARAMETER && !use_v1 => {
                warn!("volume {}: FSCTL_READ_USN_JOURNAL rejected READ_USN_JOURNAL_DATA_V0, retrying with V1", slot.drive);
                use_v1 = true;
            }
            Ok(n) if n >= 8 => {
                errors = 0;
                let bytes = unsafe { std::slice::from_raw_parts(buf.as_ptr() as *const u8, n.min(READ_BUF)) };
                next = rd_u64(bytes, 0) as i64;
                let mut recs = Vec::new();
                for_each_record(&bytes[8..], |r| {
                    recs.push(Rec { frn: r.frn, parent: r.parent, reason: r.reason, attrs: r.attrs, name: utf16le_to_string(r.name) });
                });
                if recs.is_empty() {
                    // защита от холостого цикла, если вызов вернулся без записей
                    unsafe { WaitForSingleObject(slot.stop_ev.0, 50) };
                    continue;
                }
                let mut out = Vec::new();
                {
                    let mut v = slot.write();
                    apply_records(&mut v, &recs, &mut out);
                    v.compact_if_needed();
                }
                eng.touch();
                eng.emit(out);
            }
            Ok(_) => unsafe {
                WaitForSingleObject(slot.stop_ev.0, 50);
            },
            Err(e) if e == ERROR_OPERATION_ABORTED && slot.stopped() => return Ok(()),
            Err(e) if e == ERROR_JOURNAL_NOT_ACTIVE || e == ERROR_JOURNAL_DELETE_IN_PROGRESS => {
                return Err(format!("USN journal on {} was deleted (err {e})", slot.drive));
            }
            Err(e) => {
                // ERROR_JOURNAL_ENTRY_DELETED — журнал «обогнал» нас; другой ID — журнал пересоздан
                let nj = query_journal(h);
                let need_rescan = e == ERROR_JOURNAL_ENTRY_DELETED || nj.map(|x| x.id != j.id || next < x.first).unwrap_or(false);
                if need_rescan {
                    warn!("volume {}: USN journal wrapped or recreated (err {e}) → full re-enumeration", slot.drive);
                    j = full_enum(eng, slot, h, true)?;
                    next = j.next;
                    continue;
                }
                errors += 1;
                warn!("volume {}: FSCTL_READ_USN_JOURNAL failed (err {e}), retry {errors}", slot.drive);
                if errors > 30 {
                    return Err(format!("USN journal read keeps failing (err {e})"));
                }
                unsafe { WaitForSingleObject(slot.stop_ev.0, 1000) };
            }
        }
    }
}

// ---------------------------------------------------------------- diagnostics

/// Проверка FSCTL_READ_UNPRIVILEGED_USN_JOURNAL (Win10 1709+) без прав администратора.
/// На практике имена в записях обнуляются (FileNameLength = 0) — для индекса имён непригодно.
pub fn probe_unprivileged(drive: &str) -> Value {
    let w = wide_z(&format!("{drive}\\"));
    unsafe {
        let h = CreateFileW(
            w.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OVERLAPPED,
            std::ptr::null_mut(),
        );
        if h == INVALID_HANDLE_VALUE {
            return json!({"ok": false, "error": format!("open root failed: {}", GetLastError())});
        }
        let h = Handle(h);
        let j = match query_journal(h.0) {
            Ok(j) => j,
            Err(e) => return json!({"ok": false, "error": format!("query journal failed: {e}")}),
        };
        let rd = READ_USN_JOURNAL_DATA_V1 {
            StartUsn: j.first,
            ReasonMask: 0xFFFF_FFFF,
            ReturnOnlyOnClose: 0,
            Timeout: 0,
            BytesToWaitFor: 0,
            UsnJournalID: j.id,
            MinMajorVersion: 2,
            MaxMajorVersion: 3,
        };
        let mut buf: Vec<u64> = vec![0u64; 8192];
        match ioctl(h.0, FSCTL_READ_UNPRIVILEGED_USN_JOURNAL, &rd as *const _ as *const _, std::mem::size_of::<READ_USN_JOURNAL_DATA_V1>(), &mut buf, None) {
            Ok(n) => {
                let bytes = std::slice::from_raw_parts(buf.as_ptr() as *const u8, n);
                let (mut cnt, mut named) = (0u64, 0u64);
                if n >= 8 {
                    for_each_record(&bytes[8..], |r| {
                        cnt += 1;
                        if !r.name.is_empty() {
                            named += 1;
                        }
                    });
                }
                json!({"ok": true, "records": cnt, "with_names": named, "usable": named > 0})
            }
            Err(e) => json!({"ok": false, "error": format!("read failed: {e}")}),
        }
    }
}

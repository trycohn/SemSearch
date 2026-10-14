//! Общее состояние движка: тома, подписчики, события, фоновый режим ввода-вывода.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::*};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex, RwLock, RwLockReadGuard, RwLockWriteGuard};

use crate::index::Volume;
use crate::util::*;

#[derive(Clone, Debug, Default, Serialize)]
pub struct VolInfo {
    pub drive: String,
    pub fs: String,
    /// "mft" | "walk" | "" (ещё не выбран)
    pub mode: String,
    pub ready: bool,
    pub build_ms: u64,
    /// почему не MFT / последняя ошибка
    pub note: String,
    pub journal_id: u64,
    pub links: u64,
    pub rescans: u64,
    /// MFT: записей в перечислении, «сирот» без родителя, скрытых (WinSxS, корзина, метафайлы)
    pub records: u64,
    pub orphans: u64,
    pub hidden: u64,
    /// MFT: время фаз сборки (мс) и число потоков перечисления
    pub enum_ms: u64,
    pub parents_ms: u64,
    pub finish_ms: u64,
    pub links_ms: u64,
    pub enum_threads: u64,
}

pub struct VolSlot {
    pub drive: String,
    pub vol: RwLock<Volume>,
    pub info: Mutex<VolInfo>,
    pub stop: AtomicBool,
    pub stop_ev: Handle,
}

impl VolSlot {
    pub fn new(drive: &str) -> Arc<VolSlot> {
        Arc::new(VolSlot {
            drive: drive.to_string(),
            vol: RwLock::new(Volume::new(drive, false)),
            info: Mutex::new(VolInfo { drive: drive.to_string(), ..Default::default() }),
            stop: AtomicBool::new(false),
            stop_ev: new_event(true),
        })
    }
    pub fn stopped(&self) -> bool {
        self.stop.load(Relaxed)
    }
    pub fn request_stop(&self) {
        self.stop.store(true, SeqCst);
        unsafe {
            windows_sys::Win32::System::Threading::SetEvent(self.stop_ev.0);
        }
    }
    pub fn read(&self) -> RwLockReadGuard<'_, Volume> {
        self.vol.read().unwrap_or_else(|e| e.into_inner())
    }
    pub fn write(&self) -> RwLockWriteGuard<'_, Volume> {
        self.vol.write().unwrap_or_else(|e| e.into_inner())
    }
    pub fn info(&self) -> VolInfo {
        self.info.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn set_info(&self, f: impl FnOnce(&mut VolInfo)) {
        let mut g = self.info.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut g);
    }
    /// Подменить индекс целиком (старый освобождается вне блокировки).
    pub fn install(&self, v: Volume) {
        let old = std::mem::replace(&mut *self.write(), v);
        drop(old);
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Event {
    pub ev: &'static str,
    pub path: String,
    pub old: Option<String>,
    pub dir: bool,
}

impl Event {
    pub fn new(ev: &'static str, path: String, old: Option<String>, dir: bool) -> Event {
        Event { ev, path, old, dir }
    }
}

pub struct Subscriber {
    /// свёрнутые корни без хвостового '\' (кроме "c:\")
    pub roots: Vec<String>,
    pub tx: SyncSender<Arc<str>>,
    pub overflow: AtomicBool,
    pub dead: AtomicBool,
}

fn under(lpath: &str, root: &str) -> bool {
    if !lpath.starts_with(root) {
        return false;
    }
    lpath.len() == root.len() || root.ends_with('\\') || lpath.as_bytes()[root.len()] == b'\\'
}

impl Subscriber {
    pub fn new(roots: &[String], tx: SyncSender<Arc<str>>) -> Subscriber {
        let roots = roots.iter().map(|r| fold_string(&norm_path(r))).filter(|r| !r.is_empty()).collect();
        Subscriber { roots, tx, overflow: AtomicBool::new(false), dead: AtomicBool::new(false) }
    }

    fn wants(&self, ev: &Event, lpath: &str, lold: Option<&str>) -> bool {
        if self.roots.is_empty() {
            return true;
        }
        if ev.ev == "rescan" {
            let d = lpath.get(..2).unwrap_or("");
            return self.roots.iter().any(|r| r.starts_with(d) || lpath.starts_with(r.as_str()));
        }
        self.roots.iter().any(|r| under(lpath, r) || lold.map(|o| under(o, r)).unwrap_or(false))
    }

    /// Строки «rescan» для этого подписчика (после переполнения его очереди).
    pub fn rescan_lines(&self) -> Vec<String> {
        let paths: Vec<String> = if self.roots.is_empty() { vec![String::new()] } else { self.roots.clone() };
        paths
            .into_iter()
            .map(|p| serde_json::to_string(&Event::new("rescan", p, None, true)).unwrap_or_default())
            .collect()
    }
}

pub struct Engine {
    pub root: PathBuf,
    pub elevated: bool,
    pub force_walk: bool,
    pub vols: RwLock<Vec<Arc<VolSlot>>>,
    pub subs: Mutex<Vec<Arc<Subscriber>>>,
    pub updated_ms: AtomicU64,
    bg: Mutex<u32>,
}

impl Engine {
    pub fn new(root: PathBuf, elevated: bool, force_walk: bool) -> Arc<Engine> {
        Arc::new(Engine {
            root,
            elevated,
            force_walk,
            vols: RwLock::new(Vec::new()),
            subs: Mutex::new(Vec::new()),
            updated_ms: AtomicU64::new(0),
            bg: Mutex::new(0),
        })
    }

    pub fn slots(&self) -> Vec<Arc<VolSlot>> {
        self.vols.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn touch(&self) {
        self.updated_ms.store(now_ms(), Relaxed);
    }

    /// Ожидаемый/текущий режим: "walk", если хоть один том в режиме обхода.
    pub fn mode(&self) -> &'static str {
        let slots = self.slots();
        let mut any_mft = false;
        for s in &slots {
            match s.info().mode.as_str() {
                "walk" => return "walk",
                "mft" => any_mft = true,
                _ => {}
            }
        }
        if any_mft || (self.elevated && !self.force_walk) {
            "mft"
        } else {
            "walk"
        }
    }

    /// Фоновый режим процесса (низкий приоритет ввода-вывода) на время сборки индекса.
    pub fn bg_begin(&self) {
        let mut g = self.bg.lock().unwrap_or_else(|e| e.into_inner());
        if *g == 0 && !set_background(true) {
            warn!("PROCESS_MODE_BACKGROUND_BEGIN failed");
        }
        *g += 1;
    }
    pub fn bg_end(&self) {
        let mut g = self.bg.lock().unwrap_or_else(|e| e.into_inner());
        if *g > 0 {
            *g -= 1;
            if *g == 0 {
                set_background(false);
                set_below_normal();
            }
        }
    }

    pub fn emit(&self, evs: Vec<Event>) {
        if evs.is_empty() {
            return;
        }
        let subs: Vec<Arc<Subscriber>> = self.subs.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if subs.is_empty() {
            return;
        }
        let prepared: Vec<(Arc<str>, String, Option<String>, &Event)> = evs
            .iter()
            .map(|e| {
                let line: Arc<str> = Arc::from(serde_json::to_string(e).unwrap_or_default());
                (line, fold_string(&e.path), e.old.as_deref().map(fold_string), e)
            })
            .collect();
        let mut any_dead = false;
        for s in &subs {
            if s.dead.load(Relaxed) {
                any_dead = true;
                continue;
            }
            for (line, lp, lo, e) in &prepared {
                if !s.wants(e, lp, lo.as_deref()) {
                    continue;
                }
                match s.tx.try_send(line.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        s.overflow.store(true, SeqCst);
                    }
                    Err(TrySendError::Disconnected(_)) => {
                        s.dead.store(true, SeqCst);
                        any_dead = true;
                        break;
                    }
                }
            }
        }
        if any_dead {
            self.subs.lock().unwrap_or_else(|e| e.into_inner()).retain(|s| !s.dead.load(Relaxed));
        }
    }

    pub fn add_sub(&self, s: Arc<Subscriber>) {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).push(s);
    }

    pub fn remove_sub(&self, s: &Arc<Subscriber>) {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).retain(|x| !Arc::ptr_eq(x, s));
    }

    // ------------------------------------------------------------ volumes

    pub fn start_volume(self: &Arc<Self>, drive: &str) {
        let slot = VolSlot::new(drive);
        self.vols.write().unwrap_or_else(|e| e.into_inner()).push(slot.clone());
        let eng = self.clone();
        let name = format!("vol-{}", &drive[..1]);
        let _ = std::thread::Builder::new().name(name).spawn(move || loop {
            // ошибка в коде тома не должна молча останавливать обновления: перезапуск с полной пересборкой
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drive_volume(eng.clone(), slot.clone())));
            if r.is_ok() || slot.stopped() {
                break;
            }
            warn!("volume {}: worker panicked, restarting in 10 s", slot.drive);
            slot.set_info(|i| i.rescans += 1);
            unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(slot.stop_ev.0, 10_000) };
            if slot.stopped() {
                break;
            }
        });
    }

    pub fn stop_volume(&self, drive: &str) {
        let mut g = self.vols.write().unwrap_or_else(|e| e.into_inner());
        g.retain(|s| {
            if s.drive.eq_ignore_ascii_case(drive) {
                s.request_stop();
                false
            } else {
                true
            }
        });
    }

    /// Применить список дисков из config.json: остановить лишние, запустить новые.
    pub fn apply_drives(self: &Arc<Self>, drives: &[String]) {
        let cur: Vec<String> = self.slots().iter().map(|s| s.drive.clone()).collect();
        for d in &cur {
            if !drives.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                info!("volume {d}: removed from config, stopping");
                self.stop_volume(d);
            }
        }
        for d in drives {
            if !cur.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                info!("volume {d}: starting");
                self.start_volume(d);
            }
        }
    }
}

/// Поток тома: MFT/USN, если можно, иначе обход + ReadDirectoryChangesW.
fn drive_volume(eng: Arc<Engine>, slot: Arc<VolSlot>) {
    let fs = drive_fs(&slot.drive).unwrap_or_default();
    slot.set_info(|i| i.fs = fs.clone());
    if fs.is_empty() {
        warn!("volume {}: not accessible (GetVolumeInformation failed)", slot.drive);
        slot.set_info(|i| i.note = "volume not accessible".into());
        return;
    }
    let reason = if eng.force_walk {
        "walk mode forced (--walk)".to_string()
    } else if !eng.elevated {
        "not elevated".to_string()
    } else if fs != "NTFS" {
        format!("file system {fs} is not NTFS")
    } else {
        match crate::mft::run(&eng, &slot) {
            Ok(()) => return,
            Err(e) => e,
        }
    };
    if slot.stopped() {
        return;
    }
    info!("volume {}: walk mode ({reason})", slot.drive);
    slot.set_info(|i| i.note = reason.clone());
    crate::walk::run(&eng, &slot);
}

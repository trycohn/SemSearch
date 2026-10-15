//! Общее состояние движка: тома, подписчики, события, фоновый режим ввода-вывода.

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering::*};
use std::sync::mpsc::{SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, RwLock, RwLockReadGuard, RwLockWriteGuard};
use std::time::Duration;

use crate::index::Volume;
use crate::util::*;

/// Сколько после запроса на извлечение устройства том не запускается заново, пока диск ещё виден
/// (отмена извлечения без уведомления; обычно диск пропадает за секунды).
const EJECT_GRACE_MS: u64 = 30_000;
/// Как часто сверять список дисков с подключёнными (внешний диск, разблокированный BitLocker, смена буквы).
const DRIVE_POLL: Duration = Duration::from_secs(10);

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
    /// серийный номер тома при запуске (0 — не узнали): другой номер — под этой буквой уже другой диск
    pub serial: u32,
    /// том подключён уже после запуска движка (внешний диск, разблокированный BitLocker): пока строится его
    /// индекс, общая готовность `ready` не сбрасывается — иначе backend на время сборки уйдёт на запасной индекс
    pub late: bool,
    pub vol: RwLock<Volume>,
    pub info: Mutex<VolInfo>,
    pub stop: AtomicBool,
    pub stop_ev: Handle,
    /// тип носителя (HDD/SSD), определяет поток тома: от него зависит число потоков обхода
    pub media: OnceLock<Media>,
    /// сколько потоков тома ещё работает (они держат дескрипторы тома: корень для ReadDirectoryChangesW, сам том)
    pub live: AtomicUsize,
    /// когда (мс с начала эпохи) пришёл запрос на извлечение устройства; 0 — не приходил
    pub ejecting: AtomicU64,
    /// уведомление об извлечении для тома уже зарегистрировано
    pub watched: AtomicBool,
}

/// Поток тома жив: пока такая метка существует, дескрипторы тома могут быть открыты.
pub struct Live(Arc<VolSlot>);

impl Drop for Live {
    fn drop(&mut self) {
        self.0.live.fetch_sub(1, SeqCst);
    }
}

impl VolSlot {
    pub fn new(drive: &str, late: bool) -> Arc<VolSlot> {
        Arc::new(VolSlot {
            drive: drive.to_string(),
            serial: drive_info(drive).map(|x| x.1).unwrap_or(0),
            late,
            vol: RwLock::new(Volume::new(drive, false)),
            info: Mutex::new(VolInfo { drive: drive.to_string(), ..Default::default() }),
            stop: AtomicBool::new(false),
            stop_ev: new_event(true),
            media: OnceLock::new(),
            live: AtomicUsize::new(0),
            ejecting: AtomicU64::new(0),
            watched: AtomicBool::new(false),
        })
    }
    /// Отметить поток тома (создавать до запуска потока, метку отдать в поток).
    pub fn live_guard(self: &Arc<Self>) -> Live {
        self.live.fetch_add(1, SeqCst);
        Live(self.clone())
    }
    pub fn media(&self) -> Media {
        self.media.get().copied().unwrap_or(Media::Unknown)
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
    /// папка данных (docs/API.md §1): config.json ищется сначала здесь, потом в root
    pub data: PathBuf,
    pub elevated: bool,
    pub force_walk: bool,
    pub vols: RwLock<Vec<Arc<VolSlot>>>,
    pub subs: Mutex<Vec<Arc<Subscriber>>>,
    pub updated_ms: AtomicU64,
    bg: Mutex<u32>,
    /// apply_drives вызывают и сверка дисков, и reload_config: одновременно — нельзя (том запустился бы дважды)
    apply: Mutex<()>,
    /// уведомления об извлечении устройств (по одному на том), см. eject.rs
    pub ejects: Mutex<Vec<crate::eject::Watch>>,
}

impl Engine {
    pub fn new(root: PathBuf, data: PathBuf, elevated: bool, force_walk: bool) -> Arc<Engine> {
        Arc::new(Engine {
            root,
            data,
            elevated,
            force_walk,
            vols: RwLock::new(Vec::new()),
            subs: Mutex::new(Vec::new()),
            updated_ms: AtomicU64::new(0),
            bg: Mutex::new(0),
            apply: Mutex::new(()),
            ejects: Mutex::new(Vec::new()),
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
        self.spawn_volume(drive, false);
    }

    fn spawn_volume(self: &Arc<Self>, drive: &str, late: bool) {
        let slot = VolSlot::new(drive, late);
        self.vols.write().unwrap_or_else(|e| e.into_inner()).push(slot.clone());
        let eng = self.clone();
        let name = format!("vol-{}", &drive[..1]);
        let live = slot.live_guard();
        let _ = std::thread::Builder::new().name(name).spawn(move || {
            let _live = live;
            loop {
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

    /// Снять уведомления об извлечении у томов, которых уже нет в списке. Закрытие ждёт завершения
    /// обратных вызовов, поэтому делается здесь, а не в самом обратном вызове и не под блокировками.
    fn sweep_ejects(&self) {
        let slots = self.slots();
        let dead: Vec<crate::eject::Watch> = {
            let mut g = self.ejects.lock().unwrap_or_else(|e| e.into_inner());
            let (keep, dead): (Vec<_>, Vec<_>) = g.drain(..).partition(|w| slots.iter().any(|s| Arc::ptr_eq(s, w.slot())));
            *g = keep;
            dead
        };
        for w in dead {
            let t0 = std::time::Instant::now();
            let d = w.slot().drive.clone();
            drop(w);
            if t0.elapsed() > Duration::from_millis(500) {
                warn!("volume {d}: closing the eject notification took {} ms", t0.elapsed().as_millis());
            }
        }
    }

    /// Применить список дисков (config.json или подключённые несъёмные): остановить лишние и исчезнувшие,
    /// перезапустить тома, под буквой которых оказался другой диск, запустить новые. late — вызов сверки
    /// дисков (а не запуска или reload_config): запущенные тома не снимают общую готовность, см. VolSlot::late.
    pub fn apply_drives(self: &Arc<Self>, drives: &[String], late: bool) {
        let _g = self.apply.lock().unwrap_or_else(|e| e.into_inner());
        for s in self.slots() {
            let d = s.drive.as_str();
            if !drives.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                info!("volume {d}: not in the drive list any more (removed from config or disk gone), stopping");
                self.stop_volume(d);
                continue;
            }
            let ej = s.ejecting.load(Relaxed);
            if ej != 0 {
                if now_ms().saturating_sub(ej) < EJECT_GRACE_MS {
                    continue; // идёт извлечение: том заново не открываем
                }
                info!("volume {d}: still present after an eject request, restarting");
                self.stop_volume(d);
            } else if s.stopped() {
                info!("volume {d}: stopped without being removed (eject cancelled?), restarting");
                self.stop_volume(d);
            } else if s.live.load(SeqCst) == 0 {
                // поток тома закончил сам (том был недоступен в момент запуска и т. п.): иначе слот так и остался бы пустым
                info!("volume {d}: worker has finished, restarting");
                self.stop_volume(d);
            } else if s.serial != 0 {
                if let Some((_, now)) = drive_info(d) {
                    if now != s.serial {
                        info!("volume {d}: another volume under the same letter (serial {:08X} -> {now:08X}), rebuilding", s.serial);
                        self.stop_volume(d);
                    }
                }
            }
        }
        let cur: Vec<String> = self.slots().iter().map(|s| s.drive.clone()).collect();
        for d in drives {
            if !cur.iter().any(|x| x.eq_ignore_ascii_case(d)) {
                info!("volume {d}: starting");
                self.spawn_volume(d, late);
            }
        }
        self.sweep_ejects();
    }

    /// Раз в несколько секунд сверяет диски с подключёнными: внешний диск подключили или отключили,
    /// разблокировали BitLocker, сменили букву — тома запускаются и останавливаются сами, без перезапуска.
    pub fn watch_drives(self: &Arc<Self>, initial: Vec<String>) {
        let eng = self.clone();
        let _ = std::thread::Builder::new().name("drives".into()).spawn(move || {
            let mut last = initial;
            loop {
                std::thread::sleep(DRIVE_POLL);
                let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    // config.json как раз переписывают — пропускаем такт, а не откатываемся к умолчаниям
                    let Some(drives) = config_drives_checked(&eng.data, &eng.root) else { return };
                    if drives != last {
                        info!("drives: {:?} -> {:?}", last, drives);
                        last = drives.clone();
                    }
                    eng.apply_drives(&drives, true);
                }));
                if r.is_err() {
                    warn!("drive watcher panicked; continuing");
                }
            }
        });
    }
}

/// Поток тома: MFT/USN, если можно (NTFS и права администратора), иначе обход + ReadDirectoryChangesW
/// (в том числе для несъёмных дисков exFAT, FAT32, ReFS).
fn drive_volume(eng: Arc<Engine>, slot: Arc<VolSlot>) {
    let fs = drive_fs(&slot.drive).unwrap_or_default();
    slot.set_info(|i| i.fs = fs.clone());
    if fs.is_empty() {
        warn!("volume {}: not accessible (GetVolumeInformation failed)", slot.drive);
        slot.set_info(|i| i.note = "volume not accessible".into());
        return;
    }
    let media = *slot.media.get_or_init(|| drive_media(&slot.drive));
    info!("volume {}: {fs}, storage: {}", slot.drive, media.name());
    // Windows не извлечёт внешний диск, пока мы держим его дескрипторы: просим сказать, когда его собираются извлечь
    if !slot.watched.swap(true, SeqCst) {
        if let Some(w) = crate::eject::Watch::new(&slot) {
            eng.ejects.lock().unwrap_or_else(|e| e.into_inner()).push(w);
        }
    }
    let reason = if eng.force_walk {
        "walk mode forced (--walk)".to_string()
    } else if fs != "NTFS" {
        format!("file system {fs} is not NTFS")
    } else if !eng.elevated {
        "not elevated".to_string()
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

//! Безопасное извлечение внешних дисков. Пока том открыт (MFT: сам том для чтения USN-журнала; обход: корневой
//! каталог для ReadDirectoryChangesW), Windows отвечает на «Извлечь» отказом «устройство используется».
//! На каждый том регистрируем уведомление об извлечении (CM_Register_Notification, Windows 8+) на отдельном
//! дескрипторе тома. Запрос на извлечение (DEVICEQUERYREMOVE): останавливаем том, ждём, пока его потоки закроют
//! свои дескрипторы, закрываем свой и возвращаем управление. Диск исчез — Engine::apply_drives уберёт том;
//! извлечение отменили — он запустится заново (с полной пересборкой индекса).

use std::ffi::c_void;
use std::sync::atomic::Ordering::SeqCst;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Storage::FileSystem::*;

use crate::engine::VolSlot;
use crate::util::*;

/// Сколько обратный вызов ждёт, пока потоки тома закроют дескрипторы (система ждёт нас, но не бесконечно).
const RELEASE_WAIT: Duration = Duration::from_secs(8);

/// Состояние, на которое указывает обратный вызов. Живёт, пока не снято уведомление (Watch::drop).
struct Cb {
    slot: Arc<VolSlot>,
    /// дескриптор, на котором висит уведомление; закрывается в ответ на запрос извлечения
    handle: Mutex<Option<Handle>>,
}

impl Cb {
    fn release(&self) {
        let s = &self.slot;
        s.ejecting.store(now_ms().max(1), SeqCst);
        let first = self.handle.lock().unwrap_or_else(|e| e.into_inner()).is_some();
        if first {
            info!("volume {}: device removal requested, releasing the volume", s.drive);
        }
        s.request_stop();
        let t0 = Instant::now();
        while s.live.load(SeqCst) > 0 && t0.elapsed() < RELEASE_WAIT {
            std::thread::sleep(Duration::from_millis(20));
        }
        // ждать нечего и после таймаута: свой дескриптор закрываем в любом случае
        drop(self.handle.lock().unwrap_or_else(|e| e.into_inner()).take());
        let left = s.live.load(SeqCst);
        if first {
            info!("volume {}: released in {} ms{}", s.drive, t0.elapsed().as_millis(), if left > 0 { format!(", {left} thread(s) still running") } else { String::new() });
        }
    }
}

pub struct Watch {
    notif: HCMNOTIFICATION,
    cb: *mut Cb,
}

// указатель на Cb используется только в обратном вызове и в Drop
unsafe impl Send for Watch {}

impl Watch {
    /// None — не получилось (том закрыт, нет прав, старая система): тогда том просто не даст себя извлечь.
    pub fn new(slot: &Arc<VolSlot>) -> Option<Watch> {
        let w = wide_z(&format!(r"\\.\{}", slot.drive));
        // доступ 0: достаточно для уведомлений, администратор не нужен
        let h = unsafe { CreateFileW(w.as_ptr(), 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
        if h == INVALID_HANDLE_VALUE {
            warn!("volume {}: no eject notification (cannot open the volume, err {})", slot.drive, unsafe { GetLastError() });
            return None;
        }
        let cb = Box::into_raw(Box::new(Cb { slot: slot.clone(), handle: Mutex::new(Some(Handle(h))) }));
        let mut f: CM_NOTIFY_FILTER = unsafe { std::mem::zeroed() };
        f.cbSize = std::mem::size_of::<CM_NOTIFY_FILTER>() as u32;
        f.FilterType = CM_NOTIFY_FILTER_TYPE_DEVICEHANDLE;
        f.u.DeviceHandle = CM_NOTIFY_FILTER_0_1 { hTarget: h };
        let mut n: HCMNOTIFICATION = std::ptr::null_mut();
        let cr = unsafe { CM_Register_Notification(&f, cb as *const c_void, Some(on_event), &mut n) };
        if cr != CR_SUCCESS || n.is_null() {
            warn!("volume {}: no eject notification (CM_Register_Notification returned {cr})", slot.drive);
            drop(unsafe { Box::from_raw(cb) }); // закроет и дескриптор
            return None;
        }
        info!("volume {}: eject notification registered", slot.drive);
        Some(Watch { notif: n, cb })
    }

    pub fn slot(&self) -> &Arc<VolSlot> {
        // SAFETY: cb жив до Drop
        unsafe { &(*self.cb).slot }
    }
}

impl Drop for Watch {
    /// Ждёт завершения идущих обратных вызовов: не вызывать из обратного вызова и под блокировками,
    /// от которых зависят потоки тома.
    fn drop(&mut self) {
        unsafe {
            CM_Unregister_Notification(self.notif);
            drop(Box::from_raw(self.cb));
        }
    }
}

unsafe extern "system" fn on_event(_notify: HCMNOTIFICATION, ctx: *const c_void, action: CM_NOTIFY_ACTION, _data: *const CM_NOTIFY_EVENT_DATA, _len: u32) -> u32 {
    // SAFETY: ctx — Cb из Watch::new; CM_Unregister_Notification дожидается нас до освобождения
    let cb = unsafe { &*(ctx as *const Cb) };
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match action {
        // запрос на извлечение; а если устройство выдернули без запроса — те же действия, пока система ждёт дескрипторы
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVE | CM_NOTIFY_ACTION_DEVICEREMOVEPENDING | CM_NOTIFY_ACTION_DEVICEREMOVECOMPLETE => cb.release(),
        // извлечение отменено: том уже остановлен, сверка дисков запустит его заново
        CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED => cb.slot.ejecting.store(0, SeqCst),
        _ => {}
    }));
    0 // ERROR_SUCCESS: не возражаем
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering::SeqCst;

    fn cb_for(drive: &str) -> Box<Cb> {
        Box::new(Cb { slot: VolSlot::new(drive, false), handle: Mutex::new(None) })
    }

    fn fire(cb: &Cb, action: CM_NOTIFY_ACTION) -> u32 {
        unsafe { on_event(std::ptr::null_mut(), cb as *const Cb as *const c_void, action, std::ptr::null(), 0) }
    }

    /// Запрос на извлечение: том останавливается, обратный вызов ждёт потоки тома и возвращает успех.
    #[test]
    fn query_remove_stops_slot_and_waits_for_threads() {
        let cb = cb_for("Q:"); // такой диск не нужен: тома мы не открываем
        let guard = cb.slot.live_guard();
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(guard);
        });
        let t0 = Instant::now();
        assert_eq!(fire(&cb, CM_NOTIFY_ACTION_DEVICEQUERYREMOVE), 0);
        assert!(t0.elapsed() >= Duration::from_millis(250), "returned before the volume thread finished");
        assert!(cb.slot.stopped());
        assert_ne!(cb.slot.ejecting.load(SeqCst), 0);
        assert_eq!(cb.slot.live.load(SeqCst), 0);
        t.join().unwrap();
        // извлечение отменили: метка снимается, сверка дисков запустит том заново
        assert_eq!(fire(&cb, CM_NOTIFY_ACTION_DEVICEQUERYREMOVEFAILED), 0);
        assert_eq!(cb.slot.ejecting.load(SeqCst), 0);
    }

    /// Регистрация на настоящем томе системного диска проходит и снимается без зависаний.
    #[test]
    fn register_and_unregister_on_system_volume() {
        let d = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".into());
        let slot = VolSlot::new(&d, false);
        if let Some(w) = Watch::new(&slot) {
            assert!(Arc::ptr_eq(w.slot(), &slot));
            drop(w);
        }
    }
}

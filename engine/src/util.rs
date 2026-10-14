//! Общие помощники: логирование, строки UTF-16, свёртка регистра, время, атрибуты файлов, токен процесса.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
use windows_sys::Win32::Security::*;
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Console::*;
use windows_sys::Win32::System::ProcessStatus::*;
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::*;

// ---------------------------------------------------------------- logging

static LOG: Mutex<Option<Box<dyn Write + Send>>> = Mutex::new(None);

macro_rules! info {
    ($($a:tt)*) => { $crate::util::log_write('I', &format!($($a)*)) };
}
macro_rules! warn {
    ($($a:tt)*) => { $crate::util::log_write('W', &format!($($a)*)) };
}

pub fn log_write(level: char, msg: &str) {
    let ts = local_ts();
    let line = format!("{ts} {level} {msg}\n");
    if let Ok(mut g) = LOG.lock() {
        if let Some(w) = g.as_mut() {
            let _ = w.write_all(line.as_bytes());
            let _ = w.flush();
        }
    }
}

struct Tee(Box<dyn Write + Send>, Box<dyn Write + Send>);
impl Write for Tee {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        let _ = self.0.write_all(b);
        let _ = self.1.write_all(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let _ = self.0.flush();
        self.1.flush()
    }
}

/// Открыть файл на запись, не следуя по ссылкам: точка повторной обработки или файл с
/// несколькими жёсткими ссылками отвергаются (защита от подмены файла лога/отчёта).
fn open_nofollow(p: &Path, append: bool) -> std::io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    let mut o = OpenOptions::new();
    o.create(true).custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    if append {
        o.append(true);
    } else {
        o.write(true);
    }
    let f = o.open(p)?;
    let mut bi: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe { GetFileInformationByHandle(f.as_raw_handle() as HANDLE, &mut bi) } != 0;
    if !ok || bi.nNumberOfLinks > 1 || bi.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(std::io::Error::other(format!("refusing to write {}: link or reparse point", p.display())));
    }
    Ok(f)
}

fn open_append_nofollow(p: &Path) -> std::io::Result<File> {
    open_nofollow(p, true)
}

fn file_sink(dir: &Path, name: &str) -> Box<dyn Write + Send> {
    let _ = std::fs::create_dir_all(dir);
    let p = dir.join(name);
    if std::fs::symlink_metadata(&p).map(|m| m.len() > 8 << 20).unwrap_or(false) {
        let _ = std::fs::rename(&p, dir.join(format!("{name}.1")));
    }
    match open_append_nofollow(&p) {
        Ok(f) => Box::new(f),
        Err(_) => Box::new(std::io::sink()),
    }
}

/// Лог: в консоль и/или в файл `<dir>\<file>` (ротация при > 8 МБ).
/// dir — `<root>\data` без повышенных прав, `<папка exe>\logs` с ними.
pub fn log_init(dir: &Path, console: bool, file: Option<&str>) {
    let sink: Box<dyn Write + Send> = match (console, file) {
        (true, Some(f)) => Box::new(Tee(console_sink(), file_sink(dir, f))),
        (true, None) => console_sink(),
        (false, Some(f)) => file_sink(dir, f),
        (false, None) => Box::new(std::io::sink()),
    };
    *LOG.lock().unwrap() = Some(sink);
}

/// Процесс собран как GUI (`windows_subsystem`), поэтому stdout может отсутствовать:
/// если его нет — подключаемся к консоли родителя и пишем в CONOUT$.
pub fn console_sink() -> Box<dyn Write + Send> {
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            // повторный AttachConsole вернёт ошибку «уже подключён» — это нормально
            AttachConsole(ATTACH_PARENT_PROCESS);
            if let Ok(f) = OpenOptions::new().write(true).open("CONOUT$") {
                return Box::new(f);
            }
            return Box::new(std::io::sink());
        }
    }
    Box::new(std::io::stdout())
}

/// Отдельный вывод для отчёта --selftest (stdout или консоль родителя).
pub fn stdout_writer() -> Box<dyn Write + Send> {
    console_sink()
}

fn local_ts() -> String {
    unsafe {
        let mut t: SYSTEMTIME = std::mem::zeroed();
        GetLocalTime(&mut t);
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond, t.wMilliseconds
        )
    }
}

// ---------------------------------------------------------------- handles

pub struct Handle(pub HANDLE);
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    pub fn valid(&self) -> bool {
        !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        if self.valid() {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

pub fn new_event(manual: bool) -> Handle {
    unsafe { Handle(CreateEventW(std::ptr::null(), manual as i32, 0, std::ptr::null())) }
}

// ---------------------------------------------------------------- strings

pub fn wide_z(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `\\?\`-форма пути (длинные пути, имена с точкой/пробелом в конце) + NUL.
pub fn long_w(path: &str) -> Vec<u16> {
    let mut v: Vec<u16> = Vec::with_capacity(path.len() + 6);
    if !path.starts_with(r"\\") {
        v.extend(r"\\?\".encode_utf16());
    }
    v.extend(path.encode_utf16().map(|c| if c == b'/' as u16 { b'\\' as u16 } else { c }));
    v.push(0);
    v
}

/// `\\?\`-форма пути для std::fs (путь уже нормализован: без "." и "..").
pub fn long_path(path: &str) -> String {
    let p = path.replace('/', "\\");
    if p.starts_with(r"\\") {
        p
    } else {
        format!(r"\\?\{p}")
    }
}

/// Свёртка регистра для поиска без учёта регистра. Сохраняет длину в байтах UTF-8
/// (символы, у которых строчная форма другой длины, оставляются как есть), поэтому
/// строчная копия имени лежит по тем же смещениям, что и оригинал. Ё → е.
pub fn fold_into(s: &str, out: &mut Vec<u8>) {
    let b = s.as_bytes();
    if b.is_ascii() {
        out.extend(b.iter().map(|c| c.to_ascii_lowercase()));
        return;
    }
    let mut buf = [0u8; 4];
    for ch in s.chars() {
        if (ch as u32) < 0x80 {
            out.push((ch as u8).to_ascii_lowercase());
        } else {
            out.extend_from_slice(fold_char(ch).encode_utf8(&mut buf).as_bytes());
        }
    }
}

#[inline]
pub fn fold_char(ch: char) -> char {
    match ch {
        'ё' | 'Ё' => return 'е',
        '\u{0410}'..='\u{042F}' => return char::from_u32(ch as u32 + 0x20).unwrap_or(ch),
        '\u{0430}'..='\u{044F}' => return ch,
        _ => {}
    }
    let mut it = ch.to_lowercase();
    let lc = it.next().unwrap_or(ch);
    if it.next().is_some() || lc.len_utf8() != ch.len_utf8() {
        ch
    } else {
        lc
    }
}

pub fn fold(s: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(s.len());
    fold_into(s, &mut v);
    v
}

pub fn fold_string(s: &str) -> String {
    // fold сохраняет корректность UTF-8 (символ заменяется символом)
    String::from_utf8(fold(s)).unwrap_or_else(|_| s.to_lowercase())
}

/// Нормализация пути от клиента: `/` → `\`, без `\\?\`, без хвостового `\` (кроме `C:\`),
/// буква диска заглавная.
pub fn norm_path(p: &str) -> String {
    let mut s: String = p.trim().replace('/', "\\");
    if let Some(r) = s.strip_prefix(r"\\?\") {
        s = r.to_string();
    }
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    if s.len() == 2 && s.as_bytes()[1] == b':' {
        s.push('\\');
    }
    if s.len() >= 2 && s.as_bytes()[1] == b':' && s.as_bytes()[0].is_ascii_lowercase() {
        let d = s[..1].to_ascii_uppercase();
        s.replace_range(..1, &d);
    }
    s
}

/// "C:" для пути "c:\..." (или None).
pub fn drive_of(p: &str) -> Option<String> {
    let b = p.as_bytes();
    if b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic() {
        Some(format!("{}:", (b[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

pub fn norm_drive(d: &str) -> Option<String> {
    let t = d.trim();
    let b = t.as_bytes();
    if !b.is_empty() && b[0].is_ascii_alphabetic() && (b.len() == 1 || b[1] == b':') {
        Some(format!("{}:", (b[0] as char).to_ascii_uppercase()))
    } else {
        None
    }
}

// ---------------------------------------------------------------- time

pub fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn ft_epoch(ft: &FILETIME) -> f64 {
    let v = ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64;
    if v == 0 {
        return 0.0;
    }
    (v as f64 - 116_444_736_000_000_000.0) / 1e7
}

// ---------------------------------------------------------------- file stat

#[derive(Clone, Copy, Debug)]
pub struct Stat {
    pub attrs: u32,
    pub size: u64,
    pub mtime: f64,
    pub ctime: f64,
}

/// Атрибуты/размер/время без открытия файла (GetFileAttributesExW, запасной путь — FindFirstFileExW).
pub fn stat(path: &str) -> Option<Stat> {
    let w = long_w(path);
    unsafe {
        let mut d: WIN32_FILE_ATTRIBUTE_DATA = std::mem::zeroed();
        if GetFileAttributesExW(w.as_ptr(), GetFileExInfoStandard, &mut d as *mut _ as *mut _) != 0 {
            return Some(Stat {
                attrs: d.dwFileAttributes,
                size: ((d.nFileSizeHigh as u64) << 32) | d.nFileSizeLow as u64,
                mtime: ft_epoch(&d.ftLastWriteTime),
                ctime: ft_epoch(&d.ftCreationTime),
            });
        }
        let mut fd: WIN32_FIND_DATAW = std::mem::zeroed();
        let h = FindFirstFileExW(w.as_ptr(), FindExInfoBasic, &mut fd as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), 0);
        if h != INVALID_HANDLE_VALUE {
            FindClose(h);
            return Some(Stat {
                attrs: fd.dwFileAttributes,
                size: ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64,
                mtime: ft_epoch(&fd.ftLastWriteTime),
                ctime: ft_epoch(&fd.ftCreationTime),
            });
        }
    }
    None
}

/// (атрибуты, тег точки повторной обработки) записи каталога по пути; None — не существует.
pub fn find_data(path: &str) -> Option<(u32, u32)> {
    let w = long_w(path);
    unsafe {
        let mut fd: WIN32_FIND_DATAW = std::mem::zeroed();
        let h = FindFirstFileExW(w.as_ptr(), FindExInfoBasic, &mut fd as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), 0);
        if h != INVALID_HANDLE_VALUE {
            FindClose(h);
            let tag = if fd.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 { fd.dwReserved0 } else { 0 };
            return Some((fd.dwFileAttributes, tag));
        }
        let a = GetFileAttributesW(w.as_ptr());
        if a != INVALID_FILE_ATTRIBUTES {
            return Some((a, 0));
        }
    }
    None
}

/// Перечислить каталог (FindExInfoBasic + LARGE_FETCH): f(имя, атрибуты, размер, mtime).
pub fn for_each_dir_entry(dir: &str, mut f: impl FnMut(&str, u32, u64, f64)) -> bool {
    let mut pat = long_w(dir);
    pat.pop();
    if pat.last() != Some(&(b'\\' as u16)) {
        pat.push(b'\\' as u16);
    }
    pat.push(b'*' as u16);
    pat.push(0);
    unsafe {
        let mut fd: WIN32_FIND_DATAW = std::mem::zeroed();
        let h = FindFirstFileExW(pat.as_ptr(), FindExInfoBasic, &mut fd as *mut _ as *mut _, FindExSearchNameMatch, std::ptr::null(), FIND_FIRST_EX_LARGE_FETCH);
        if h == INVALID_HANDLE_VALUE {
            return false;
        }
        loop {
            let n = fd.cFileName.iter().position(|&c| c == 0).unwrap_or(fd.cFileName.len());
            let name = String::from_utf16_lossy(&fd.cFileName[..n]);
            f(&name, fd.dwFileAttributes, ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64, ft_epoch(&fd.ftLastWriteTime));
            if FindNextFileW(h, &mut fd) == 0 {
                break;
            }
        }
        FindClose(h);
    }
    true
}

pub fn exists(path: &str) -> bool {
    let w = long_w(path);
    unsafe { GetFileAttributesW(w.as_ptr()) != INVALID_FILE_ATTRIBUTES }
}

// ---------------------------------------------------------------- process / token

pub fn is_elevated() -> bool {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let tok = Handle(tok);
        let mut el: TOKEN_ELEVATION = std::mem::zeroed();
        let mut n = 0u32;
        let ok = GetTokenInformation(tok.0, TokenElevation, &mut el as *mut _ as *mut _, std::mem::size_of::<TOKEN_ELEVATION>() as u32, &mut n);
        ok != 0 && el.TokenIsElevated != 0
    }
}

/// Строковый SID пользователя процесса (TokenUser), например "S-1-5-21-…-1001".
pub fn user_sid() -> Option<String> {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return None;
        }
        let tok = Handle(tok);
        let mut buf = vec![0u64; 128];
        let mut n = 0u32;
        if GetTokenInformation(tok.0, TokenUser, buf.as_mut_ptr() as *mut _, (buf.len() * 8) as u32, &mut n) == 0 {
            return None;
        }
        let tu = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(tu.User.Sid, &mut s) == 0 || s.is_null() {
            return None;
        }
        let mut len = 0;
        while *s.add(len) != 0 {
            len += 1;
        }
        let out = String::from_utf16_lossy(std::slice::from_raw_parts(s, len));
        LocalFree(s as *mut _);
        Some(out)
    }
}

pub struct Mem {
    pub ws: u64,
    pub peak_ws: u64,
    pub private: u64,
}

pub fn mem() -> Mem {
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb);
        Mem { ws: c.WorkingSetSize as u64, peak_ws: c.PeakWorkingSetSize as u64, private: c.PagefileUsage as u64 }
    }
}

pub fn mem_str() -> String {
    let m = mem();
    let (k, u) = cpu_times();
    format!("rss {} MB, private {} MB, peak rss {} MB, cpu kernel {k:.1}s user {u:.1}s", m.ws >> 20, m.private >> 20, m.peak_ws >> 20)
}

/// Процессорное время процесса (ядро, пользователь), секунды.
pub fn cpu_times() -> (f64, f64) {
    unsafe {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
        let s = |f: FILETIME| (((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64) as f64 / 1e7;
        (s(k), s(u))
    }
}

pub fn set_below_normal() {
    unsafe {
        SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS);
    }
}

pub fn set_background(on: bool) -> bool {
    unsafe { SetPriorityClass(GetCurrentProcess(), if on { PROCESS_MODE_BACKGROUND_BEGIN } else { PROCESS_MODE_BACKGROUND_END }) != 0 }
}

// ---------------------------------------------------------------- config / root

pub fn exe_dir() -> PathBuf {
    let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    exe.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| PathBuf::from("."))
}

/// child лежит в parent (или совпадает), без учёта регистра.
pub fn path_under(child: &Path, parent: &Path) -> bool {
    let c = fold_string(&norm_path(&child.to_string_lossy()));
    let p = fold_string(&norm_path(&parent.to_string_lossy()));
    !p.is_empty() && (c == p || c.starts_with(&format!("{}\\", p.trim_end_matches('\\'))))
}

/// Проверка значения `--root`: абсолютный путь к существующей папке.
pub fn check_root(r: &str) -> Result<PathBuf, String> {
    let n = norm_path(r.trim().trim_matches('"'));
    if drive_of(&n).is_none() || n.as_bytes().get(2) != Some(&b'\\') {
        return Err(format!("not an absolute local path: {r}"));
    }
    let p = PathBuf::from(&n);
    if !p.is_dir() {
        return Err(format!("not a directory: {r}"));
    }
    Ok(p)
}

/// Корень проекта: ближайший предок папки exe, где лежит config.json.
pub fn find_root() -> PathBuf {
    let dir = exe_dir();
    for a in dir.ancestors() {
        if a.join("config.json").is_file() {
            return a.to_path_buf();
        }
    }
    if dir.file_name().map(|n| n.eq_ignore_ascii_case("bin")).unwrap_or(false) {
        if let Some(p) = dir.parent() {
            return p.to_path_buf();
        }
    }
    dir
}

/// config.json — недоверенный ввод (папка проекта доступна пользователю на запись):
/// читаем не больше 1 МБ, используем только "name_drives".
pub fn read_config(root: &Path) -> serde_json::Value {
    use std::io::Read;
    let p = root.join("config.json");
    let mut b = Vec::new();
    let ok = File::open(&p).and_then(|f| f.take(1 << 20).read_to_end(&mut b)).is_ok();
    if !ok || b.len() >= 1 << 20 {
        return serde_json::Value::Null;
    }
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&b);
    serde_json::from_slice(b).unwrap_or(serde_json::Value::Null)
}

/// Диски для индекса имён: config.json "name_drives", иначе все несъёмные NTFS.
/// Принимаются только буквы реальных несъёмных NTFS-томов.
pub fn config_drives(root: &Path) -> Vec<String> {
    let cfg = read_config(root);
    let valid = fixed_ntfs_drives();
    let mut out: Vec<String> = Vec::new();
    if let Some(arr) = cfg.get("name_drives").and_then(|v| v.as_array()) {
        for v in arr.iter().take(26) {
            let Some(s) = v.as_str() else { continue };
            match norm_drive(s).filter(|d| valid.contains(d)) {
                Some(d) => {
                    if !out.contains(&d) {
                        out.push(d);
                    }
                }
                None => warn!("config.json name_drives: ignoring {:?} (not a fixed NTFS volume)", s.chars().take(16).collect::<String>()),
            }
        }
        return out;
    }
    valid
}

pub fn fixed_ntfs_drives() -> Vec<String> {
    let mut out = Vec::new();
    let mask = unsafe { GetLogicalDrives() };
    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let d = format!("{}:", (b'A' + i as u8) as char);
        let w = wide_z(&format!("{d}\\"));
        if unsafe { GetDriveTypeW(w.as_ptr()) } != 3 {
            continue; // DRIVE_FIXED
        }
        if drive_fs(&d).as_deref() == Some("NTFS") {
            out.push(d);
        }
    }
    out
}

pub fn drive_fs(drive: &str) -> Option<String> {
    let w = wide_z(&format!("{drive}\\"));
    let mut fs = [0u16; 64];
    let ok = unsafe {
        GetVolumeInformationW(w.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), fs.len() as u32)
    };
    if ok == 0 {
        return None;
    }
    let n = fs.iter().position(|&c| c == 0).unwrap_or(fs.len());
    Some(String::from_utf16_lossy(&fs[..n]))
}

pub fn write_file(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let mut f = open_nofollow(path, false)?;
    f.set_len(0)?;
    f.write_all(data)
}

//! Компактный индекс имён одного тома.
//!
//! Запись = 12 байт: индекс родителя, смещение/длина имени в общем буфере, флаги.
//! Имена лежат подряд в `names` (UTF-8, как есть) и в `lnames` (свёрнутый регистр,
//! те же смещения — свёртка сохраняет длину). Путь восстанавливается по родителям.
//! `children` — хеш-таблица (родитель, имя без регистра) → индекс, для разбора путей.
//! В режиме MFT дополнительно `frn_map` (номер записи MFT → индекс) и `ent_rec` (обратно).

use hashbrown::HashTable;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use crate::util::{fold, fold_into};

pub const NONE: u32 = u32::MAX;

pub const F_DIR: u16 = 1;
pub const F_HIDDEN: u16 = 2;
pub const F_SYSTEM: u16 = 4;
pub const F_REPARSE: u16 = 8;
pub const F_CLOUD: u16 = 16;
pub const F_DELETED: u16 = 32;
/// скрыто от поиска: $Recycle.Bin, System Volume Information, Windows\WinSxS, метафайлы NTFS (режим MFT)
pub const F_EXCLUDED: u16 = 64;
pub const F_ROOT: u16 = 128;
/// дополнительное имя жёсткой ссылки (MFT): не связано с frn_map
pub const F_LINK: u16 = 256;

pub const HIDE: u16 = F_DELETED | F_EXCLUDED | F_ROOT;
const ATTR_BITS: u16 = F_DIR | F_HIDDEN | F_SYSTEM | F_REPARSE | F_CLOUD;

pub const A_DIRECTORY: u32 = 0x10;
pub const A_REPARSE: u32 = 0x400;
pub const A_CLOUD: u32 = 0x1000 | 0x40000 | 0x400000; // OFFLINE | RECALL_ON_OPEN | RECALL_ON_DATA_ACCESS

pub fn flags_from_attrs(a: u32) -> u16 {
    let mut f = 0;
    if a & A_DIRECTORY != 0 {
        f |= F_DIR;
    }
    if a & 0x2 != 0 {
        f |= F_HIDDEN;
    }
    if a & 0x4 != 0 {
        f |= F_SYSTEM;
    }
    if a & A_REPARSE != 0 {
        f |= F_REPARSE;
    }
    if a & A_CLOUD != 0 {
        f |= F_CLOUD;
    }
    f
}

#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct Entry {
    pub parent: u32,
    pub off: u32,
    pub len: u16,
    pub flags: u16,
}

pub struct Volume {
    pub drive: String,
    pub ents: Vec<Entry>,
    pub names: Vec<u8>,
    pub lnames: Vec<u8>,
    pub nchild: Vec<u32>,
    pub children: HashTable<u32>,
    pub free: Vec<u32>,
    pub garbage: usize,
    pub files: i64,
    pub dirs: i64,
    pub mft: bool,
    pub frn_map: Vec<u32>,
    pub ent_rec: Vec<u32>,
}

#[inline]
fn hkey(parent: u32, lname: &[u8]) -> u64 {
    const K: u64 = 0x517c_c1b7_2722_0a95;
    let mut h: u64 = (parent as u64 ^ 0x9E37_79B9_7F4A_7C15).wrapping_mul(K);
    let mut it = lname.chunks_exact(8);
    for c in &mut it {
        h = (h.rotate_left(5) ^ u64::from_le_bytes(c.try_into().unwrap())).wrapping_mul(K);
    }
    let r = it.remainder();
    if !r.is_empty() {
        let mut b = [0u8; 8];
        b[..r.len()].copy_from_slice(r);
        h = (h.rotate_left(5) ^ u64::from_le_bytes(b) ^ ((r.len() as u64) << 59)).wrapping_mul(K);
    }
    // fmix64
    h ^= h >> 33;
    h = h.wrapping_mul(0xff51_afd7_ed55_8ccd);
    h ^= h >> 33;
    h = h.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    h ^ (h >> 33)
}

#[inline]
fn ent_hash(ents: &[Entry], lnames: &[u8], i: u32) -> u64 {
    let e = ents[i as usize];
    hkey(e.parent, &lnames[e.off as usize..e.off as usize + e.len as usize])
}

#[inline]
pub fn rec_of(frn: u64) -> usize {
    (frn & 0x0000_FFFF_FFFF_FFFF) as usize
}

impl Volume {
    pub fn new(drive: &str, mft: bool) -> Self {
        let mut v = Volume {
            drive: drive.to_string(),
            ents: Vec::new(),
            names: Vec::new(),
            lnames: Vec::new(),
            nchild: Vec::new(),
            children: HashTable::new(),
            free: Vec::new(),
            garbage: 0,
            files: 0,
            dirs: 0,
            mft,
            frn_map: Vec::new(),
            ent_rec: Vec::new(),
        };
        let (off, len) = v.push_name(drive);
        v.ents.push(Entry { parent: NONE, off, len, flags: F_DIR | F_ROOT });
        v.nchild.push(0);
        if mft {
            v.ent_rec.push(NONE);
        }
        v
    }

    pub fn reserve(&mut self, n: usize, name_bytes: usize) {
        self.ents.reserve(n);
        self.nchild.reserve(n);
        self.names.reserve(name_bytes);
        self.lnames.reserve(name_bytes);
        if self.mft {
            self.ent_rec.reserve(n);
        }
    }

    // ------------------------------------------------------------ accessors

    #[inline]
    pub fn name(&self, i: u32) -> &str {
        let e = &self.ents[i as usize];
        // SAFETY: в names пишутся только целые строки UTF-8
        unsafe { std::str::from_utf8_unchecked(&self.names[e.off as usize..e.off as usize + e.len as usize]) }
    }
    #[inline]
    pub fn lname(&self, i: u32) -> &[u8] {
        let e = &self.ents[i as usize];
        &self.lnames[e.off as usize..e.off as usize + e.len as usize]
    }
    #[inline]
    pub fn is_dir(&self, i: u32) -> bool {
        self.ents[i as usize].flags & F_DIR != 0
    }
    #[inline]
    pub fn visible(&self, i: u32) -> bool {
        self.ents[i as usize].flags & HIDE == 0
    }
    #[inline]
    pub fn live(&self, i: u32) -> bool {
        (i as usize) < self.ents.len() && self.ents[i as usize].flags & F_DELETED == 0
    }

    // ------------------------------------------------------------ names

    pub fn push_name(&mut self, name: &str) -> (u32, u16) {
        let off = self.names.len() as u32;
        let name = if name.len() > u16::MAX as usize { truncate_utf8(name, u16::MAX as usize) } else { name };
        grow(&mut self.names, name.len());
        grow(&mut self.lnames, name.len());
        self.names.extend_from_slice(name.as_bytes());
        fold_into(name, &mut self.lnames);
        if self.lnames.len() != self.names.len() {
            // не должно случаться (свёртка сохраняет длину) — подстраховка
            self.lnames.truncate(off as usize);
            self.lnames.extend_from_slice(name.as_bytes());
        }
        (off, name.len() as u16)
    }

    /// Имя из UTF-16LE байтов (записи USN) прямо в буферы, без промежуточных строк.
    pub fn push_name_u16le(&mut self, raw: &[u8]) -> (u32, u16) {
        let off = self.names.len();
        let units = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]]));
        let mut buf = [0u8; 4];
        for r in char::decode_utf16(units) {
            let ch = r.unwrap_or('\u{FFFD}');
            if self.names.len() - off + ch.len_utf8() > u16::MAX as usize {
                break;
            }
            self.names.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            let f = if (ch as u32) < 0x80 { (ch as u8).to_ascii_lowercase() as char } else { crate::util::fold_char(ch) };
            self.lnames.extend_from_slice(f.encode_utf8(&mut buf).as_bytes());
        }
        (off as u32, (self.names.len() - off) as u16)
    }

    #[inline]
    fn count(&mut self, flags: u16, d: i64) {
        if flags & HIDE == 0 {
            if flags & F_DIR != 0 {
                self.dirs += d;
            } else {
                self.files += d;
            }
        }
    }

    pub fn recount(&mut self) {
        let (mut f, mut d) = (0i64, 0i64);
        for e in &self.ents {
            if e.flags & HIDE == 0 {
                if e.flags & F_DIR != 0 {
                    d += 1;
                } else {
                    f += 1;
                }
            }
        }
        self.files = f;
        self.dirs = d;
    }

    // ------------------------------------------------------------ build

    /// Добавление при первичной сборке: без хеш-таблицы (строится в finish_build).
    pub fn add_nomap(&mut self, parent: u32, name: &str, flags: u16) -> u32 {
        let (off, len) = self.push_name(name);
        let i = self.ents.len() as u32;
        self.ents.push(Entry { parent, off, len, flags });
        self.nchild.push(0);
        if self.mft {
            self.ent_rec.push(NONE);
        }
        i
    }

    /// После сборки: число детей, счётчики, хеш-таблица детей, ужатие буферов.
    pub fn finish_build(&mut self) {
        let n = self.ents.len();
        self.nchild.clear();
        self.nchild.resize(n, 0);
        for i in 0..n {
            let e = self.ents[i];
            if e.flags & F_DELETED == 0 && e.parent != NONE {
                self.nchild[e.parent as usize] += 1;
            }
        }
        self.recount();
        let mut t: HashTable<u32> = HashTable::with_capacity(n);
        {
            let ents = &self.ents;
            let lnames = &self.lnames;
            for i in 1..n as u32 {
                let e = ents[i as usize];
                if e.flags & F_DELETED != 0 || e.parent == NONE {
                    continue;
                }
                let h = ent_hash(ents, lnames, i);
                t.insert_unique(h, i, |&j| ent_hash(ents, lnames, j));
            }
        }
        self.children = t;
        // небольшой запас: после shrink_to_fit первая же вставка удвоила бы буферы (+сотни МБ commit)
        let head = |n: usize| n + n / 32 + 1024;
        self.ents.shrink_to(head(self.ents.len()));
        self.names.shrink_to(head(self.names.len()));
        self.lnames.shrink_to(head(self.lnames.len()));
        self.nchild.shrink_to(head(self.nchild.len()));
        self.ent_rec.shrink_to(head(self.ent_rec.len()));
        self.frn_map.shrink_to(head(self.frn_map.len()));
    }

    // ------------------------------------------------------------ child map

    fn map_insert(&mut self, i: u32) {
        let Volume { children, ents, lnames, .. } = self;
        let h = ent_hash(ents, lnames, i);
        children.insert_unique(h, i, |&j| ent_hash(ents, lnames, j));
    }

    fn map_remove(&mut self, i: u32) {
        let h = ent_hash(&self.ents, &self.lnames, i);
        if let Ok(o) = self.children.find_entry(h, |&j| j == i) {
            o.remove();
        }
    }

    pub fn find_child(&self, parent: u32, lname: &[u8]) -> Option<u32> {
        let h = hkey(parent, lname);
        self.children
            .find(h, |&j| {
                let e = &self.ents[j as usize];
                e.parent == parent && &self.lnames[e.off as usize..e.off as usize + e.len as usize] == lname
            })
            .copied()
    }

    // ------------------------------------------------------------ updates

    pub fn add(&mut self, parent: u32, name: &str, flags: u16) -> u32 {
        let (off, len) = self.push_name(name);
        let e = Entry { parent, off, len, flags: flags & !F_DELETED };
        let i = if let Some(i) = self.free.pop() {
            self.ents[i as usize] = e;
            self.nchild[i as usize] = 0;
            if self.mft {
                self.ent_rec[i as usize] = NONE;
            }
            i
        } else {
            grow(&mut self.ents, 1);
            grow(&mut self.nchild, 1);
            self.ents.push(e);
            self.nchild.push(0);
            if self.mft {
                grow(&mut self.ent_rec, 1);
                self.ent_rec.push(NONE);
            }
            (self.ents.len() - 1) as u32
        };
        if parent != NONE {
            self.nchild[parent as usize] += 1;
        }
        self.map_insert(i);
        self.count(e.flags, 1);
        i
    }

    /// Удалить одну запись (дети должны быть уже удалены — см. remove_tree).
    pub fn remove_one(&mut self, i: u32) {
        let e = self.ents[i as usize];
        if e.flags & F_DELETED != 0 || e.flags & F_ROOT != 0 {
            return;
        }
        self.map_remove(i);
        if e.parent != NONE {
            let c = &mut self.nchild[e.parent as usize];
            *c = c.saturating_sub(1);
        }
        self.count(e.flags, -1);
        self.garbage += e.len as usize;
        self.ents[i as usize] = Entry { parent: NONE, off: 0, len: 0, flags: F_DELETED };
        if self.mft {
            let r = self.ent_rec[i as usize];
            if r != NONE && self.frn_map.get(r as usize) == Some(&i) {
                self.frn_map[r as usize] = NONE;
            }
            self.ent_rec[i as usize] = NONE;
        }
        self.free.push(i);
    }

    /// Удалить запись со всем поддеревом. Полный проход по индексу — только если у каталога
    /// ещё остались дети (перенос в корзину, пропущенные события).
    pub fn remove_tree(&mut self, i: u32) -> usize {
        if !self.live(i) {
            return 0;
        }
        if self.nchild[i as usize] == 0 {
            self.remove_one(i);
            return 1;
        }
        let mask = self.descendants_mask(&[i]);
        let mut n = 0;
        for j in 0..self.ents.len() {
            if mask[j] == 2 && self.ents[j].flags & F_DELETED == 0 {
                self.remove_one(j as u32);
                n += 1;
            }
        }
        n
    }

    /// mask[j] == 2 ⇔ j — один из roots или их потомок.
    pub fn descendants_mask(&self, roots: &[u32]) -> Vec<u8> {
        let n = self.ents.len();
        let mut memo = vec![0u8; n];
        for &r in roots {
            if (r as usize) < n {
                memo[r as usize] = 2;
            }
        }
        let mut stack: Vec<u32> = Vec::new();
        for i in 0..n {
            if memo[i] != 0 || self.ents[i].flags & F_DELETED != 0 {
                continue;
            }
            let mut cur = i as u32;
            let val;
            loop {
                if cur == NONE || stack.len() > 8192 {
                    val = 1;
                    break;
                }
                let m = memo[cur as usize];
                if m != 0 {
                    val = m;
                    break;
                }
                if self.ents[cur as usize].flags & F_DELETED != 0 {
                    val = 1;
                    break;
                }
                stack.push(cur);
                cur = self.ents[cur as usize].parent;
            }
            for x in stack.drain(..) {
                memo[x as usize] = val;
            }
        }
        memo
    }

    /// Пометить/снять F_EXCLUDED на поддереве; вернуть записи, у которых флаг изменился.
    pub fn set_excluded_tree(&mut self, root: u32, excl: bool) -> Vec<u32> {
        let targets: Vec<u32> = if self.nchild[root as usize] == 0 {
            vec![root]
        } else {
            let mask = self.descendants_mask(&[root]);
            (0..self.ents.len() as u32).filter(|&j| mask[j as usize] == 2).collect()
        };
        let mut changed = Vec::new();
        for j in targets {
            let f = self.ents[j as usize].flags;
            if f & F_DELETED != 0 {
                continue;
            }
            let nf = if excl { f | F_EXCLUDED } else { f & !F_EXCLUDED };
            if nf != f {
                self.count(f, -1);
                self.ents[j as usize].flags = nf;
                self.count(nf, 1);
                changed.push(j);
            }
        }
        changed
    }

    pub fn is_ancestor_or_self(&self, anc: u32, mut i: u32) -> bool {
        let mut guard = 0;
        while i != NONE && guard < 8192 {
            if i == anc {
                return true;
            }
            i = self.ents[i as usize].parent;
            guard += 1;
        }
        false
    }

    /// Переименование/перемещение (родитель уже проверен вызывающим).
    pub fn rename(&mut self, i: u32, np: u32, name: &str) {
        self.map_remove(i);
        let e = self.ents[i as usize];
        if e.parent != np {
            if e.parent != NONE {
                let c = &mut self.nchild[e.parent as usize];
                *c = c.saturating_sub(1);
            }
            self.nchild[np as usize] += 1;
            self.ents[i as usize].parent = np;
        }
        if self.name(i) != name {
            self.garbage += e.len as usize;
            let (off, len) = self.push_name(name);
            self.ents[i as usize].off = off;
            self.ents[i as usize].len = len;
        }
        self.map_insert(i);
    }

    /// Обновить флаги из атрибутов файла (каталог/скрытый/облачный…), не трогая служебные.
    pub fn set_attr_flags(&mut self, i: u32, af: u16) {
        let f = self.ents[i as usize].flags;
        if f & F_DELETED != 0 {
            return;
        }
        // каталог не становится файлом и наоборот
        let nf = (f & !ATTR_BITS) | (af & !F_DIR) | (f & F_DIR);
        if nf != f {
            self.count(f, -1);
            self.ents[i as usize].flags = nf;
            self.count(nf, 1);
        }
    }

    // ------------------------------------------------------------ paths

    pub fn path_into(&self, i: u32, out: &mut String) {
        let mut chain: [u32; 128] = [0; 128];
        let mut n = 0;
        let mut cur = i;
        let mut deep: Vec<u32> = Vec::new();
        while cur != NONE {
            if n < chain.len() {
                chain[n] = cur;
                n += 1;
            } else {
                if deep.len() > 8192 {
                    break;
                }
                deep.push(cur);
            }
            cur = self.ents[cur as usize].parent;
        }
        let mut first = true;
        for &c in deep.iter().rev().chain(chain[..n].iter().rev()) {
            if !first {
                out.push('\\');
            }
            out.push_str(self.name(c));
            first = false;
        }
        if n == 1 && deep.is_empty() && self.ents[i as usize].flags & F_ROOT != 0 {
            out.push('\\');
        }
    }

    pub fn path(&self, i: u32) -> String {
        let mut s = String::with_capacity(96);
        self.path_into(i, &mut s);
        s
    }

    /// Путь «родитель\имя» без записи в индексе.
    pub fn child_path(&self, parent: u32, name: &str) -> String {
        let mut s = self.path(parent);
        if !s.ends_with('\\') {
            s.push('\\');
        }
        s.push_str(name);
        s
    }

    /// Длина пути в байтах (для сортировки) без построения строки.
    pub fn path_len(&self, i: u32) -> u32 {
        let mut n = 0u32;
        let mut cur = i;
        let mut guard = 0;
        while cur != NONE && guard < 8192 {
            n += self.ents[cur as usize].len as u32 + 1;
            cur = self.ents[cur as usize].parent;
            guard += 1;
        }
        n.saturating_sub(1)
    }

    /// Разбор абсолютного пути ("C:\a\b", "c:/a/b") в индекс записи.
    pub fn resolve(&self, path: &str) -> Option<u32> {
        let p = path.replace('/', "\\");
        let mut comps = p.split('\\').filter(|c| !c.is_empty() && *c != "?" && *c != ".");
        let first = comps.next()?;
        if !first.eq_ignore_ascii_case(&self.drive) {
            return None;
        }
        let mut cur = 0u32;
        let mut buf = Vec::with_capacity(64);
        for c in comps {
            buf.clear();
            fold_into(c, &mut buf);
            cur = self.find_child(cur, &buf)?;
        }
        Some(cur)
    }

    pub fn find_child_name(&self, parent: u32, name: &str) -> Option<u32> {
        self.find_child(parent, &fold(name))
    }

    // ------------------------------------------------------------ maintenance

    /// Сборка мусора в буферах имён (после множества переименований/удалений).
    pub fn compact_if_needed(&mut self) -> bool {
        if self.garbage < (16 << 20) || self.garbage * 3 < self.names.len() {
            return false;
        }
        let live = self.names.len() - self.garbage;
        let mut names = Vec::with_capacity(live + live / 8);
        let mut lnames = Vec::with_capacity(live + live / 8);
        for e in self.ents.iter_mut() {
            if e.flags & F_DELETED != 0 {
                e.off = 0;
                e.len = 0;
                continue;
            }
            let (s, l) = (e.off as usize, e.len as usize);
            let no = names.len() as u32;
            names.extend_from_slice(&self.names[s..s + l]);
            lnames.extend_from_slice(&self.lnames[s..s + l]);
            e.off = no;
        }
        self.names = names;
        self.lnames = lnames;
        self.garbage = 0;
        true
    }

    pub fn mem_bytes(&self) -> usize {
        self.ents.capacity() * std::mem::size_of::<Entry>()
            + self.names.capacity()
            + self.lnames.capacity()
            + self.nchild.capacity() * 4
            + self.children.capacity() * 5
            + self.free.capacity() * 4
            + self.frn_map.capacity() * 4
            + self.ent_rec.capacity() * 4
    }

    // ------------------------------------------------------------ MFT

    pub fn set_frn(&mut self, i: u32, frn: u64) {
        let r = rec_of(frn);
        if r >= u32::MAX as usize {
            return;
        }
        if r >= self.frn_map.len() {
            let want = (r + 1 + (r >> 4)).max(1024);
            self.frn_map.reserve_exact(want - self.frn_map.len());
            self.frn_map.resize(want, NONE);
        }
        self.frn_map[r] = i;
        if (i as usize) < self.ent_rec.len() {
            self.ent_rec[i as usize] = r as u32;
        }
    }

    pub fn lookup_frn(&self, frn: u64) -> Option<u32> {
        let r = rec_of(frn);
        let i = *self.frn_map.get(r)?;
        if i == NONE || (i as usize) >= self.ents.len() {
            return None;
        }
        if self.ent_rec.get(i as usize) != Some(&(r as u32)) || self.ents[i as usize].flags & F_DELETED != 0 {
            return None;
        }
        Some(i)
    }
}

/// Рост больших буферов шагом 1/8 вместо удвоения (Vec по умолчанию удваивает ёмкость).
#[inline]
fn grow<T>(v: &mut Vec<T>, extra: usize) {
    if v.len() + extra > v.capacity() {
        v.reserve_exact((v.capacity() / 8).max(extra).max(4096));
    }
}

fn truncate_utf8(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ---------------------------------------------------------------- memo

/// Мемо «по каталогам» для одного запроса: 0 — неизвестно, 1 — нет, 2 — да.
/// Значения идемпотентны, поэтому гонки между потоками безвредны.
pub struct Memo(Vec<AtomicU8>);

impl Memo {
    pub fn new(n: usize) -> Memo {
        let v: Vec<u8> = vec![0u8; n];
        let mut v = std::mem::ManuallyDrop::new(v);
        // SAFETY: AtomicU8 имеет ту же раскладку, что и u8
        let m = unsafe { Vec::from_raw_parts(v.as_mut_ptr() as *mut AtomicU8, v.len(), v.capacity()) };
        Memo(m)
    }

    /// f(start) = local(d) для первого предка, где local решает, иначе step(x, f(parent(x))).
    /// Выше корня — false.
    #[inline]
    pub fn eval<L, S>(&self, v: &Volume, start: u32, scratch: &mut Vec<u32>, local: L, step: S) -> bool
    where
        L: Fn(u32) -> Option<bool>,
        S: Fn(u32, bool) -> bool,
    {
        scratch.clear();
        let mut cur = start;
        let mut val;
        loop {
            if cur == NONE || (cur as usize) >= self.0.len() {
                val = false;
                break;
            }
            let m = self.0[cur as usize].load(Relaxed);
            if m != 0 {
                val = m == 2;
                break;
            }
            if let Some(b) = local(cur) {
                self.0[cur as usize].store(if b { 2 } else { 1 }, Relaxed);
                val = b;
                break;
            }
            scratch.push(cur);
            if scratch.len() > 8192 {
                return false;
            }
            cur = v.ents[cur as usize].parent;
        }
        while let Some(x) = scratch.pop() {
            val = step(x, val);
            self.0[x as usize].store(if val { 2 } else { 1 }, Relaxed);
        }
        val
    }
}

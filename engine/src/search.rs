//! Поиск по имени (подстрока / glob / regex, несколько слов через И, слова-пути),
//! ранжирование, выборка `list`, `same_name`.

use memchr::memmem::Finder;
use rayon::prelude::*;
use regex::bytes::{Regex, RegexBuilder};
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU8, Ordering::Relaxed};

use crate::index::*;
use crate::util::*;

const CHUNK: usize = 16 * 1024;

// ---------------------------------------------------------------- glob

#[derive(Clone, Copy, PartialEq, Eq)]
enum GlobKind {
    Exact,
    Prefix,
    Suffix,
    Contains,
    General,
}

pub struct Glob {
    pat: Vec<u8>,
    kind: GlobKind,
    lit: Vec<u8>,
    pre: Option<Finder<'static>>,
}

fn is_wild(c: u8) -> bool {
    c == b'*' || c == b'?'
}

impl Glob {
    fn new(p: &[u8]) -> Glob {
        // схлопнуть "**"
        let mut pat = Vec::with_capacity(p.len());
        for &c in p {
            if c == b'*' && pat.last() == Some(&b'*') {
                continue;
            }
            pat.push(c);
        }
        let n = pat.len();
        let no_wild = |s: &[u8]| !s.iter().any(|&c| is_wild(c));
        let (kind, lit) = if no_wild(&pat) {
            (GlobKind::Exact, pat.clone())
        } else if n >= 1 && pat[0] == b'*' && no_wild(&pat[1..]) {
            (GlobKind::Suffix, pat[1..].to_vec())
        } else if n >= 1 && pat[n - 1] == b'*' && no_wild(&pat[..n - 1]) {
            (GlobKind::Prefix, pat[..n - 1].to_vec())
        } else if n >= 2 && pat[0] == b'*' && pat[n - 1] == b'*' && no_wild(&pat[1..n - 1]) {
            (GlobKind::Contains, pat[1..n - 1].to_vec())
        } else {
            // самый длинный литерал — для предфильтра
            let best = pat.split(|&c| is_wild(c)).max_by_key(|s| s.len()).unwrap_or(&[]).to_vec();
            (GlobKind::General, best)
        };
        let pre = if (kind == GlobKind::Contains || kind == GlobKind::General) && !lit.is_empty() {
            Some(Finder::new(&lit).into_owned())
        } else {
            None
        };
        Glob { pat, kind, lit, pre }
    }

    #[inline]
    fn is_match(&self, s: &[u8]) -> bool {
        match self.kind {
            GlobKind::Exact => s == &self.lit[..],
            GlobKind::Prefix => s.starts_with(&self.lit),
            GlobKind::Suffix => s.ends_with(&self.lit),
            GlobKind::Contains => self.pre.as_ref().map(|f| f.find(s).is_some()).unwrap_or(true),
            GlobKind::General => {
                if let Some(f) = &self.pre {
                    if f.find(s).is_none() {
                        return false;
                    }
                }
                glob_match(&self.pat, s)
            }
        }
    }

    /// Для glob по пути: чем должно оканчиваться имя (хвостовой литерал после последнего
    /// * / ?, часть после последнего '\'), и должно ли имя совпадать с ним целиком.
    fn name_tail(&self) -> (Vec<u8>, bool) {
        let start = self.pat.iter().rposition(|&c| is_wild(c)).map(|p| p + 1).unwrap_or(0);
        let tail = &self.pat[start..];
        match tail.iter().rposition(|&c| c == b'\\') {
            Some(p) => (tail[p + 1..].to_vec(), true),
            None => (tail.to_vec(), false),
        }
    }

    /// литеральный префикс до первого * / ?
    fn prefix(&self) -> &[u8] {
        let end = self.pat.iter().position(|&c| is_wild(c)).unwrap_or(self.pat.len());
        &self.pat[..end]
    }
}

#[inline]
fn utf8_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1,
    }
}

/// `*` — любая последовательность, `?` — один символ (UTF-8); совпадение всей строки.
pub fn glob_match(p: &[u8], s: &[u8]) -> bool {
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star_p, mut star_s) = (usize::MAX, 0usize);
    while si < s.len() {
        if pi < p.len() {
            match p[pi] {
                b'*' => {
                    star_p = pi;
                    star_s = si;
                    pi += 1;
                    continue;
                }
                b'?' => {
                    pi += 1;
                    si += utf8_len(s[si]);
                    continue;
                }
                c if c == s[si] => {
                    pi += 1;
                    si += 1;
                    continue;
                }
                _ => {}
            }
        }
        if star_p != usize::MAX {
            pi = star_p + 1;
            star_s += utf8_len(s[star_s]);
            si = star_s;
            continue;
        }
        return false;
    }
    while pi < p.len() && p[pi] == b'*' {
        pi += 1;
    }
    pi == p.len()
}

// ---------------------------------------------------------------- query

pub enum Pat {
    /// подстрока имени
    Sub(Finder<'static>),
    /// glob по имени целиком
    Glob(Glob),
    /// подстрока полного пути (слово содержит '\' или '/'), разбита по '\'
    PathSub(Vec<Vec<u8>>),
    /// glob по полному пути
    PathGlob(Glob),
}

pub struct Query {
    pub name_pats: Vec<Pat>,
    pub path_pats: Vec<Pat>,
    pub regex: Option<Regex>,
    pub regex_path: bool,
    pub exact: Vec<u8>,
    pub prefix: Vec<u8>,
    pub files_only: bool,
    pub exts: Vec<Vec<u8>>,
}

fn split_words(q: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut inq = false;
    for ch in q.chars() {
        if ch == '"' {
            inq = !inq;
            continue;
        }
        if ch.is_whitespace() && !inq {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        cur.push(ch);
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn norm_exts(exts: &[String]) -> Vec<Vec<u8>> {
    exts.iter()
        .map(|e| e.trim())
        .filter(|e| !e.is_empty())
        .map(|e| {
            let mut v = fold(e);
            if v[0] != b'.' {
                v.insert(0, b'.');
            }
            v
        })
        .collect()
}

fn path_glob_pattern(w: &[u8]) -> Vec<u8> {
    let abs = w.first() == Some(&b'*') || (w.len() >= 2 && w[1] == b':');
    if abs {
        w.to_vec()
    } else if w.first() == Some(&b'\\') {
        [&b"*"[..], w].concat()
    } else {
        [&b"*\\"[..], w].concat()
    }
}

pub fn parse_query(q: &str, mode: &str, files_only: bool, exts: &[String]) -> Result<Query, String> {
    let mut qr = Query {
        name_pats: Vec::new(),
        path_pats: Vec::new(),
        regex: None,
        regex_path: false,
        exact: Vec::new(),
        prefix: Vec::new(),
        files_only,
        exts: norm_exts(exts),
    };
    let qt = q.trim();
    let mut mode = mode;
    let mut text = qt;
    if mode == "auto" || mode.is_empty() {
        if let Some(rest) = qt.strip_prefix("re:") {
            mode = "regex";
            text = rest.trim_start();
        } else {
            mode = "auto";
        }
    }
    match mode {
        "regex" => {
            if text.len() > 4096 {
                return Err("regex too long (max 4096)".into());
            }
            if text.is_empty() {
                return Ok(qr);
            }
            let re = RegexBuilder::new(text)
                .case_insensitive(true)
                .unicode(true)
                .size_limit(2 << 20)
                .dfa_size_limit(8 << 20)
                .nest_limit(64)
                .build()
                .map_err(|e| format!("bad regex: {e}"))?;
            qr.regex_path = text.contains(r"\\") || text.contains('/');
            qr.regex = Some(re);
        }
        "auto" | "substr" | "glob" => {
            let words = split_words(text);
            for (k, w) in words.iter().enumerate() {
                let lw: Vec<u8> = fold(w).into_iter().map(|c| if c == b'/' { b'\\' } else { c }).collect();
                if lw.is_empty() {
                    continue;
                }
                let wild = mode != "substr" && lw.iter().any(|&c| is_wild(c));
                let path = lw.contains(&b'\\');
                let globbed = mode == "glob" || wild;
                let pat = if globbed && path {
                    Pat::PathGlob(Glob::new(&path_glob_pattern(&lw)))
                } else if globbed {
                    Pat::Glob(Glob::new(&lw))
                } else if path {
                    Pat::PathSub(lw.split(|&c| c == b'\\').map(|s| s.to_vec()).collect())
                } else {
                    Pat::Sub(Finder::new(&lw).into_owned())
                };
                if k == 0 {
                    qr.prefix = match &pat {
                        Pat::Sub(_) => lw.clone(),
                        Pat::Glob(g) => g.prefix().to_vec(),
                        Pat::PathSub(segs) => segs.last().cloned().unwrap_or_default(),
                        Pat::PathGlob(_) => Vec::new(),
                    };
                }
                match pat {
                    Pat::Sub(_) | Pat::Glob(_) => qr.name_pats.push(pat),
                    _ => qr.path_pats.push(pat),
                }
            }
            // длинные (более избирательные) подстроки — первыми
            qr.name_pats.sort_by_key(|p| match p {
                Pat::Sub(f) => usize::MAX - f.needle().len(),
                _ => usize::MAX,
            });
            if !words.is_empty() && qr.path_pats.is_empty() && !words.iter().any(|w| w.contains('*') || w.contains('?')) {
                qr.exact = fold(&words.join(" "));
            }
        }
        _ => return Err(format!("unknown mode: {mode}")),
    }
    Ok(qr)
}

#[inline]
fn stem(l: &[u8]) -> &[u8] {
    match memchr::memrchr(b'.', l) {
        Some(p) if p > 0 => &l[..p],
        _ => l,
    }
}

#[inline]
fn rank(q: &Query, lname: &[u8]) -> u32 {
    let class: u32 = if !q.exact.is_empty() && lname == &q.exact[..] {
        0
    } else if !q.exact.is_empty() && stem(lname) == &q.exact[..] {
        1
    } else if !q.prefix.is_empty() && lname.starts_with(&q.prefix) {
        2
    } else {
        3
    };
    (class << 16) | (lname.len().min(0xFFFF) as u32)
}

#[inline]
fn ext_ok(exts: &[Vec<u8>], lname: &[u8]) -> bool {
    exts.is_empty() || exts.iter().any(|x| lname.len() > x.len() && lname.ends_with(x))
}

/// Запрос заканчивается на компоненте d: имя d начинается с последнего сегмента,
/// предки совпадают со средними сегментами, предок на расстоянии k оканчивается первым.
fn ends_at(v: &Volume, d: u32, segs: &[Vec<u8>]) -> bool {
    let k = segs.len() - 1;
    if !v.lname(d).starts_with(&segs[k]) {
        return false;
    }
    let mut cur = d;
    for j in 1..=k {
        cur = v.ents[cur as usize].parent;
        if cur == NONE {
            return false;
        }
        let lc = v.lname(cur);
        if j < k {
            if lc != &segs[k - j][..] {
                return false;
            }
        } else if !lc.ends_with(&segs[0]) {
            return false;
        }
    }
    true
}

fn lpath_into(v: &Volume, i: u32, out: &mut Vec<u8>) {
    let mut s = String::new();
    v.path_into(i, &mut s);
    out.clear();
    fold_into(&s, out);
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Cand {
    pub k1: u32,
    pub plen: u32,
    pub vol: u32,
    pub idx: u32,
}

/// Корни (абсолютные пути) → индексы записей этого тома. None — корни заданы, но ни один не на этом томе.
pub fn resolve_roots(v: &Volume, roots: &[String]) -> Option<Vec<u32>> {
    if roots.is_empty() {
        return Some(Vec::new());
    }
    let mut out = Vec::new();
    for r in roots {
        let r = norm_path(r);
        if drive_of(&r).map(|d| d.eq_ignore_ascii_case(&v.drive)).unwrap_or(false) {
            if let Some(i) = v.resolve(&r) {
                if !out.contains(&i) {
                    out.push(i);
                }
            }
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Поиск по одному тому: (число совпадений, лучшие ≤ k кандидатов).
pub fn search_volume(v: &Volume, vol_no: u32, q: &Query, roots: &[String], k: usize) -> (u64, Vec<Cand>) {
    let Some(root_idx) = resolve_roots(v, roots) else {
        return (0, Vec::new());
    };
    let n = v.ents.len();
    let path_memos: Vec<Option<Memo>> = q
        .path_pats
        .iter()
        .map(|p| if matches!(p, Pat::PathSub(_)) { Some(Memo::new(n)) } else { None })
        .collect();
    let root_memo = if root_idx.is_empty() { None } else { Some(Memo::new(n)) };
    let path_tails: Vec<(Vec<u8>, bool)> = q
        .path_pats
        .iter()
        .map(|p| if let Pat::PathGlob(g) = p { g.name_tail() } else { (Vec::new(), false) })
        .collect();
    let nchunks = n.div_ceil(CHUNK);
    let parts: Vec<(u64, Vec<Cand>)> = (0..nchunks)
        .into_par_iter()
        .map(|c| {
            let lo = c * CHUNK;
            let hi = (lo + CHUNK).min(n);
            let re = q.regex.clone();
            let mut heap: BinaryHeap<Cand> = BinaryHeap::with_capacity(k.min(4096) + 1);
            let mut count = 0u64;
            let mut scratch: Vec<u32> = Vec::new();
            let mut pbuf: Vec<u8> = Vec::new();
            let mut sbuf = String::new();
            'ent: for i in lo..hi {
                let e = v.ents[i];
                if e.flags & HIDE != 0 || (q.files_only && e.flags & F_DIR != 0) {
                    continue;
                }
                let (o, l) = (e.off as usize, e.len as usize);
                let lname = &v.lnames[o..o + l];
                if !ext_ok(&q.exts, lname) {
                    continue;
                }
                for p in &q.name_pats {
                    let ok = match p {
                        Pat::Sub(f) => f.find(lname).is_some(),
                        Pat::Glob(g) => g.is_match(lname),
                        _ => true,
                    };
                    if !ok {
                        continue 'ent;
                    }
                }
                let i32_ = i as u32;
                if let Some(re) = &re {
                    let ok = if q.regex_path {
                        sbuf.clear();
                        v.path_into(i32_, &mut sbuf);
                        re.is_match(sbuf.as_bytes())
                    } else {
                        re.is_match(&v.names[o..o + l])
                    };
                    if !ok {
                        continue;
                    }
                }
                let mut have_lpath = false;
                for (pi, p) in q.path_pats.iter().enumerate() {
                    let ok = match p {
                        Pat::PathSub(segs) => {
                            if ends_at(v, i32_, segs) {
                                true
                            } else {
                                let m = path_memos[pi].as_ref().unwrap();
                                m.eval(v, e.parent, &mut scratch, |d| if ends_at(v, d, segs) { Some(true) } else { None }, |_, pv| pv)
                            }
                        }
                        Pat::PathGlob(g) => {
                            let (tail, exact) = &path_tails[pi];
                            if (*exact && lname != &tail[..]) || (!*exact && !lname.ends_with(tail)) {
                                false
                            } else {
                                if !have_lpath {
                                    lpath_into(v, i32_, &mut pbuf);
                                    have_lpath = true;
                                }
                                g.is_match(&pbuf)
                            }
                        }
                        _ => true,
                    };
                    if !ok {
                        continue 'ent;
                    }
                }
                if let Some(m) = &root_memo {
                    let ok = m.eval(v, e.parent, &mut scratch, |d| if root_idx.contains(&d) { Some(true) } else { None }, |_, pv| pv);
                    if !ok {
                        continue;
                    }
                }
                count += 1;
                if k == 0 {
                    continue;
                }
                let k1 = rank(q, lname);
                if heap.len() < k {
                    heap.push(Cand { k1, plen: v.path_len(i32_), vol: vol_no, idx: i32_ });
                } else {
                    let w = *heap.peek().unwrap();
                    if k1 > w.k1 {
                        continue;
                    }
                    let c = Cand { k1, plen: v.path_len(i32_), vol: vol_no, idx: i32_ };
                    if c < w {
                        heap.pop();
                        heap.push(c);
                    }
                }
            }
            (count, heap.into_vec())
        })
        .collect();
    let mut total = 0u64;
    let mut all = Vec::new();
    for (c, h) in parts {
        total += c;
        all.extend(h);
    }
    if all.len() > k {
        all.select_nth_unstable(k);
        all.truncate(k);
    }
    (total, all)
}

// ---------------------------------------------------------------- list

pub struct ListSpec {
    pub roots: Vec<String>,
    pub exts: Vec<Vec<u8>>,
    pub excl_dirs: Vec<Vec<u8>>,
    pub excl_paths: Vec<String>,
    pub skip_dot: bool,
    pub skip_cloud: bool,
    /// пропускать папки сборки/зависимостей по контексту (classify::SmartMarks)
    pub smart: bool,
}

/// Группа результатов `list`: каталог, число его детей в индексе, нужные файлы.
pub struct ListGroup {
    pub dir: String,
    pub dir_idx: u32,
    pub nchild: u32,
    pub names: Vec<String>,
    pub files: Vec<u32>,
    /// classify: по файлу — (тег, корень репозитория); пусто без classify
    pub tags: Vec<(Option<&'static str>, Option<std::sync::Arc<str>>)>,
}

pub struct ListOut {
    pub groups: Vec<ListGroup>,
    /// индексы корней списка на этом томе
    pub roots: Vec<u32>,
    /// smart_excludes: каталогов-«корней» пропуска и файлов, которые иначе попали бы в список
    pub smart_dirs: u64,
    pub smart_files: u64,
}

const ST_BLOCK: u8 = 1;
const ST_OK: u8 = 2;
const ST_SMART: u8 = 3;
const ST_SMART_ROOT: u8 = 4;

/// Файлы под корнями с нужными расширениями (без исключённых папок/путей, без точечных
/// папок, без облачных заглушек и точек повторной обработки; со smart — без папок
/// сборки/зависимостей), сгруппированные по каталогам.
pub fn list_volume(v: &Volume, spec: &ListSpec) -> ListOut {
    let empty = || ListOut { groups: Vec::new(), roots: Vec::new(), smart_dirs: 0, smart_files: 0 };
    let Some(roots) = resolve_roots(v, &spec.roots) else {
        return empty();
    };
    if roots.is_empty() {
        return empty();
    }
    let excl_idx: Vec<u32> = spec
        .excl_paths
        .iter()
        .map(|p| norm_path(p))
        .filter(|p| drive_of(p).map(|d| d.eq_ignore_ascii_case(&v.drive)).unwrap_or(false))
        .filter_map(|p| v.resolve(&p))
        .collect();
    let n = v.ents.len();
    // состояние каталога: 0 — неизвестно, BLOCK, OK, SMART (внутри пропущенной папки), SMART_ROOT
    let memo: Vec<AtomicU8> = {
        let mut z = std::mem::ManuallyDrop::new(vec![0u8; n]);
        // SAFETY: AtomicU8 имеет ту же раскладку, что и u8
        unsafe { Vec::from_raw_parts(z.as_mut_ptr() as *mut AtomicU8, z.len(), z.capacity()) }
    };
    let marks = if spec.smart { Some(crate::classify::SmartMarks::build(v)) } else { None };
    // Каталог — точка повторной обработки: папку OneDrive с данными на диске (облачный тег) обходим — в ней обычные
    // локальные файлы (на Windows 11 так лежат Документы и Рабочий стол при резервном копировании); junction, symlink
    // и папки-заглушки (данных нет) — нет. Тега в индексе нет (в записях USN-журнала его не бывает), спрашиваем диск;
    // результат запоминает dir_state, поэтому запрос — по разу на каталог, где лежат подходящие файлы.
    let cloud_dir_blocked = |flags: u16, x: u32| -> bool {
        if flags & F_CLOUD != 0 {
            return true;
        }
        match find_data(&v.path(x)) {
            Some((a, tag)) => a & A_CLOUD != 0 || (a & A_REPARSE != 0 && !is_cloud_tag(tag)),
            None => true,
        }
    };
    let user_excluded = |x: u32| -> bool {
        let e = v.ents[x as usize];
        if e.flags & HIDE != 0 {
            return true;
        }
        if spec.skip_cloud && e.flags & (F_REPARSE | F_CLOUD) != 0 && cloud_dir_blocked(e.flags, x) {
            return true;
        }
        let ln = v.lname(x);
        (spec.skip_dot && ln.first() == Some(&b'.')) || spec.excl_dirs.iter().any(|d| d[..] == *ln) || excl_idx.contains(&x)
    };
    let dir_state = |start: u32, scratch: &mut Vec<u32>| -> u8 {
        scratch.clear();
        let mut cur = start;
        let mut base;
        loop {
            if cur == NONE || cur as usize >= n {
                base = ST_BLOCK;
                break;
            }
            let m = memo[cur as usize].load(Relaxed);
            if m != 0 {
                base = m;
                break;
            }
            if roots.contains(&cur) {
                // явный корень разрешён всегда (даже если он сам «точечный» или node_modules)
                memo[cur as usize].store(ST_OK, Relaxed);
                base = ST_OK;
                break;
            }
            scratch.push(cur);
            if scratch.len() > 8192 {
                return ST_BLOCK;
            }
            cur = v.ents[cur as usize].parent;
        }
        while let Some(x) = scratch.pop() {
            let st = if base == ST_BLOCK || user_excluded(x) {
                ST_BLOCK
            } else if base == ST_SMART || base == ST_SMART_ROOT {
                ST_SMART
            } else if marks.as_ref().map(|m| m.skip(v, x)).unwrap_or(false) {
                ST_SMART_ROOT
            } else {
                ST_OK
            };
            memo[x as usize].store(st, Relaxed);
            base = st;
        }
        base
    };
    let nchunks = n.div_ceil(CHUNK);
    let hits: Vec<(Vec<u32>, u64)> = (0..nchunks)
        .into_par_iter()
        .map(|c| {
            let lo = c * CHUNK;
            let hi = (lo + CHUNK).min(n);
            let mut out = Vec::new();
            let mut smart_files = 0u64;
            let mut scratch = Vec::new();
            for i in lo..hi {
                let e = v.ents[i];
                if e.flags & (HIDE | F_DIR) != 0 {
                    continue;
                }
                // точки повторной обработки (файлы OneDrive с данными на диске, символические ссылки) решает
                // проверка на диске по атрибутам и тегу (ops::stat_group); здесь отсекаем только заглушки
                if spec.skip_cloud && e.flags & F_CLOUD != 0 {
                    continue;
                }
                let lname = &v.lnames[e.off as usize..e.off as usize + e.len as usize];
                if !ext_ok(&spec.exts, lname) {
                    continue;
                }
                if !excl_idx.is_empty() && excl_idx.contains(&(i as u32)) {
                    continue;
                }
                match dir_state(e.parent, &mut scratch) {
                    ST_OK => out.push(i as u32),
                    ST_SMART | ST_SMART_ROOT => smart_files += 1,
                    _ => {}
                }
            }
            (out, smart_files)
        })
        .collect();
    let smart_files: u64 = hits.iter().map(|h| h.1).sum();
    let smart_dirs = if spec.smart { memo.par_iter().filter(|m| m.load(Relaxed) == ST_SMART_ROOT).count() as u64 } else { 0 };
    let mut idx: Vec<(u32, u32)> = hits.into_iter().flat_map(|h| h.0).map(|i| (v.ents[i as usize].parent, i)).collect();
    idx.sort_unstable();
    let mut groups: Vec<(u32, Vec<u32>)> = Vec::new();
    for (p, i) in idx {
        match groups.last_mut() {
            Some((lp, l)) if *lp == p => l.push(i),
            _ => groups.push((p, vec![i])),
        }
    }
    let groups = groups
        .into_par_iter()
        .map(|(p, l)| ListGroup {
            dir: v.path(p),
            dir_idx: p,
            nchild: v.nchild[p as usize],
            names: l.iter().map(|&i| v.name(i).to_string()).collect(),
            files: l,
            tags: Vec::new(),
        })
        .collect();
    ListOut { groups, roots, smart_dirs, smart_files }
}

/// Записи с точно таким же именем (без учёта регистра).
pub fn same_name_volume(v: &Volume, lname: &[u8], files_only: bool) -> Vec<u32> {
    let n = v.ents.len();
    let nchunks = n.div_ceil(CHUNK);
    let parts: Vec<Vec<u32>> = (0..nchunks)
        .into_par_iter()
        .map(|c| {
            let lo = c * CHUNK;
            let hi = (lo + CHUNK).min(n);
            let mut out = Vec::new();
            for i in lo..hi {
                let e = v.ents[i];
                if e.len as usize != lname.len() || e.flags & HIDE != 0 || (files_only && e.flags & F_DIR != 0) {
                    continue;
                }
                if &v.lnames[e.off as usize..e.off as usize + e.len as usize] == lname {
                    out.push(i as u32);
                }
            }
            out
        })
        .collect();
    parts.into_iter().flatten().collect()
}

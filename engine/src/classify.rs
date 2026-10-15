//! Для операции `list`: «умные» исключения папок сборки/зависимостей по контексту
//! (smart_excludes) и пометка «чужого» кода (classify): submodule / gitignored / archive / foreign.
//!
//! Всё, что зависит от индекса, считается под блокировкой чтения тома; файлы git
//! (.git, .gitmodules, .gitignore, info/exclude, config, logs/HEAD) только читаются,
//! с ограничением размера — это недоверенный ввод.

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use rayon::prelude::*;
use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, OnceLock};

use crate::index::*;
use crate::search::ListGroup;

// ---------------------------------------------------------------- smart excludes

const B_DOTNET: u32 = 1 << 0; // *.csproj / *.vbproj / *.fsproj
const B_SLN: u32 = 1 << 1; // *.sln
const B_CARGO: u32 = 1 << 2; // Cargo.toml
const B_POM: u32 = 1 << 3; // pom.xml
const B_BUILDFILE: u32 = 1 << 4; // package.json, CMakeLists.txt, pyproject.toml, setup.py, build.gradle(.kts), meson.build
const B_UPROJECT: u32 = 1 << 5; // *.uproject
const B_VENDORFILE: u32 = 1 << 6; // composer.json, go.mod, Gemfile
const B_PKGCONFIG: u32 = 1 << 7; // packages.config
const B_PODFILE: u32 = 1 << 8; // Podfile
const B_PYVENV: u32 = 1 << 9; // pyvenv.cfg
const B_ASSETS: u32 = 1 << 10; // папка Assets (Unity)
const B_PROJSETTINGS: u32 = 1 << 11; // папка ProjectSettings (Unity)

fn marker_bits(l: &[u8], is_dir: bool) -> u32 {
    if is_dir {
        return match l {
            b"assets" => B_ASSETS,
            b"projectsettings" => B_PROJSETTINGS,
            _ => 0,
        };
    }
    let mut b = match l {
        b"cargo.toml" => B_CARGO,
        b"pom.xml" => B_POM,
        b"package.json" | b"cmakelists.txt" | b"pyproject.toml" | b"setup.py" | b"build.gradle" | b"build.gradle.kts" | b"meson.build" => B_BUILDFILE,
        b"composer.json" | b"go.mod" | b"gemfile" => B_VENDORFILE,
        b"packages.config" => B_PKGCONFIG,
        b"podfile" => B_PODFILE,
        b"pyvenv.cfg" => B_PYVENV,
        _ => 0,
    };
    if l.ends_with(b".csproj") || l.ends_with(b".vbproj") || l.ends_with(b".fsproj") {
        b |= B_DOTNET;
    } else if l.ends_with(b".sln") {
        b |= B_SLN;
    } else if l.ends_with(b".uproject") {
        b |= B_UPROJECT;
    }
    b
}

/// Маркеры проектов по каталогам: биты «в этом каталоге есть Cargo.toml / *.sln / Assets …».
pub struct SmartMarks {
    marks: HashMap<u32, u32>,
}

impl SmartMarks {
    /// Один параллельный проход по индексу тома (~10 мс на 4 млн записей).
    pub fn build(v: &Volume) -> SmartMarks {
        const CH: usize = 64 * 1024;
        let n = v.ents.len();
        let parts: Vec<Vec<(u32, u32)>> = (0..n.div_ceil(CH))
            .into_par_iter()
            .map(|c| {
                let mut out = Vec::new();
                for i in c * CH..((c + 1) * CH).min(n) {
                    let e = v.ents[i];
                    if e.flags & (F_DELETED | F_ROOT) != 0 || e.parent == NONE || e.len < 5 {
                        continue;
                    }
                    let b = marker_bits(&v.lnames[e.off as usize..e.off as usize + e.len as usize], e.flags & F_DIR != 0);
                    if b != 0 {
                        out.push((e.parent, b));
                    }
                }
                out
            })
            .collect();
        let mut marks: HashMap<u32, u32> = HashMap::new();
        for (p, b) in parts.into_iter().flatten() {
            *marks.entry(p).or_insert(0) |= b;
        }
        SmartMarks { marks }
    }

    /// Пропустить ли каталог d (со всем поддеревом) как папку сборки/зависимостей.
    pub fn skip(&self, v: &Volume, d: u32) -> bool {
        let l = v.lname(d);
        if matches!(l, b"node_modules" | b"bower_components" | b"jspm_packages" | b"__pycache__" | b"site-packages" | b"dist-packages") {
            return true;
        }
        if self.marks.get(&d).copied().unwrap_or(0) & B_PYVENV != 0 {
            return true; // корень виртуального окружения Python
        }
        let p = v.ents[d as usize].parent;
        if p == NONE {
            return false;
        }
        let m = self.marks.get(&p).copied().unwrap_or(0);
        if m == 0 {
            return false;
        }
        let unity = m & B_ASSETS != 0 && m & B_PROJSETTINGS != 0;
        match l {
            b"bin" => m & (B_DOTNET | B_SLN) != 0,
            b"obj" => m & (B_DOTNET | B_SLN) != 0 || unity,
            b"target" => m & (B_CARGO | B_POM) != 0,
            b"build" => m & B_BUILDFILE != 0 || unity,
            b"dist" | b"out" => m & B_BUILDFILE != 0,
            b"library" | b"temp" | b"logs" | b"builds" => unity,
            b"intermediate" | b"saved" | b"deriveddatacache" | b"binaries" => m & B_UPROJECT != 0,
            b"vendor" => m & B_VENDORFILE != 0,
            b"packages" => m & (B_SLN | B_PKGCONFIG) != 0,
            b"pods" => m & B_PODFILE != 0,
            _ => false,
        }
    }
}

// ---------------------------------------------------------------- small file helpers

/// Прочитать не больше cap байт. Файлы git — недоверенный ввод (движок может работать с
/// повышенными правами), поэтому символьные ссылки/точки повторной обработки не открываем.
fn read_capped(path: &str, cap: u64) -> Option<String> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(crate::util::long_path(path))
        .ok()?;
    let md = f.metadata().ok()?;
    if md.file_attributes() & (A_REPARSE | A_DIRECTORY) != 0 {
        return None;
    }
    let mut b = Vec::new();
    f.take(cap).read_to_end(&mut b).ok()?;
    let b = b.strip_prefix(&[0xEF, 0xBB, 0xBF][..]).unwrap_or(&b);
    Some(String::from_utf8_lossy(b).into_owned())
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

/// Лексическая нормализация пути: '/' → '\', без "." и "..".
fn normalize(p: &str) -> String {
    let p = p.replace('/', "\\");
    let (prefix, rest) = if p.starts_with(r"\\") {
        // UNC: \\server\share\...
        let mut it = p[2..].splitn(3, '\\');
        let srv = it.next().unwrap_or("");
        let share = it.next().unwrap_or("");
        (format!(r"\\{srv}\{share}"), it.next().unwrap_or("").to_string())
    } else if p.len() >= 2 && p.as_bytes()[1] == b':' {
        (p[..2].to_string(), p[2..].to_string())
    } else {
        (String::new(), p.clone())
    };
    let mut parts: Vec<&str> = Vec::new();
    for c in rest.split('\\') {
        match c {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            _ => parts.push(c),
        }
    }
    format!("{prefix}\\{}", parts.join("\\"))
}

fn is_abs(p: &str) -> bool {
    let b = p.as_bytes();
    (b.len() >= 2 && b[1] == b':') || p.starts_with('\\') || p.starts_with('/')
}

fn resolve_rel(base: &str, p: &str) -> String {
    if is_abs(p) {
        normalize(p)
    } else {
        normalize(&join(base, p))
    }
}

// ---------------------------------------------------------------- git identity

#[derive(Clone, Default)]
pub struct Identity {
    names: Vec<String>,
    emails: Vec<String>,
}

impl Identity {
    fn is_empty(&self) -> bool {
        self.names.is_empty() && self.emails.is_empty()
    }
    fn add_config(&mut self, text: &str) {
        let mut in_user = false;
        for raw in text.lines() {
            let l = raw.trim();
            if l.starts_with('[') {
                let sec = l[1..].split(']').next().unwrap_or("").trim().to_ascii_lowercase();
                in_user = sec == "user";
                continue;
            }
            if !in_user || l.starts_with('#') || l.starts_with(';') {
                continue;
            }
            let Some((k, val)) = l.split_once('=') else { continue };
            let k = k.trim().to_ascii_lowercase();
            let val = val.trim();
            let val = if let Some(q) = val.strip_prefix('"') {
                q.split('"').next().unwrap_or("")
            } else {
                val.split(['#', ';']).next().unwrap_or("")
            };
            let val = val.trim().to_lowercase();
            if val.is_empty() {
                continue;
            }
            match k.as_str() {
                "name" if !self.names.contains(&val) => self.names.push(val),
                "email" if !self.emails.contains(&val) => self.emails.push(val),
                _ => {}
            }
        }
    }
    fn matches(&self, name: &str, email: &str) -> bool {
        let (n, e) = (name.trim().to_lowercase(), email.trim().to_lowercase());
        (!n.is_empty() && self.names.contains(&n)) || (!e.is_empty() && self.emails.contains(&e))
    }
}

/// Глобальная личность git: `.gitconfig` и `.config\git\config` в домашней папке, %XDG_CONFIG_HOME%\git\config.
/// Домашняя папка, как у Git for Windows: %HOME%, иначе %HOMEDRIVE%%HOMEPATH%, иначе профиль пользователя
/// (известная папка — переменные окружения у повышенного процесса или задачи Планировщика бывают не теми).
pub fn global_identity() -> Identity {
    let mut id = Identity::default();
    let mut files = Vec::new();
    let mut homes: Vec<String> = Vec::new();
    let env = |k: &str| std::env::var(k).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    homes.extend(env("HOME"));
    if let (Some(d), Some(p)) = (env("HOMEDRIVE"), env("HOMEPATH")) {
        homes.push(format!("{d}{p}"));
    }
    homes.extend(crate::util::known_folder(&windows_sys::Win32::UI::Shell::FOLDERID_Profile).map(|p| p.to_string_lossy().into_owned()));
    homes.extend(env("USERPROFILE"));
    let mut seen: Vec<String> = Vec::new();
    for h in homes {
        if !is_abs(&h) || seen.contains(&h.to_lowercase()) {
            continue;
        }
        seen.push(h.to_lowercase());
        files.push(join(&h, ".gitconfig"));
        files.push(join(&h, r".config\git\config"));
    }
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME") {
        if !x.trim().is_empty() {
            files.push(join(x.trim(), r"git\config"));
        }
    }
    for f in files {
        if let Some(t) = read_capped(&f, 1 << 20) {
            id.add_config(&t);
        }
    }
    id
}

// ---------------------------------------------------------------- repositories

struct RepoPre {
    path: String,
    git_is_dir: bool,
    /// пути репозиториев-предков, у которых есть .gitmodules
    ancestor_modules: Vec<String>,
}

pub struct RepoInfo {
    pub root: Arc<str>,
    pub submodule: bool,
    pub foreign: bool,
    exclude: Option<Gitignore>,
}

fn has_child(v: &Volume, d: u32, lname: &[u8], want_dir: Option<bool>) -> Option<u32> {
    let c = v.find_child(d, lname)?;
    if !v.live(c) {
        return None;
    }
    match want_dir {
        Some(w) if v.is_dir(c) != w => None,
        _ => Some(c),
    }
}

/// .gitmodules предка перечисляет path = <путь, совпадающий с repo>?
fn gitmodules_lists(ancestor: &str, repo: &str) -> bool {
    let Some(t) = read_capped(&join(ancestor, ".gitmodules"), 1 << 20) else { return false };
    let want = normalize(repo).to_lowercase();
    for raw in t.lines() {
        let l = raw.trim();
        let Some((k, val)) = l.split_once('=') else { continue };
        if !k.trim().eq_ignore_ascii_case("path") {
            continue;
        }
        let val = val.trim().trim_matches('"');
        if val.is_empty() {
            continue;
        }
        if normalize(&join(ancestor, val)).to_lowercase() == want {
            return true;
        }
    }
    false
}

/// Сообщение журнала ссылок начинается с действия пользователя (своя работа в репозитории).
fn is_user_work(msg: &str) -> bool {
    let m = msg.trim_start().to_ascii_lowercase();
    ["commit", "merge", "rebase", "cherry-pick", "revert"].iter().any(|p| m.starts_with(p))
}

/// «Чужой» клон: logs/HEAD начинается с clone: и нет ни одной записи commit/merge/rebase/
/// cherry-pick/revert от локального пользователя. Нет личности или журнала — не «чужой».
fn is_foreign(gitdir: &str, common: &str, global: &Identity) -> bool {
    let mut id = global.clone();
    if let Some(t) = read_capped(&join(common, "config"), 1 << 20) {
        id.add_config(&t);
    }
    if id.is_empty() {
        return false;
    }
    let Some(log) = read_capped(&join(gitdir, r"logs\HEAD"), 16 << 20) else { return false };
    let mut cloned = false;
    for line in log.lines() {
        let Some((head, msg)) = line.split_once('\t') else { continue };
        if msg.trim_start().to_ascii_lowercase().starts_with("clone:") {
            cloned = true;
        }
        if !is_user_work(msg) {
            continue;
        }
        // "<old> <new> Имя <email> <время> <пояс>"
        let Some(lt) = head.rfind('<') else { continue };
        let Some(gt) = head[lt..].find('>') else { continue };
        let email = &head[lt + 1..lt + gt];
        let name = head[..lt].splitn(3, ' ').nth(2).unwrap_or("");
        if id.matches(name, email) {
            return false;
        }
    }
    cloned
}

fn load_gitignore(dir: &str, file: &str) -> Option<Gitignore> {
    let text = read_capped(file, 1 << 20)?;
    let mut b = GitignoreBuilder::new(dir);
    b.case_insensitive(true).ok()?;
    for line in text.lines() {
        let _ = b.add_line(None, line);
    }
    let g = b.build().ok()?;
    if g.is_empty() {
        None
    } else {
        Some(g)
    }
}

fn load_repo(pre: &RepoPre, global: &Identity) -> RepoInfo {
    let dotgit = join(&pre.path, ".git");
    let mut submodule = false;
    let gitdir: Option<String> = if pre.git_is_dir {
        Some(dotgit)
    } else {
        let g = read_capped(&dotgit, 4096)
            .and_then(|t| t.lines().find_map(|l| l.trim().strip_prefix("gitdir:").map(|x| x.trim().to_string())))
            .filter(|g| !g.is_empty())
            .map(|g| resolve_rel(&pre.path, &g));
        if let Some(gd) = &g {
            let low = gd.to_lowercase();
            // .git-файл в …\modules\… — подмодуль; в …\worktrees\… — рабочая копия (worktree)
            if low.contains(r"\modules\") && !low.contains(r"\worktrees\") {
                submodule = true;
            }
        }
        g
    };
    if !submodule {
        submodule = pre.ancestor_modules.iter().any(|a| gitmodules_lists(a, &pre.path));
    }
    let mut foreign = false;
    let mut exclude = None;
    if let Some(gd) = &gitdir {
        // у worktree общие config и info/exclude — в commondir
        let common = read_capped(&join(gd, "commondir"), 4096)
            .map(|t| t.lines().next().unwrap_or("").trim().to_string())
            .filter(|c| !c.is_empty())
            .map(|c| resolve_rel(gd, &c))
            .unwrap_or_else(|| gd.clone());
        exclude = load_gitignore(&pre.path, &join(&common, r"info\exclude"));
        if !submodule {
            foreign = is_foreign(gd, &common, global);
        }
    }
    RepoInfo { root: Arc::from(pre.path.as_str()), submodule, foreign, exclude }
}

// ---------------------------------------------------------------- classification

fn archive_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)^.+-(main|master|develop|dev|release|v?\d+(\.\d+){1,3}[\w.+-]*)$").unwrap())
}

#[derive(Default, Clone, Copy)]
pub struct TagCounts {
    pub submodule: u64,
    pub gitignored: u64,
    pub archive: u64,
    pub foreign: u64,
    pub none: u64,
    pub repos: u64,
}

impl TagCounts {
    pub fn add(&mut self, o: &TagCounts) {
        self.submodule += o.submodule;
        self.gitignored += o.gitignored;
        self.archive += o.archive;
        self.foreign += o.foreign;
        self.none += o.none;
        self.repos += o.repos;
    }
}

/// Ближайший предок-каталог с .git (папка или файл) или NONE; memo по каталогам.
fn repo_of(v: &Volume, memo: &mut HashMap<u32, u32>, d: u32) -> u32 {
    let mut stack = Vec::new();
    let mut cur = d;
    let res;
    loop {
        if cur == NONE || stack.len() > 8192 {
            res = NONE;
            break;
        }
        if let Some(&r) = memo.get(&cur) {
            res = r;
            break;
        }
        if has_child(v, cur, b".git", None).is_some() {
            res = cur;
            break;
        }
        stack.push(cur);
        cur = v.ents[cur as usize].parent;
    }
    memo.insert(cur, res);
    for x in stack {
        memo.insert(x, res);
    }
    res
}

/// Путь target относительно каталога base (через '/'), для сопоставления с .gitignore.
fn rel_path(v: &Volume, base: u32, target: u32) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    let mut cur = target;
    while cur != base {
        if cur == NONE || parts.len() > 4096 {
            return None;
        }
        parts.push(v.name(cur));
        cur = v.ents[cur as usize].parent;
    }
    parts.reverse();
    Some(parts.join("/"))
}

/// Решение по правилам игнорирования для target (файл или каталог) в репозитории repo:
/// от ближайшего .gitignore к корню (первое совпадение решает, «!» учитывается), затем info/exclude.
fn rules_ignore(v: &Volume, gi: &HashMap<u32, Gitignore>, info: &RepoInfo, repo: u32, target: u32, is_dir: bool) -> bool {
    let mut lvl = v.ents[target as usize].parent;
    loop {
        if lvl == NONE {
            return false;
        }
        if let Some(g) = gi.get(&lvl) {
            if let Some(rel) = rel_path(v, lvl, target) {
                let m = g.matched(&rel, is_dir);
                if m.is_ignore() {
                    return true;
                }
                if m.is_whitelist() {
                    return false;
                }
            }
        }
        if lvl == repo {
            break;
        }
        lvl = v.ents[lvl as usize].parent;
    }
    if let Some(ex) = &info.exclude {
        if let Some(rel) = rel_path(v, repo, target) {
            return ex.matched(&rel, is_dir).is_ignore();
        }
    }
    false
}

/// Каталог проигнорирован (он сам или предок ниже корня репозитория) — memo.
fn dir_ignored(v: &Volume, gi: &HashMap<u32, Gitignore>, info: &RepoInfo, repo: u32, d: u32, memo: &mut HashMap<u32, bool>) -> bool {
    let mut chain = Vec::new();
    let mut cur = d;
    let mut base = false;
    while cur != repo && cur != NONE && chain.len() < 4096 {
        if let Some(&b) = memo.get(&cur) {
            base = b;
            break;
        }
        chain.push(cur);
        cur = v.ents[cur as usize].parent;
    }
    for &x in chain.iter().rev() {
        base = base || rules_ignore(v, gi, info, repo, x, true);
        memo.insert(x, base);
    }
    base
}

/// Пометить файлы групп: tags[k] = (тег, корень репозитория). Возвращает счётчики.
pub fn classify_groups(v: &Volume, roots: &[u32], groups: &mut [ListGroup]) -> TagCounts {
    // 1. репозиторий каждого каталога
    let mut repo_memo: HashMap<u32, u32> = HashMap::new();
    let group_repo: Vec<u32> = groups.iter().map(|g| repo_of(v, &mut repo_memo, g.dir_idx)).collect();
    let mut repos: Vec<u32> = group_repo.iter().copied().filter(|&r| r != NONE).collect();
    repos.sort_unstable();
    repos.dedup();
    // 2. сведения о репозиториях (предки с .gitmodules — по индексу), затем чтение файлов параллельно
    let pres: Vec<RepoPre> = repos
        .iter()
        .map(|&r| {
            let mut ancestor_modules = Vec::new();
            let mut a = repo_of(v, &mut repo_memo, v.ents[r as usize].parent);
            while a != NONE && ancestor_modules.len() < 32 {
                if has_child(v, a, b".gitmodules", Some(false)).is_some() {
                    ancestor_modules.push(v.path(a));
                }
                a = repo_of(v, &mut repo_memo, v.ents[a as usize].parent);
            }
            let git_is_dir = has_child(v, r, b".git", None).map(|c| v.is_dir(c)).unwrap_or(true);
            RepoPre { path: v.path(r), git_is_dir, ancestor_modules }
        })
        .collect();
    let global = global_identity();
    let infos: Vec<RepoInfo> = pres.par_iter().map(|p| load_repo(p, &global)).collect();
    let info_of: HashMap<u32, &RepoInfo> = repos.iter().copied().zip(infos.iter()).collect();
    // 3. .gitignore на пути от каталогов групп к корням (не нужны для подмодулей)
    let mut gi_dirs: Vec<u32> = Vec::new();
    {
        let mut seen: HashMap<u32, ()> = HashMap::new();
        for (g, &r) in groups.iter().zip(&group_repo) {
            if r == NONE || info_of[&r].submodule {
                continue;
            }
            let mut cur = g.dir_idx;
            loop {
                if seen.insert(cur, ()).is_some() {
                    break;
                }
                if has_child(v, cur, b".gitignore", Some(false)).is_some() {
                    gi_dirs.push(cur);
                }
                if cur == r || cur == NONE {
                    break;
                }
                cur = v.ents[cur as usize].parent;
            }
        }
    }
    let gi_paths: Vec<(u32, String)> = gi_dirs.iter().map(|&d| (d, v.path(d))).collect();
    let gi: HashMap<u32, Gitignore> = gi_paths
        .par_iter()
        .filter_map(|(d, p)| load_gitignore(p, &join(p, ".gitignore")).map(|g| (*d, g)))
        .collect();
    // 4. проигнорированные каталоги (memo), «архивные» каталоги вне репозиториев
    let mut dmemo: HashMap<u32, bool> = HashMap::new();
    let mut group_dir_ign = vec![false; groups.len()];
    let mut group_archive = vec![false; groups.len()];
    let mut amemo: HashMap<u32, bool> = HashMap::new();
    let re = archive_re();
    for (k, g) in groups.iter().enumerate() {
        let r = group_repo[k];
        if r != NONE {
            if !info_of[&r].submodule {
                group_dir_ign[k] = dir_ignored(v, &gi, info_of[&r], r, g.dir_idx, &mut dmemo);
            }
        } else {
            // предок между корнем списка (не включая его) и файлом с именем вида name-main / name-1.2.3
            let mut chain = Vec::new();
            let mut cur = g.dir_idx;
            let mut base = false;
            while cur != NONE && !roots.contains(&cur) && chain.len() < 4096 {
                if let Some(&b) = amemo.get(&cur) {
                    base = b;
                    break;
                }
                chain.push(cur);
                cur = v.ents[cur as usize].parent;
            }
            for &x in chain.iter().rev() {
                base = base || re.is_match(v.name(x));
                amemo.insert(x, base);
            }
            group_archive[k] = base;
        }
    }
    // 5. теги файлов (параллельно, всё выше — только чтение)
    let counts: Vec<TagCounts> = groups
        .par_iter_mut()
        .enumerate()
        .map(|(k, g)| {
            let mut c = TagCounts::default();
            let r = group_repo[k];
            let info = if r != NONE { Some(info_of[&r]) } else { None };
            g.tags = g
                .files
                .iter()
                .map(|&f| {
                    let tag = match info {
                        Some(i) if i.submodule => Some("submodule"),
                        Some(i) => {
                            if group_dir_ign[k] || rules_ignore(v, &gi, i, r, f, false) {
                                Some("gitignored")
                            } else if i.foreign {
                                Some("foreign")
                            } else {
                                None
                            }
                        }
                        None if group_archive[k] => Some("archive"),
                        None => None,
                    };
                    match tag {
                        Some("submodule") => c.submodule += 1,
                        Some("gitignored") => c.gitignored += 1,
                        Some("archive") => c.archive += 1,
                        Some(_) => c.foreign += 1,
                        None => c.none += 1,
                    }
                    (tag, info.map(|i| i.root.clone()))
                })
                .collect();
            c
        })
        .collect();
    let mut total = TagCounts { repos: repos.len() as u64, ..Default::default() };
    for c in &counts {
        total.add(c);
    }
    total
}

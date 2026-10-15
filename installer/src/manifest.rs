//! What the installer ships: `installer/payload.txt` (program files) and `installer/python.txt` (the pinned
//! Python runtime). Shared by the installer (dev mode, SEMSEARCH_PAYLOAD_DIR) and the `pack` build tool.
//!
//! payload.txt format:
//!   * one glob per line, relative to the repository root, `/` or `\` as the separator;
//!   * a line starting with `#` is a comment; blank lines are ignored;
//!   * `*` and `?` stay within one folder, `**` spans folders, `[abc]` is a character class; case is ignored;
//!   * a line ending with `/` means everything under that folder (`ui/` is `ui/**`);
//!   * `!glob` excludes files matched by the other lines (order does not matter);
//!   * every include line must match at least one file: a missing file (say, SemSearch.exe not built yet)
//!     fails the build instead of producing a broken installer.
//! The walk never enters .git, .claude, .venv, target, __pycache__ or node_modules (at any depth), nor the
//! repository's data\ (personal data of the old layout), dist\ and installer\.cache\ (build outputs).
//!
//! python.txt format: `url = <https URL of the .tar.gz>` and `sha256 = <hex>` lines, `#` comments.

use std::fs;
use std::path::{Path, PathBuf};

use globset::{Glob, GlobBuilder, GlobMatcher, GlobSet, GlobSetBuilder};

/// Folders the walk never enters, at any depth (compared ignoring case).
const SKIP_ANYWHERE: &[&str] = &[".git", ".claude", ".venv", "target", "__pycache__", "node_modules"];
/// Folders the walk never enters, relative to the repository root.
const SKIP_AT_ROOT: &[&str] = &["data", "dist", "installer/.cache"];

/// Files of the payload: repository-relative paths with `/`, sorted.
pub fn payload_files(repo: &Path) -> Result<Vec<String>, String> {
    let list_path = repo.join("installer").join("payload.txt");
    let text = fs::read_to_string(&list_path).map_err(|e| format!("{}: {e}", list_path.display()))?;
    let (includes, exclude) = parse_payload_list(&text)?;
    let mut all = Vec::new();
    walk(repo, "", &mut all).map_err(|e| format!("{}: {e}", repo.display()))?;
    let mut hits = vec![0usize; includes.len()];
    let mut out = Vec::new();
    for rel in all {
        let mut matched = false;
        for (i, (_, m)) in includes.iter().enumerate() {
            if m.is_match(&rel) {
                hits[i] += 1;
                matched = true;
            }
        }
        if matched && !exclude.is_match(&rel) {
            out.push(rel);
        }
    }
    let missing: Vec<&str> = includes.iter().zip(&hits).filter(|(_, n)| **n == 0).map(|((p, _), _)| p.as_str()).collect();
    if !missing.is_empty() {
        return Err(format!("payload.txt: no files match {}", missing.join(", ")));
    }
    out.sort();
    Ok(out)
}

fn glob(pattern: &str) -> Result<Glob, String> {
    GlobBuilder::new(pattern)
        .case_insensitive(true)
        .literal_separator(true)
        .backslash_escape(false)
        .build()
        .map_err(|e| format!("payload.txt: bad pattern {pattern:?}: {e}"))
}

#[allow(clippy::type_complexity)]
fn parse_payload_list(text: &str) -> Result<(Vec<(String, GlobMatcher)>, GlobSet), String> {
    let mut includes = Vec::new();
    let mut exclude = GlobSetBuilder::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (negate, pat) = match line.strip_prefix('!') {
            Some(rest) => (true, rest.trim()),
            None => (false, line),
        };
        let mut pat = pat.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_string();
        if pat.ends_with('/') {
            pat.push_str("**");
        }
        if pat.is_empty() {
            continue;
        }
        let g = glob(&pat)?;
        if negate {
            exclude.add(g);
        } else {
            includes.push((pat, g.compile_matcher()));
        }
    }
    if includes.is_empty() {
        return Err("payload.txt lists no files".into());
    }
    let exclude = exclude.build().map_err(|e| format!("payload.txt: {e}"))?;
    Ok((includes, exclude))
}

fn walk(dir: &Path, rel: &str, out: &mut Vec<String>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
        // file_type() does not follow links: symlinks and junctions are skipped
        let ft = entry.file_type()?;
        if ft.is_dir() {
            let skip = SKIP_ANYWHERE.iter().any(|s| s.eq_ignore_ascii_case(&name))
                || SKIP_AT_ROOT.iter().any(|s| s.eq_ignore_ascii_case(&child));
            if !skip {
                walk(&entry.path(), &child, out)?;
            }
        } else if ft.is_file() {
            out.push(child);
        }
    }
    Ok(())
}

/// The pinned Python runtime (installer/python.txt).
pub struct PythonPin {
    pub url: String,
    pub sha256: String,
}

impl PythonPin {
    pub fn read(repo: &Path) -> Result<PythonPin, String> {
        let path = repo.join("installer").join("python.txt");
        let text = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let (mut url, mut sha) = (None, None);
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once('=') {
                match k.trim() {
                    "url" => url = Some(v.trim().to_string()),
                    "sha256" => sha = Some(v.trim().to_ascii_lowercase()),
                    _ => {}
                }
            }
        }
        match (url, sha) {
            (Some(url), Some(sha256)) if sha256.len() == 64 => Ok(PythonPin { url, sha256 }),
            _ => Err(format!("{}: needs `url = …` and `sha256 = <64 hex digits>`", path.display())),
        }
    }

    /// File name of the archive: the last URL segment, percent-decoded (`%2B` -> `+`).
    pub fn file_name(&self) -> String {
        let last = self.url.rsplit('/').next().unwrap_or("python.tar.gz");
        percent_decode(last)
    }

    /// `<repo>\installer\.cache\<file name>`
    pub fn cache_path(&self, repo: &Path) -> PathBuf {
        repo.join("installer").join(".cache").join(self.file_name())
    }
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

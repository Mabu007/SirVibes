use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

/// Lexically remove `.` and `..` components without touching the filesystem.
pub fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Canonicalize as much of the path as exists (resolving symlinks), keeping the
/// not-yet-existing tail. Needed because files the agent is about to create
/// cannot be canonicalized yet.
pub fn canonical_ish(p: &Path) -> PathBuf {
    let normalized = normalize(p);
    let mut existing = normalized.clone();
    let mut tail: Vec<OsString> = Vec::new();
    loop {
        if existing.exists() {
            return match existing.canonicalize() {
                Ok(mut out) => {
                    for part in tail.iter().rev() {
                        out.push(part);
                    }
                    out
                }
                Err(_) => normalized,
            };
        }
        match existing.file_name() {
            Some(n) => tail.push(n.to_os_string()),
            None => return normalized,
        }
        if !existing.pop() {
            return normalized;
        }
    }
}

#[derive(Clone, Debug)]
pub struct Workspace {
    pub root: PathBuf,
}

impl Workspace {
    pub fn open(root: &str) -> Result<Self, String> {
        let p = PathBuf::from(expand_home(root));
        let c = p
            .canonicalize()
            .map_err(|e| format!("workspace '{}' is not accessible: {}", root, e))?;
        if !c.is_dir() {
            return Err(format!("workspace '{}' is not a directory", root));
        }
        Ok(Self { root: c })
    }

    /// Resolve a model-supplied path against the workspace root.
    pub fn resolve(&self, raw: &str) -> PathBuf {
        let expanded = expand_home(raw);
        let r = PathBuf::from(&expanded);
        let joined = if r.is_absolute() { r } else { self.root.join(r) };
        canonical_ish(&joined)
    }

    pub fn contains(&self, p: &Path) -> bool {
        p == self.root.as_path() || p.starts_with(&self.root)
    }

    /// Path relative to the workspace root, for display.
    pub fn rel(&self, p: &Path) -> String {
        p.strip_prefix(&self.root)
            .map(|r| r.to_string_lossy().to_string())
            .unwrap_or_else(|_| p.to_string_lossy().to_string())
    }
}

pub fn expand_home(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return Path::new(&home).join(rest).to_string_lossy().to_string();
        }
    }
    if raw == "~" {
        if let Some(home) = std::env::var_os("HOME") {
            return home.to_string_lossy().to_string();
        }
    }
    raw.to_string()
}

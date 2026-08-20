//! Generic filesystem capability. Nothing here knows about video; it is the
//! same set of primitives any agent would need to inspect and author files.

use crate::workspace::Workspace;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

const MAX_READ_BYTES: usize = 200_000;
const MAX_ENTRIES: usize = 500;

#[derive(Serialize)]
pub struct Entry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: u64,
    pub modified_ms: u64,
}

pub fn modified_ms(meta: &std::fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn arg_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing required argument '{}'", key))
}

pub fn list(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let raw = args.get("path").and_then(Value::as_str).unwrap_or(".");
    let dir = ws.resolve(raw);
    let recursive = args
        .get("recursive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", raw));
    }

    let mut entries = Vec::new();
    let mut truncated = false;
    let walker = walkdir::WalkDir::new(&dir)
        .max_depth(if recursive { 6 } else { 1 })
        .follow_links(false)
        .sort_by_file_name();
    for item in walker.into_iter().filter_entry(|e| !is_noise(e.path())) {
        let item = match item {
            Ok(i) => i,
            Err(_) => continue,
        };
        if item.path() == dir {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            truncated = true;
            break;
        }
        let meta = match item.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        entries.push(Entry {
            name: item.file_name().to_string_lossy().to_string(),
            path: ws.rel(item.path()),
            is_dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified_ms: modified_ms(&meta),
        });
    }

    Ok(serde_json::json!({
        "directory": ws.rel(&dir),
        "entries": entries,
        "truncated": truncated,
    }))
}

fn is_noise(p: &Path) -> bool {
    match p.file_name().and_then(|n| n.to_str()) {
        Some(name) => {
            matches!(name, "node_modules" | ".git" | "target" | "__pycache__" | ".venv")
                || (name.starts_with('.') && name.len() > 1 && p.is_dir())
        }
        None => false,
    }
}

pub fn read(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let raw = arg_str(args, "path")?;
    let path = ws.resolve(raw);
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;
    if meta.is_dir() {
        return Err(format!("'{}' is a directory; use fs_list", raw));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;
    let truncated = bytes.len() > MAX_READ_BYTES;
    let slice = if truncated {
        &bytes[..MAX_READ_BYTES]
    } else {
        &bytes[..]
    };
    let text = match std::str::from_utf8(slice) {
        Ok(t) => t.to_string(),
        Err(_) => {
            return Err(format!(
                "'{}' is not a UTF-8 text file ({} bytes). Use the shell (ffprobe, exiftool, …) to inspect binary media.",
                raw,
                meta.len()
            ))
        }
    };
    Ok(serde_json::json!({
        "path": ws.rel(&path),
        "bytes": meta.len(),
        "truncated": truncated,
        "content": text,
    }))
}

pub fn write(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let raw = arg_str(args, "path")?;
    let content = arg_str(args, "content")?;
    let path = ws.resolve(raw);
    let existed = path.exists();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {}", raw, e))?;
    }
    std::fs::write(&path, content).map_err(|e| format!("cannot write '{}': {}", raw, e))?;
    Ok(serde_json::json!({
        "path": ws.rel(&path),
        "bytes_written": content.len(),
        "created": !existed,
    }))
}

pub fn edit(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let raw = arg_str(args, "path")?;
    let old = arg_str(args, "old_text")?;
    let new = arg_str(args, "new_text")?;
    let replace_all = args
        .get("replace_all")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let path = ws.resolve(raw);
    let current =
        std::fs::read_to_string(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;
    let count = current.matches(old).count();
    if count == 0 {
        return Err(format!("old_text was not found in '{}'", raw));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "old_text appears {} times in '{}'; pass replace_all or include more context",
            count, raw
        ));
    }
    let updated = if replace_all {
        current.replace(old, new)
    } else {
        current.replacen(old, new, 1)
    };
    std::fs::write(&path, &updated).map_err(|e| format!("cannot write '{}': {}", raw, e))?;
    Ok(serde_json::json!({
        "path": ws.rel(&path),
        "replacements": if replace_all { count } else { 1 },
        "bytes": updated.len(),
    }))
}

pub fn mkdir(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let raw = arg_str(args, "path")?;
    let path = ws.resolve(raw);
    std::fs::create_dir_all(&path).map_err(|e| format!("cannot create '{}': {}", raw, e))?;
    Ok(serde_json::json!({ "path": ws.rel(&path), "created": true }))
}

pub fn stat(ws: &Workspace, args: &Value) -> Result<Value, String> {
    let raw = arg_str(args, "path")?;
    let path = ws.resolve(raw);
    match std::fs::metadata(&path) {
        Ok(meta) => Ok(serde_json::json!({
            "path": ws.rel(&path),
            "absolute_path": path.to_string_lossy(),
            "exists": true,
            "is_dir": meta.is_dir(),
            "size": meta.len(),
            "modified_ms": modified_ms(&meta),
        })),
        Err(_) => Ok(serde_json::json!({
            "path": ws.rel(&path),
            "absolute_path": path.to_string_lossy(),
            "exists": false,
        })),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn workspace(name: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("eplug-fs-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Workspace::open(dir.to_str().unwrap()).unwrap()
    }

    #[test]
    fn write_read_edit_round_trip() {
        let ws = workspace("roundtrip");
        let w = write(&ws, &json!({ "path": "notes/plan.md", "content": "one\ntwo\n" })).unwrap();
        assert_eq!(w["created"], true);
        assert_eq!(w["path"], "notes/plan.md");

        let r = read(&ws, &json!({ "path": "notes/plan.md" })).unwrap();
        assert_eq!(r["content"], "one\ntwo\n");

        edit(
            &ws,
            &json!({ "path": "notes/plan.md", "old_text": "two", "new_text": "three" }),
        )
        .unwrap();
        let r = read(&ws, &json!({ "path": "notes/plan.md" })).unwrap();
        assert_eq!(r["content"], "one\nthree\n");
    }

    #[test]
    fn ambiguous_edits_are_refused_rather_than_guessed() {
        let ws = workspace("ambiguous");
        write(&ws, &json!({ "path": "a.txt", "content": "x\nx\n" })).unwrap();
        let err = edit(&ws, &json!({ "path": "a.txt", "old_text": "x", "new_text": "y" }))
            .unwrap_err();
        assert!(err.contains("appears 2 times"));
        let ok = edit(
            &ws,
            &json!({ "path": "a.txt", "old_text": "x", "new_text": "y", "replace_all": true }),
        )
        .unwrap();
        assert_eq!(ok["replacements"], 2);
    }

    #[test]
    fn binary_files_are_rejected_with_a_useful_message() {
        let ws = workspace("binary");
        std::fs::write(ws.root.join("clip.mp4"), [0u8, 159, 146, 150, 255]).unwrap();
        let err = read(&ws, &json!({ "path": "clip.mp4" })).unwrap_err();
        assert!(err.contains("not a UTF-8 text file"));
        assert!(err.contains("ffprobe"));
    }

    #[test]
    fn listing_reports_entries_and_skips_noise() {
        let ws = workspace("listing");
        write(&ws, &json!({ "path": "out/final.mp4", "content": "x" })).unwrap();
        write(&ws, &json!({ "path": "node_modules/dep/index.js", "content": "x" })).unwrap();
        let root = list(&ws, &json!({ "path": "." })).unwrap();
        let names: Vec<String> = root["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&"out".to_string()));
        assert!(!names.contains(&"node_modules".to_string()));

        let deep = list(&ws, &json!({ "path": ".", "recursive": true })).unwrap();
        let paths: Vec<String> = deep["entries"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["path"].as_str().unwrap().to_string())
            .collect();
        assert!(paths.contains(&"out/final.mp4".to_string()));
    }

    #[test]
    fn stat_distinguishes_missing_from_present() {
        let ws = workspace("stat");
        write(&ws, &json!({ "path": "render.mp4", "content": "0123456789" })).unwrap();
        let there = stat(&ws, &json!({ "path": "render.mp4" })).unwrap();
        assert_eq!(there["exists"], true);
        assert_eq!(there["size"], 10);
        let missing = stat(&ws, &json!({ "path": "nope.mp4" })).unwrap();
        assert_eq!(missing["exists"], false);
    }

    #[test]
    fn paths_resolve_against_the_workspace_root() {
        let ws = workspace("resolve");
        write(&ws, &json!({ "path": "deep/dir/file.txt", "content": "hi" })).unwrap();
        assert!(ws.root.join("deep/dir/file.txt").exists());
        assert!(ws.contains(&ws.resolve("deep/../deep/dir/file.txt")));
        assert!(!ws.contains(&ws.resolve("../escape.txt")));
    }
}

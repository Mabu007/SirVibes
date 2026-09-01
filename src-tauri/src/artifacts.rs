//! Artifact detection: files in the workspace that appeared or changed while
//! the agent was working. The artifact is the product, so the UI surfaces these
//! rather than trying to represent the work in some other way.

use crate::tools_fs::modified_ms;
use crate::workspace::Workspace;
use serde::Serialize;

const ARTIFACT_EXTS: &[&str] = &[
    // video
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mpg", "mpeg", "wmv", "flv", "gif", // audio
    "wav", "mp3", "aac", "flac", "m4a", "ogg", "opus", // images
    "png", "jpg", "jpeg", "webp", "tiff", "bmp", "svg", // subtitles & documents
    "srt", "vtt", "md", "txt", "json", "csv", "edl", "xml", "fcpxml", "otio",
];

/// Filesystem timestamps come from a coarse kernel clock and can read a few
/// milliseconds *behind* a `SystemTime::now()` taken just before the write.
/// Without a grace window, a file the agent creates in the first instants of a
/// turn looks older than the turn and is missed.
const CLOCK_GRACE_MS: u64 = 1_000;

#[derive(Serialize, Clone)]
pub struct Artifact {
    pub name: String,
    pub path: String,
    pub absolute_path: String,
    pub size: u64,
    pub modified_ms: u64,
    pub kind: String,
}

pub fn scan(ws: &Workspace, since_ms: u64) -> Vec<Artifact> {
    let threshold = since_ms.saturating_sub(CLOCK_GRACE_MS);
    let mut found = Vec::new();
    for entry in walkdir::WalkDir::new(&ws.root)
        .max_depth(6)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            !(name.starts_with('.') && name != ".")
                && !matches!(name.as_ref(), "node_modules" | "target" | "__pycache__")
        })
        .filter_map(Result::ok)
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let ext = entry
            .path()
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        if !ARTIFACT_EXTS.contains(&ext.as_str()) {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let modified = modified_ms(&meta);
        if modified < threshold {
            continue;
        }
        found.push(Artifact {
            name: entry.file_name().to_string_lossy().to_string(),
            path: ws.rel(entry.path()),
            absolute_path: entry.path().to_string_lossy().to_string(),
            size: meta.len(),
            modified_ms: modified,
            kind: kind_of(&ext).to_string(),
        });
    }
    found.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    found.truncate(50);
    found
}

fn kind_of(ext: &str) -> &'static str {
    match ext {
        "mp4" | "mov" | "mkv" | "webm" | "avi" | "m4v" | "mpg" | "mpeg" | "wmv" | "flv" | "gif" => {
            "video"
        }
        "wav" | "mp3" | "aac" | "flac" | "m4a" | "ogg" | "opus" => "audio",
        "png" | "jpg" | "jpeg" | "webp" | "tiff" | "bmp" | "svg" => "image",
        "srt" | "vtt" => "subtitles",
        _ => "document",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> Workspace {
        let dir = std::env::temp_dir().join(format!("eplug-artifacts-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Workspace::open(dir.to_str().unwrap()).unwrap()
    }

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[test]
    fn finds_media_created_during_the_turn_and_classifies_it() {
        let ws = workspace("basic");
        let since = now_ms();
        std::fs::create_dir_all(ws.root.join("out")).unwrap();
        std::fs::write(ws.root.join("out/short-01.mp4"), "video").unwrap();
        std::fs::write(ws.root.join("captions.srt"), "1\n").unwrap();
        std::fs::write(ws.root.join("edit-plan.md"), "plan").unwrap();
        std::fs::write(ws.root.join("scratch.bin"), "junk").unwrap();

        let found = scan(&ws, since);
        let names: Vec<&str> = found.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"short-01.mp4"));
        assert!(names.contains(&"captions.srt"));
        assert!(names.contains(&"edit-plan.md"));
        assert!(!names.contains(&"scratch.bin"));

        let video = found.iter().find(|a| a.name == "short-01.mp4").unwrap();
        assert_eq!(video.kind, "video");
        assert_eq!(video.path, "out/short-01.mp4");
        assert!(video.absolute_path.ends_with("out/short-01.mp4"));

        let subs = found.iter().find(|a| a.name == "captions.srt").unwrap();
        assert_eq!(subs.kind, "subtitles");
    }

    #[test]
    fn ignores_files_that_predate_the_turn() {
        let ws = workspace("predate");
        std::fs::write(ws.root.join("source.mp4"), "old").unwrap();
        // Anything modified after this instant is "new"; the source is not.
        let since = now_ms() + 5_000;
        assert!(scan(&ws, since).is_empty());
    }

    #[test]
    fn skips_hidden_and_dependency_directories() {
        let ws = workspace("noise");
        let since = now_ms();
        for dir in ["node_modules/pkg", ".cache"] {
            std::fs::create_dir_all(ws.root.join(dir)).unwrap();
            std::fs::write(ws.root.join(dir).join("thing.png"), "x").unwrap();
        }
        std::fs::write(ws.root.join("thumb.png"), "x").unwrap();
        let found = scan(&ws, since);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "thumb.png");
    }
}

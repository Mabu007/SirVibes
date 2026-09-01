//! Seeing. The agent's own model is chosen for reasoning and tool calling, and
//! may not be able to look at anything at all — so looking is a tool of its
//! own, and it always runs on the same vision model rather than on whatever
//! the user happens to have selected.
//!
//! This is how a reference gets understood. The user drops in a still, a frame,
//! a poster, a clip they like and says "make it look like this"; `see` turns
//! that into words specific enough for the agent — or for a generative model it
//! commissions — to work from.

use crate::generate::text_of;
use crate::model;
use crate::workspace::Workspace;
use base64::Engine;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// Every look goes through Qwen. Vision is a fixed capability of the runtime,
/// not something the agent chooses per call, so the answers stay consistent
/// whatever model the user is running the agent on.
pub const DEFAULT_MODEL: &str = "qwen/qwen3.8-flash";

/// Qwen's current vision line, newest first. A model can be momentarily
/// rate-limited upstream or pulled from the catalogue, and "the agent cannot
/// see today" is not an acceptable way for that to show up — so when the user
/// has not named a model themselves, the next one takes the look. A model they
/// did name is used as given and never quietly swapped.
const LINE: &[&str] = &[DEFAULT_MODEL, "qwen/qwen3.8-27b", "qwen/qwen3.8-max"];

/// An image is sent inline, so it has to be small enough to be a request body.
const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
/// A look is a look, not a batch job.
const MAX_IMAGES: usize = 8;
/// Frames pulled from a reference clip, when the user points at a video.
const DEFAULT_FRAMES: usize = 4;

const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "gif", "bmp"];
const VIDEO_EXTS: &[&str] = &[
    "mp4", "mov", "mkv", "webm", "avi", "m4v", "mpg", "mpeg", "wmv", "flv",
];

/// What the vision model is for. It is not being asked to admire anything: its
/// answer is working material for whoever acts on it next.
const SYSTEM: &str = "You are the eyes of a production agent that cannot see. \
Describe only what is actually in the image — never guess at what is outside the frame, \
and say plainly when something is unreadable or ambiguous. \
Be concrete and specific: name things, quote text exactly as it appears, give positions and \
proportions. Lead with one line saying what this is, then the detail. No preamble, no praise, \
no advice about what the user should do.";

/// The extra framing for a reference the user wants matched. A description is
/// not enough here — what comes back has to be rebuildable.
const STYLE: &str = "\n\nThis image is a style reference: someone wants their own work to look \
like it. Break the look down into specifics that could be rebuilt from your answer alone:\n\
- palette: the actual colours, as hex, and where each one is used\n\
- typography: serif or sans, weight, case, tracking, size relative to the frame, alignment\n\
- layout and composition: where things sit in the frame, margins, spacing, what dominates\n\
- light and grade: contrast, highlight and shadow treatment, colour cast, saturation\n\
- texture and finish: grain, noise, blur, glow, halation, chromatic aberration, edges\n\
- motion, if there is any evidence of it: blur, trails, smear, direction\n\
- anything distinctive that would be missed if it were left out\n\
End with one short paragraph a generative model could be given verbatim as a style prompt.";

pub async fn see(
    ws: &Workspace,
    api_key: &str,
    configured_model: &str,
    args: &Value,
) -> Result<Value, String> {
    let requested = paths_of(args)?;
    let question = args
        .get("question")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .unwrap_or("What is this? Describe it.");
    let style = args
        .get("mode")
        .and_then(Value::as_str)
        .map(|m| m.eq_ignore_ascii_case("style"))
        .unwrap_or(false);

    // Frames are only kept for the length of the call; the reference itself is
    // the user's file and is never touched.
    let scratch = temp_dir();
    let mut parts: Vec<Value> = vec![json!({
        "type": "text",
        "text": if style { format!("{}{}", question, STYLE) } else { question.to_string() }
    })];
    let looked = match gather(ws, &requested, args, &scratch, &mut parts) {
        Ok(looked) => looked,
        Err(why) => {
            let _ = std::fs::remove_dir_all(&scratch);
            return Err(why);
        }
    };

    let messages = json!([
        { "role": "system", "content": SYSTEM },
        { "role": "user", "content": parts }
    ]);

    let candidates = candidates_for(configured_model);
    let mut outcome = Err("no vision model to ask".to_string());
    let mut asked = String::new();
    for (index, candidate) in candidates.iter().enumerate() {
        asked = candidate.clone();
        outcome = model::generate(api_key, candidate, messages.clone(), None).await;
        let last = index + 1 == candidates.len();
        match &outcome {
            Ok(_) => break,
            Err(why) if last || !worth_another_model(why) => break,
            Err(_) => continue,
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    let response = outcome?;

    let answer = response
        .pointer("/choices/0/message")
        .map(text_of)
        .unwrap_or_default();
    if answer.trim().is_empty() {
        return Err(format!(
            "{} looked but said nothing. Try again with a more specific question.",
            response.get("model").and_then(Value::as_str).unwrap_or(&asked)
        ));
    }

    Ok(json!({
        "model": response.get("model").and_then(Value::as_str).unwrap_or(&asked),
        "looked_at": looked,
        "answer": answer,
        "usage": response.get("usage").cloned().unwrap_or(Value::Null),
    }))
}

/// Turn what was asked for into images to send, and a record of what each one
/// actually is — a file, or a moment in a clip.
fn gather(
    ws: &Workspace,
    requested: &[String],
    args: &Value,
    scratch: &Path,
    parts: &mut Vec<Value>,
) -> Result<Vec<Value>, String> {
    let mut looked: Vec<Value> = Vec::new();

    for raw in requested {
        if looked.len() >= MAX_IMAGES {
            break;
        }
        let path = ws.resolve(raw);
        if !path.exists() {
            return Err(format!("'{}' does not exist.", raw));
        }
        let extension = extension_of(&path);

        if VIDEO_EXTS.contains(&extension.as_str()) {
            // A clip is looked at by sampling it: one frame says nothing about
            // what happens over six seconds.
            let count = args
                .get("frames")
                .and_then(Value::as_u64)
                .map(|n| n.clamp(1, MAX_IMAGES as u64) as usize)
                .unwrap_or(DEFAULT_FRAMES)
                .min(MAX_IMAGES - looked.len());
            for (at, frame) in frames_of(&path, count, scratch)? {
                push_image(parts, &frame)?;
                looked.push(json!({ "path": display(ws, raw), "frame_at_seconds": at }));
            }
        } else if IMAGE_EXTS.contains(&extension.as_str()) {
            push_image(parts, &path)?;
            looked.push(json!({ "path": display(ws, raw) }));
        } else {
            return Err(format!(
                "'{}' is a {} file, which is not something to look at. Images ({}) and video ({}) \
                 are. For a PDF or a document, read it instead.",
                raw,
                if extension.is_empty() { "typeless".into() } else { extension },
                IMAGE_EXTS.join(", "),
                VIDEO_EXTS.join(", ")
            ));
        }
    }

    Ok(looked)
}

/// The user may override the vision model in settings; unset means Qwen.
pub fn model_for(configured: &str) -> &str {
    let configured = configured.trim();
    if configured.is_empty() {
        DEFAULT_MODEL
    } else {
        configured
    }
}

/// One model when the user chose one, the whole line when they did not.
fn candidates_for(configured: &str) -> Vec<String> {
    if configured.trim().is_empty() {
        LINE.iter().map(|m| m.to_string()).collect()
    } else {
        vec![configured.trim().to_string()]
    }
}

/// Worth asking the next model in the line: the request itself was fine, this
/// particular model just could not take it. A rejected key or an empty account
/// is not going to be fixed by asking someone else.
fn worth_another_model(error: &str) -> bool {
    ["(429)", "(404)", "(500)", "(502)", "(503)", "(529)"]
        .iter()
        .any(|code| error.contains(code))
        || error.contains("could not reach OpenRouter")
}

/// One path or several, under either name, because a model will reach for both.
pub fn paths_of(args: &Value) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(one) = args.get("path").and_then(Value::as_str) {
        if !one.trim().is_empty() {
            out.push(one.trim().to_string());
        }
    }
    if let Some(many) = args.get("paths").and_then(Value::as_array) {
        for p in many.iter().filter_map(Value::as_str) {
            if !p.trim().is_empty() {
                out.push(p.trim().to_string());
            }
        }
    }
    if out.is_empty() {
        return Err("no file was named. Give the path of an image or a video to look at.".into());
    }
    out.truncate(MAX_IMAGES);
    Ok(out)
}

fn push_image(parts: &mut Vec<Value>, path: &Path) -> Result<(), String> {
    let meta = std::fs::metadata(path)
        .map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
    if meta.len() > MAX_IMAGE_BYTES {
        return Err(format!(
            "'{}' is {} bytes — too large to send. Scale it down first: \
             ffmpeg -i in.png -vf scale=1600:-2 small.jpg",
            path.display(),
            meta.len()
        ));
    }
    let bytes =
        std::fs::read(path).map_err(|e| format!("cannot read '{}': {}", path.display(), e))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    parts.push(json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{};base64,{}", mime_of(&extension_of(path)), encoded) }
    }));
    Ok(())
}

/// Evenly spaced stills from a clip, taken at the middle of each slice so a
/// black first frame or a trailing fade is never all the model gets to see.
fn frames_of(video: &Path, count: usize, scratch: &Path) -> Result<Vec<(f64, PathBuf)>, String> {
    let duration = duration_of(video)?;
    std::fs::create_dir_all(scratch)
        .map_err(|e| format!("cannot prepare a place for the frames: {}", e))?;

    let mut frames = Vec::new();
    for index in 0..count {
        let at = if duration > 0.0 {
            duration * (index as f64 + 0.5) / count as f64
        } else {
            0.0
        };
        let out = scratch.join(format!("frame-{}.jpg", index));
        let result = std::process::Command::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-ss",
                &format!("{:.3}", at),
                "-i",
            ])
            .arg(video)
            .args([
                "-frames:v",
                "1",
                // Full resolution buys nothing here and costs tokens.
                "-vf",
                "scale='min(1280,iw)':-2",
                "-q:v",
                "3",
                "-y",
            ])
            .arg(&out)
            .output()
            .map_err(|e| {
                format!(
                    "frames could not be taken from '{}': ffmpeg would not run ({}).",
                    video.display(),
                    e
                )
            })?;
        if !result.status.success() || !out.exists() {
            return Err(format!(
                "frames could not be taken from '{}': {}",
                video.display(),
                String::from_utf8_lossy(&result.stderr).trim()
            ));
        }
        frames.push((round_to_ms(at), out));
    }
    Ok(frames)
}

fn duration_of(video: &Path) -> Result<f64, String> {
    let out = std::process::Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(video)
        .output()
        .map_err(|e| format!("cannot measure '{}': ffprobe would not run ({}).", video.display(), e))?;
    if !out.status.success() {
        return Err(format!(
            "cannot measure '{}': {}",
            video.display(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("'{}' reports no duration — is it really a video?", video.display()))
}

fn round_to_ms(seconds: f64) -> f64 {
    (seconds * 1000.0).round() / 1000.0
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join(format!("sirvibe-see-{}", now_ns()))
}

fn now_ns() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
}

/// A workspace file is named the way the user sees it; a file they handed us
/// from elsewhere keeps the path they gave.
fn display(ws: &Workspace, raw: &str) -> String {
    let path = ws.resolve(raw);
    if ws.contains(&path) {
        ws.rel(&path)
    } else {
        raw.to_string()
    }
}

fn mime_of(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        _ => "image/jpeg",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> Workspace {
        let root = std::env::temp_dir().join(format!("sirvibe-vision-{}", name));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Workspace::open(&root.to_string_lossy()).unwrap()
    }

    #[test]
    fn vision_is_qwen_unless_the_user_says_otherwise() {
        assert_eq!(model_for(""), DEFAULT_MODEL);
        assert_eq!(model_for("   "), DEFAULT_MODEL);
        assert!(DEFAULT_MODEL.starts_with("qwen/"));
        assert_eq!(model_for("z-ai/glm-5v"), "z-ai/glm-5v");
    }

    #[test]
    fn the_whole_qwen_line_is_tried_but_only_when_the_user_chose_nothing() {
        assert_eq!(candidates_for("")[0], DEFAULT_MODEL);
        assert!(candidates_for("").len() > 1, "one flaky provider must not blind the agent");
        assert_eq!(candidates_for("z-ai/glm-5v"), vec!["z-ai/glm-5v".to_string()]);
    }

    #[test]
    fn only_a_failure_of_that_one_model_moves_on_to_the_next() {
        assert!(worth_another_model("Rate limited by OpenRouter (429). Provider returned error"));
        assert!(worth_another_model("Model not found on OpenRouter (404)."));
        assert!(!worth_another_model("OpenRouter rejected the API key (401)."));
        assert!(!worth_another_model("OpenRouter reports insufficient credit (402)."));
    }

    #[test]
    fn one_path_or_many_are_both_understood() {
        assert_eq!(paths_of(&json!({ "path": "ref.png" })).unwrap(), vec!["ref.png"]);
        assert_eq!(
            paths_of(&json!({ "paths": ["a.png", "b.jpg"] })).unwrap(),
            vec!["a.png", "b.jpg"]
        );
        assert!(paths_of(&json!({})).unwrap_err().contains("no file was named"));
    }

    #[tokio::test]
    async fn a_file_that_is_not_visual_is_refused_with_a_reason() {
        let ws = workspace("kinds");
        std::fs::write(ws.root.join("notes.txt"), b"x").unwrap();
        let err = see(&ws, "k", "", &json!({ "path": "notes.txt" }))
            .await
            .unwrap_err();
        assert!(err.contains("not something to look at"), "{}", err);
    }

    #[tokio::test]
    async fn a_missing_reference_is_named_rather_than_sent() {
        let ws = workspace("missing");
        let err = see(&ws, "k", "", &json!({ "path": "nope.png" }))
            .await
            .unwrap_err();
        assert!(err.contains("does not exist"), "{}", err);
    }

    #[test]
    fn an_oversized_image_says_how_to_shrink_it() {
        let ws = workspace("big");
        let big = ws.root.join("poster.png");
        std::fs::write(&big, vec![0u8; (MAX_IMAGE_BYTES + 1) as usize]).unwrap();
        let err = push_image(&mut Vec::new(), &big).unwrap_err();
        assert!(err.contains("too large to send"), "{}", err);
        assert!(err.contains("scale"), "it should say what to do instead: {}", err);
    }

    /// Not part of `cargo test`: it calls OpenRouter and spends real money.
    /// Run it deliberately when the seeing path itself needs proving:
    ///
    ///   SIRVIBE_TEST_OPENROUTER_KEY=sk-or-… SIRVIBE_TEST_MEDIA=/path/to/frame.png \
    ///     cargo test -- --ignored --nocapture really_looks
    #[tokio::test]
    #[ignore]
    async fn really_looks_at_a_file_and_reads_what_is_on_it() {
        let key = std::env::var("SIRVIBE_TEST_OPENROUTER_KEY")
            .expect("set SIRVIBE_TEST_OPENROUTER_KEY to run this");
        let media = std::env::var("SIRVIBE_TEST_MEDIA").expect("set SIRVIBE_TEST_MEDIA to a file");
        let path = PathBuf::from(&media);
        let ws = Workspace::open(&path.parent().unwrap().to_string_lossy()).unwrap();
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        let out = see(
            &ws,
            &key,
            "",
            &json!({
                "path": name,
                "frames": 2,
                "mode": std::env::var("SIRVIBE_TEST_MODE").unwrap_or_else(|_| "describe".into()),
                "question": std::env::var("SIRVIBE_TEST_QUESTION").unwrap_or_else(|_| {
                    "Quote every word of text that appears in this frame, exactly as written.".into()
                })
            }),
        )
        .await
        .unwrap();

        let answer = out["answer"].as_str().unwrap_or_default();
        println!("model: {}\nlooked at: {}\n\n{}", out["model"], out["looked_at"], answer);
        assert!(!answer.trim().is_empty());
        assert!(
            out["looked_at"].as_array().map(|l| !l.is_empty()).unwrap_or(false),
            "it should report what it looked at"
        );
    }

    #[test]
    fn a_reference_from_outside_the_workspace_keeps_the_path_the_user_gave() {
        let ws = workspace("display");
        std::fs::write(ws.root.join("inside.png"), b"x").unwrap();
        assert_eq!(display(&ws, "inside.png"), "inside.png");
        assert_eq!(display(&ws, "/home/someone/moodboard.png"), "/home/someone/moodboard.png");
    }
}

//! Commissioning a piece of work from a named model.
//!
//! This is not the agent thinking — it is the agent asking a specific model,
//! chosen by the user in their prompt, to produce something: a voiceover, an
//! image, a clip. The user's own OpenRouter key pays for it, the request is
//! approved before it runs, and anything the model returns is written into the
//! workspace so it shows up as an artifact like any other output.

use crate::model;
use crate::workspace::Workspace;
use base64::Engine;
use serde_json::{json, Value};
use std::path::PathBuf;

/// A generated file has to land on disk, so cap what a model can hand back.
const MAX_MEDIA_BYTES: u64 = 512 * 1024 * 1024;
/// What can be sent up with a prompt. Whole videos are far too big for this
/// path — reference them by frame or by a hosted URL instead.
const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;
const DOWNLOAD_TIMEOUT_SECS: u64 = 600;

pub async fn run(ws: &Workspace, api_key: &str, args: &Value) -> Result<Value, String> {
    let model_id = args
        .get("model")
        .and_then(Value::as_str)
        .filter(|m| !m.trim().is_empty())
        .ok_or("missing required argument 'model'")?;
    let prompt = args
        .get("prompt")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .ok_or("missing required argument 'prompt'")?;
    let expect = args
        .get("expect")
        .and_then(Value::as_str)
        .unwrap_or("text")
        .to_lowercase();

    let mut parts: Vec<Value> = vec![json!({ "type": "text", "text": prompt })];
    if let Some(attachments) = args.get("attachments").and_then(Value::as_array) {
        for path in attachments.iter().filter_map(Value::as_str) {
            parts.push(attachment_part(ws, path)?);
        }
    }

    let mut messages: Vec<Value> = Vec::new();
    if let Some(system) = args.get("system").and_then(Value::as_str) {
        if !system.trim().is_empty() {
            messages.push(json!({ "role": "system", "content": system }));
        }
    }
    messages.push(json!({ "role": "user", "content": parts }));

    // A model that produces media has to be told to; text is the default.
    let modalities = match expect.as_str() {
        "text" | "" => None,
        other => Some(vec![other.to_string(), "text".to_string()]),
    };

    let response = model::generate(api_key, model_id, json!(messages), modalities).await?;
    let message = response
        .pointer("/choices/0/message")
        .cloned()
        .ok_or_else(|| format!("{} returned no message.", model_id))?;

    let text = text_of(&message);
    let mut saved: Vec<Value> = Vec::new();
    let base = args.get("save_as").and_then(Value::as_str);
    for (index, media) in media_of(&message).into_iter().enumerate() {
        saved.push(save(ws, model_id, base, index, media).await?);
    }

    if saved.is_empty() && expect != "text" {
        return Err(format!(
            "{} returned only text, no {}. Either the model does not produce {} output, or the \
             request needs rewording. Use find_models to check what it produces.{}",
            model_id,
            expect,
            expect,
            if text.is_empty() {
                String::new()
            } else {
                format!(" It said: {}", text.chars().take(400).collect::<String>())
            }
        ));
    }

    Ok(json!({
        "model": response.get("model").and_then(Value::as_str).unwrap_or(model_id),
        "text": text,
        "files": saved,
        "usage": response.get("usage").cloned().unwrap_or(Value::Null),
    }))
}

/// A file the model returned, before it has been written down.
enum Media {
    Inline { bytes: Vec<u8>, mime: String },
    Remote { url: String },
}

fn media_of(message: &Value) -> Vec<Media> {
    let mut found = Vec::new();

    // Generated images come back alongside the text, not inside it.
    if let Some(images) = message.get("images").and_then(Value::as_array) {
        for image in images {
            let url = image
                .pointer("/image_url/url")
                .or_else(|| image.get("url"))
                .and_then(Value::as_str);
            if let Some(url) = url {
                found.extend(from_url(url));
            }
        }
    }

    // Audio output is base64 with the container named separately.
    if let Some(audio) = message.get("audio") {
        if let Some(data) = audio.get("data").and_then(Value::as_str) {
            let format = audio
                .get("format")
                .and_then(Value::as_str)
                .unwrap_or("mp3")
                .to_string();
            if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(data) {
                found.push(Media::Inline {
                    bytes,
                    mime: format!("audio/{}", format),
                });
            }
        }
    }

    // Some models answer with a content array carrying the media inline.
    if let Some(parts) = message.get("content").and_then(Value::as_array) {
        for part in parts {
            for key in ["image_url", "video_url", "audio_url"] {
                if let Some(url) = part.pointer(&format!("/{}/url", key)).and_then(Value::as_str) {
                    found.extend(from_url(url));
                }
            }
        }
    }

    found
}

fn from_url(url: &str) -> Option<Media> {
    if let Some(rest) = url.strip_prefix("data:") {
        let (meta, payload) = rest.split_once(',')?;
        let mime = meta.split(';').next().unwrap_or("application/octet-stream");
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(payload.trim())
            .ok()?;
        return Some(Media::Inline {
            bytes,
            mime: mime.to_string(),
        });
    }
    if url.starts_with("https://") || url.starts_with("http://") {
        return Some(Media::Remote {
            url: url.to_string(),
        });
    }
    None
}

async fn save(
    ws: &Workspace,
    model_id: &str,
    save_as: Option<&str>,
    index: usize,
    media: Media,
) -> Result<Value, String> {
    let (bytes, mime) = match media {
        Media::Inline { bytes, mime } => (bytes, mime),
        Media::Remote { url } => download(&url).await?,
    };
    if bytes.len() as u64 > MAX_MEDIA_BYTES {
        return Err(format!(
            "the model returned {} bytes, over the {} byte limit",
            bytes.len(),
            MAX_MEDIA_BYTES
        ));
    }

    let path = target_path(ws, model_id, save_as, index, &mime)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
    }
    std::fs::write(&path, &bytes)
        .map_err(|e| format!("cannot save '{}': {}", ws.rel(&path), e))?;

    Ok(json!({
        "path": ws.rel(&path),
        "bytes": bytes.len(),
        "kind": kind_of(&mime),
    }))
}

/// Where a generated file lands. A model-chosen name can never escape the
/// workspace: the path is resolved against the root and rejected if it leaves.
fn target_path(
    ws: &Workspace,
    model_id: &str,
    save_as: Option<&str>,
    index: usize,
    mime: &str,
) -> Result<PathBuf, String> {
    let extension = extension_of(mime);
    let candidate = match save_as {
        Some(raw) if !raw.trim().is_empty() => {
            let raw = raw.trim();
            let with_extension = if PathBuf::from(raw).extension().is_some() {
                raw.to_string()
            } else {
                format!("{}.{}", raw, extension)
            };
            if index == 0 {
                with_extension
            } else {
                // A second file cannot quietly overwrite the first.
                let p = PathBuf::from(&with_extension);
                let stem = p.file_stem().unwrap_or_default().to_string_lossy();
                let parent = p.parent().map(|d| d.to_string_lossy().to_string());
                let name = format!("{}-{}.{}", stem, index + 1, extension);
                match parent.filter(|d| !d.is_empty()) {
                    Some(dir) => format!("{}/{}", dir, name),
                    None => name,
                }
            }
        }
        _ => format!(
            "out/generated/{}-{}{}.{}",
            slug(model_id),
            now_ms(),
            if index == 0 {
                String::new()
            } else {
                format!("-{}", index + 1)
            },
            extension
        ),
    };

    let path = ws.resolve(&candidate);
    if !ws.contains(&path) {
        return Err(format!(
            "'{}' is outside the workspace. Generated files are saved inside it.",
            candidate
        ));
    }
    Ok(path)
}

async fn download(url: &str) -> Result<(Vec<u8>, String), String> {
    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(DOWNLOAD_TIMEOUT_SECS))
        .build()
        .map_err(|e| e.to_string())?
        .get(url)
        .send()
        .await
        .map_err(|e| format!("could not fetch the generated file: {}", e))?;
    if !response.status().is_success() {
        return Err(format!(
            "the generated file could not be fetched: {} returned {}",
            url,
            response.status()
        ));
    }
    let mime = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .split(';')
        .next()
        .unwrap_or("application/octet-stream")
        .to_string();
    if let Some(length) = response.content_length() {
        if length > MAX_MEDIA_BYTES {
            return Err(format!("the generated file is {} bytes, too large to save", length));
        }
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("the generated file could not be read: {}", e))?;
    Ok((bytes.to_vec(), mime))
}

fn attachment_part(ws: &Workspace, raw: &str) -> Result<Value, String> {
    let path = ws.resolve(raw);
    if !ws.contains(&path) {
        return Err(format!("'{}' is outside the workspace.", raw));
    }
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;
    if meta.len() > MAX_ATTACHMENT_BYTES {
        return Err(format!(
            "'{}' is {} bytes — too large to send. Send a frame or a short excerpt instead.",
            raw,
            meta.len()
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;
    let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    Ok(match extension.as_str() {
        "mp3" | "wav" | "ogg" | "flac" | "m4a" => json!({
            "type": "input_audio",
            "input_audio": { "data": encoded, "format": extension }
        }),
        "pdf" => json!({
            "type": "file",
            "file": {
                "filename": path.file_name().unwrap_or_default().to_string_lossy(),
                "file_data": format!("data:application/pdf;base64,{}", encoded)
            }
        }),
        _ => json!({
            "type": "image_url",
            "image_url": { "url": format!("data:{};base64,{}", mime_of(&extension), encoded) }
        }),
    })
}

fn text_of(message: &Value) -> String {
    match message.get("content") {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn mime_of(extension: &str) -> &'static str {
    match extension {
        "png" => "image/png",
        "webp" => "image/webp",
        "gif" => "image/gif",
        _ => "image/jpeg",
    }
}

fn extension_of(mime: &str) -> &'static str {
    match mime {
        "image/png" => "png",
        "image/jpeg" | "image/jpg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/flac" => "flac",
        "audio/aac" | "audio/mp4" | "audio/m4a" => "m4a",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/quicktime" => "mov",
        _ => "bin",
    }
}

fn kind_of(mime: &str) -> &'static str {
    match mime.split('/').next().unwrap_or("") {
        "image" => "image",
        "audio" => "audio",
        "video" => "video",
        _ => "file",
    }
}

fn slug(model_id: &str) -> String {
    let cleaned: String = model_id
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let mut out = String::new();
    let mut dash = false;
    for c in cleaned.trim_matches('-').chars() {
        if c == '-' {
            if !dash {
                out.push(c);
            }
            dash = true;
        } else {
            out.push(c);
            dash = false;
        }
    }
    out.chars().take(40).collect()
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> Workspace {
        let root = std::env::temp_dir().join(format!("sirvibe-generate-{}", name));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Workspace::open(&root.to_string_lossy()).unwrap()
    }

    #[test]
    fn generated_images_are_found_alongside_the_text() {
        let message = json!({
            "content": "Here is the frame.",
            "images": [{
                "type": "image_url",
                "image_url": { "url": "data:image/png;base64,aGVsbG8=" }
            }]
        });
        assert_eq!(text_of(&message), "Here is the frame.");
        let media = media_of(&message);
        assert_eq!(media.len(), 1);
        match &media[0] {
            Media::Inline { bytes, mime } => {
                assert_eq!(bytes, b"hello");
                assert_eq!(mime, "image/png");
            }
            _ => panic!("expected inline media"),
        }
    }

    #[test]
    fn audio_output_keeps_its_container() {
        let message = json!({ "audio": { "data": "aGVsbG8=", "format": "wav" } });
        match &media_of(&message)[0] {
            Media::Inline { mime, .. } => assert_eq!(mime, "audio/wav"),
            _ => panic!("expected inline media"),
        }
    }

    #[test]
    fn a_hosted_result_is_fetched_rather_than_decoded() {
        let message = json!({
            "content": [{ "type": "video_url", "video_url": { "url": "https://cdn.test/clip.mp4" } }]
        });
        match &media_of(&message)[0] {
            Media::Remote { url } => assert_eq!(url, "https://cdn.test/clip.mp4"),
            _ => panic!("expected a remote file"),
        }
    }

    #[test]
    fn a_default_name_carries_the_model_and_the_right_extension() {
        let ws = workspace("naming");
        let path = target_path(&ws, "google/gemini-2.5-flash-image", None, 0, "image/png").unwrap();
        let rel = ws.rel(&path);
        assert!(rel.starts_with("out/generated/google-gemini-2-5-flash-image-"), "{}", rel);
        assert!(rel.ends_with(".png"), "{}", rel);
    }

    #[test]
    fn a_chosen_name_gains_the_extension_and_never_collides() {
        let ws = workspace("chosen");
        let first = target_path(&ws, "m", Some("out/vo"), 0, "audio/mpeg").unwrap();
        assert_eq!(ws.rel(&first), "out/vo.mp3");
        let second = target_path(&ws, "m", Some("out/vo"), 1, "audio/mpeg").unwrap();
        assert_eq!(ws.rel(&second), "out/vo-2.mp3");
    }

    #[test]
    fn generated_files_cannot_be_written_outside_the_workspace() {
        let ws = workspace("escape");
        let err = target_path(&ws, "m", Some("../../etc/passwd"), 0, "image/png").unwrap_err();
        assert!(err.contains("outside the workspace"), "{}", err);
    }

    #[test]
    fn an_oversized_attachment_is_refused_with_advice() {
        let ws = workspace("attach");
        let big = ws.root.join("clip.mp4");
        std::fs::write(&big, vec![0u8; (MAX_ATTACHMENT_BYTES + 1) as usize]).unwrap();
        let err = attachment_part(&ws, "clip.mp4").unwrap_err();
        assert!(err.contains("too large to send"), "{}", err);
        assert!(err.contains("frame"), "it should say what to do instead: {}", err);
    }

    #[test]
    fn an_attachment_is_sent_in_the_form_its_type_calls_for() {
        let ws = workspace("kinds");
        std::fs::write(ws.root.join("shot.png"), b"x").unwrap();
        std::fs::write(ws.root.join("vo.mp3"), b"x").unwrap();
        assert_eq!(attachment_part(&ws, "shot.png").unwrap()["type"], "image_url");
        let audio = attachment_part(&ws, "vo.mp3").unwrap();
        assert_eq!(audio["type"], "input_audio");
        assert_eq!(audio["input_audio"]["format"], "mp3");
    }
}

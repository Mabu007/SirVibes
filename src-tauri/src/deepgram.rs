//! Deepgram: the built-in route for speech. Transcription and voiceover are
//! needed by almost every piece of video work, so they are first-class tools
//! rather than something the user has to wire up as a generic API. The key
//! lives in settings like the OpenRouter one; the agent never sees it.

use crate::workspace::Workspace;
use serde_json::{json, Value};
use std::path::PathBuf;

const BASE: &str = "https://api.deepgram.com/v1";
const DEFAULT_STT_MODEL: &str = "nova-3";
const DEFAULT_VOICE: &str = "aura-2-thalia-en";
/// Whole files are uploaded, so keep this to something a machine can hold.
/// Video should have its audio extracted first, which is far smaller anyway.
const MAX_UPLOAD_BYTES: u64 = 250 * 1024 * 1024;
const TRANSCRIBE_TIMEOUT_SECS: u64 = 1800;
const SPEAK_TIMEOUT_SECS: u64 = 300;
/// Deepgram will not read an unbounded script in one request.
const MAX_SPEAK_CHARS: usize = 40_000;
/// Enough timing detail for the agent to cut on, without flooding its context.
const MAX_RETURNED_UTTERANCES: usize = 300;

fn key(settings_key: &str) -> Result<String, String> {
    let key = settings_key.trim();
    if key.is_empty() {
        return Err("No Deepgram API key is set. Ask the user to add one in Settings → Deepgram; \
                    it takes one paste and then transcription and voiceover work everywhere."
            .into());
    }
    Ok(key.to_string())
}

fn client(timeout_secs: u64) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .build()
        .map_err(|e| e.to_string())
}

/// Deepgram rejects a request whose declared type does not match the bytes, so
/// the extension has to be honoured rather than guessed at broadly.
fn content_type(path: &std::path::Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "flac" => "audio/flac",
        "webm" => "audio/webm",
        "mp4" | "mov" => "video/mp4",
        "mkv" => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

pub async fn transcribe(ws: &Workspace, api_key: &str, args: &Value) -> Result<Value, String> {
    let key = key(api_key)?;
    let raw = args
        .get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.trim().is_empty())
        .ok_or("missing required argument 'path'")?;
    let path = ws.resolve(raw);
    let meta = std::fs::metadata(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;
    if meta.len() > MAX_UPLOAD_BYTES {
        return Err(format!(
            "'{}' is {:.1} GB, too large to upload. Extract the audio first — \
             ffmpeg -i <file> -ac 1 -ar 16000 audio.wav — and transcribe that.",
            raw,
            meta.len() as f64 / 1_073_741_824.0
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read '{}': {}", raw, e))?;

    let mut query: Vec<(String, String)> = vec![
        (
            "model".into(),
            args.get("model")
                .and_then(Value::as_str)
                .unwrap_or(DEFAULT_STT_MODEL)
                .to_string(),
        ),
        ("smart_format".into(), "true".into()),
        ("punctuate".into(), "true".into()),
        ("paragraphs".into(), "true".into()),
        ("utterances".into(), "true".into()),
        (
            "diarize".into(),
            args.get("diarize")
                .and_then(Value::as_bool)
                .unwrap_or(true)
                .to_string(),
        ),
    ];
    if let Some(language) = args.get("language").and_then(Value::as_str) {
        if !language.trim().is_empty() {
            query.push(("language".into(), language.trim().to_string()));
        }
    }

    let response = client(TRANSCRIBE_TIMEOUT_SECS)?
        .post(format!("{}/listen", BASE))
        .header("Authorization", format!("Token {}", key))
        .header("Content-Type", content_type(&path))
        .query(&query)
        .body(bytes)
        .send()
        .await
        .map_err(|e| format!("could not reach Deepgram: {}", e))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(explain(status.as_u16(), &text));
    }
    let result: Value =
        serde_json::from_str(&text).map_err(|_| "Deepgram returned something unreadable")?;

    let transcript = result
        .pointer("/results/channels/0/alternatives/0/transcript")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let duration = result
        .pointer("/metadata/duration")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let utterances = utterances_of(&result);

    // The word-level detail is what cutting decisions are made from, but it is
    // far too much to hand back through the conversation. It goes to disk.
    let stem = args
        .get("save_as")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| {
            format!(
                "out/transcripts/{}",
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "transcript".into())
            )
        });

    let mut files = Vec::new();
    files.push(write_out(ws, &stem, "json", serde_json::to_string_pretty(&result)
        .unwrap_or_default()
        .as_bytes())?);
    files.push(write_out(ws, &stem, "srt", srt(&utterances).as_bytes())?);

    let returned: Vec<Value> = utterances
        .iter()
        .take(MAX_RETURNED_UTTERANCES)
        .map(|u| {
            json!({
                "start": round(u.start),
                "end": round(u.end),
                "speaker": u.speaker,
                "text": u.text,
            })
        })
        .collect();

    Ok(json!({
        "transcript": transcript,
        "duration_seconds": round(duration),
        "utterances": returned,
        "utterance_count": utterances.len(),
        "truncated": utterances.len() > returned.len(),
        "files": files,
        "note": "The .json file holds every word with its exact start and end. Read it when you need frame-accurate cut points.",
    }))
}

pub async fn speak(ws: &Workspace, api_key: &str, args: &Value) -> Result<Value, String> {
    let key = key(api_key)?;
    let text = args
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or("missing required argument 'text'")?;
    if text.chars().count() > MAX_SPEAK_CHARS {
        return Err(format!(
            "the script is {} characters, over the {} character limit. Read it in sections and \
             join the audio with ffmpeg.",
            text.chars().count(),
            MAX_SPEAK_CHARS
        ));
    }
    let voice = args
        .get("voice")
        .and_then(Value::as_str)
        .filter(|v| !v.trim().is_empty())
        .unwrap_or(DEFAULT_VOICE);

    let response = client(SPEAK_TIMEOUT_SECS)?
        .post(format!("{}/speak", BASE))
        .header("Authorization", format!("Token {}", key))
        .query(&[("model", voice)])
        .json(&json!({ "text": text }))
        .send()
        .await
        .map_err(|e| format!("could not reach Deepgram: {}", e))?;

    let status = response.status();
    let extension = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .map(extension_for)
        .unwrap_or("mp3");
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(explain(status.as_u16(), &body));
    }
    let audio = response
        .bytes()
        .await
        .map_err(|e| format!("the audio could not be read: {}", e))?;

    let stem = args
        .get("save_as")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().trim_end_matches(&format!(".{}", extension)).to_string())
        .unwrap_or_else(|| format!("out/generated/speech-{}", now_ms()));
    let file = write_out(ws, &stem, extension, &audio)?;

    Ok(json!({
        "voice": voice,
        "characters": text.chars().count(),
        "file": file,
    }))
}

// ------------------------------------------------------------------ shaping

struct Utterance {
    start: f64,
    end: f64,
    speaker: Option<u64>,
    text: String,
}

fn utterances_of(result: &Value) -> Vec<Utterance> {
    if let Some(list) = result.pointer("/results/utterances").and_then(Value::as_array) {
        return list
            .iter()
            .filter_map(|u| {
                let text = u.get("transcript").and_then(Value::as_str)?.trim().to_string();
                if text.is_empty() {
                    return None;
                }
                Some(Utterance {
                    start: u.get("start").and_then(Value::as_f64).unwrap_or(0.0),
                    end: u.get("end").and_then(Value::as_f64).unwrap_or(0.0),
                    speaker: u.get("speaker").and_then(Value::as_u64),
                    text,
                })
            })
            .collect();
    }

    // No utterances came back, so fall back to the plain transcript as one
    // block rather than losing the result entirely.
    let text = result
        .pointer("/results/channels/0/alternatives/0/transcript")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Vec::new();
    }
    vec![Utterance {
        start: 0.0,
        end: result
            .pointer("/metadata/duration")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        speaker: None,
        text,
    }]
}

fn srt(utterances: &[Utterance]) -> String {
    let mut out = String::new();
    for (index, u) in utterances.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            index + 1,
            timecode(u.start),
            timecode(u.end.max(u.start + 0.1)),
            u.text
        ));
    }
    out
}

fn timecode(seconds: f64) -> String {
    let total = seconds.max(0.0);
    let hours = (total / 3600.0).floor() as u64;
    let minutes = ((total % 3600.0) / 60.0).floor() as u64;
    let secs = (total % 60.0).floor() as u64;
    let millis = ((total - total.floor()) * 1000.0).round() as u64;
    format!("{:02}:{:02}:{:02},{:03}", hours, minutes, secs, millis.min(999))
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// Everything written lands inside the workspace, whatever name was asked for.
fn write_out(ws: &Workspace, stem: &str, extension: &str, bytes: &[u8]) -> Result<Value, String> {
    let named = if PathBuf::from(stem)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case(extension))
        .unwrap_or(false)
    {
        stem.to_string()
    } else {
        format!("{}.{}", stem, extension)
    };
    let path = ws.resolve(&named);
    if !ws.contains(&path) {
        return Err(format!(
            "'{}' is outside the workspace. Results are saved inside it.",
            named
        ));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
    }
    std::fs::write(&path, bytes).map_err(|e| format!("cannot save '{}': {}", named, e))?;
    Ok(json!({ "path": ws.rel(&path), "bytes": bytes.len() }))
}

fn extension_for(content_type: &str) -> &'static str {
    match content_type.split(';').next().unwrap_or("").trim() {
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/flac" => "flac",
        "audio/aac" => "aac",
        _ => "mp3",
    }
}

fn explain(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("err_msg")
                .or_else(|| v.get("message"))
                .or_else(|| v.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(300).collect());

    match status {
        401 | 403 => format!(
            "Deepgram rejected the API key. Ask the user to check it in Settings → Deepgram. ({})",
            detail
        ),
        402 => format!(
            "The Deepgram account is out of credit. ({})",
            detail
        ),
        429 => format!(
            "Deepgram is rate limiting this account. Wait before trying again. ({})",
            detail
        ),
        400 => format!(
            "Deepgram could not process that request: {}. If it is a media file, check it actually \
             contains an audio track — ffprobe will say.",
            detail
        ),
        _ => format!("Deepgram returned {}: {}", status, detail),
    }
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
        let root = std::env::temp_dir().join(format!("sirvibe-deepgram-{}", name));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Workspace::open(&root.to_string_lossy()).unwrap()
    }

    fn sample() -> Value {
        json!({
            "metadata": { "duration": 12.5 },
            "results": {
                "channels": [{ "alternatives": [{ "transcript": "Hello there. This is a test." }] }],
                "utterances": [
                    { "start": 0.08, "end": 1.94, "speaker": 0, "transcript": "Hello there." },
                    { "start": 2.1, "end": 4.0, "speaker": 1, "transcript": "This is a test." }
                ]
            }
        })
    }

    #[test]
    fn utterances_carry_their_timing_and_speaker() {
        let found = utterances_of(&sample());
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "Hello there.");
        assert_eq!(found[1].speaker, Some(1));
        assert!((found[1].start - 2.1).abs() < 1e-9);
    }

    #[test]
    fn a_result_without_utterances_still_yields_the_transcript() {
        let plain = json!({
            "metadata": { "duration": 3.0 },
            "results": { "channels": [{ "alternatives": [{ "transcript": "One block." }] }] }
        });
        let found = utterances_of(&plain);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].text, "One block.");
        assert!((found[0].end - 3.0).abs() < 1e-9);
    }

    #[test]
    fn the_srt_is_the_shape_a_player_expects() {
        let out = srt(&utterances_of(&sample()));
        assert!(out.starts_with("1\n00:00:00,080 --> 00:00:01,940\nHello there.\n\n"), "{}", out);
        assert!(out.contains("2\n00:00:02,100 --> 00:00:04,000\nThis is a test."), "{}", out);
    }

    #[test]
    fn timecodes_carry_hours_and_never_go_negative() {
        assert_eq!(timecode(3661.5), "01:01:01,500");
        assert_eq!(timecode(0.0), "00:00:00,000");
        assert_eq!(timecode(-4.0), "00:00:00,000");
    }

    #[test]
    fn a_zero_length_utterance_still_produces_a_visible_cue() {
        let same = vec![Utterance { start: 5.0, end: 5.0, speaker: None, text: "Hi".into() }];
        assert!(srt(&same).contains("00:00:05,000 --> 00:00:05,100"));
    }

    #[test]
    fn results_are_written_inside_the_workspace_with_the_right_extension() {
        let ws = workspace("write");
        let file = write_out(&ws, "out/transcripts/clip", "srt", b"x").unwrap();
        assert_eq!(file["path"], "out/transcripts/clip.srt");
        // An extension already given is not doubled up.
        let again = write_out(&ws, "out/vo.mp3", "mp3", b"x").unwrap();
        assert_eq!(again["path"], "out/vo.mp3");
    }

    #[test]
    fn results_cannot_be_written_outside_the_workspace() {
        let ws = workspace("escape");
        let err = write_out(&ws, "../../etc/passwd", "json", b"x").unwrap_err();
        assert!(err.contains("outside the workspace"), "{}", err);
    }

    #[test]
    fn a_missing_key_says_exactly_where_to_put_one() {
        let err = key("   ").unwrap_err();
        assert!(err.contains("Settings"), "{}", err);
        assert!(err.contains("Deepgram"), "{}", err);
    }

    #[test]
    fn a_rejected_key_is_not_reported_as_a_broken_file() {
        let message = explain(401, r#"{"err_msg":"Invalid credentials"}"#);
        assert!(message.contains("rejected the API key"));
        assert!(message.contains("Invalid credentials"));
        assert!(explain(402, "{}").contains("out of credit"));
    }

    #[test]
    fn a_video_container_is_declared_honestly() {
        assert_eq!(content_type(std::path::Path::new("a.wav")), "audio/wav");
        assert_eq!(content_type(std::path::Path::new("a.MP4")), "video/mp4");
        assert_eq!(content_type(std::path::Path::new("a.xyz")), "application/octet-stream");
    }
}

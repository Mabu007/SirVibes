//! Looking at a video someone else made, without taking a copy of it.
//!
//! A reference is somebody's YouTube link. It is not ours, we do not want it on
//! the user's disk, and downloading it to look at it would be both rude and
//! slow. Google's models can watch a YouTube URL where they are, so that is the
//! path: the link goes to the model, the model watches it, and what comes back
//! is a structured description of how the thing was made.
//!
//! Three things this module refuses to do:
//!
//! - **Download.** Not as a fallback, not "just this once". If the reference
//!   cannot be watched remotely, it says so and stops.
//! - **Guess.** A YouTube link that does not resolve is accepted by the API and
//!   answered from imagination — an invented `/shorts/` id came back with
//!   "ginger cat on tiled floor" and 108 prompt tokens. So the token count is
//!   checked: if the video was not ingested, there is no analysis, whatever the
//!   model wrote.
//! - **Ramble.** The answer is JSON in a shape the caller asked for, so it can
//!   be acted on rather than read.

use crate::generate::text_of;
use crate::model;
use crate::workspace::Workspace;
use serde_json::{json, Value};

/// First pass. Watches a YouTube link, cheap enough to use freely.
pub const DEFAULT_MODEL: &str = "google/gemini-3.7-flash";
/// Second opinion, when the first one is unsure. Same interface, more money.
const STRONGER_MODEL: &str = "google/gemini-3.1-pro-preview";
/// Below this the first answer is not worth acting on, and a stronger model
/// gets one look.
const ESCALATE_BELOW: f64 = 0.55;
/// A video that was really ingested adds thousands of tokens to the prompt.
/// Anything under this much beyond the text means nothing was watched.
const MIN_VIDEO_TOKENS: u64 = 300;

/// Where a reference lives, and whether it can be watched from here.
#[derive(Debug, PartialEq)]
pub enum Source {
    /// Watchable in place by the model.
    YouTube,
    /// Reachable in principle, but the model cannot open it.
    Instagram,
    /// A direct media URL. Providers decline to fetch these.
    DirectMedia,
    Unknown,
}

pub fn classify(url: &str) -> Source {
    let lower = url.trim().to_lowercase();
    let host = lower
        .split("://")
        .nth(1)
        .unwrap_or(&lower)
        .split('/')
        .next()
        .unwrap_or("");

    if host.ends_with("youtube.com") || host.ends_with("youtu.be") || host.ends_with("youtube-nocookie.com") {
        return Source::YouTube;
    }
    if host.ends_with("instagram.com") || host.ends_with("instagr.am") {
        return Source::Instagram;
    }
    if [".mp4", ".mov", ".webm", ".mkv", ".m4v"]
        .iter()
        .any(|ext| lower.split('?').next().unwrap_or(&lower).ends_with(ext))
    {
        return Source::DirectMedia;
    }
    Source::Unknown
}

/// What the user wants taken from the reference. Analysing everything when they
/// asked about captions wastes their money and buries the answer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Scope {
    Full,
    Captions,
    Transitions,
    Pacing,
    Camera,
    Graphics,
    Color,
    Audio,
    Composition,
}

impl Scope {
    pub fn parse(raw: &str) -> Scope {
        match raw.trim().to_lowercase().as_str() {
            "captions" | "caption" | "subtitles" | "text" => Scope::Captions,
            "transitions" | "transition" => Scope::Transitions,
            "pacing" | "rhythm" | "timing" | "cuts" => Scope::Pacing,
            "camera" | "movement" | "framing" => Scope::Camera,
            "graphics" | "overlays" | "motion-graphics" => Scope::Graphics,
            "color" | "colour" | "grade" | "grading" => Scope::Color,
            "audio" | "music" | "sound" => Scope::Audio,
            "composition" | "layout" | "framing-composition" => Scope::Composition,
            _ => Scope::Full,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Scope::Full => "full",
            Scope::Captions => "captions",
            Scope::Transitions => "transitions",
            Scope::Pacing => "pacing",
            Scope::Camera => "camera",
            Scope::Graphics => "graphics",
            Scope::Color => "color",
            Scope::Audio => "audio",
            Scope::Composition => "composition",
        }
    }

    /// What to look for, and the shape to say it in. Written as instructions to
    /// someone who has to rebuild the thing, not describe it.
    fn brief(&self) -> &'static str {
        match self {
            Scope::Captions => {
                "Study ONLY the on-screen captions. Ignore the camera, the edit and the music \
                 unless they change how the captions behave. Report:\n\
                 \"captions\": {\n\
                 \"present\": true/false,\n\
                 \"position\": where on screen, e.g. \"lower-centre, ~18% from the bottom\",\n\
                 \"typeface\": serif/sans/rounded/condensed and how heavy,\n\
                 \"weight\": e.g. \"800\",\n\
                 \"case\": \"sentence\"|\"upper\"|\"mixed\",\n\
                 \"sizeRelativeToFrameHeight\": a number, e.g. 0.045,\n\
                 \"colors\": {\"text\": hex, \"highlight\": hex or null, \"stroke\": hex or null, \"background\": hex or null},\n\
                 \"strokeOrShadow\": what separates the text from the picture,\n\
                 \"wordsPerPhrase\": a number,\n\
                 \"lineWrapping\": e.g. \"two lines maximum\",\n\
                 \"activeWordHighlight\": true/false and how it is marked,\n\
                 \"entrance\": how a phrase arrives (e.g. \"scale pop, ~0.15s\"),\n\
                 \"exit\": how it leaves,\n\
                 \"timingWithSpeech\": how the words track the voice,\n\
                 \"rebuildNotes\": one paragraph another editor could build from\n}"
            }
            Scope::Transitions => {
                "Study ONLY how one shot becomes the next. Report:\n\
                 \"transitions\": {\"types\": [..], \"typicalDurationSeconds\": number, \
                 \"direction\": .., \"easing\": .., \"triggeredBy\": .., \"onBeat\": true/false, \
                 \"frequency\": .., \"rebuildNotes\": \"..\"}"
            }
            Scope::Pacing => {
                "Study ONLY the rhythm of the edit. Report:\n\
                 \"pacing\": {\"averageShotSeconds\": number, \"shortestShotSeconds\": number, \
                 \"longestShotSeconds\": number, \"cutRhythm\": \"fast\"|\"medium\"|\"slow\", \
                 \"cutsOnEmphasis\": true/false, \"pauses\": .., \"accelerates\": true/false, \
                 \"hookSeconds\": number, \"rebuildNotes\": \"..\"}"
            }
            Scope::Camera => {
                "Study ONLY the camera. Report:\n\
                 \"camera\": {\"movement\": [..], \"punchIns\": true/false, \"punchInScale\": number, \
                 \"shotSizes\": [..], \"stability\": .., \"rebuildNotes\": \"..\"}"
            }
            Scope::Graphics => {
                "Study ONLY graphics and overlays that are not captions. Report:\n\
                 \"graphics\": {\"present\": true/false, \"kinds\": [..], \"density\": \
                 \"low\"|\"medium\"|\"high\", \"placement\": .., \"animation\": .., \"rebuildNotes\": \"..\"}"
            }
            Scope::Color => {
                "Study ONLY the colour treatment. Report:\n\
                 \"color\": {\"palette\": [hex, ..], \"contrast\": .., \"saturation\": .., \
                 \"shadows\": .., \"highlights\": .., \"grain\": .., \"rebuildNotes\": \"..\"}"
            }
            Scope::Audio => {
                "Study ONLY what you can tell about the sound. Report:\n\
                 \"audio\": {\"music\": true/false, \"mood\": .., \"tempoFeel\": .., \
                 \"voiceLevelVsMusic\": .., \"sfx\": [..], \"rebuildNotes\": \"..\"}"
            }
            Scope::Composition => {
                "Study ONLY how the frame is arranged. Report:\n\
                 \"composition\": {\"aspectRatio\": .., \"subjectPlacement\": .., \"headroom\": .., \
                 \"safeAreas\": .., \"visualDensity\": .., \"rebuildNotes\": \"..\"}"
            }
            Scope::Full => {
                "Describe how this was edited, as a recipe another editor could follow. Report \
                 every block you can judge:\n\
                 \"style\": one line naming the genre,\n\
                 \"hook\": what the first seconds do,\n\
                 \"structure\": the shape of the piece,\n\
                 \"pacing\": {\"averageShotSeconds\": number, \"cutRhythm\": .., \"cutsOnEmphasis\": bool},\n\
                 \"captions\": {\"present\": bool, \"position\": .., \"typeface\": .., \"wordsPerPhrase\": number, \"activeWordHighlight\": bool, \"entrance\": ..},\n\
                 \"camera\": {\"punchIns\": bool, \"movement\": [..]},\n\
                 \"transitions\": {\"types\": [..], \"typicalDurationSeconds\": number},\n\
                 \"graphics\": {\"density\": ..},\n\
                 \"color\": {\"palette\": [hex, ..], \"contrast\": ..},\n\
                 \"audio\": {\"music\": bool, \"mood\": ..},\n\
                 \"rebuildNotes\": one paragraph of direction"
            }
        }
    }
}

pub async fn analyze(
    ws: &Workspace,
    api_key: &str,
    configured_model: &str,
    args: &Value,
) -> Result<Value, String> {
    let url = args
        .get("url")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|u| !u.is_empty())
        .ok_or("no reference url was given")?;

    let source = classify(url);
    if source != Source::YouTube {
        return Err(unsupported(&source, url));
    }

    let scope = Scope::parse(args.get("scope").and_then(Value::as_str).unwrap_or("full"));
    let window = window_of(args);
    let instruction = args
        .get("instruction")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|i| !i.is_empty());

    let prompt = build_prompt(scope, window.as_deref(), instruction);
    let messages = json!([
        { "role": "system", "content": SYSTEM },
        { "role": "user", "content": [
            { "type": "text", "text": prompt },
            { "type": "video_url", "video_url": { "url": url } }
        ]}
    ]);

    let pinned = !configured_model.trim().is_empty();
    let first = if pinned {
        configured_model.trim().to_string()
    } else {
        DEFAULT_MODEL.to_string()
    };

    let (mut analysis, mut used, mut confidence) =
        watch(api_key, &first, &messages, &prompt).await?;

    // A first pass that is not sure of itself is worth one second opinion — but
    // only when the user has not chosen the model themselves.
    let mut escalated = false;
    if !pinned && confidence < ESCALATE_BELOW {
        if let Ok((better, model_used, better_confidence)) =
            watch(api_key, STRONGER_MODEL, &messages, &prompt).await
        {
            if better_confidence > confidence {
                analysis = better;
                used = model_used;
                confidence = better_confidence;
                escalated = true;
            }
        }
    }

    let saved = save(ws, args, url, scope, &analysis)?;

    Ok(json!({
        "url": url,
        "scope": scope.name(),
        "model": used,
        "escalated": escalated,
        "confidence": confidence,
        "window": window,
        "analysis": analysis,
        "saved": saved,
        "note": "The reference was watched where it lives. Nothing was downloaded.",
    }))
}

/// One look at the reference, checked for having actually happened.
async fn watch(
    api_key: &str,
    model_id: &str,
    messages: &Value,
    prompt: &str,
) -> Result<(Value, String, f64), String> {
    let response = model::generate(api_key, model_id, messages.clone(), None).await?;

    // The reference is only analysed if it was really watched. An unresolvable
    // link is accepted by the API and answered from imagination, and the token
    // count is the only thing that tells them apart.
    let prompt_tokens = response
        .pointer("/usage/prompt_tokens")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let text_tokens = (prompt.len() / 4) as u64 + SYSTEM.len() as u64 / 4;
    if prompt_tokens < text_tokens + MIN_VIDEO_TOKENS {
        return Err(
            "That link was accepted but no video came back with it — the reference could not be \
             opened, so anything said about it would be invented. Check the link is public and \
             still live. If it is a private or platform-locked video, upload the clip here \
             instead and it can be looked at directly."
                .into(),
        );
    }

    let answer = response
        .pointer("/choices/0/message")
        .map(text_of)
        .unwrap_or_default();
    let analysis = parse_json(&answer)?;

    if analysis
        .get("error")
        .and_then(Value::as_str)
        .map(|e| e.eq_ignore_ascii_case("NO_VIDEO"))
        .unwrap_or(false)
    {
        return Err(
            "The model could not see the reference video. Nothing was downloaded and nothing was \
             guessed. Upload the clip here and it can be looked at directly."
                .into(),
        );
    }

    let confidence = analysis
        .get("confidence")
        .and_then(Value::as_f64)
        .unwrap_or(0.5)
        .clamp(0.0, 1.0);
    let used = response
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or(model_id)
        .to_string();
    Ok((analysis, used, confidence))
}

const SYSTEM: &str = "You are the eye of a video editor. You watch a reference and report how it \
was made, precisely enough for someone to rebuild that look without ever seeing it. \
Answer with JSON only — no prose around it, no code fences. Every answer includes a \
\"confidence\" between 0 and 1 for how sure you are of what you saw. \
Report only what is actually on screen: if a detail is not visible, use null rather than a \
plausible guess. If you cannot see the video at all, answer exactly {\"error\":\"NO_VIDEO\"}.";

fn build_prompt(scope: Scope, window: Option<&str>, instruction: Option<&str>) -> String {
    let mut prompt = String::new();
    if let Some(window) = window {
        prompt.push_str(&format!(
            "Look at {} of this video and report on that part only. Ignore the rest.\n\n",
            window
        ));
    }
    prompt.push_str(scope.brief());
    prompt.push_str("\n\nAlso include \"confidence\": a number between 0 and 1.");
    if let Some(instruction) = instruction {
        prompt.push_str(&format!(
            "\n\nWhat the person actually asked for: \"{}\". Let that decide what matters most.",
            instruction
        ));
    }
    prompt
}

/// A described time range, for the model to attend to.
///
/// The range narrows what is *reported*, not what is sent: the provider takes
/// the whole video however it is asked, so this is honest about being an
/// instruction rather than a saving.
fn window_of(args: &Value) -> Option<String> {
    let start = args.get("start_seconds").and_then(Value::as_f64);
    let end = args.get("end_seconds").and_then(Value::as_f64);
    match (start, end) {
        (Some(start), Some(end)) if end > start => {
            Some(format!("the section from {} to {}", clock(start), clock(end)))
        }
        (Some(start), None) => Some(format!("everything from {} onwards", clock(start))),
        (None, Some(end)) => Some(format!("the first {}", clock(end))),
        _ => None,
    }
}

fn clock(seconds: f64) -> String {
    let total = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", total / 60, total % 60)
}

/// Models wrap JSON in prose and fences however often they are asked not to.
fn parse_json(answer: &str) -> Result<Value, String> {
    let trimmed = answer.trim();
    let body = match trimmed.find("```") {
        Some(start) => {
            let after = &trimmed[start + 3..];
            let after = after.strip_prefix("json").unwrap_or(after);
            after.split("```").next().unwrap_or(after).trim()
        }
        None => trimmed,
    };
    let body = match (body.find('{'), body.rfind('}')) {
        (Some(start), Some(end)) if end > start => &body[start..=end],
        _ => body,
    };
    serde_json::from_str::<Value>(body).map_err(|e| {
        format!(
            "the analysis did not come back as usable JSON ({}). What it said: {}",
            e,
            answer.chars().take(300).collect::<String>()
        )
    })
}

fn save(
    ws: &Workspace,
    args: &Value,
    url: &str,
    scope: Scope,
    analysis: &Value,
) -> Result<String, String> {
    let name = args
        .get("save_as")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.to_string())
        .unwrap_or_else(|| format!("references/{}-{}.json", slug(url), scope.name()));
    let name = if name.ends_with(".json") {
        name
    } else {
        format!("{}.json", name)
    };

    let path = ws.resolve(&name);
    if !ws.contains(&path) {
        return Err(format!("'{}' is outside the workspace.", name));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("cannot create '{}': {}", parent.display(), e))?;
    }
    let document = json!({
        "source": url,
        "scope": scope.name(),
        "analysis": analysis,
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&document).unwrap_or_default(),
    )
    .map_err(|e| format!("cannot save '{}': {}", ws.rel(&path), e))?;
    Ok(ws.rel(&path))
}

fn slug(url: &str) -> String {
    let id: String = url
        .rsplit(['/', '=', '?'])
        .find(|part| !part.is_empty() && part.len() > 4)
        .unwrap_or("reference")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    if id.is_empty() {
        "reference".into()
    } else {
        id
    }
}

/// What to say when a reference cannot be watched from here. Never "I'll
/// download it instead".
fn unsupported(source: &Source, url: &str) -> String {
    match source {
        Source::Instagram => format!(
            "Instagram links cannot be opened by the video viewer available here ({}), and the \
             reference is not going to be downloaded. Ask the user to save the clip and add it to \
             the chat, and it can be looked at directly with `see`.",
            url
        ),
        Source::DirectMedia => format!(
            "A direct media link ({}) is refused by the provider — it only opens YouTube links. \
             Ask the user to add the file to the chat instead, and look at it with `see`.",
            url
        ),
        _ => format!(
            "{} is not a video reference that can be watched from here. YouTube links work; \
             anything else needs the file itself, added to the chat and looked at with `see`. \
             The reference is not going to be downloaded.",
            url
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_youtube_link_is_watchable_and_the_rest_is_not() {
        for url in [
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            "https://youtu.be/aqz-KE-bpKQ",
            "https://www.youtube.com/shorts/abc123",
            "https://m.youtube.com/watch?v=x",
        ] {
            assert_eq!(classify(url), Source::YouTube, "{}", url);
        }
        assert_eq!(classify("https://www.instagram.com/reel/C2X/"), Source::Instagram);
        assert_eq!(classify("https://cdn.example.com/clip.mp4"), Source::DirectMedia);
        assert_eq!(classify("https://vimeo.com/12345"), Source::Unknown);
        // A lookalike host must not pass for YouTube.
        assert_eq!(classify("https://youtube.com.evil.test/watch?v=x"), Source::Unknown);
    }

    #[tokio::test]
    async fn an_unwatchable_reference_is_refused_and_never_fetched() {
        let root = std::env::temp_dir().join("sirvibe-reference-refuse");
        std::fs::create_dir_all(&root).unwrap();
        let ws = Workspace::open(&root.to_string_lossy()).unwrap();

        for url in [
            "https://www.instagram.com/reel/C2Xm1YQr8kM/",
            "https://cdn.example.com/reference.mp4",
            "https://vimeo.com/999",
        ] {
            let err = analyze(&ws, "key", "", &json!({ "url": url }))
                .await
                .unwrap_err();
            assert!(
                err.contains("not going to be downloaded") || err.contains("look at it with `see`"),
                "it must refuse without offering to download: {}",
                err
            );
            assert!(!err.to_lowercase().contains("yt-dlp"), "{}", err);
        }
    }

    #[test]
    fn each_kind_of_question_asks_for_only_what_it_needs() {
        let captions = Scope::Captions.brief();
        assert!(captions.contains("ONLY the on-screen captions"));
        assert!(captions.contains("activeWordHighlight"));
        assert!(captions.contains("wordsPerPhrase"));
        // A caption question must not drag in the rest of the grammar.
        assert!(!captions.contains("averageShotSeconds"), "captions asked about pacing");
        assert!(!captions.contains("punchIns"), "captions asked about the camera");

        let pacing = Scope::Pacing.brief();
        assert!(pacing.contains("averageShotSeconds"));
        assert!(!pacing.contains("typeface"), "pacing asked about type");

        // The full brief is the one that covers everything.
        let full = Scope::Full.brief();
        for expected in ["pacing", "captions", "camera", "transitions", "color", "audio"] {
            assert!(full.contains(expected), "the full recipe is missing {}", expected);
        }
    }

    #[test]
    fn the_words_a_user_would_use_reach_the_right_scope() {
        assert_eq!(Scope::parse("captions"), Scope::Captions);
        assert_eq!(Scope::parse("Subtitles"), Scope::Captions);
        assert_eq!(Scope::parse("cuts"), Scope::Pacing);
        assert_eq!(Scope::parse("colour"), Scope::Color);
        assert_eq!(Scope::parse("anything else"), Scope::Full);
    }

    #[test]
    fn a_section_is_asked_for_in_the_prompt() {
        let window = window_of(&json!({ "start_seconds": 80.0, "end_seconds": 100.0 }));
        assert_eq!(window.as_deref(), Some("the section from 1:20 to 1:40"));
        let prompt = build_prompt(Scope::Captions, window.as_deref(), None);
        assert!(prompt.starts_with("Look at the section from 1:20 to 1:40"), "{}", prompt);

        assert_eq!(
            window_of(&json!({ "end_seconds": 15.0 })).as_deref(),
            Some("the first 0:15")
        );
        assert_eq!(window_of(&json!({})), None);
    }

    #[test]
    fn json_survives_the_wrapping_a_model_puts_around_it() {
        let fenced = "Here you go:\n```json\n{\"confidence\":0.9,\"captions\":{}}\n```\nHope that helps.";
        assert_eq!(parse_json(fenced).unwrap()["confidence"], 0.9);
        assert_eq!(parse_json("{\"confidence\":0.4}").unwrap()["confidence"], 0.4);
        assert!(parse_json("not json at all").is_err());
    }

    #[test]
    fn a_reference_is_filed_under_a_name_that_says_what_it_is() {
        let root = std::env::temp_dir().join("sirvibe-reference-save");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let ws = Workspace::open(&root.to_string_lossy()).unwrap();

        let saved = save(
            &ws,
            &json!({}),
            "https://www.youtube.com/watch?v=aqz-KE-bpKQ",
            Scope::Captions,
            &json!({ "confidence": 0.9 }),
        )
        .unwrap();
        assert!(saved.starts_with("references/"), "{}", saved);
        assert!(saved.ends_with("-captions.json"), "{}", saved);
        let written: Value =
            serde_json::from_str(&std::fs::read_to_string(ws.resolve(&saved)).unwrap()).unwrap();
        assert_eq!(written["scope"], "captions");
        assert_eq!(written["analysis"]["confidence"], 0.9);

        // And it cannot be written outside the workspace.
        assert!(save(&ws, &json!({ "save_as": "../escape.json" }), "u", Scope::Full, &json!({}))
            .unwrap_err()
            .contains("outside the workspace"));
    }
}

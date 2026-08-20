//! Model provider. The agent talks to a generic chat-completions interface;
//! OpenRouter is the implementation behind it. Requests are made from the
//! native layer so the API key never crosses into the webview.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

const OPENROUTER: &str = "https://openrouter.ai/api/v1";

/// Always OpenRouter in a release build. Debug builds may be pointed at a local
/// stand-in so the agent loop can be tested without spending tokens; the
/// override is compiled out entirely when it is not a debug build, so a stray
/// environment variable can never redirect a user's API key.
fn base() -> String {
    #[cfg(debug_assertions)]
    if let Ok(url) = std::env::var("SIRVIBE_MODEL_BASE_URL") {
        if !url.trim().is_empty() {
            return url;
        }
    }
    OPENROUTER.to_string()
}
const APP_TITLE: &str = "ePlug Video Agent";
const APP_URL: &str = "https://github.com/eplug/video-agent";

#[derive(Serialize, Clone)]
pub struct DeltaEvent {
    pub stream_id: String,
    pub kind: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct AssistantMessage {
    pub content: String,
    pub reasoning: String,
    pub tool_calls: Vec<ToolCall>,
    pub finish_reason: Option<String>,
    pub usage: Option<Value>,
    pub model: Option<String>,
}

#[derive(Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Value>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    error: Option<Value>,
}

#[derive(Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<Delta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ToolCallDelta>>,
}

#[derive(Deserialize)]
struct ToolCallDelta {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<FunctionDelta>,
}

#[derive(Deserialize)]
struct FunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

pub type Cancellations = Arc<Mutex<HashSet<String>>>;

fn is_cancelled(cancel: &Cancellations, stream_id: &str) -> bool {
    cancel
        .lock()
        .map(|c| c.contains(stream_id))
        .unwrap_or(false)
}

#[allow(clippy::too_many_arguments)]
pub async fn chat(
    app: &AppHandle,
    api_key: &str,
    model: &str,
    messages: Value,
    tools: Value,
    stream_id: &str,
    cancel: Cancellations,
) -> Result<AssistantMessage, String> {
    if api_key.trim().is_empty() {
        return Err("No OpenRouter API key is configured. Add one in Settings.".into());
    }
    if model.trim().is_empty() {
        return Err("No model is selected. Choose one in Settings.".into());
    }

    let mut body = json!({
        "model": model,
        "messages": messages,
        "stream": true,
    });
    if tools.as_array().map(|t| !t.is_empty()).unwrap_or(false) {
        body["tools"] = tools;
        body["tool_choice"] = json!("auto");
    }

    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(format!("{}/chat/completions", base()))
        .bearer_auth(api_key)
        .header("HTTP-Referer", APP_URL)
        .header("X-Title", APP_TITLE)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("could not reach OpenRouter: {}", e))?;

    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(format_api_error(status.as_u16(), &text));
    }

    let mut acc = Accumulator::default();
    let mut buffer = String::new();
    let mut stream = response.bytes_stream();

    while let Some(chunk) = stream.next().await {
        if is_cancelled(&cancel, stream_id) {
            return Err("cancelled".into());
        }
        let bytes = chunk.map_err(|e| format!("stream interrupted: {}", e))?;
        buffer.push_str(&String::from_utf8_lossy(&bytes));

        while let Some(idx) = buffer.find('\n') {
            let line = buffer[..idx].to_string();
            buffer.drain(..idx + 1);
            for (kind, text) in acc.push_line(&line)? {
                emit(app, stream_id, kind, &text);
            }
        }
    }
    // A final line with no trailing newline.
    for (kind, text) in acc.push_line(&buffer)? {
        emit(app, stream_id, kind, &text);
    }

    Ok(acc.finish(stream_id))
}

/// Assembles a streamed chat completion. Content and reasoning arrive as text
/// fragments; tool calls arrive as fragments too, addressed by index, with the
/// arguments JSON split across any number of chunks.
#[derive(Default)]
pub struct Accumulator {
    message: AssistantMessage,
    pending: Vec<(String, String, String)>,
}

impl Accumulator {
    /// Feed one SSE line. Returns the (kind, text) fragments to stream to the UI.
    pub fn push_line(&mut self, line: &str) -> Result<Vec<(&'static str, String)>, String> {
        let line = line.trim();
        // Keep-alive comments such as `: OPENROUTER PROCESSING` and blank
        // separator lines carry nothing.
        let payload = match line.strip_prefix("data:") {
            Some(p) => p.trim(),
            None => return Ok(Vec::new()),
        };
        if payload == "[DONE]" || payload.is_empty() {
            return Ok(Vec::new());
        }
        let parsed: StreamChunk = match serde_json::from_str(payload) {
            Ok(p) => p,
            // A fragment we cannot parse is not worth failing the whole turn for.
            Err(_) => return Ok(Vec::new()),
        };
        if let Some(err) = parsed.error {
            return Err(format!(
                "model error: {}",
                err.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or(&err.to_string())
            ));
        }
        if parsed.usage.is_some() {
            self.message.usage = parsed.usage;
        }
        if let Some(m) = parsed.model {
            self.message.model = Some(m);
        }

        let mut emitted = Vec::new();
        for choice in parsed.choices {
            if let Some(reason) = choice.finish_reason {
                self.message.finish_reason = Some(reason);
            }
            let delta = match choice.delta {
                Some(d) => d,
                None => continue,
            };
            if let Some(text) = delta.content.filter(|t| !t.is_empty()) {
                self.message.content.push_str(&text);
                emitted.push(("text", text));
            }
            if let Some(text) = delta.reasoning.filter(|t| !t.is_empty()) {
                self.message.reasoning.push_str(&text);
                emitted.push(("reasoning", text));
            }
            for tc in delta.tool_calls.unwrap_or_default() {
                let i = tc.index.unwrap_or(0);
                while self.pending.len() <= i {
                    self.pending
                        .push((String::new(), String::new(), String::new()));
                }
                let slot = &mut self.pending[i];
                if let Some(id) = tc.id.filter(|s| !s.is_empty()) {
                    slot.0 = id;
                }
                if let Some(f) = tc.function {
                    if let Some(name) = f.name.filter(|s| !s.is_empty()) {
                        slot.1 = name;
                    }
                    if let Some(args) = f.arguments {
                        slot.2.push_str(&args);
                    }
                }
            }
        }
        Ok(emitted)
    }

    pub fn finish(mut self, stream_id: &str) -> AssistantMessage {
        self.message.tool_calls = self
            .pending
            .into_iter()
            .filter(|(_, name, _)| !name.is_empty())
            .enumerate()
            .map(|(i, (id, name, arguments))| ToolCall {
                id: if id.is_empty() {
                    format!("{}-call-{}", stream_id, i)
                } else {
                    id
                },
                name,
                arguments,
            })
            .collect();
        self.message
    }
}

fn emit(app: &AppHandle, stream_id: &str, kind: &str, text: &str) {
    let _ = app.emit(
        "agent://delta",
        DeltaEvent {
            stream_id: stream_id.to_string(),
            kind: kind.to_string(),
            text: text.to_string(),
        },
    );
}

fn format_api_error(status: u16, body: &str) -> String {
    let detail = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .unwrap_or_else(|| body.chars().take(400).collect());
    match status {
        401 => format!("OpenRouter rejected the API key (401). {}", detail),
        402 => format!("OpenRouter reports insufficient credit (402). {}", detail),
        404 => format!("Model not found on OpenRouter (404). {}", detail),
        429 => format!("Rate limited by OpenRouter (429). {}", detail),
        _ => format!("OpenRouter request failed ({}). {}", status, detail),
    }
}

#[derive(Serialize)]
pub struct ModelInfo {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    pub prompt_price: String,
    pub supports_tools: bool,
    /// The organisation the model comes from — the part of the id before the
    /// slash, which is how OpenRouter namespaces them.
    pub provider: String,
    /// What the model can be given: "text", "image", "audio", "video", "file".
    pub input_modalities: Vec<String>,
    /// What it produces. This is what makes a model an image model or a video
    /// model rather than a text one.
    pub output_modalities: Vec<String>,
    pub completion_price: String,
    pub description: String,
}

pub async fn list_models(api_key: &str) -> Result<Vec<ModelInfo>, String> {
    let client = reqwest::Client::new();
    let mut req = client.get(format!("{}/models", base()));
    if !api_key.trim().is_empty() {
        req = req.bearer_auth(api_key);
    }
    let response = req
        .send()
        .await
        .map_err(|e| format!("could not reach OpenRouter: {}", e))?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format_api_error(status.as_u16(), &text));
    }
    parse_models(&text)
}

pub fn parse_models(text: &str) -> Result<Vec<ModelInfo>, String> {
    let parsed: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let mut models: Vec<ModelInfo> = parsed
        .get("data")
        .and_then(Value::as_array)
        .ok_or("unexpected response from OpenRouter")?
        .iter()
        .map(|m| {
            let params = m
                .get("supported_parameters")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let id = m.get("id").and_then(Value::as_str).unwrap_or("").to_string();
            let architecture = m.get("architecture");
            let price = |key: &str| {
                m.get("pricing")
                    .and_then(|p| p.get(key))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string()
            };
            ModelInfo {
                provider: id.split('/').next().unwrap_or("other").to_string(),
                name: m
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                context_length: m
                    .get("context_length")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                prompt_price: price("prompt"),
                completion_price: price("completion"),
                supports_tools: params.iter().any(|p| p.as_str() == Some("tools")),
                input_modalities: modalities(architecture, "input_modalities", 0),
                output_modalities: modalities(architecture, "output_modalities", 1),
                description: m
                    .get("description")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .chars()
                    .take(300)
                    .collect(),
                id,
            }
        })
        .filter(|m| !m.id.is_empty())
        .collect();
    models.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(models)
}

/// Read the modality lists, falling back to the older `modality` string
/// ("text+image->text") when a model predates the structured fields.
fn modalities(architecture: Option<&Value>, key: &str, side: usize) -> Vec<String> {
    let Some(architecture) = architecture else {
        return vec!["text".into()];
    };
    if let Some(list) = architecture.get(key).and_then(Value::as_array) {
        let found: Vec<String> = list
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_lowercase)
            .collect();
        if !found.is_empty() {
            return found;
        }
    }
    architecture
        .get("modality")
        .and_then(Value::as_str)
        .and_then(|m| m.split("->").nth(side).map(str::to_string))
        .map(|part| part.split('+').map(|p| p.trim().to_lowercase()).collect())
        .unwrap_or_else(|| vec!["text".into()])
}

/// One non-streaming generation on a named model. Separate from `chat` because
/// nothing here is the agent's own conversation: it is the agent commissioning
/// a piece of work — a voiceover, an image, a clip — from a model the user
/// named, paid for by the user's own OpenRouter key.
pub async fn generate(
    api_key: &str,
    model: &str,
    messages: Value,
    modalities: Option<Vec<String>>,
) -> Result<Value, String> {
    if api_key.trim().is_empty() {
        return Err("No OpenRouter API key is configured. Add one in Settings.".into());
    }
    if model.trim().is_empty() {
        return Err("No model id was given. Use find_models to look one up.".into());
    }

    let mut body = json!({ "model": model, "messages": messages });
    if let Some(modalities) = modalities {
        body["modalities"] = json!(modalities);
    }

    let response = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|e| e.to_string())?
        .post(format!("{}/chat/completions", base()))
        .bearer_auth(api_key)
        .header("HTTP-Referer", APP_URL)
        .header("X-Title", APP_TITLE)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("could not reach OpenRouter: {}", e))?;

    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format_api_error(status.as_u16(), &text));
    }
    let parsed: Value = serde_json::from_str(&text)
        .map_err(|_| "OpenRouter returned something that is not JSON".to_string())?;
    if let Some(error) = parsed.get("error") {
        return Err(format!(
            "{} refused the request: {}",
            model,
            error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
        ));
    }
    Ok(parsed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(acc: &mut Accumulator, lines: &[&str]) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        for line in lines {
            out.extend(acc.push_line(line).unwrap());
        }
        out
    }

    #[test]
    fn assembles_streamed_text() {
        let mut acc = Accumulator::default();
        let emitted = feed(
            &mut acc,
            &[
                r#"data: {"model":"anthropic/claude-sonnet-4.5","choices":[{"delta":{"content":"I'll "}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"probe the "}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"footage."},"finish_reason":"stop"}]}"#,
                "data: [DONE]",
            ],
        );
        assert_eq!(emitted.len(), 3);
        assert_eq!(emitted[0], ("text", "I'll ".to_string()));
        let msg = acc.finish("s1");
        assert_eq!(msg.content, "I'll probe the footage.");
        assert_eq!(msg.finish_reason.as_deref(), Some("stop"));
        assert_eq!(msg.model.as_deref(), Some("anthropic/claude-sonnet-4.5"));
        assert!(msg.tool_calls.is_empty());
    }

    #[test]
    fn reassembles_tool_call_arguments_split_across_chunks() {
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","function":{"name":"shell","arguments":""}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"comm"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"and\": \"ffprobe a.mp4\"}"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            ],
        );
        let msg = acc.finish("s1");
        assert_eq!(msg.tool_calls.len(), 1);
        let call = &msg.tool_calls[0];
        assert_eq!(call.id, "call_abc");
        assert_eq!(call.name, "shell");
        let args: Value = serde_json::from_str(&call.arguments).expect("valid JSON");
        assert_eq!(args["command"], "ffprobe a.mp4");
    }

    #[test]
    fn keeps_parallel_tool_calls_separate() {
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"fs_read","arguments":"{\"path\":"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"fs_stat","arguments":"{\"path\":"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"\"out.mp4\"}"}}]}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"plan.md\"}"}}]}}]}"#,
            ],
        );
        let msg = acc.finish("s1");
        assert_eq!(msg.tool_calls.len(), 2);
        assert_eq!(msg.tool_calls[0].name, "fs_read");
        assert_eq!(msg.tool_calls[0].arguments, r#"{"path":"plan.md"}"#);
        assert_eq!(msg.tool_calls[1].name, "fs_stat");
        assert_eq!(msg.tool_calls[1].arguments, r#"{"path":"out.mp4"}"#);
    }

    #[test]
    fn text_and_tool_calls_can_arrive_together() {
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"content":"Let me look."}}]}"#,
                r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"fs_list","arguments":"{}"}}]}}]}"#,
            ],
        );
        let msg = acc.finish("s1");
        assert_eq!(msg.content, "Let me look.");
        assert_eq!(msg.tool_calls.len(), 1);
    }

    #[test]
    fn ignores_keepalives_blank_lines_and_junk() {
        let mut acc = Accumulator::default();
        let emitted = feed(
            &mut acc,
            &[
                ": OPENROUTER PROCESSING",
                "",
                "data: ",
                "data: {not json",
                r#"data: {"choices":[{"delta":{"content":"ok"}}]}"#,
            ],
        );
        assert_eq!(emitted, vec![("text", "ok".to_string())]);
    }

    #[test]
    fn reasoning_is_kept_separate_from_the_reply() {
        let mut acc = Accumulator::default();
        let emitted = feed(
            &mut acc,
            &[
                r#"data: {"choices":[{"delta":{"reasoning":"The file is 4K…"}}]}"#,
                r#"data: {"choices":[{"delta":{"content":"Downscaling."}}]}"#,
            ],
        );
        assert_eq!(emitted[0].0, "reasoning");
        assert_eq!(emitted[1].0, "text");
        let msg = acc.finish("s1");
        assert_eq!(msg.reasoning, "The file is 4K…");
        assert_eq!(msg.content, "Downscaling.");
    }

    #[test]
    fn a_mid_stream_error_surfaces_to_the_caller() {
        let mut acc = Accumulator::default();
        let err = acc
            .push_line(r#"data: {"error":{"message":"rate limited","code":429}}"#)
            .unwrap_err();
        assert!(err.contains("rate limited"));
    }

    #[test]
    fn a_tool_call_without_an_id_still_gets_one() {
        // Some providers omit the id; the loop needs one to match the result.
        let mut acc = Accumulator::default();
        feed(
            &mut acc,
            &[r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"fs_list","arguments":"{}"}}]}}]}"#],
        );
        let msg = acc.finish("stream7");
        assert_eq!(msg.tool_calls[0].id, "stream7-call-0");
    }

    #[test]
    fn model_listings_are_parsed_and_tool_support_detected() {
        let body = r#"{"data":[
          {"id":"anthropic/claude-sonnet-4.5","name":"Claude Sonnet 4.5",
           "context_length":200000,"pricing":{"prompt":"0.000003"},
           "supported_parameters":["tools","temperature"]},
          {"id":"some/text-only","name":"Text Only",
           "context_length":8192,"pricing":{"prompt":"0"},
           "supported_parameters":["temperature"]},
          {"name":"no id here"}
        ]}"#;
        let models = parse_models(body).unwrap();
        // Entries without an id are unusable and dropped; the rest sort by id.
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "anthropic/claude-sonnet-4.5");
        assert_eq!(models[0].context_length, 200000);
        assert!(models[0].supports_tools);
        assert_eq!(models[0].prompt_price, "0.000003");
        assert!(!models[1].supports_tools);
    }

    #[test]
    fn a_model_is_grouped_by_provider_and_known_by_what_it_produces() {
        let body = r#"{"data":[
          {"id":"google/gemini-2.5-flash-image","name":"Google: Gemini Flash Image",
           "pricing":{"prompt":"0.0000003","completion":"0.0000025"},
           "architecture":{"input_modalities":["text","image"],"output_modalities":["image","text"]}},
          {"id":"openai/gpt-4o-audio","name":"OpenAI: GPT-4o Audio",
           "pricing":{"prompt":"0"},
           "architecture":{"modality":"text+audio->audio"}},
          {"id":"plain/model","name":"Plain","pricing":{"prompt":"0"}}
        ]}"#;
        let models = parse_models(body).unwrap();

        let image = &models[0];
        assert_eq!(image.provider, "google");
        assert_eq!(image.output_modalities, vec!["image", "text"]);
        assert_eq!(image.input_modalities, vec!["text", "image"]);
        assert_eq!(image.completion_price, "0.0000025");

        // Older entries only carry the combined "in->out" string.
        let audio = &models[1];
        assert_eq!(audio.provider, "openai");
        assert_eq!(audio.output_modalities, vec!["audio"]);
        assert_eq!(audio.input_modalities, vec!["text", "audio"]);

        // And one with no architecture at all is treated as plain text.
        assert_eq!(models[2].output_modalities, vec!["text"]);
    }

    #[test]
    fn an_unexpected_model_listing_is_an_error_not_a_panic() {
        assert!(parse_models("{}").is_err());
        assert!(parse_models("not json").is_err());
    }

    #[test]
    fn api_errors_are_reported_in_plain_language() {
        let msg = format_api_error(401, r#"{"error":{"message":"No auth credentials found"}}"#);
        assert!(msg.contains("rejected the API key"));
        assert!(msg.contains("No auth credentials found"));
        assert!(format_api_error(402, "{}").contains("insufficient credit"));
    }
}

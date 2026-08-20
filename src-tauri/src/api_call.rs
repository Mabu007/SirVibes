//! Executing a call against a connected API.
//!
//! This is the only place a stored credential is read, and it is read at the
//! last possible moment — after the runtime has approved the call and while the
//! request is being signed. The credential is never returned, logged, or put
//! anywhere the model can see.
//!
//! Everything here is deliberately bounded: a timeout, a request ceiling, a
//! response ceiling, a concurrency limit, a loop detector and a small retry
//! budget. None of them cap how many calls a task may legitimately make — the
//! user approving each call is the authorisation control.

use crate::apis::{ApiConnection, AuthConfig};
use crate::secrets::{redact, SecretStore};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{oneshot, Semaphore};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct CallLimits {
    pub timeout_secs: u64,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
    pub max_retries: u32,
    pub max_concurrent: usize,
    /// How many identical calls within the window count as a loop.
    pub repeat_threshold: usize,
    pub repeat_window_secs: u64,
}

impl Default for CallLimits {
    fn default() -> Self {
        Self {
            timeout_secs: 60,
            max_request_bytes: 1_000_000,
            max_response_bytes: 1_000_000,
            max_retries: 2,
            max_concurrent: 4,
            repeat_threshold: 4,
            repeat_window_secs: 120,
        }
    }
}

#[derive(Deserialize, Debug, Default, Clone)]
#[serde(default)]
pub struct ApiRequest {
    pub capability_id: Option<String>,
    pub method: Option<String>,
    pub path: Option<String>,
    /// Values for `{placeholders}` in the path, e.g. {"datasetId": "abc"}.
    pub path_params: Option<Value>,
    pub query: Option<Value>,
    pub body: Option<Value>,
    pub purpose: Option<String>,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct ApiUsage {
    pub api_id: String,
    pub calls: u64,
    pub errors: u64,
    pub bytes_received: u64,
    pub last_ms: u64,
}

/// Shared, per-process state for in-flight and recent API activity.
pub struct CallGuard {
    permits: Arc<Semaphore>,
    recent: Mutex<Vec<(u64, Instant)>>,
    cancels: Mutex<HashMap<String, oneshot::Sender<()>>>,
    usage: Mutex<HashMap<String, ApiUsage>>,
    limits: Mutex<CallLimits>,
}

impl CallGuard {
    pub fn new(limits: CallLimits) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(limits.max_concurrent.max(1))),
            recent: Mutex::new(Vec::new()),
            cancels: Mutex::new(HashMap::new()),
            usage: Mutex::new(HashMap::new()),
            limits: Mutex::new(limits),
        }
    }

    pub fn limits(&self) -> CallLimits {
        self.limits.lock().map(|l| l.clone()).unwrap_or_default()
    }

    pub fn set_limits(&self, limits: CallLimits) {
        if let Ok(mut l) = self.limits.lock() {
            *l = limits;
        }
    }

    pub fn usage(&self) -> Vec<ApiUsage> {
        self.usage
            .lock()
            .map(|u| u.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Stop a request that is in flight. Returns false if it already finished.
    pub fn cancel(&self, call_id: &str) -> bool {
        let sender = self
            .cancels
            .lock()
            .ok()
            .and_then(|mut c| c.remove(call_id));
        match sender {
            Some(tx) => tx.send(()).is_ok(),
            None => false,
        }
    }

    fn record(&self, api_id: &str, ok: bool, bytes: u64) {
        if let Ok(mut usage) = self.usage.lock() {
            let entry = usage.entry(api_id.to_string()).or_insert_with(|| ApiUsage {
                api_id: api_id.to_string(),
                ..Default::default()
            });
            entry.calls += 1;
            if !ok {
                entry.errors += 1;
            }
            entry.bytes_received += bytes;
            entry.last_ms = crate::apis::now_ms();
        }
    }

    /// True when this exact call has already been made too many times recently.
    fn is_looping(&self, signature: u64) -> bool {
        let limits = self.limits();
        let window = Duration::from_secs(limits.repeat_window_secs);
        let mut recent = match self.recent.lock() {
            Ok(r) => r,
            Err(_) => return false,
        };
        let now = Instant::now();
        recent.retain(|(_, at)| now.duration_since(*at) < window);
        let seen = recent.iter().filter(|(h, _)| *h == signature).count();
        recent.push((signature, now));
        if recent.len() > 500 {
            let drop = recent.len() - 500;
            recent.drain(0..drop);
        }
        seen + 1 > limits.repeat_threshold
    }
}

fn signature(api_id: &str, method: &str, url: &str, body: &Option<Value>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    api_id.hash(&mut hasher);
    method.hash(&mut hasher);
    url.hash(&mut hasher);
    body.as_ref().map(|b| b.to_string()).hash(&mut hasher);
    hasher.finish()
}

/// Resolve the request against the connection, refusing anything that would
/// send the credential somewhere other than this API's own origin.
pub fn resolve_target(
    connection: &ApiConnection,
    request: &ApiRequest,
) -> Result<(String, String), String> {
    let (method, path) = match &request.capability_id {
        Some(id) => {
            let capability = connection
                .capabilities
                .iter()
                .find(|c| &c.id == id || c.name == *id)
                .ok_or_else(|| {
                    format!(
                        "'{}' is not a known capability of {}. Use search_api_capabilities first.",
                        id, connection.name
                    )
                })?;
            (capability.method.clone(), capability.path.clone())
        }
        None => (
            request
                .method
                .clone()
                .unwrap_or_else(|| "GET".into())
                .to_uppercase(),
            request
                .path
                .clone()
                .ok_or("either capability_id or path is required")?,
        ),
    };

    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err(format!("unsupported HTTP method '{}'", method));
    }

    let path = substitute_path(&path, request.path_params.as_ref())?;

    let base = connection
        .base_url
        .as_deref()
        .map(|b| b.trim_end_matches('/').to_string());

    let url = if path.starts_with("http://") || path.starts_with("https://") {
        // An absolute URL is only allowed if it stays on this API's own host,
        // so a poisoned document cannot redirect the credential elsewhere.
        let base = base.as_deref().ok_or_else(|| {
            needs_base_url(
                connection,
                origin(&path).or_else(|| crate::apis::suggested_base_url(connection.doc_url.as_deref())),
            )
        })?;
        if origin(&path).is_some() && origin(&path) == origin(base) {
            path.clone()
        } else {
            return Err(format!(
                "refused: '{}' is not on {}'s own host",
                path, connection.name
            ));
        }
    } else {
        let base = base.ok_or_else(|| {
            needs_base_url(
                connection,
                crate::apis::suggested_base_url(connection.doc_url.as_deref()),
            )
        })?;
        format!("{}/{}", base, path.trim_start_matches('/'))
    };

    Ok((method, url))
}

/// Fill `{placeholders}` from `path_params`. An unfilled placeholder is an
/// error rather than a literal brace in a URL, which would otherwise produce a
/// confusing 404 from the API.
fn substitute_path(path: &str, params: Option<&Value>) -> Result<String, String> {
    let mut out = path.to_string();
    if let Some(map) = params.and_then(Value::as_object) {
        for (key, value) in map {
            let text = value
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| value.to_string());
            out = out.replace(&format!("{{{}}}", key), &urlencode(&text));
        }
    }
    if let Some(start) = out.find('{') {
        if let Some(end) = out[start..].find('}') {
            return Err(format!(
                "the path still needs a value for '{}'. Pass it in path_params.",
                &out[start + 1..start + end]
            ));
        }
    }
    Ok(out)
}

fn urlencode(value: &str) -> String {
    value
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            _ => c
                .to_string()
                .as_bytes()
                .iter()
                .map(|b| format!("%{:02X}", b))
                .collect(),
        })
        .collect()
}

/// The API is connected and the key is stored; only the destination is
/// unknown. That is the agent's to work out from the documentation and set
/// with `configure_api` — not something to send the user hunting for.
fn needs_base_url(connection: &ApiConnection, suggestion: Option<String>) -> String {
    format!(
        "{} has no base URL yet, so there is nowhere to send this request. Work out its API root \
         from the documentation and set it with configure_api (api_id: '{}'){}. \
         Check how the key must be sent while you are there — not every API uses a bearer token.",
        connection.name,
        connection.id,
        suggestion
            .map(|s| format!(". It is most likely {}", s))
            .unwrap_or_default(),
    )
}

fn origin(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split('/').next()?;
    Some(format!("{}://{}", scheme.to_lowercase(), host.to_lowercase()))
}

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    connection: &ApiConnection,
    request: &ApiRequest,
    secrets: &SecretStore,
    guard: &CallGuard,
    call_id: &str,
) -> Result<Value, String> {
    let limits = guard.limits();
    let (method, url) = resolve_target(connection, request)?;

    let body_text = match &request.body {
        Some(body) => {
            let text = serde_json::to_string(body).map_err(|_| "request body is not valid JSON")?;
            if text.len() > limits.max_request_bytes {
                return Err(format!(
                    "request body is {} bytes, over the {} byte limit. Send less data.",
                    text.len(),
                    limits.max_request_bytes
                ));
            }
            Some(text)
        }
        None => None,
    };

    if guard.is_looping(signature(&connection.id, &method, &url, &request.body)) {
        return Err(format!(
            "This identical call to {} has already been made {} times in the last {} seconds. \
             Something is looping — change the approach or ask the user what to do instead.",
            connection.name,
            limits.repeat_threshold,
            limits.repeat_window_secs
        ));
    }

    let _permit = guard
        .permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| "the request queue is closed")?;

    let (cancel_tx, mut cancel_rx) = oneshot::channel::<()>();
    if let Ok(mut cancels) = guard.cancels.lock() {
        cancels.insert(call_id.to_string(), cancel_tx);
    }
    let _cleanup = CancelCleanup(guard, call_id.to_string());

    // Read the secret here, immediately before signing, and never before.
    let secret = secrets.get(&connection.id);
    if secret.is_none() && connection.auth != AuthConfig::None {
        return Err(format!(
            "No API key is stored for {}. Ask the user to open the APIs panel in the sidebar and add one under Manage.",
            connection.name
        ));
    }

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(limits.timeout_secs))
        .user_agent("SirVibe/0.1")
        .build()
        .map_err(|e| format!("could not start a network client: {}", e))?;

    let started = Instant::now();
    let mut attempt = 0u32;
    loop {
        let mut builder = match method.as_str() {
            "GET" => client.get(&url),
            "POST" => client.post(&url),
            "PUT" => client.put(&url),
            "PATCH" => client.patch(&url),
            "DELETE" => client.delete(&url),
            _ => client.head(&url),
        };

        if let Some(query) = request.query.as_ref().and_then(Value::as_object) {
            let pairs: Vec<(String, String)> = query
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()),
                    )
                })
                .collect();
            builder = builder.query(&pairs);
        }

        // Credential injection. This is the only use of the plaintext.
        if let Some(secret) = secret.as_deref() {
            builder = match &connection.auth {
                AuthConfig::Bearer => builder.bearer_auth(secret),
                AuthConfig::Header { name, prefix } => builder.header(
                    name.as_str(),
                    if prefix.is_empty() {
                        secret.to_string()
                    } else {
                        format!("{} {}", prefix, secret)
                    },
                ),
                AuthConfig::QueryParam { name } => {
                    builder.query(&[(name.as_str(), secret)])
                }
                AuthConfig::None => builder,
            };
        }

        if let Some(text) = &body_text {
            builder = builder
                .header("content-type", "application/json")
                .body(text.clone());
        }

        let send = builder.send();
        tokio::pin!(send);

        let response = tokio::select! {
            _ = &mut cancel_rx => {
                return Err("The user stopped this API call.".into());
            }
            result = &mut send => result,
        };

        match response {
            Ok(response) => {
                let status = response.status();
                if should_retry(status.as_u16()) && attempt < limits.max_retries {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt))).await;
                    continue;
                }
                let content_type = response
                    .headers()
                    .get("content-type")
                    .and_then(|v| v.to_str().ok())
                    .unwrap_or("")
                    .to_string();

                let (text, truncated, received) =
                    read_capped(response, limits.max_response_bytes, &mut cancel_rx).await?;
                let safe = redact(&text, secret.as_deref());
                guard.record(&connection.id, status.is_success(), received);

                if !status.is_success() {
                    return Err(explain_status(
                        status.as_u16(),
                        &connection.name,
                        &safe,
                    ));
                }

                let parsed: Value = serde_json::from_str(&safe)
                    .unwrap_or_else(|_| Value::String(safe.clone()));

                return Ok(json!({
                    "api": connection.name,
                    "method": method,
                    "url": url,
                    "status": status.as_u16(),
                    "content_type": content_type,
                    "duration_ms": started.elapsed().as_millis() as u64,
                    "truncated": truncated,
                    "bytes": received,
                    "body": parsed,
                }));
            }
            Err(error) => {
                let transient = error.is_timeout() || error.is_connect() || error.is_request();
                if transient && attempt < limits.max_retries {
                    attempt += 1;
                    tokio::time::sleep(Duration::from_millis(400 * 2u64.pow(attempt))).await;
                    continue;
                }
                guard.record(&connection.id, false, 0);
                return Err(explain_transport(&error, &connection.name, limits.timeout_secs));
            }
        }
    }
}

struct CancelCleanup<'a>(&'a CallGuard, String);

impl Drop for CancelCleanup<'_> {
    fn drop(&mut self) {
        if let Ok(mut cancels) = self.0.cancels.lock() {
            cancels.remove(&self.1);
        }
    }
}

/// Read the body in chunks and stop at the ceiling, so a huge or endless
/// response cannot exhaust memory or flood the model's context.
async fn read_capped(
    response: reqwest::Response,
    max_bytes: usize,
    cancel: &mut oneshot::Receiver<()>,
) -> Result<(String, bool, u64), String> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut total: u64 = 0;
    let mut truncated = false;
    let mut stream = response.bytes_stream();

    loop {
        let next = tokio::select! {
            _ = &mut *cancel => return Err("The user stopped this API call.".into()),
            chunk = stream.next() => chunk,
        };
        let chunk = match next {
            Some(Ok(c)) => c,
            Some(Err(e)) => return Err(format!("the response was interrupted: {}", e)),
            None => break,
        };
        total += chunk.len() as u64;
        if buffer.len() < max_bytes {
            let room = max_bytes - buffer.len();
            buffer.extend_from_slice(&chunk[..room.min(chunk.len())]);
            if buffer.len() >= max_bytes {
                truncated = true;
            }
        } else {
            truncated = true;
        }
        if truncated && total > (max_bytes as u64) * 4 {
            break; // stop pulling a response we are already discarding
        }
    }

    let text = String::from_utf8_lossy(&buffer).to_string();
    Ok((text, truncated, total))
}

fn should_retry(status: u16) -> bool {
    matches!(status, 408 | 429 | 500 | 502 | 503 | 504)
}

/// Human-readable failures that never carry a credential.
fn explain_status(status: u16, api: &str, body: &str) -> String {
    let detail: String = body.chars().take(400).collect();
    match status {
        401 | 403 => format!(
            "API authentication failed.\n\nThe API key was rejected by {}. Ask the user to check the key in the APIs panel in the sidebar.",
            api
        ),
        404 => format!("{} returned 404 — that endpoint does not exist. Check the path against the API's capabilities.", api),
        408 => format!("{} timed out handling the request.", api),
        422 => format!("{} rejected the parameters (422). {}", api, detail),
        429 => format!(
            "{} is rate limiting these requests (429). Wait before trying again, or slow the workflow down.",
            api
        ),
        500..=599 => format!("{} had a server error ({}). This is on their side, not yours.", api, status),
        _ => format!("{} returned {}. {}", api, status, detail),
    }
}

fn explain_transport(error: &reqwest::Error, api: &str, timeout: u64) -> String {
    if error.is_timeout() {
        format!("{} did not respond within {} seconds.", api, timeout)
    } else if error.is_connect() {
        format!("Could not reach {}. Check the base URL and your network connection.", api)
    } else {
        format!("The request to {} failed before it completed.", api)
    }
}

#[cfg(test)]
mod base_url_tests {
    use super::*;
    use crate::apis::ApiConnection;

    fn deepgram() -> ApiConnection {
        ApiConnection {
            id: "deepgram".into(),
            name: "Deepgram".into(),
            doc_url: Some("https://developers.deepgram.com/docs".into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_missing_base_url_tells_the_agent_to_fix_it_itself() {
        let request = ApiRequest {
            method: Some("POST".into()),
            path: Some("/v1/listen".into()),
            ..Default::default()
        };
        let err = resolve_target(&deepgram(), &request).unwrap_err();
        // The user connected it and stored the key; the rest is the agent's job.
        assert!(err.contains("configure_api"), "{}", err);
        assert!(err.contains("deepgram"), "{}", err);
        assert!(err.contains("https://api.deepgram.com"), "must suggest a root: {}", err);
        assert!(!err.contains("not connected"), "it is connected: {}", err);
    }

    #[test]
    fn once_configured_the_path_resolves() {
        let mut connection = deepgram();
        connection.base_url = Some("https://api.deepgram.com".into());
        let request = ApiRequest {
            method: Some("POST".into()),
            path: Some("/v1/listen".into()),
            ..Default::default()
        };
        let (method, url) = resolve_target(&connection, &request).unwrap();
        assert_eq!(method, "POST");
        assert_eq!(url, "https://api.deepgram.com/v1/listen");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apis::{ApiCapability, ApiConnection};

    fn connection() -> ApiConnection {
        ApiConnection {
            id: "demo".into(),
            name: "Demo".into(),
            base_url: Some("https://api.demo.test/v2".into()),
            capabilities: vec![ApiCapability {
                id: "demo::listacts".into(),
                api_id: "demo".into(),
                name: "listActs".into(),
                method: "GET".into(),
                path: "/acts".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn a_capability_resolves_to_a_method_and_url() {
        let (method, url) = resolve_target(
            &connection(),
            &ApiRequest { capability_id: Some("demo::listacts".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(method, "GET");
        assert_eq!(url, "https://api.demo.test/v2/acts");
    }

    #[test]
    fn a_raw_path_works_for_apis_without_a_spec() {
        let (method, url) = resolve_target(
            &connection(),
            &ApiRequest {
                method: Some("post".into()),
                path: Some("runs/start".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(method, "POST");
        assert_eq!(url, "https://api.demo.test/v2/runs/start");
    }

    #[test]
    fn the_credential_cannot_be_sent_to_another_host() {
        // The decisive test: a poisoned doc or model output must not be able to
        // point an authenticated request at somebody else's server.
        let err = resolve_target(
            &connection(),
            &ApiRequest { path: Some("https://evil.test/collect".into()), ..Default::default() },
        )
        .unwrap_err();
        assert!(err.contains("not on Demo's own host"), "{}", err);

        // Same host, absolute, is fine.
        let (_, url) = resolve_target(
            &connection(),
            &ApiRequest { path: Some("https://api.demo.test/v2/acts".into()), ..Default::default() },
        )
        .unwrap();
        assert_eq!(url, "https://api.demo.test/v2/acts");
    }

    #[test]
    fn path_placeholders_are_filled_from_path_params() {
        let mut conn = connection();
        conn.capabilities[0].path = "/datasets/{datasetId}/items".into();
        let (_, url) = resolve_target(
            &conn,
            &ApiRequest {
                capability_id: Some("demo::listacts".into()),
                path_params: Some(serde_json::json!({ "datasetId": "abc 123" })),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(url, "https://api.demo.test/v2/datasets/abc%20123/items");
    }

    #[test]
    fn a_missing_path_value_is_an_error_not_a_broken_url() {
        let mut conn = connection();
        conn.capabilities[0].path = "/datasets/{datasetId}/items".into();
        let err = resolve_target(
            &conn,
            &ApiRequest { capability_id: Some("demo::listacts".into()), ..Default::default() },
        )
        .unwrap_err();
        assert!(err.contains("datasetId"), "{}", err);
    }

    #[test]
    fn unknown_capabilities_and_methods_are_refused() {
        let err = resolve_target(
            &connection(),
            &ApiRequest { capability_id: Some("demo::nope".into()), ..Default::default() },
        )
        .unwrap_err();
        assert!(err.contains("not a known capability"));

        let err = resolve_target(
            &connection(),
            &ApiRequest {
                method: Some("TRACE".into()),
                path: Some("/x".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(err.contains("unsupported HTTP method"));
    }

    #[test]
    fn repeated_identical_calls_are_detected_as_a_loop() {
        let guard = CallGuard::new(CallLimits { repeat_threshold: 3, ..Default::default() });
        let sig = signature("demo", "GET", "https://api.demo.test/v2/acts", &None);
        assert!(!guard.is_looping(sig));
        assert!(!guard.is_looping(sig));
        assert!(!guard.is_looping(sig));
        assert!(guard.is_looping(sig), "the fourth identical call should trip the detector");

        // A different call is unaffected.
        let other = signature("demo", "GET", "https://api.demo.test/v2/other", &None);
        assert!(!guard.is_looping(other));
    }

    #[test]
    fn cancelling_an_unknown_call_is_harmless() {
        let guard = CallGuard::new(CallLimits::default());
        assert!(!guard.cancel("nope"));
    }

    #[test]
    fn errors_are_readable_and_never_carry_credentials() {
        let message = explain_status(401, "Apify", "Authorization: Bearer sk-live-secret");
        assert!(message.contains("API authentication failed"));
        assert!(message.contains("Apify"));
        assert!(!message.contains("sk-live-secret"));

        assert!(explain_status(429, "Apify", "").contains("rate limiting"));
        assert!(explain_status(404, "Apify", "").contains("does not exist"));
    }

    #[test]
    fn usage_is_recorded_per_api() {
        let guard = CallGuard::new(CallLimits::default());
        guard.record("demo", true, 120);
        guard.record("demo", false, 30);
        let usage = guard.usage();
        let demo = usage.iter().find(|u| u.api_id == "demo").unwrap();
        assert_eq!(demo.calls, 2);
        assert_eq!(demo.errors, 1);
        assert_eq!(demo.bytes_received, 150);
    }

    #[test]
    fn retry_policy_covers_transient_statuses_only() {
        assert!(should_retry(429) && should_retry(503) && should_retry(500));
        assert!(!should_retry(400) && !should_retry(401) && !should_retry(404));
    }
}

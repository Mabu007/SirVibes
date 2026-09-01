//! Composio: the external-app integration layer.
//!
//! Composio brokers OAuth to third-party applications (Gmail, GitHub, Drive,
//! …) so the user never pastes an app's API key into SirVibe. This module is
//! the only place that talks to Composio's REST API.
//!
//! Two credentials matter here and neither one ever leaves this process:
//!
//! * the Composio **project API key**, read from the secret store at the moment
//!   a request is signed, exactly like `api_call.rs` does for connected APIs;
//! * the **connected app's** OAuth tokens, which SirVibe never receives at all —
//!   Composio holds them and injects them server-side when a tool runs.
//!
//! Everything fetched from Composio is third-party data: it is size-capped,
//! redacted, and handed to the model as information, never as instructions.

use crate::secrets::redact;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;

/// Composio's production API. Pinned to v3, the version whose OpenAPI document
/// these calls were written against.
const BASE_URL: &str = "https://backend.composio.dev/api/v3";
const TIMEOUT_SECS: u64 = 30;
/// A toolkit catalogue or a tool listing is third-party text. Cap it so a
/// large response cannot exhaust memory or flood the model's context.
const MAX_RESPONSE_BYTES: usize = 512_000;
/// The id under which the Composio project key lives in the secret store.
pub const SECRET_ID: &str = "composio";
/// Environment fallback, for a machine that provisions the key outside the UI.
pub const ENV_KEY: &str = "COMPOSIO_API_KEY";

/// Resolve the project API key: the secret store first, then the environment.
///
/// Returns the plaintext, so call it as late as possible and never put the
/// result anywhere but a request header.
pub fn resolve_key(secrets: &crate::secrets::SecretStore) -> Result<String, String> {
    if let Some(key) = secrets.get(SECRET_ID).filter(|k| !k.trim().is_empty()) {
        return Ok(key.trim().to_string());
    }
    if let Ok(key) = std::env::var(ENV_KEY) {
        if !key.trim().is_empty() {
            return Ok(key.trim().to_string());
        }
    }
    Err(
        "No Composio API key is configured. Open the Apps panel in the sidebar and add one, \
         or set COMPOSIO_API_KEY in the environment. Connected apps are unavailable until then."
            .to_string(),
    )
}

pub fn is_configured(secrets: &crate::secrets::SecretStore) -> bool {
    resolve_key(secrets).is_ok()
}

// -------------------------------------------------------------------- models

/// One application Composio can broker a connection to.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Toolkit {
    pub slug: String,
    pub name: String,
    /// Composio-hosted logo URL. Used instead of shipping an app catalogue.
    pub logo: Option<String>,
    pub categories: Vec<String>,
    pub tools_count: u64,
    /// True when the app needs no credential at all.
    pub no_auth: bool,
    /// Auth schemes Composio can broker without the user registering an OAuth
    /// client of their own. If this is empty, connecting needs custom setup.
    pub composio_managed_auth_schemes: Vec<String>,
    /// Whether SirVibe can start a sign-in for this app as things stand.
    /// Computed when the toolkit is read, so the interface does not have to
    /// re-derive Composio's auth rules.
    pub connectable: bool,
}

impl Toolkit {
    fn from_json(v: &Value) -> Option<Self> {
        let slug = v.get("slug").and_then(Value::as_str)?.to_string();
        Some(Self {
            name: v
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or(&slug)
                .to_string(),
            logo: v
                .pointer("/meta/logo")
                .and_then(Value::as_str)
                .map(str::to_string),
            categories: v
                .pointer("/meta/categories")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            c.as_str()
                                .map(str::to_string)
                                .or_else(|| c.get("name").and_then(Value::as_str).map(str::to_string))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            tools_count: v
                .pointer("/meta/tools_count")
                .and_then(Value::as_u64)
                .unwrap_or(0),
            no_auth: v.get("no_auth").and_then(Value::as_bool).unwrap_or(false),
            composio_managed_auth_schemes: v
                .get("composio_managed_auth_schemes")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|s| s.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            slug,
            connectable: false,
        }
        .settled())
    }

    /// Fill in the derived fields once the parsed ones are in place.
    fn settled(mut self) -> Self {
        self.connectable = self.no_auth || !self.composio_managed_auth_schemes.is_empty();
        self
    }

}

/// The result of asking Composio to start an authentication flow.
#[derive(Serialize, Clone, Debug)]
pub struct LinkSession {
    pub connected_account_id: String,
    /// Where the user must go to sign in. Opened in their real browser.
    pub redirect_url: String,
    pub expires_at: Option<String>,
}

/// A connection as Composio currently sees it.
#[derive(Serialize, Clone, Debug)]
pub struct ConnectionStatus {
    pub id: String,
    pub toolkit_slug: String,
    pub status: String,
    pub status_reason: Option<String>,
    pub is_disabled: bool,
}

impl ConnectionStatus {
    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            id: v.get("id").and_then(Value::as_str)?.to_string(),
            toolkit_slug: v
                .pointer("/toolkit/slug")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            status: v
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("UNKNOWN")
                .to_string(),
            status_reason: v
                .get("status_reason")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            is_disabled: v.get("is_disabled").and_then(Value::as_bool).unwrap_or(false),
        })
    }

    pub fn usable(&self) -> bool {
        self.status == "ACTIVE" && !self.is_disabled
    }

    /// A sentence a person can act on, for a connection that is not usable.
    pub fn explain(&self, app: &str) -> String {
        let detail = self
            .status_reason
            .as_deref()
            .map(|r| format!(" ({})", r))
            .unwrap_or_default();
        match self.status.as_str() {
            "ACTIVE" if self.is_disabled => {
                format!("The {} connection is disabled in Composio.{}", app, detail)
            }
            "INITIALIZING" | "INITIATED" => format!(
                "The {} connection has not finished signing in yet. Finish the sign-in in the browser window, then try again.{}",
                app, detail
            ),
            "EXPIRED" => format!(
                "The {} connection has expired. Reconnect it in the Apps panel.{}",
                app, detail
            ),
            "REVOKED" => format!(
                "Access to {} was revoked. Reconnect it in the Apps panel.{}",
                app, detail
            ),
            "FAILED" => format!(
                "Connecting to {} failed. Reconnect it in the Apps panel.{}",
                app, detail
            ),
            "INACTIVE" => format!(
                "The {} connection is inactive and cannot run tools. Reconnect it in the Apps panel.{}",
                app, detail
            ),
            other => format!("The {} connection is {}.{}", app, other, detail),
        }
    }
}

/// One action a connected app exposes, as discovered at runtime.
#[derive(Serialize, Clone, Debug)]
pub struct AppTool {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub toolkit_slug: String,
    /// JSON Schema for the arguments. Only ever fetched for tools that
    /// actually matched a search, never for the whole catalogue.
    pub input_parameters: Value,
}

impl AppTool {
    fn from_json(v: &Value) -> Option<Self> {
        Some(Self {
            slug: v.get("slug").and_then(Value::as_str)?.to_string(),
            name: v.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
            description: v
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .chars()
                .take(400)
                .collect(),
            toolkit_slug: v
                .pointer("/toolkit/slug")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            input_parameters: v.get("input_parameters").cloned().unwrap_or(Value::Null),
        })
    }
}

// -------------------------------------------------------------------- client

pub struct Composio {
    key: String,
    client: reqwest::Client,
}

impl Composio {
    pub fn new(key: String) -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent("SirVibe/0.1 (+composio)")
            .build()
            .map_err(|e| format!("could not start a network client: {}", e))?;
        Ok(Self { key, client })
    }

    pub fn from_secrets(secrets: &crate::secrets::SecretStore) -> Result<Self, String> {
        Self::new(resolve_key(secrets)?)
    }

    /// One request. The key goes into a header here and nowhere else; the
    /// response is capped and redacted before anything else sees it.
    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        query: &[(&str, String)],
        body: Option<Value>,
    ) -> Result<Value, String> {
        let url = format!("{}{}", BASE_URL, path);
        let mut builder = self
            .client
            .request(method, &url)
            .header("x-api-key", &self.key)
            .header("accept", "application/json");
        if !query.is_empty() {
            builder = builder.query(query);
        }
        if let Some(payload) = &body {
            builder = builder.json(payload);
        }

        let response = builder.send().await.map_err(|e| transport_error(&e))?;
        let status = response.status();

        let bytes = response
            .bytes()
            .await
            .map_err(|e| format!("the Composio response was interrupted: {}", e))?;
        let capped = &bytes[..bytes.len().min(MAX_RESPONSE_BYTES)];
        let text = redact(&String::from_utf8_lossy(capped), Some(&self.key));

        if !status.is_success() {
            return Err(explain_status(status.as_u16(), &text));
        }
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }
        serde_json::from_str(&text)
            .map_err(|_| "Composio returned a response that could not be read as JSON.".to_string())
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> Result<Value, String> {
        self.request(reqwest::Method::GET, path, query, None).await
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, String> {
        self.request(reqwest::Method::POST, path, &[], Some(body)).await
    }

    /// Confirm the key works, without changing anything.
    pub async fn verify(&self) -> Result<(), String> {
        self.get("/toolkits", &[("limit", "1".into())]).await.map(|_| ())
    }

    // ------------------------------------------------------------- toolkits

    /// Browse the apps Composio supports. `search` is passed to Composio
    /// rather than filtered here, so the catalogue never has to be downloaded.
    pub async fn list_toolkits(&self, search: Option<&str>, limit: u32) -> Result<Vec<Toolkit>, String> {
        let mut query: Vec<(&str, String)> = vec![
            ("limit", limit.clamp(1, 100).to_string()),
            ("sort_by", "usage".into()),
        ];
        if let Some(term) = search.map(str::trim).filter(|t| !t.is_empty()) {
            query.push(("search", term.to_string()));
        }
        let body = self.get("/toolkits", &query).await?;
        Ok(body
            .get("items")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(Toolkit::from_json).collect())
            .unwrap_or_default())
    }

    pub async fn get_toolkit(&self, slug: &str) -> Result<Toolkit, String> {
        let body = self.get(&format!("/toolkits/{}", slug.to_lowercase()), &[]).await?;
        Toolkit::from_json(&body)
            .ok_or_else(|| format!("Composio does not have an app called '{}'.", slug))
    }

    // --------------------------------------------------------- auth configs

    /// The auth config is the per-project registration of an app. Reuse the
    /// existing one when there is one, so repeated connects do not pile up
    /// duplicate configs in the user's Composio project.
    pub async fn ensure_auth_config(&self, toolkit_slug: &str) -> Result<String, String> {
        let slug = toolkit_slug.to_lowercase();
        let existing = self
            .get(
                "/auth_configs",
                &[("toolkit_slug", slug.clone()), ("limit", "10".into())],
            )
            .await?;
        if let Some(found) = existing
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|c| c.get("status").and_then(Value::as_str) != Some("DISABLED"))
                    .or_else(|| items.first())
            })
            .and_then(|c| c.get("id").and_then(Value::as_str))
        {
            return Ok(found.to_string());
        }

        // None yet: create one using Composio's own OAuth application, which is
        // what lets the user connect without registering anything themselves.
        let created = self
            .post(
                "/auth_configs",
                json!({
                    "toolkit": { "slug": slug },
                    "auth_config": { "type": "use_composio_managed_auth" }
                }),
            )
            .await
            .map_err(|e| {
                format!(
                    "{} could not be set up for connecting. {}",
                    toolkit_slug, e
                )
            })?;

        created
            .pointer("/auth_config/id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| {
                format!(
                    "Composio did not return a usable setup for {}. It may need custom OAuth credentials.",
                    toolkit_slug
                )
            })
    }

    // ----------------------------------------------------- connected accounts

    /// Start an OAuth flow. Composio hosts the callback, so SirVibe needs no
    /// local web server and no redirect URI registration.
    pub async fn create_link(
        &self,
        auth_config_id: &str,
        user_id: &str,
    ) -> Result<LinkSession, String> {
        let body = self
            .post(
                "/connected_accounts/link",
                json!({ "auth_config_id": auth_config_id, "user_id": user_id }),
            )
            .await?;

        let redirect_url = body
            .get("redirect_url")
            .and_then(Value::as_str)
            .filter(|u| u.starts_with("https://"))
            .ok_or("Composio did not return a sign-in link for this app.")?
            .to_string();

        Ok(LinkSession {
            connected_account_id: body
                .get("connected_account_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            redirect_url,
            expires_at: body
                .get("expires_at")
                .and_then(Value::as_str)
                .map(str::to_string),
        })
    }

    pub async fn connection(&self, connected_account_id: &str) -> Result<ConnectionStatus, String> {
        let body = self
            .get(&format!("/connected_accounts/{}", connected_account_id), &[])
            .await?;
        ConnectionStatus::from_json(&body)
            .ok_or_else(|| "Composio returned a connection that could not be read.".to_string())
    }

    /// Every connection belonging to one SirVibe user. The `user_ids` filter is
    /// what keeps one person's Gmail separate from another's.
    pub async fn connections_for(&self, user_id: &str) -> Result<Vec<ConnectionStatus>, String> {
        let body = self
            .get(
                "/connected_accounts",
                &[("user_ids", user_id.to_string()), ("limit", "100".into())],
            )
            .await?;
        Ok(body
            .get("items")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(ConnectionStatus::from_json).collect())
            .unwrap_or_default())
    }

    /// Disconnect, revoking the token at the provider where that is supported.
    pub async fn disconnect(&self, connected_account_id: &str) -> Result<(), String> {
        self.request(
            reqwest::Method::DELETE,
            &format!("/connected_accounts/{}", connected_account_id),
            &[("revoke_on_delete", "true".into())],
            None,
        )
        .await
        .map(|_| ())
    }

    // ----------------------------------------------------------------- tools

    /// Runtime tool discovery. Only tools matching the query come back, which
    /// is what keeps thousands of schemas out of the model's context.
    pub async fn search_tools(
        &self,
        toolkit_slugs: &[String],
        query: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AppTool>, String> {
        let mut params: Vec<(&str, String)> = vec![("limit", limit.clamp(1, 25).to_string())];
        if !toolkit_slugs.is_empty() {
            params.push(("toolkit_slug", toolkit_slugs.join(",")));
        }
        if let Some(term) = query.map(str::trim).filter(|t| !t.is_empty()) {
            params.push(("search", term.to_string()));
        }
        let body = self.get("/tools", &params).await?;
        Ok(body
            .get("items")
            .and_then(Value::as_array)
            .map(|items| items.iter().filter_map(AppTool::from_json).collect())
            .unwrap_or_default())
    }

    pub async fn get_tool(&self, slug: &str) -> Result<AppTool, String> {
        let body = self.get(&format!("/tools/{}", slug.to_uppercase()), &[]).await?;
        AppTool::from_json(&body)
            .ok_or_else(|| format!("'{}' is not a tool Composio knows about.", slug))
    }

    /// Run one action against a connected app. Composio injects the app's own
    /// credential server-side; it never passes through SirVibe.
    pub async fn execute_tool(
        &self,
        tool_slug: &str,
        user_id: &str,
        connected_account_id: &str,
        arguments: Value,
    ) -> Result<Value, String> {
        let mut payload = json!({
            "user_id": user_id,
            "arguments": if arguments.is_null() { json!({}) } else { arguments },
        });
        if !connected_account_id.is_empty() {
            payload["connected_account_id"] = json!(connected_account_id);
        }

        let body = self
            .post(&format!("/tools/execute/{}", tool_slug.to_uppercase()), payload)
            .await?;

        // Composio reports a failed action with HTTP 200 and successful:false.
        // Surface that as an error rather than handing the model a silent no-op.
        let successful = body.get("successful").and_then(Value::as_bool).unwrap_or(false);
        if !successful {
            let reason = body
                .get("error")
                .and_then(Value::as_str)
                .filter(|e| !e.is_empty())
                .unwrap_or("the app rejected the request and gave no reason");
            return Err(format!("{} failed: {}", tool_slug, reason));
        }
        Ok(body.get("data").cloned().unwrap_or(Value::Null))
    }
}

// ------------------------------------------------------------------- errors

/// Composio's HTTP failures, said in a way a person can act on. The response
/// body is already redacted by the caller; only a short excerpt is kept.
fn explain_status(status: u16, body: &str) -> String {
    let detail: String = body
        .chars()
        .filter(|c| *c != '\n')
        .take(300)
        .collect::<String>()
        .trim()
        .to_string();
    match status {
        400 | 422 => format!("Composio rejected the request ({}). {}", status, detail),
        401 => "The Composio API key was rejected. Check it in the Apps panel in the sidebar."
            .to_string(),
        403 => "This Composio project is not allowed to do that. Check the key's permissions in the Composio dashboard.".to_string(),
        404 => format!("Composio could not find that. {}", detail),
        409 => format!("Composio reports a conflict — it may already exist. {}", detail),
        429 => "Composio is rate limiting these requests. Wait a moment before trying again."
            .to_string(),
        500..=599 => format!(
            "Composio had a server error ({}). This is on their side; try again shortly.",
            status
        ),
        other => format!("Composio returned {}. {}", other, detail),
    }
}

fn transport_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "Composio did not respond in time. Check the network connection and try again.".to_string()
    } else if error.is_connect() {
        "Could not reach Composio. Check the network connection.".to_string()
    } else {
        // The error's Display can contain the URL but never the key, which is
        // only ever sent as a header.
        format!("The request to Composio failed: {}", error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_toolkit_is_read_from_composios_shape() {
        let raw = json!({
            "slug": "github",
            "name": "GitHub",
            "no_auth": false,
            "composio_managed_auth_schemes": ["OAUTH2"],
            "meta": {
                "logo": "https://logos.composio.dev/api/github",
                "tools_count": 128,
                "categories": [{ "name": "developer-tools" }]
            }
        });
        let kit = Toolkit::from_json(&raw).unwrap();
        assert_eq!(kit.slug, "github");
        assert_eq!(kit.name, "GitHub");
        assert_eq!(kit.tools_count, 128);
        assert_eq!(kit.logo.as_deref(), Some("https://logos.composio.dev/api/github"));
        assert_eq!(kit.categories, vec!["developer-tools"]);
        assert!(kit.connectable);
    }

    #[test]
    fn an_app_needing_custom_oauth_is_not_connectable() {
        let raw = json!({ "slug": "obscure", "name": "Obscure", "no_auth": false });
        let kit = Toolkit::from_json(&raw).unwrap();
        assert!(!kit.connectable, "nothing managed and no_auth false");

        let open = json!({ "slug": "weather", "name": "Weather", "no_auth": true });
        assert!(Toolkit::from_json(&open).unwrap().connectable);
    }

    #[test]
    fn only_an_active_connection_is_usable() {
        let make = |status: &str, disabled: bool| ConnectionStatus {
            id: "ca_1".into(),
            toolkit_slug: "gmail".into(),
            status: status.into(),
            status_reason: None,
            is_disabled: disabled,
        };
        assert!(make("ACTIVE", false).usable());
        assert!(!make("ACTIVE", true).usable());
        for status in ["INITIATED", "EXPIRED", "FAILED", "INACTIVE", "REVOKED"] {
            assert!(!make(status, false).usable(), "{} must not be usable", status);
        }
    }

    #[test]
    fn an_unusable_connection_explains_what_to_do() {
        let expired = ConnectionStatus {
            id: "ca_1".into(),
            toolkit_slug: "gmail".into(),
            status: "EXPIRED".into(),
            status_reason: None,
            is_disabled: false,
        };
        let message = expired.explain("Gmail");
        assert!(message.contains("expired"), "{}", message);
        assert!(message.contains("Reconnect"), "{}", message);
    }

    #[test]
    fn a_connection_is_read_from_composios_shape() {
        let raw = json!({
            "id": "ca_abc123",
            "status": "ACTIVE",
            "is_disabled": false,
            "user_id": "sirvibe-local",
            "toolkit": { "slug": "gmail" }
        });
        let c = ConnectionStatus::from_json(&raw).unwrap();
        assert_eq!(c.id, "ca_abc123");
        assert_eq!(c.toolkit_slug, "gmail");
        assert!(c.usable());
    }

    #[test]
    fn a_tool_keeps_its_schema_but_bounds_its_prose() {
        let raw = json!({
            "slug": "GMAIL_SEND_EMAIL",
            "name": "Send email",
            "description": "x".repeat(900),
            "toolkit": { "slug": "gmail" },
            "input_parameters": { "type": "object", "properties": { "to": { "type": "string" } } }
        });
        let tool = AppTool::from_json(&raw).unwrap();
        assert_eq!(tool.slug, "GMAIL_SEND_EMAIL");
        assert_eq!(tool.toolkit_slug, "gmail");
        assert_eq!(tool.description.chars().count(), 400, "description must be capped");
        assert!(tool.input_parameters["properties"]["to"].is_object());
    }

    #[test]
    fn http_failures_are_explained_without_echoing_everything() {
        assert!(explain_status(401, "whatever").contains("Apps panel"));
        assert!(explain_status(429, "").contains("rate limiting"));
        assert!(explain_status(503, "").contains("server error"));
        let long = explain_status(400, &"y".repeat(5000));
        assert!(long.chars().count() < 400, "the excerpt must stay short");
    }

    #[test]
    fn a_missing_key_is_reported_not_guessed() {
        let dir = std::env::temp_dir().join("sirvibe-composio-key");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let secrets = crate::secrets::SecretStore::new(&dir);

        // With nothing stored and no environment variable, this must fail
        // clearly rather than producing an empty key.
        if std::env::var(ENV_KEY).is_err() {
            let error = resolve_key(&secrets).unwrap_err();
            assert!(error.contains("COMPOSIO_API_KEY"), "{}", error);
            assert!(!is_configured(&secrets));
        }

        secrets.put(SECRET_ID, "comp_live_SECRET123").unwrap();
        assert_eq!(resolve_key(&secrets).unwrap(), "comp_live_SECRET123");
        assert!(is_configured(&secrets));
    }
}

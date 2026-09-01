//! Connected apps: the local record of what the user has connected through
//! Composio, and which SirVibe user each connection belongs to.
//!
//! This registry deliberately holds no credential of any kind. An app's OAuth
//! tokens live with Composio; the Composio project key lives in `secrets.rs`.
//! What is kept here is only what is needed to address a connection again:
//! which app, which Composio connected-account id, and whose it is.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// What a connected app can actually do, kept so the agent can be told about
/// its own capabilities without asking Composio again every turn.
///
/// This exists because searching is not discovery. Composio's tool search is a
/// keyword match: a real query like "followers posts media insights" returns
/// nothing for an app that has sixteen actions, two of which are exactly what
/// was wanted. An inventory is small, cheap and complete, so it is the thing
/// the agent reasons from; search is only a way of narrowing a large one.
#[derive(Default)]
pub struct ToolInventory {
    by_app: Mutex<HashMap<String, Vec<crate::composio::AppTool>>>,
}

impl ToolInventory {
    pub fn put(&self, app: &str, tools: Vec<crate::composio::AppTool>) {
        if let Ok(mut by_app) = self.by_app.lock() {
            by_app.insert(app.to_lowercase(), tools);
        }
    }

    pub fn get(&self, app: &str) -> Option<Vec<crate::composio::AppTool>> {
        self.by_app
            .lock()
            .ok()
            .and_then(|by_app| by_app.get(&app.to_lowercase()).cloned())
    }

    /// One line per app for the system prompt: how many actions, and enough of
    /// their names to reason with. The full schemas stay out of the prompt.
    pub fn summary(&self, app: &str) -> Option<String> {
        let tools = self.get(app)?;
        if tools.is_empty() {
            return None;
        }
        let names: Vec<String> = tools
            .iter()
            .take(12)
            .map(|t| short_name(&t.slug, app))
            .collect();
        Some(format!(
            "{} action{}: {}{}",
            tools.len(),
            if tools.len() == 1 { "" } else { "s" },
            names.join(", "),
            if tools.len() > names.len() { ", …" } else { "" }
        ))
    }

    pub fn forget(&self, app: &str) {
        if let Ok(mut by_app) = self.by_app.lock() {
            by_app.remove(&app.to_lowercase());
        }
    }
}

/// `INSTAGRAM_GET_USER_INSIGHTS` → `get user insights`. The slug is what a call
/// needs; the words are what a decision needs.
fn short_name(slug: &str, app: &str) -> String {
    let prefix = format!("{}_", app.to_uppercase());
    slug.strip_prefix(&prefix)
        .unwrap_or(slug)
        .to_lowercase()
        .replace('_', " ")
}

/// One app this user has connected, or is part-way through connecting.
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ConnectedApp {
    /// Composio's toolkit slug, e.g. "gmail". Unique per user.
    pub toolkit_slug: String,
    pub name: String,
    pub logo: Option<String>,
    /// Composio's id for this connection. The handle every later call uses.
    pub connected_account_id: String,
    /// The project-level registration of the app the connection was made under.
    pub auth_config_id: String,
    /// Which SirVibe user owns this connection. Scoping lives on this field.
    pub user_id: String,
    /// Last status seen from Composio. A cache for display; the authoritative
    /// answer is always re-read before a tool runs.
    pub status: String,
    pub status_reason: Option<String>,
    pub connected_ms: u64,
    pub updated_ms: u64,
}

impl ConnectedApp {
    pub fn view(&self) -> AppView {
        AppView {
            toolkit_slug: self.toolkit_slug.clone(),
            name: self.name.clone(),
            logo: self.logo.clone(),
            status: self.status.clone(),
            status_reason: self.status_reason.clone(),
            connected: self.status == "ACTIVE",
            pending: matches!(self.status.as_str(), "INITIALIZING" | "INITIATED"),
            connected_ms: self.connected_ms,
            updated_ms: self.updated_ms,
        }
    }
}

/// What the interface may see. No connected-account id, no auth-config id, no
/// user id — nothing that could be used to address someone else's connection
/// from outside the backend.
#[derive(Serialize, Clone, Debug)]
pub struct AppView {
    pub toolkit_slug: String,
    pub name: String,
    pub logo: Option<String>,
    pub status: String,
    pub status_reason: Option<String>,
    pub connected: bool,
    /// Sign-in was started but has not completed yet.
    pub pending: bool,
    pub connected_ms: u64,
    pub updated_ms: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct RegistryFile {
    #[serde(default)]
    apps: Vec<ConnectedApp>,
}

pub struct AppRegistry {
    path: PathBuf,
}

impl AppRegistry {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("connected-apps.json"),
        }
    }

    fn all(&self) -> Vec<ConnectedApp> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str::<RegistryFile>(&raw).ok())
            .map(|f| f.apps)
            .unwrap_or_default()
    }

    /// Every app belonging to one user. This is the only listing the rest of
    /// the application uses, so a connection can never be read across users by
    /// forgetting a filter.
    pub fn for_user(&self, user_id: &str) -> Vec<ConnectedApp> {
        self.all()
            .into_iter()
            .filter(|a| a.user_id == user_id)
            .collect()
    }

    pub fn get(&self, user_id: &str, toolkit_slug: &str) -> Option<ConnectedApp> {
        let slug = toolkit_slug.to_lowercase();
        self.all()
            .into_iter()
            .find(|a| a.user_id == user_id && a.toolkit_slug == slug)
    }

    fn write(&self, apps: Vec<ConnectedApp>) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw =
            serde_json::to_string_pretty(&RegistryFile { apps }).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, raw).map_err(|e| e.to_string())
    }

    /// Replace this user's record for an app, or add it. Keyed on the pair, so
    /// two users connecting the same app keep two separate rows.
    pub fn upsert(&self, app: ConnectedApp) -> Result<(), String> {
        let mut apps = self.all();
        match apps
            .iter_mut()
            .find(|a| a.user_id == app.user_id && a.toolkit_slug == app.toolkit_slug)
        {
            Some(existing) => *existing = app,
            None => apps.push(app),
        }
        self.write(apps)
    }

    pub fn remove(&self, user_id: &str, toolkit_slug: &str) -> Result<(), String> {
        let slug = toolkit_slug.to_lowercase();
        let apps = self
            .all()
            .into_iter()
            .filter(|a| !(a.user_id == user_id && a.toolkit_slug == slug))
            .collect();
        self.write(apps)
    }

    /// Record what Composio last reported, without disturbing anything else.
    pub fn set_status(
        &self,
        user_id: &str,
        toolkit_slug: &str,
        status: &str,
        reason: Option<String>,
    ) -> Result<(), String> {
        if let Some(mut app) = self.get(user_id, toolkit_slug) {
            app.status = status.to_string();
            app.status_reason = reason;
            app.updated_ms = crate::apis::now_ms();
            return self.upsert(app);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry(name: &str) -> AppRegistry {
        let dir = std::env::temp_dir().join(format!("sirvibe-apps-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        AppRegistry::new(&dir)
    }

    fn app(user: &str, slug: &str) -> ConnectedApp {
        ConnectedApp {
            toolkit_slug: slug.into(),
            name: slug.to_uppercase(),
            connected_account_id: format!("ca_{}_{}", user, slug),
            auth_config_id: format!("ac_{}", slug),
            user_id: user.into(),
            status: "ACTIVE".into(),
            connected_ms: 1,
            updated_ms: 1,
            ..Default::default()
        }
    }

    #[test]
    fn two_users_keep_separate_connections_to_the_same_app() {
        let reg = registry("scoping");
        reg.upsert(app("user-a", "gmail")).unwrap();
        reg.upsert(app("user-b", "gmail")).unwrap();

        let a = reg.for_user("user-a");
        let b = reg.for_user("user-b");
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 1);
        assert_ne!(
            a[0].connected_account_id, b[0].connected_account_id,
            "the two users must not share one connected account"
        );
        assert_eq!(reg.get("user-a", "gmail").unwrap().connected_account_id, "ca_user-a_gmail");
        assert_eq!(reg.get("user-b", "gmail").unwrap().connected_account_id, "ca_user-b_gmail");
    }

    #[test]
    fn a_user_only_ever_sees_their_own_apps() {
        let reg = registry("isolation");
        reg.upsert(app("user-a", "gmail")).unwrap();
        reg.upsert(app("user-a", "github")).unwrap();
        reg.upsert(app("user-b", "slack")).unwrap();

        let a: Vec<String> = reg.for_user("user-a").iter().map(|x| x.toolkit_slug.clone()).collect();
        assert_eq!(a.len(), 2);
        assert!(!a.contains(&"slack".to_string()));
        assert!(reg.get("user-a", "slack").is_none(), "cross-user read must not resolve");
    }

    #[test]
    fn reconnecting_replaces_rather_than_duplicates() {
        let reg = registry("upsert");
        reg.upsert(app("u", "gmail")).unwrap();
        let mut again = app("u", "gmail");
        again.connected_account_id = "ca_new".into();
        reg.upsert(again).unwrap();

        assert_eq!(reg.for_user("u").len(), 1);
        assert_eq!(reg.get("u", "gmail").unwrap().connected_account_id, "ca_new");
    }

    #[test]
    fn removing_one_users_app_leaves_the_others_alone() {
        let reg = registry("remove");
        reg.upsert(app("user-a", "gmail")).unwrap();
        reg.upsert(app("user-b", "gmail")).unwrap();

        reg.remove("user-a", "gmail").unwrap();
        assert!(reg.get("user-a", "gmail").is_none());
        assert!(reg.get("user-b", "gmail").is_some(), "the other user's connection survived");
    }

    #[test]
    fn a_status_update_touches_nothing_else() {
        let reg = registry("status");
        reg.upsert(app("u", "gmail")).unwrap();
        reg.set_status("u", "gmail", "EXPIRED", Some("token expired".into()))
            .unwrap();

        let stored = reg.get("u", "gmail").unwrap();
        assert_eq!(stored.status, "EXPIRED");
        assert_eq!(stored.status_reason.as_deref(), Some("token expired"));
        assert_eq!(stored.connected_account_id, "ca_u_gmail", "the handle is unchanged");
        assert!(!stored.view().connected);
    }

    #[test]
    fn the_view_carries_no_handles() {
        let view = app("u", "gmail").view();
        let encoded = serde_json::to_string(&view).unwrap();
        assert!(!encoded.contains("ca_u_gmail"), "connected account id leaked: {}", encoded);
        assert!(!encoded.contains("ac_gmail"), "auth config id leaked: {}", encoded);
        assert!(!encoded.contains("user_id"), "user id leaked: {}", encoded);
        assert!(view.connected);
    }
}

#[cfg(test)]
mod inventory_tests {
    use super::*;
    use crate::composio::AppTool;
    use serde_json::json;

    fn tool(slug: &str) -> AppTool {
        AppTool {
            slug: slug.into(),
            name: slug.into(),
            description: "does a thing".into(),
            toolkit_slug: "instagram".into(),
            input_parameters: json!({}),
        }
    }

    #[test]
    fn an_apps_actions_are_summarised_in_words_a_decision_can_use() {
        let inventory = ToolInventory::default();
        assert!(inventory.summary("instagram").is_none(), "nothing is known yet");

        inventory.put(
            "instagram",
            vec![
                tool("INSTAGRAM_GET_USER_INSIGHTS"),
                tool("INSTAGRAM_CREATE_POST"),
                tool("INSTAGRAM_GET_POST_COMMENTS"),
            ],
        );
        let summary = inventory.summary("instagram").expect("a summary");
        assert!(summary.starts_with("3 actions:"), "{}", summary);
        // The slug is what a call needs; the words are what a decision needs.
        assert!(summary.contains("get user insights"), "{}", summary);
        assert!(summary.contains("create post"), "{}", summary);
        assert!(!summary.contains("INSTAGRAM_"), "the prompt gets words, not slugs: {}", summary);
    }

    #[test]
    fn a_long_inventory_is_trimmed_rather_than_flooding_the_prompt() {
        let inventory = ToolInventory::default();
        let many: Vec<AppTool> = (0..40).map(|i| tool(&format!("GMAIL_ACTION_{}", i))).collect();
        inventory.put("gmail", many);
        let summary = inventory.summary("gmail").unwrap();
        assert!(summary.starts_with("40 actions:"), "{}", summary);
        assert!(summary.ends_with(", …"), "the rest is fetched when needed: {}", summary);
        assert!(summary.len() < 600, "the prompt line stays short: {}", summary.len());
    }

    #[test]
    fn a_disconnected_app_is_forgotten() {
        let inventory = ToolInventory::default();
        inventory.put("instagram", vec![tool("INSTAGRAM_CREATE_POST")]);
        assert!(inventory.get("instagram").is_some());
        inventory.forget("instagram");
        assert!(inventory.get("instagram").is_none());
        assert!(inventory.summary("instagram").is_none());
    }

    #[test]
    fn lookups_do_not_care_about_case() {
        let inventory = ToolInventory::default();
        inventory.put("Instagram", vec![tool("INSTAGRAM_CREATE_POST")]);
        assert!(inventory.get("instagram").is_some());
        assert!(inventory.get("INSTAGRAM").is_some());
    }
}

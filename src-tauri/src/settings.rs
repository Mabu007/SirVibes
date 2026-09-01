use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    /// Every tool execution requires user approval.
    Ask,
    /// Normal production work runs; risky actions ask.
    #[default]
    Smart,
    /// Anything inside the workspace runs unattended.
    Full,
}

fn default_timeout() -> u64 {
    900
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Settings {
    pub api_key: String,
    /// Deepgram's key. Speech is needed by almost every piece of video work, so
    /// it sits here beside the model key rather than being wired up as a
    /// generic API connection.
    pub deepgram_api_key: String,
    pub model: String,
    /// The model every `see` call runs on. Vision is a fixed capability rather
    /// than a per-call choice, so it does not follow the agent's own model —
    /// empty means the built-in default in `vision::DEFAULT_MODEL`.
    pub vision_model: String,
    /// The model that watches a reference video where it lives. Not the same
    /// job as `vision_model`: only some providers can open a YouTube link at
    /// all. Empty means the built-in default in `reference::DEFAULT_MODEL`.
    pub reference_model: String,
    pub permission_mode: PermissionMode,
    pub workspace: Option<String>,
    /// Extra directories scanned for skills, in addition to the bundled and
    /// user skill directories.
    pub skill_dirs: Vec<String>,
    /// Workspaces the user has opened before, most recent first. These are the
    /// "projects" the sidebar switches between.
    pub recent_workspaces: Vec<String>,
    #[serde(default = "default_timeout")]
    pub shell_timeout_secs: u64,
    /// Stable local identity for this install, used as Composio's `user_id` so
    /// connected apps belong to this user rather than to the project as a
    /// whole. Generated once, on first use, and never shown to the model.
    pub composio_user_id: String,
}

/// What the frontend is allowed to see. The API key never crosses the IPC
/// boundary; only whether one is configured.
#[derive(Serialize, Clone)]
pub struct SettingsView {
    pub api_key_set: bool,
    pub api_key_hint: String,
    pub deepgram_key_set: bool,
    pub deepgram_key_hint: String,
    pub model: String,
    /// What `see` actually runs on, default included, so the UI can show it
    /// without knowing the default.
    pub vision_model: String,
    /// What watches reference videos, default included.
    pub reference_model: String,
    pub permission_mode: PermissionMode,
    pub workspace: Option<String>,
    pub skill_dirs: Vec<String>,
    pub recent_workspaces: Vec<String>,
    pub shell_timeout_secs: u64,
}

impl Settings {
    pub fn view(&self) -> SettingsView {
        let key = self.api_key.trim();
        let deepgram = self.deepgram_api_key.trim();
        SettingsView {
            api_key_set: !key.is_empty(),
            api_key_hint: hint(key),
            deepgram_key_set: !deepgram.is_empty(),
            deepgram_key_hint: hint(deepgram),
            model: self.model.clone(),
            vision_model: crate::vision::model_for(&self.vision_model).to_string(),
            reference_model: if self.reference_model.trim().is_empty() {
                crate::reference::DEFAULT_MODEL.to_string()
            } else {
                self.reference_model.trim().to_string()
            },
            permission_mode: self.permission_mode,
            workspace: self.workspace.clone(),
            skill_dirs: self.skill_dirs.clone(),
            recent_workspaces: self.recent_workspaces.clone(),
            shell_timeout_secs: if self.shell_timeout_secs == 0 {
                default_timeout()
            } else {
                self.shell_timeout_secs
            },
        }
    }

    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Settings {
                shell_timeout_secs: default_timeout(),
                ..Default::default()
            },
        }
    }

    /// The identity connected apps are scoped to. Created on first use; the
    /// caller persists the settings when this returns true in `created`.
    pub fn ensure_composio_user_id(&mut self) -> (String, bool) {
        if !self.composio_user_id.trim().is_empty() {
            return (self.composio_user_id.clone(), false);
        }
        self.composio_user_id = generate_user_id();
        (self.composio_user_id.clone(), true)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let raw = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, raw).map_err(|e| e.to_string())?;
        restrict_permissions(path);
        Ok(())
    }
}

/// A local identifier with no personal information in it. This is not a
/// security boundary — it is a stable name for "the person using this install",
/// which is what Composio needs in order to keep connections apart.
fn generate_user_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    // A counter as well as the clock, so two ids minted in the same nanosecond
    // still differ.
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let seq = SEQUENCE.fetch_add(1, Ordering::Relaxed) as u128;
    let mixed = nanos
        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
        .wrapping_add(pid << 17)
        .wrapping_add(seq.wrapping_mul(0xD1B5_4A32_D192_ED03));
    format!("sirvibe-{:032x}", mixed)
}

/// Enough of a key to recognise it, never enough to use it.
fn hint(key: &str) -> String {
    if key.chars().count() > 8 {
        format!("…{}", key.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect::<String>())
    } else {
        String::new()
    }
}

/// The settings file holds the API keys, so keep it owner-readable only.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Fields the frontend may write. Absent fields are left unchanged, so the UI
/// can patch a single setting without round-tripping the API key.
#[derive(Deserialize, Default)]
#[serde(default)]
pub struct SettingsPatch {
    pub api_key: Option<String>,
    pub deepgram_api_key: Option<String>,
    pub model: Option<String>,
    pub vision_model: Option<String>,
    pub reference_model: Option<String>,
    pub permission_mode: Option<PermissionMode>,
    pub workspace: Option<String>,
    pub skill_dirs: Option<Vec<String>>,
    pub shell_timeout_secs: Option<u64>,
}

impl Settings {
    pub fn apply(&mut self, patch: SettingsPatch) {
        if let Some(k) = patch.api_key {
            self.api_key = k.trim().to_string();
        }
        if let Some(k) = patch.deepgram_api_key {
            self.deepgram_api_key = k.trim().to_string();
        }
        if let Some(m) = patch.model {
            self.model = m.trim().to_string();
        }
        if let Some(m) = patch.vision_model {
            self.vision_model = m.trim().to_string();
        }
        if let Some(m) = patch.reference_model {
            self.reference_model = m.trim().to_string();
        }
        if let Some(p) = patch.permission_mode {
            self.permission_mode = p;
        }
        if let Some(w) = patch.workspace {
            if w.trim().is_empty() {
                self.workspace = None;
            } else {
                self.recent_workspaces.retain(|r| r != &w);
                self.recent_workspaces.insert(0, w.clone());
                self.recent_workspaces.truncate(12);
                self.workspace = Some(w);
            }
        }
        if let Some(d) = patch.skill_dirs {
            self.skill_dirs = d;
        }
        if let Some(t) = patch.shell_timeout_secs {
            self.shell_timeout_secs = t.clamp(5, 7200);
        }
    }
}

pub fn settings_path(config_dir: &Path) -> PathBuf {
    config_dir.join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("eplug-settings-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        settings_path(&dir)
    }

    #[test]
    fn settings_survive_a_save_and_load() {
        let path = temp("roundtrip");
        let mut s = Settings::default();
        s.apply(SettingsPatch {
            api_key: Some("sk-or-v1-secret1234".into()),
            model: Some("anthropic/claude-sonnet-4.5".into()),
            permission_mode: Some(PermissionMode::Full),
            workspace: Some("/tmp/ws".into()),
            shell_timeout_secs: Some(600),
            ..Default::default()
        });
        s.save(&path).unwrap();

        let loaded = Settings::load(&path);
        assert_eq!(loaded.api_key, "sk-or-v1-secret1234");
        assert_eq!(loaded.model, "anthropic/claude-sonnet-4.5");
        assert_eq!(loaded.permission_mode, PermissionMode::Full);
        assert_eq!(loaded.workspace.as_deref(), Some("/tmp/ws"));
        assert_eq!(loaded.shell_timeout_secs, 600);
    }

    #[test]
    fn a_patch_only_changes_what_it_names() {
        let mut s = Settings::default();
        s.apply(SettingsPatch {
            api_key: Some("sk-or-v1-secret1234".into()),
            model: Some("a/b".into()),
            ..Default::default()
        });
        // Changing the mode alone must not clear the stored key.
        s.apply(SettingsPatch {
            permission_mode: Some(PermissionMode::Ask),
            ..Default::default()
        });
        assert_eq!(s.api_key, "sk-or-v1-secret1234");
        assert_eq!(s.model, "a/b");
        assert_eq!(s.permission_mode, PermissionMode::Ask);
    }

    #[test]
    fn the_deepgram_key_is_stored_and_masked_like_the_model_key() {
        let mut s = Settings::default();
        s.apply(SettingsPatch {
            deepgram_api_key: Some("  dg-secret-key-9876  ".into()),
            ..Default::default()
        });
        assert_eq!(s.deepgram_api_key, "dg-secret-key-9876");

        let view = s.view();
        assert!(view.deepgram_key_set);
        assert!(!view.api_key_set, "the two keys are independent");
        assert_eq!(view.deepgram_key_hint, "…9876");
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("secret-key"), "key leaked to the frontend: {}", json);
    }

    #[test]
    fn the_view_never_carries_the_api_key() {
        let mut s = Settings::default();
        s.apply(SettingsPatch {
            api_key: Some("sk-or-v1-secret1234".into()),
            ..Default::default()
        });
        let view = s.view();
        assert!(view.api_key_set);
        assert_eq!(view.api_key_hint, "…1234");
        let json = serde_json::to_string(&view).unwrap();
        assert!(!json.contains("secret1234"), "key leaked to the frontend: {}", json);
    }

    #[test]
    fn the_settings_file_is_not_world_readable() {
        let path = temp("perms");
        Settings::default().save(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "group/other can read the API key");
        }
    }

    #[test]
    fn a_corrupt_settings_file_falls_back_to_defaults() {
        let path = temp("corrupt");
        std::fs::write(&path, "{not json").unwrap();
        let loaded = Settings::load(&path);
        assert_eq!(loaded.permission_mode, PermissionMode::Smart);
    }

    #[test]
    fn the_local_identity_is_generated_once_and_then_kept() {
        let mut s = Settings::default();
        assert!(s.composio_user_id.is_empty());

        let (first, created) = s.ensure_composio_user_id();
        assert!(created, "the first call creates it");
        assert!(first.starts_with("sirvibe-"));

        let (second, created_again) = s.ensure_composio_user_id();
        assert!(!created_again, "it is only ever created once");
        assert_eq!(first, second, "the identity must be stable");

        // And it survives a round trip, or every restart would orphan the
        // user's connected apps.
        let path = temp("identity");
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).composio_user_id, first);
    }

    #[test]
    fn two_installs_do_not_collide_on_one_identity() {
        let mut a = Settings::default();
        let mut b = Settings::default();
        assert_ne!(a.ensure_composio_user_id().0, b.ensure_composio_user_id().0);
    }

    #[test]
    fn the_timeout_is_clamped_to_something_sane() {
        let mut s = Settings::default();
        s.apply(SettingsPatch { shell_timeout_secs: Some(0), ..Default::default() });
        assert_eq!(s.shell_timeout_secs, 5);
        s.apply(SettingsPatch { shell_timeout_secs: Some(999_999), ..Default::default() });
        assert_eq!(s.shell_timeout_secs, 7200);
    }
}

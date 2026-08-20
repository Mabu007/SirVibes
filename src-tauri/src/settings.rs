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
    fn the_timeout_is_clamped_to_something_sane() {
        let mut s = Settings::default();
        s.apply(SettingsPatch { shell_timeout_secs: Some(0), ..Default::default() });
        assert_eq!(s.shell_timeout_secs, 5);
        s.apply(SettingsPatch { shell_timeout_secs: Some(999_999), ..Default::default() });
        assert_eq!(s.shell_timeout_secs, 7200);
    }
}

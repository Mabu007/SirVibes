//! Credential storage. Secrets live in one owner-readable file that only this
//! native process reads. Nothing here is ever serialised into a command
//! response, a log line, a prompt, or an error message — callers get a masked
//! hint, and the plaintext is fetched only at the moment a request is signed.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Serialize, Deserialize, Default)]
struct Vault {
    #[serde(default)]
    secrets: BTreeMap<String, String>,
}

pub struct SecretStore {
    path: PathBuf,
}

impl SecretStore {
    pub fn new(config_dir: &Path) -> Self {
        Self {
            path: config_dir.join("credentials.json"),
        }
    }

    fn load(&self) -> Vault {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    /// Written to a temporary file and renamed, so an update replaces the old
    /// secret atomically and a crash can never leave a half-written vault.
    fn save(&self, vault: &Vault) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|_| "cannot create config directory")?;
        }
        let raw = serde_json::to_string(vault).map_err(|_| "cannot serialise credentials")?;
        let temp = self.path.with_extension("tmp");
        std::fs::write(&temp, raw).map_err(|_| "cannot write credentials")?;
        restrict(&temp);
        std::fs::rename(&temp, &self.path).map_err(|_| "cannot replace credentials")?;
        restrict(&self.path);
        Ok(())
    }

    pub fn put(&self, id: &str, secret: &str) -> Result<(), String> {
        let mut vault = self.load();
        vault.secrets.insert(id.to_string(), secret.to_string());
        self.save(&vault)
    }

    /// The only way plaintext leaves this module. Call it as late as possible.
    pub fn get(&self, id: &str) -> Option<String> {
        self.load().secrets.get(id).cloned()
    }

    pub fn has(&self, id: &str) -> bool {
        self.load().secrets.contains_key(id)
    }

    pub fn remove(&self, id: &str) -> Result<(), String> {
        let mut vault = self.load();
        vault.secrets.remove(id);
        self.save(&vault)
    }

    /// What the interface is allowed to see: enough to recognise a key, never
    /// enough to use one.
    pub fn hint(&self, id: &str) -> String {
        match self.get(id) {
            Some(secret) if secret.chars().count() >= 8 => {
                let tail: String = secret.chars().rev().take(4).collect::<Vec<_>>()
                    .into_iter().rev().collect();
                format!("••••••••{}", tail)
            }
            Some(_) => "••••••••".to_string(),
            None => String::new(),
        }
    }
}

fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Strip anything that looks like a credential out of text bound for a log, an
/// error message, or the model. Defence in depth: the request builder already
/// keeps secrets out, this catches the case where an upstream API echoes one
/// back in its response body.
pub fn redact(text: &str, secret: Option<&str>) -> String {
    let mut out = text.to_string();
    if let Some(secret) = secret {
        if secret.len() >= 6 {
            out = out.replace(secret, "[redacted]");
        }
    }
    for header in ["authorization:", "Authorization:", "x-api-key:", "X-Api-Key:"] {
        if let Some(start) = out.find(header) {
            let rest = &out[start + header.len()..];
            let end = rest.find(['\n', '\r', '"', ',']).unwrap_or(rest.len());
            let span = &out[start..start + header.len() + end];
            out = out.replace(span, &format!("{} [redacted]", header));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> SecretStore {
        let dir = std::env::temp_dir().join(format!("sirvibe-secrets-{}", name));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        SecretStore::new(&dir)
    }

    #[test]
    fn stores_and_returns_a_secret() {
        let s = store("basic");
        s.put("apify", "apify_api_ABCDEFGH1234").unwrap();
        assert_eq!(s.get("apify").unwrap(), "apify_api_ABCDEFGH1234");
        assert!(s.has("apify"));
    }

    #[test]
    fn the_hint_cannot_be_used_as_a_credential() {
        let s = store("hint");
        s.put("apify", "apify_api_ABCDEFGH1234").unwrap();
        let hint = s.hint("apify");
        assert_eq!(hint, "••••••••1234");
        assert!(!hint.contains("apify_api"));
        assert!(s.hint("missing").is_empty());
    }

    #[test]
    fn an_update_replaces_the_old_secret() {
        let s = store("update");
        s.put("x", "old-secret-value").unwrap();
        s.put("x", "new-secret-value").unwrap();
        assert_eq!(s.get("x").unwrap(), "new-secret-value");
        let raw = std::fs::read_to_string(s.path.clone()).unwrap();
        assert!(!raw.contains("old-secret-value"), "old secret survived the update");
    }

    #[test]
    fn removing_deletes_the_credential_from_disk() {
        let s = store("remove");
        s.put("gone", "super-secret-token").unwrap();
        s.remove("gone").unwrap();
        assert!(s.get("gone").is_none());
        assert!(!s.has("gone"));
        let raw = std::fs::read_to_string(s.path.clone()).unwrap();
        assert!(!raw.contains("super-secret-token"));
    }

    #[test]
    fn the_vault_is_not_world_readable() {
        let s = store("perms");
        s.put("a", "b").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&s.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o077, 0, "group or other can read the vault");
        }
    }

    #[test]
    fn redaction_removes_secrets_and_auth_headers() {
        let out = redact(
            "called with token sk-live-9999 and Authorization: Bearer sk-live-9999\nnext",
            Some("sk-live-9999"),
        );
        assert!(!out.contains("sk-live-9999"), "{}", out);
        assert!(out.contains("[redacted]"));
    }
}

//! Secure API credential storage.
//!
//! Provider API keys are credentials, not configuration. They must not be
//! written into normal JSON/TOML configuration files (`providers.json`,
//! `config.toml`) that are parsed, echoed, diffed, or exported casually.
//!
//! `CredentialStore` keeps secrets in a dedicated
//! `~/.codebro/credentials.json` file written with owner-only permissions
//! (mode `0600` on Unix). On platforms without a keyring this is the safest
//! repository-compatible boundary: secrets are isolated from configuration,
//! masked in the UI, redacted from tool output and logs, and never included in
//! task context or model prompts.
//!
//! The design intentionally keeps the *values* out of any `Debug`/`Serialize`
//! output of configuration structs: `CredentialStore` only ever returns
//! values on demand and only reports presence in listings.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The on-disk representation of stored credentials.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct CredentialFile {
    /// provider-id -> API key
    keys: BTreeMap<String, String>,
}

/// Owner-only file mode on Unix (`-rw-------`).
const SECURE_FILE_MODE: u32 = 0o600;

/// Stores API keys for providers in a dedicated, owner-only file.
#[derive(Debug, Clone, Default)]
pub struct CredentialStore {
    dir: PathBuf,
    keys: BTreeMap<String, String>,
    dirty: bool,
}

impl CredentialStore {
    /// Construct a store rooted at a config directory (e.g. `~/.codebro`).
    pub fn new(config_dir: PathBuf) -> Self {
        CredentialStore {
            dir: config_dir,
            keys: BTreeMap::new(),
            dirty: false,
        }
    }

    /// Load persisted credentials from disk. A missing file is not an error.
    pub fn load(&mut self) -> Result<()> {
        let path = self.path();
        if !path.exists() {
            return Ok(());
        }
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read credentials from {:?}", path))?;
        let file: CredentialFile =
            serde_json::from_str(&content).with_context(|| "Failed to parse credentials file")?;
        self.keys = file.keys;
        self.dirty = false;
        Ok(())
    }

    /// Persist credentials to disk with owner-only permissions.
    pub fn persist(&self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }
        let path = self.path();
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create {:?}", parent))?;
            }
        }
        let file = CredentialFile {
            keys: self.keys.clone(),
        };
        let json = serde_json::to_string_pretty(&file)?;
        std::fs::write(&path, json)
            .with_context(|| format!("Failed to write credentials to {:?}", path))?;
        secure_permissions(&path);
        Ok(())
    }

    /// The credentials file path.
    pub fn path(&self) -> PathBuf {
        self.dir.join("credentials.json")
    }

    /// Whether any credentials are persisted on disk.
    pub fn exists_on_disk(&self) -> bool {
        self.path().exists()
    }

    /// Store (or replace) the API key for a provider. Persists immediately.
    pub fn set(&mut self, provider: &str, key: &str) -> Result<()> {
        if key.trim().is_empty() {
            anyhow::bail!("API key cannot be empty");
        }
        self.keys.insert(provider.to_string(), key.to_string());
        self.dirty = true;
        self.persist()
    }

    /// Retrieve the API key for a provider, if set.
    pub fn get(&self, provider: &str) -> Option<&str> {
        self.keys.get(provider).map(|s| s.as_str())
    }

    /// Whether a key exists for the provider.
    pub fn has(&self, provider: &str) -> bool {
        self.keys.contains_key(provider)
    }

    /// Remove the API key for a provider. Persists immediately.
    pub fn delete(&mut self, provider: &str) -> Result<()> {
        if self.keys.remove(provider).is_some() {
            self.dirty = true;
            self.persist()?;
        }
        Ok(())
    }

    /// All providers that have a stored key (values are never returned).
    pub fn providers_with_keys(&self) -> Vec<String> {
        self.keys.keys().cloned().collect()
    }

    /// Total number of stored keys.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the store has no keys.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Restrict file permissions to the owning user. Best-effort on Unix; the
/// secret is never exposed by a failure to harden permissions here.
fn secure_permissions(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(path) {
            let mut perms = meta.permissions();
            perms.set_mode(SECURE_FILE_MODE);
            let _ = std::fs::set_permissions(path, perms);
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// A masked view of a secret for display: never the full value.
///
/// Keys with 4 or fewer characters render as `••••`; longer keys show the
/// final four characters, e.g. `••••cdef`.
pub fn mask_secret(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    if chars.len() <= 4 {
        "••••".to_string()
    } else {
        format!(
            "••••{}",
            chars[chars.len() - 4..].iter().collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_get_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = CredentialStore::new(dir.path().to_path_buf());
        store.set("openai", "sk-test-123456").unwrap();
        assert_eq!(store.get("openai"), Some("sk-test-123456"));
        assert!(store.has("openai"));
        assert!(!store.has("deepseek"));
    }

    #[test]
    fn test_persist_and_reload() {
        let dir = tempfile::tempdir().expect("tempdir");
        {
            let mut store = CredentialStore::new(dir.path().to_path_buf());
            store.set("openai", "sk-secret-value").unwrap();
            store.set("ollama", "").unwrap_err();
        }
        let mut reloaded = CredentialStore::new(dir.path().to_path_buf());
        reloaded.load().unwrap();
        assert_eq!(reloaded.get("openai"), Some("sk-secret-value"));
        assert_eq!(reloaded.len(), 1);
    }

    #[test]
    fn test_delete() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = CredentialStore::new(dir.path().to_path_buf());
        store.set("openai", "sk-x").unwrap();
        store.delete("openai").unwrap();
        assert!(!store.has("openai"));
        assert!(store.is_empty());
    }

    #[test]
    fn test_secure_permissions_unix() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().expect("tempdir");
            let mut store = CredentialStore::new(dir.path().to_path_buf());
            store.set("openai", "sk-perm-check-123456").unwrap();
            let meta = std::fs::metadata(store.path()).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    #[test]
    fn test_mask_secret() {
        assert_eq!(mask_secret("abcd"), "••••");
        assert_eq!(mask_secret("sk-1234567890abcdef"), "••••cdef");
        assert_eq!(mask_secret(""), "••••");
    }

    #[test]
    fn test_empty_key_rejected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = CredentialStore::new(dir.path().to_path_buf());
        store.set("openai", "").unwrap_err();
        store.set("openai", "   ").unwrap_err();
        assert!(!store.has("openai"));
    }
}

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
use std::path::{Path, PathBuf};

/// The on-disk representation of stored credentials. Never `Debug`-printed:
/// the values are secrets and must not leak through diagnostics.
#[derive(Clone, Default, Serialize, Deserialize)]
struct CredentialFile {
    /// provider-id -> API key
    keys: BTreeMap<String, String>,
}

/// Owner-only file mode on Unix (`-rw-------`).
const SECURE_FILE_MODE: u32 = 0o600;

/// Stores API keys for providers in a dedicated, owner-only file.
///
/// The `Debug` implementation intentionally exposes provider presence only,
/// never key values. The struct never derives `Serialize`/`Deserialize`; the
/// secure file format is written through the atomic [`CredentialStore::persist`]
/// path.
#[derive(Clone, Default)]
pub struct CredentialStore {
    dir: PathBuf,
    keys: BTreeMap<String, String>,
    dirty: bool,
}

impl std::fmt::Debug for CredentialStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialStore")
            .field("dir", &self.dir)
            .field(
                "providers",
                &self.keys.keys().cloned().collect::<Vec<String>>(),
            )
            .field("dirty", &self.dirty)
            .finish()
    }
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
    ///
    /// The path is validated before reading so a pre-placed symlink cannot
    /// redirect the read to an attacker-controlled file.
    pub fn load(&mut self) -> Result<()> {
        let path = self.path();
        if !path.exists() {
            return Ok(());
        }
        reject_symlink(&path)?;
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read credentials from {:?}", path))?;
        let file: CredentialFile =
            serde_json::from_str(&content).with_context(|| "Failed to parse credentials file")?;
        self.keys = file.keys;
        self.dirty = false;
        Ok(())
    }

    /// Persist credentials to disk with owner-only permissions.
    ///
    /// The write is atomic and symlink-safe:
    /// 1. the target path is validated (a symlink here is refused);
    /// 2. content is written to a uniquely-named temporary file created with
    ///    mode `0600` and `create_new` (never following an existing entry);
    /// 3. the temp file is flushed and fsynced;
    /// 4. it is atomically renamed over the target;
    /// 5. mode `0600` is re-asserted and verified.
    ///
    /// Security-critical failures (permissions, sync, rename) are propagated as
    /// errors and never silently ignored.
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
        reject_symlink(&path)?;

        let file = CredentialFile {
            keys: self.keys.clone(),
        };
        let json = serde_json::to_string_pretty(&file)?;

        let tmp = unique_temp_path(&path);
        write_secure_file(&tmp, &json)?;
        if let Err(e) = sync_file(&tmp) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e).with_context(|| {
                format!(
                    "Failed to atomically move credentials into place at {:?}",
                    path
                )
            });
        }
        // Re-assert and verify 0600 after the rename (belt-and-braces for
        // filesystems that do not preserve the temp file's mode).
        secure_permissions_strict(&path)?;
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

/// Refuse to operate on a credentials path that is a symlink. On Unix a
/// symlink here means an attacker (or a compromised process) could redirect
/// reads/writes; the safe behaviour is to fail loudly.
fn reject_symlink(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileTypeExt;
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            if meta.file_type().is_symlink() {
                anyhow::bail!(
                    "Refusing to access credentials through a symlink: {:?}",
                    path
                );
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

/// A uniquely-named temporary file in the same directory as `path`, so the
/// final `rename` stays on one filesystem (atomic).
fn unique_temp_path(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "credentials.json".to_string());
    parent.join(format!(
        ".{}.{}.{}.tmp",
        name,
        std::process::id(),
        uuid::Uuid::new_v4()
    ))
}

/// Write `content` to `path` creating it with mode `0600` and refusing to
/// follow an existing entry (`create_new`).
fn write_secure_file(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true).mode(SECURE_FILE_MODE);
        let mut f = opts
            .open(path)
            .with_context(|| format!("Failed to create temporary credentials file {:?}", path))?;
        f.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write credentials to {:?}", path))?;
        f.flush()
            .with_context(|| format!("Failed to flush credentials to {:?}", path))?;
    }
    #[cfg(not(unix))]
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .with_context(|| format!("Failed to create temporary credentials file {:?}", path))?;
        f.write_all(content.as_bytes())
            .with_context(|| format!("Failed to write credentials to {:?}", path))?;
        f.flush()
            .with_context(|| format!("Failed to flush credentials to {:?}", path))?;
    }
    Ok(())
}

/// Flush file contents to stable storage before the atomic rename so a crash
/// cannot leave a torn credentials file at the final path.
fn sync_file(path: &Path) -> Result<()> {
    let f = std::fs::File::open(path)
        .with_context(|| format!("Failed to reopen {:?} for sync", path))?;
    f.sync_all()
        .with_context(|| format!("Failed to sync credentials file {:?}", path))?;
    Ok(())
}

/// Set mode `0600` on `path` and verify it actually took effect. A permission
/// failure is a security regression and is reported as an error, never
/// swallowed.
fn secure_permissions_strict(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let meta = std::fs::metadata(path)
            .with_context(|| format!("Failed to stat credentials file {:?}", path))?;
        let mut perms = meta.permissions();
        perms.set_mode(SECURE_FILE_MODE);
        std::fs::set_permissions(path, perms)
            .with_context(|| format!("Failed to set 0600 permissions on {:?}", path))?;
        let meta = std::fs::metadata(path)
            .with_context(|| format!("Failed to re-stat credentials file {:?}", path))?;
        if meta.permissions().mode() & 0o777 != SECURE_FILE_MODE {
            anyhow::bail!(
                "Credentials file {:?} has insecure permissions (mode {:o})",
                path,
                meta.permissions().mode() & 0o777
            );
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
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

    #[test]
    fn test_debug_masks_secret_values() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = CredentialStore::new(dir.path().to_path_buf());
        store
            .set("openai", "sk-never-debug-1234567890abcdef")
            .unwrap();
        let debug = format!("{:?}", store);
        assert!(
            !debug.contains("sk-never-debug-1234567890abcdef"),
            "Debug output leaked the secret: {}",
            debug
        );
        assert!(
            debug.contains("openai"),
            "provider presence must be visible"
        );
    }

    #[test]
    fn test_load_refuses_symlink() {
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().expect("tempdir");
            // Real store writes the file first.
            {
                let mut store = CredentialStore::new(dir.path().to_path_buf());
                store.set("openai", "sk-symlink-test-123456").unwrap();
            }
            // Replace the file with a symlink pointing at a decoy.
            let target = dir.path().join("credentials.json");
            let decoy = dir.path().join("decoy.json");
            std::fs::write(&decoy, r#"{"keys":{"openai":"sk-decoy-value"}}"#).unwrap();
            std::fs::remove_file(&target).unwrap();
            std::os::unix::fs::symlink(&decoy, &target).unwrap();

            let mut store = CredentialStore::new(dir.path().to_path_buf());
            let err = store.load().unwrap_err();
            assert!(
                err.to_string().contains("symlink"),
                "load must reject a symlink, got: {}",
                err
            );
        }
    }

    #[test]
    fn test_persist_refuses_symlink() {
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().expect("tempdir");
            let decoy = dir.path().join("decoy.json");
            std::fs::write(&decoy, "{}").unwrap();
            let target = dir.path().join("credentials.json");
            std::os::unix::fs::symlink(&decoy, &target).unwrap();

            let mut store = CredentialStore::new(dir.path().to_path_buf());
            store
                .keys
                .insert("openai".to_string(), "sk-x-123456".to_string());
            store.dirty = true;
            let err = store.persist().unwrap_err();
            assert!(
                err.to_string().contains("symlink"),
                "persist must reject a symlink, got: {}",
                err
            );
            // The decoy must not have been written through the symlink.
            assert_eq!(std::fs::read_to_string(&decoy).unwrap(), "{}");
        }
    }

    #[test]
    fn test_no_temp_files_left_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut store = CredentialStore::new(dir.path().to_path_buf());
        store.set("openai", "sk-temp-check-123456").unwrap();
        store.delete("openai").unwrap();
        let leftovers: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            leftovers.iter().all(|n| !n.ends_with(".tmp")),
            "temporary credential files must not be left behind: {:?}",
            leftovers
        );
        // The deleted secret must not survive in the persisted file.
        let content =
            std::fs::read_to_string(dir.path().join("credentials.json")).unwrap_or_default();
        assert!(!content.contains("sk-temp-check-123456"));
    }

    #[test]
    fn test_persist_failure_is_reported_and_preserves_existing() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().unwrap();
            let dir_path = dir.path().to_path_buf();
            let mut store = CredentialStore::new(dir_path.clone());
            store.set("openai", "sk-first-value-123456").unwrap();

            // Make the directory read-only so the next atomic write cannot
            // create its temporary file.
            std::fs::set_permissions(&dir_path, std::fs::Permissions::from_mode(0o555)).unwrap();
            let mut store2 = CredentialStore::new(dir_path.clone());
            store2
                .keys
                .insert("ollama".to_string(), "sk-ollama-123456".to_string());
            store2.dirty = true;
            let result = store2.persist();
            std::fs::set_permissions(&dir_path, std::fs::Permissions::from_mode(0o755)).unwrap();
            assert!(
                result.is_err(),
                "persist must surface a security-critical write failure"
            );
            let content =
                std::fs::read_to_string(dir_path.join("credentials.json")).unwrap_or_default();
            assert!(
                content.contains("sk-first-value-123456"),
                "existing credentials must survive a failed write"
            );
            assert!(
                !content.contains("sk-ollama-123456"),
                "failed write must not partially persist the new key"
            );
        }
    }

    #[test]
    fn test_file_owner_only_mode_and_no_trailing_perms() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let dir = tempfile::tempdir().expect("tempdir");
            let mut store = CredentialStore::new(dir.path().to_path_buf());
            store.set("openai", "sk-perm-strict-123456").unwrap();
            let meta = std::fs::metadata(store.path()).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
            // No group/other read/write/execute bits may be set.
            assert_eq!(meta.permissions().mode() & 0o077, 0);
        }
    }
}

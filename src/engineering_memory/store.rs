//! Persistent storage for engineering memory.
//!
//! Reads and writes `.codebro/engineering_memory.json`.
//! Rejects files that belong to a different workspace root.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use super::types::{EngineeringMemoryEntry, EngineeringMemoryFile, CURRENT_SCHEMA_VERSION};

/// Errors that can occur during storage operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    DirectoryCreate(String),
    Write(String),
    Read(String),
    Serialize(String),
    Deserialize(String),
    WrongWorkspaceRoot(String),
    WrongSchemaVersion(String),
    NotFound(String),
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageError::DirectoryCreate(e) => write!(f, "directory create: {}", e),
            StorageError::Write(e) => write!(f, "write: {}", e),
            StorageError::Read(e) => write!(f, "read: {}", e),
            StorageError::Serialize(e) => write!(f, "serialize: {}", e),
            StorageError::Deserialize(e) => write!(f, "deserialize: {}", e),
            StorageError::WrongWorkspaceRoot(root) => {
                write!(f, "wrong workspace root: {}", root)
            }
            StorageError::WrongSchemaVersion(v) => {
                write!(f, "wrong schema version: {}", v)
            }
            StorageError::NotFound(p) => write!(f, "not found: {}", p),
        }
    }
}

impl std::error::Error for StorageError {}

impl From<io::Error> for StorageError {
    fn from(e: io::Error) -> Self {
        StorageError::Read(e.to_string())
    }
}

impl From<serde_json::Error> for StorageError {
    fn from(e: serde_json::Error) -> Self {
        StorageError::Deserialize(e.to_string())
    }
}

/// File-based storage for engineering memory entries.
#[derive(Debug, Clone)]
pub struct EngineeringMemoryStore {
    codebro_dir: PathBuf,
}

impl EngineeringMemoryStore {
    /// Create a new store pointing at `<workspace_root>/.codebro/`.
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let codebro_dir = workspace_root.as_ref().join(".codebro");
        EngineeringMemoryStore { codebro_dir }
    }

    /// Return the path to the `.codebro/` directory.
    pub fn codebro_dir(&self) -> &Path {
        &self.codebro_dir
    }

    /// Return the path to `engineering_memory.json`.
    pub fn memory_path(&self) -> PathBuf {
        self.codebro_dir.join("engineering_memory.json")
    }

    /// Ensure the `.codebro/` directory exists.
    pub fn ensure_directory(&self) -> Result<(), StorageError> {
        fs::create_dir_all(&self.codebro_dir)
            .map_err(|e| StorageError::DirectoryCreate(e.to_string()))?;
        Ok(())
    }

    /// Load the engineering memory file.
    ///
    /// Returns `Err(NotFound)` if the file does not exist.
    /// Returns `Err(WrongWorkspaceRoot)` if the file belongs to a different project.
    /// Returns `Err(WrongSchemaVersion)` if the schema version is unknown.
    pub fn load(&self, expected_root: &str) -> Result<EngineeringMemoryFile, StorageError> {
        let path = self.memory_path();
        if !path.exists() {
            return Err(StorageError::NotFound(
                format!("{}", path.display()),
            ));
        }
        let content = fs::read_to_string(&path)?;
        let file: EngineeringMemoryFile = serde_json::from_str(&content)?;

        if file.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(StorageError::WrongSchemaVersion(file.schema_version));
        }

        if file.workspace_root != expected_root {
            return Err(StorageError::WrongWorkspaceRoot(file.workspace_root));
        }

        Ok(file)
    }

    /// Save the engineering memory file.
    pub fn save(&self, file: &EngineeringMemoryFile) -> Result<(), StorageError> {
        self.ensure_directory()?;
        let json = serde_json::to_string_pretty(file)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut f = fs::File::create(&self.memory_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        f.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        f.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Check whether the memory file exists.
    pub fn memory_exists(&self) -> bool {
        self.memory_path().exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_memory::types::{EngineeringMemoryEntry, EngineeringMemoryMetadata};
    use tempfile::TempDir;

    fn make_entry(id: &str, key: &str, value: &str) -> EngineeringMemoryEntry {
        EngineeringMemoryEntry::new(id, key, value)
            .with_metadata(
                EngineeringMemoryMetadata::new()
                    .with_confidence(0.9)
                    .with_importance(0.8),
            )
    }

    fn setup() -> (EngineeringMemoryStore, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let store = EngineeringMemoryStore::new(tmp.path());
        (store, tmp)
    }

    #[test]
    fn test_load_missing() {
        let (store, _tmp) = setup();
        let result = store.load("/tmp/test");
        assert!(matches!(result, Err(StorageError::NotFound(_))));
    }

    #[test]
    fn test_save_and_load() {
        let (store, _tmp) = setup();
        let mut file = EngineeringMemoryFile::new("/tmp/test");
        file.entries.push(make_entry("e1", "language", "rust"));
        store.save(&file).expect("save");

        let loaded = store.load("/tmp/test").expect("load");
        assert_eq!(loaded.entries.len(), 1);
        assert_eq!(loaded.entries[0].key, "language");
        assert_eq!(loaded.entries[0].value, "rust");
    }

    #[test]
    fn test_wrong_workspace_root_rejected() {
        let (store, _tmp) = setup();
        let mut file = EngineeringMemoryFile::new("/tmp/other-project");
        file.entries.push(make_entry("e1", "key", "value"));
        store.save(&file).expect("save");

        let result = store.load("/tmp/different-project");
        assert!(matches!(
            result,
            Err(StorageError::WrongWorkspaceRoot(_))
        ));
    }

    #[test]
    fn test_wrong_schema_version_rejected() {
        let (store, _tmp) = setup();
        let mut file = EngineeringMemoryFile::new("/tmp/test");
        file.schema_version = "9.9.9".to_string();
        store.save(&file).expect("save");

        let result = store.load("/tmp/test");
        assert!(matches!(
            result,
            Err(StorageError::WrongSchemaVersion(_))
        ));
    }

    #[test]
    fn test_memory_exists_after_save() {
        let (store, _tmp) = setup();
        assert!(!store.memory_exists());
        store
            .save(&EngineeringMemoryFile::new("/tmp/test"))
            .expect("save");
        assert!(store.memory_exists());
    }
}

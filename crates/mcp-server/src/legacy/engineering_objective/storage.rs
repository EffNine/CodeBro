//! File-based persistence for engineering objectives.
//!
//! Stores the compact objective hierarchy in
//! `<workspace_root>/.codebro/engineering_objective.json` as human-readable
//! JSON. This mirrors the `project_identity` storage pattern: a single
//! canonical file, schema-versioned, read at runtime startup.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::objective::{EngineeringObjective, CURRENT_SCHEMA_VERSION};

/// Error type for objective storage operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObjectiveStorageError {
    DirectoryCreate(String),
    Write(String),
    Read(String),
    Serialize(String),
    Deserialize(String),
    NotFound(String),
}

impl std::fmt::Display for ObjectiveStorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ObjectiveStorageError::DirectoryCreate(e) => write!(f, "directory create: {}", e),
            ObjectiveStorageError::Write(e) => write!(f, "write: {}", e),
            ObjectiveStorageError::Read(e) => write!(f, "read: {}", e),
            ObjectiveStorageError::Serialize(e) => write!(f, "serialize: {}", e),
            ObjectiveStorageError::Deserialize(e) => write!(f, "deserialize: {}", e),
            ObjectiveStorageError::NotFound(p) => write!(f, "not found: {}", p),
        }
    }
}

impl std::error::Error for ObjectiveStorageError {}

impl From<io::Error> for ObjectiveStorageError {
    fn from(e: io::Error) -> Self {
        ObjectiveStorageError::Read(e.to_string())
    }
}

impl From<serde_json::Error> for ObjectiveStorageError {
    fn from(e: serde_json::Error) -> Self {
        ObjectiveStorageError::Deserialize(e.to_string())
    }
}

/// A versioned objective file written to disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectiveFile {
    pub schema_version: String,
    pub workspace_root: String,
    pub objective: EngineeringObjective,
}

impl ObjectiveFile {
    pub fn new(workspace_root: impl Into<String>, objective: EngineeringObjective) -> Self {
        ObjectiveFile {
            schema_version: CURRENT_SCHEMA_VERSION.to_string(),
            workspace_root: workspace_root.into(),
            objective,
        }
    }
}

/// File-based storage backend for engineering objectives.
#[derive(Debug, Clone)]
pub struct ObjectiveStorage {
    codebro_dir: PathBuf,
}

impl ObjectiveStorage {
    /// Create storage pointing at `<workspace_root>/.codebro/`.
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        ObjectiveStorage {
            codebro_dir: workspace_root.as_ref().join(".codebro"),
        }
    }

    /// Path to `engineering_objective.json`.
    pub fn objective_path(&self) -> PathBuf {
        self.codebro_dir.join("engineering_objective.json")
    }

    /// Whether an objective file exists.
    pub fn objective_exists(&self) -> bool {
        self.objective_path().exists()
    }

    /// Ensure the `.codebro/` directory exists.
    pub fn ensure_directory(&self) -> Result<(), ObjectiveStorageError> {
        fs::create_dir_all(&self.codebro_dir)
            .map_err(|e| ObjectiveStorageError::DirectoryCreate(e.to_string()))?;
        Ok(())
    }

    /// Persist the objective to `engineering_objective.json`.
    pub fn save(
        &self,
        workspace_root: &str,
        objective: &EngineeringObjective,
    ) -> Result<(), ObjectiveStorageError> {
        self.ensure_directory()?;
        let file = ObjectiveFile::new(workspace_root, objective.clone());
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| ObjectiveStorageError::Serialize(e.to_string()))?;
        let mut out = fs::File::create(&self.objective_path())
            .map_err(|e| ObjectiveStorageError::Write(e.to_string()))?;
        out.write_all(json.as_bytes())
            .map_err(|e| ObjectiveStorageError::Write(e.to_string()))?;
        out.flush()
            .map_err(|e| ObjectiveStorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Load the objective file, verifying workspace root and schema.
    ///
    /// Returns `Err(NotFound)` when no file exists so the caller can fall
    /// back to the documented default objective.
    pub fn load(&self, expected_root: &str) -> Result<ObjectiveFile, ObjectiveStorageError> {
        let path = self.objective_path();
        if !path.exists() {
            return Err(ObjectiveStorageError::NotFound(format!(
                "{}",
                path.display()
            )));
        }
        let content =
            fs::read_to_string(&path).map_err(|e| ObjectiveStorageError::Read(e.to_string()))?;
        let file: ObjectiveFile = serde_json::from_str(&content)
            .map_err(|e| ObjectiveStorageError::Deserialize(e.to_string()))?;
        if file.workspace_root != expected_root {
            return Err(ObjectiveStorageError::NotFound(format!(
                "objective belongs to {} not {}",
                file.workspace_root, expected_root
            )));
        }
        if file.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(ObjectiveStorageError::NotFound(format!(
                "unknown objective schema {}",
                file.schema_version
            )));
        }
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn objective() -> EngineeringObjective {
        EngineeringObjective::new("End goal", "Vision", "Current objective", "Milestone")
            .with_source("docs/vision/CODEBRO_VISION.md")
    }

    fn setup() -> (ObjectiveStorage, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let storage = ObjectiveStorage::new(tmp.path());
        (storage, tmp)
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let (storage, tmp) = setup();
        let root = tmp.path().to_string_lossy().to_string();
        storage.save(&root, &objective()).expect("save");
        assert!(storage.objective_exists());

        let loaded = storage.load(&root).expect("load");
        assert_eq!(loaded.objective, objective());
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
        assert_eq!(loaded.workspace_root, root);
    }

    #[test]
    fn test_load_missing_returns_not_found() {
        let (storage, _tmp) = setup();
        let result = storage.load("/tmp/nope");
        assert!(matches!(result, Err(ObjectiveStorageError::NotFound(_))));
    }

    #[test]
    fn test_load_wrong_workspace_rejected() {
        let (storage, tmp) = setup();
        let root = tmp.path().to_string_lossy().to_string();
        storage.save(&root, &objective()).expect("save");
        let result = storage.load("/tmp/other");
        assert!(matches!(result, Err(ObjectiveStorageError::NotFound(_))));
    }

    #[test]
    fn test_serialization_deterministic() {
        let (storage, tmp) = setup();
        let root = tmp.path().to_string_lossy().to_string();
        storage.save(&root, &objective()).expect("save");
        let content1 = fs::read_to_string(storage.objective_path()).expect("read");
        // Re-save the same objective and confirm identical content.
        storage.save(&root, &objective()).expect("save");
        let content2 = fs::read_to_string(storage.objective_path()).expect("read");
        assert_eq!(content1, content2);
    }
}

//! File-based persistence for project identity.
//!
//! Stores identity data in `.codebro/` as human-readable JSON files.
//!
//! ## Storage Layout
//!
//! | File | Content |
 //! |------|---------|
//! | `project_identity.json` | Full identity snapshot |
//! | `workspace.json` | Workspace metadata |
//! | `architecture.json` | Architecture summary and patterns |
//! | `engineering_decisions.json` | Engineering decisions |
//! | `constraints.json` | Known constraints |
//! | `roadmap.json` | Roadmap items |
//! | `current_sprint.json` | Active sprint |
//! | `metadata.json` | Metadata (version, timestamps) |

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::identity::{ProjectIdentity, EngineeringDecision, RoadmapItem};

/// Error type for storage operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageError {
    /// The `.codebro/` directory could not be created.
    DirectoryCreate(String),
    /// A file could not be written.
    Write(String),
    /// A file could not be read.
    Read(String),
    /// JSON serialization failed.
    Serialize(String),
    /// JSON deserialization failed.
    Deserialize(String),
    /// The target path is not a directory.
    NotADirectory(PathBuf),
    /// The requested file was not found.
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
            StorageError::NotADirectory(p) => {
                write!(f, "not a directory: {}", p.display())
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

/// File-based storage backend for project identity.
#[derive(Debug, Clone)]
pub struct ProjectIdentityStorage {
    /// Root directory for `.codebro/` files.
    codebro_dir: PathBuf,
}

/// Metadata stored in `metadata.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct MetadataFile {
    schema_version: String,
    created_at: String,
    updated_at: String,
}

impl ProjectIdentityStorage {
    /// Create a new storage backend pointing at `<workspace_root>/.codebro/`.
    pub fn new(workspace_root: impl AsRef<Path>) -> Self {
        let codebro_dir = workspace_root
            .as_ref()
            .join(".codebro");
        ProjectIdentityStorage { codebro_dir }
    }

    /// Return the path to the `.codebro/` directory.
    pub fn codebro_dir(&self) -> &Path {
        &self.codebro_dir
    }

    /// Ensure the `.codebro/` directory exists.
    pub fn ensure_directory(&self) -> Result<(), StorageError> {
        fs::create_dir_all(&self.codebro_dir)
            .map_err(|e| StorageError::DirectoryCreate(e.to_string()))?;
        Ok(())
    }

    /// Return the path to `project_identity.json`.
    pub fn identity_path(&self) -> PathBuf {
        self.codebro_dir.join("project_identity.json")
    }

    /// Return the path to `workspace.json`.
    pub fn workspace_path(&self) -> PathBuf {
        self.codebro_dir.join("workspace.json")
    }

    /// Return the path to `architecture.json`.
    pub fn architecture_path(&self) -> PathBuf {
        self.codebro_dir.join("architecture.json")
    }

    /// Return the path to `engineering_decisions.json`.
    pub fn decisions_path(&self) -> PathBuf {
        self.codebro_dir.join("engineering_decisions.json")
    }

    /// Return the path to `constraints.json`.
    pub fn constraints_path(&self) -> PathBuf {
        self.codebro_dir.join("constraints.json")
    }

    /// Return the path to `roadmap.json`.
    pub fn roadmap_path(&self) -> PathBuf {
        self.codebro_dir.join("roadmap.json")
    }

    /// Return the path to `current_sprint.json`.
    pub fn sprint_path(&self) -> PathBuf {
        self.codebro_dir.join("current_sprint.json")
    }

    /// Return the path to `metadata.json`.
    pub fn metadata_path(&self) -> PathBuf {
        self.codebro_dir.join("metadata.json")
    }

    /// Check whether the identity file exists.
    pub fn identity_exists(&self) -> bool {
        self.identity_path().exists()
    }

    /// Write the full `ProjectIdentity` to `project_identity.json`.
    pub fn save_identity(
        &self,
        identity: &ProjectIdentity,
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        let json = serde_json::to_string_pretty(identity)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.identity_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Read the full `ProjectIdentity` from `project_identity.json`.
    pub fn load_identity(&self) -> Result<ProjectIdentity, StorageError> {
        let path = self.identity_path();
        if !path.exists() {
            return Err(StorageError::NotFound(
                format!("{}", path.display()),
            ));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| StorageError::Read(e.to_string()))?;
        let identity: ProjectIdentity =
            serde_json::from_str(&content).map_err(|e| StorageError::Deserialize(e.to_string()))?;
        Ok(identity)
    }

    /// Write workspace metadata to `workspace.json`.
    pub fn save_workspace(
        &self,
        root_path: &str,
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        #[derive(Serialize)]
        struct WorkspaceMeta {
            root_path: String,
            created_at: String,
            updated_at: String,
        }
        let meta = WorkspaceMeta {
            root_path: root_path.to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_string_pretty(&meta)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.workspace_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Write architecture data to `architecture.json`.
    pub fn save_architecture(
        &self,
        summary: &str,
        patterns: &[String],
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        #[derive(Serialize)]
        struct ArchData {
            summary: String,
            patterns: Vec<String>,
            updated_at: String,
        }
        let data = ArchData {
            summary: summary.to_string(),
            patterns: patterns.to_vec(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.architecture_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Write engineering decisions to `engineering_decisions.json`.
    pub fn save_decisions(
        &self,
        decisions: &[EngineeringDecision],
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        let json = serde_json::to_string_pretty(decisions)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.decisions_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Write constraints to `constraints.json`.
    pub fn save_constraints(
        &self,
        constraints: &[String],
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        let json = serde_json::to_string_pretty(constraints)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.constraints_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Write roadmap to `roadmap.json`.
    pub fn save_roadmap(
        &self,
        items: &[RoadmapItem],
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        let json = serde_json::to_string_pretty(items)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.roadmap_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Write current sprint to `current_sprint.json`.
    pub fn save_sprint(
        &self,
        sprint: &str,
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        #[derive(Serialize)]
        struct SprintData {
            current_sprint: String,
            updated_at: String,
        }
        let data = SprintData {
            current_sprint: sprint.to_string(),
            updated_at: chrono::Utc::now().to_rfc3339(),
        };
        let json = serde_json::to_string_pretty(&data)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.sprint_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Write metadata to `metadata.json`.
    pub fn save_metadata(
        &self,
        schema_version: &str,
        created_at: &str,
        updated_at: &str,
    ) -> Result<(), StorageError> {
        self.ensure_directory()?;
        let meta = MetadataFile {
            schema_version: schema_version.to_string(),
            created_at: created_at.to_string(),
            updated_at: updated_at.to_string(),
        };
        let json = serde_json::to_string_pretty(&meta)
            .map_err(|e| StorageError::Serialize(e.to_string()))?;
        let mut file = fs::File::create(&self.metadata_path())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.write_all(json.as_bytes())
            .map_err(|e| StorageError::Write(e.to_string()))?;
        file.flush()
            .map_err(|e| StorageError::Write(e.to_string()))?;
        Ok(())
    }

    /// Persist a complete `ProjectIdentity` to all eight files.
    ///
    /// This is the canonical internal storage operation used after
    /// create, create_minimal, successful updates, and successful
    /// migrations. The canonical file (`project_identity.json`) and all
    /// seven supplementary projections are written sequentially (not
    /// atomically). Subsequent loads read only from the canonical file;
    /// the supplementary files are derived, inspectable views.
    pub fn save_all(&self, identity: &ProjectIdentity) -> Result<(), StorageError> {
        self.save_identity(identity)?;
        self.save_workspace(
            identity
                .workspace_root
                .as_deref()
                .unwrap_or_default(),
        )?;
        // Always write architecture projection, even when empty.
        self.save_architecture(
            identity.architecture_summary.as_deref().unwrap_or(""),
            &identity.known_patterns,
        )?;
        self.save_decisions(&identity.engineering_decisions)?;
        self.save_constraints(&identity.known_constraints)?;
        self.save_roadmap(&identity.roadmap)?;
        // Always write sprint projection, even when empty.
        self.save_sprint(
            identity.current_sprint.as_deref().unwrap_or(""),
        )?;
        self.save_metadata(
            &identity.schema_version,
            identity.created_at.as_deref().unwrap_or_default(),
            &identity.updated_at,
        )?;
        Ok(())
    }

    /// Read metadata from `metadata.json`.
    pub fn load_metadata(&self) -> Result<MetadataFile, StorageError> {
        let path = self.metadata_path();
        if !path.exists() {
            return Err(StorageError::NotFound(
                format!("{}", path.display()),
            ));
        }
        let content = fs::read_to_string(&path)
            .map_err(|e| StorageError::Read(e.to_string()))?;
        let meta: MetadataFile =
            serde_json::from_str(&content).map_err(|e| StorageError::Deserialize(e.to_string()))?;
        Ok(meta)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (ProjectIdentityStorage, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let storage = ProjectIdentityStorage::new(tmp.path());
        (storage, tmp)
    }

    #[test]
    fn test_storage_creates_directory() {
        let (storage, _tmp) = setup();
        assert!(storage.codebro_dir().ends_with(".codebro"));
    }

    #[test]
    fn test_save_and_load_identity() {
        let (storage, _tmp) = setup();
        let identity = ProjectIdentity::new("test-proj", "rust");
        storage.save_identity(&identity).expect("save");
        let loaded = storage.load_identity().expect("load");
        assert_eq!(loaded.name, "test-proj");
        assert_eq!(loaded.primary_language(), "rust");
    }

    #[test]
    fn test_load_missing_identity() {
        let (storage, _tmp) = setup();
        let result = storage.load_identity();
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_workspace() {
        let (storage, _tmp) = setup();
        storage.save_workspace("/tmp/project").expect("save");
        // workspace.json is write-only in this test; verify file exists.
        assert!(storage.workspace_path().exists());
    }

    #[test]
    fn test_save_and_load_decisions() {
        let (storage, _tmp) = setup();
        let decisions = vec![
            EngineeringDecision::new(
                "dec-1",
                "Use Rust",
                "Use Rust for the core",
                None,
            ),
        ];
        storage.save_decisions(&decisions).expect("save");
        let content = fs::read_to_string(storage.decisions_path())
            .expect("read");
        let loaded: Vec<EngineeringDecision> =
            serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "dec-1");
    }

    #[test]
    fn test_save_and_load_constraints() {
        let (storage, _tmp) = setup();
        let constraints = vec![
            "No raw SQL".to_string(),
            "Use context for timeouts".to_string(),
        ];
        storage.save_constraints(&constraints).expect("save");
        let content = fs::read_to_string(storage.constraints_path())
            .expect("read");
        let loaded: Vec<String> =
            serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0], "No raw SQL");
    }

    #[test]
    fn test_save_and_load_roadmap() {
        let (storage, _tmp) = setup();
        let items = vec![
            RoadmapItem::new("item-1", "Fix auth bug", None),
        ];
        storage.save_roadmap(&items).expect("save");
        let content = fs::read_to_string(storage.roadmap_path())
            .expect("read");
        let loaded: Vec<RoadmapItem> =
            serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id, "item-1");
    }

    #[test]
    fn test_save_and_load_sprint() {
        let (storage, _tmp) = setup();
        storage.save_sprint("sprint-23").expect("save");
        let content = fs::read_to_string(storage.sprint_path())
            .expect("read");
        #[derive(Deserialize)]
        struct SprintFile {
            current_sprint: String,
        }
        let loaded: SprintFile =
            serde_json::from_str(&content).expect("parse");
        assert_eq!(loaded.current_sprint, "sprint-23");
    }

    #[test]
    fn test_save_and_load_metadata() {
        let (storage, _tmp) = setup();
        storage
            .save_metadata("1.0.0", "2026-01-01T00:00:00Z", "2026-08-09T00:00:00Z")
            .expect("save");
        let meta = storage.load_metadata().expect("load");
        assert_eq!(meta.schema_version, "1.0.0");
        assert_eq!(meta.created_at, "2026-01-01T00:00:00Z");
        assert_eq!(meta.updated_at, "2026-08-09T00:00:00Z");
    }

    #[test]
    fn test_load_metadata_missing() {
        let (storage, _tmp) = setup();
        let result = storage.load_metadata();
        assert!(result.is_err());
    }

    #[test]
    fn test_serialization_deterministic() {
        let (storage, _tmp) = setup();
        let identity = ProjectIdentity::new("det-proj", "go")
            .with_framework("gin")
            .with_build_system("go build")
            .add_important_file("main.go")
            .add_important_file("go.mod");
        storage.save_identity(&identity).expect("save");
        let content1 = fs::read_to_string(storage.identity_path())
            .expect("read");
        // Re-serialise the same identity and compare.
        let content2 = serde_json::to_string_pretty(&identity)
            .expect("re-serialize");
        assert_eq!(content1.trim(), content2.trim());
    }

    #[test]
    fn test_save_all_persists_all_eight_files() {
        let (storage, _tmp) = setup();
        let identity = ProjectIdentity::new("all-proj", "rust")
            .with_architecture_summary("layered")
            .add_knowledge_pattern("mvc")
            .add_engineering_decision(
                EngineeringDecision::new("dec-1", "Use Rust", "Core language", None),
            )
            .add_known_constraint("no-raw-sql")
            .add_roadmap_item(RoadmapItem::new("item-1", "Fix bug", None))
            .with_current_sprint("sprint-23")
            .with_build_system("cargo");
        storage.save_all(&identity).expect("save_all");
        assert!(storage.identity_path().exists());
        assert!(storage.workspace_path().exists());
        assert!(storage.architecture_path().exists());
        assert!(storage.decisions_path().exists());
        assert!(storage.constraints_path().exists());
        assert!(storage.roadmap_path().exists());
        assert!(storage.sprint_path().exists());
        assert!(storage.metadata_path().exists());
    }

    #[test]
    fn test_save_all_canonical_match() {
        let (storage, _tmp) = setup();
        let identity = ProjectIdentity::new("match-proj", "python")
            .with_architecture_summary("monolith")
            .add_knowledge_pattern("service-oriented")
            .add_known_constraint("type-hint-all");
        storage.save_all(&identity).expect("save_all");
        let reloaded = storage.load_identity().expect("reload");
        assert_eq!(reloaded.name, "match-proj");
        assert_eq!(reloaded.architecture_summary, Some("monolith".to_string()));
        assert_eq!(reloaded.known_constraints, vec!["type-hint-all"]);
    }
}

//! Loader for project identity from `.codebro/` storage.

use std::time::Instant;

use super::identity::{ProjectIdentity, EngineeringDecision, RoadmapItem, DecisionStatus, RoadmapStatus};
use super::migration::{apply_migrations, validate_schema_version, MigrationResult};
use super::storage::{ProjectIdentityStorage, StorageError};
use super::diagnostics::{IdentitySource, ProjectIdentityDiagnostics};
use super::validation::{validate_identity, ValidationReport};

/// Errors that can occur during identity loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadError {
    /// Storage error (file not found, read error, etc.).
    Storage(StorageError),
    /// Schema version is unknown.
    UnknownSchemaVersion(String),
    /// Validation failed after loading.
    ValidationFailed(Vec<String>),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Storage(e) => write!(f, "storage error: {}", e),
            LoadError::UnknownSchemaVersion(v) => {
                write!(f, "unknown schema version: {}", v)
            }
            LoadError::ValidationFailed(errors) => {
                write!(f, "validation failed: {}", errors.join("; "))
            }
        }
    }
}

impl std::error::Error for LoadError {}

impl From<StorageError> for LoadError {
    fn from(e: StorageError) -> Self {
        LoadError::Storage(e)
    }
}

/// Result of loading project identity.
#[derive(Debug)]
pub struct LoadResult {
    pub identity: ProjectIdentity,
    pub diagnostics: ProjectIdentityDiagnostics,
    pub migrated: bool,
}

/// Loader for project identity from `.codebro/` storage.
#[derive(Debug, Clone)]
pub struct ProjectIdentityLoader {
    storage: ProjectIdentityStorage,
}

impl ProjectIdentityLoader {
    /// Create a new loader for the given workspace root.
    pub fn new(workspace_root: impl AsRef<std::path::Path>) -> Self {
        ProjectIdentityLoader {
            storage: ProjectIdentityStorage::new(workspace_root),
        }
    }

    /// Return a reference to the underlying storage.
    pub fn storage(&self) -> &ProjectIdentityStorage {
        &self.storage
    }

    /// Load project identity from `.codebro/project_identity.json`.
    ///
    /// If the file does not exist, returns `Err(LoadError::Storage)` with
    /// a `NotFound` variant so the caller can create a fresh identity.
    ///
    /// The loader validates the post-migration identity and returns
    /// `LoadError::ValidationFailed` for invalid data.
    pub fn load(&self) -> Result<LoadResult, LoadError> {
        let load_start = Instant::now();

        // Try to load the identity file.
        let identity = match self.storage.load_identity() {
            Ok(id) => id,
            Err(StorageError::NotFound(_)) => {
                return Err(LoadError::Storage(StorageError::NotFound(
                    "identity file not found".to_string(),
                )));
            }
            Err(e) => return Err(LoadError::Storage(e)),
        };

        // Reject unknown schema versions before attempting migration.
        if let Err(msg) = validate_schema_version(&identity.schema_version) {
            return Err(LoadError::UnknownSchemaVersion(msg));
        }

        // Run migrations if the schema version differs.
        let migration_result = apply_migrations(identity.clone());
        let migrated = migration_result.migrations_applied > 0;
        let identity = if migrated {
            migration_result.identity
        } else {
            identity
        };

        // Validate the (possibly migrated) identity.
        let report = validate_identity(&identity);
        if !report.is_valid() {
            let errors: Vec<String> = report
                .issues
                .iter()
                .map(|issue| issue.message.clone())
                .collect();
            return Err(LoadError::ValidationFailed(errors));
        }

        let load_time_us = load_start.elapsed().as_micros() as u64;

        let diagnostics = ProjectIdentityDiagnostics::new(
            if migrated {
                IdentitySource::Migrated
            } else {
                IdentitySource::Loaded
            },
        )
        .with_load_time(load_time_us)
        .with_migration_count(migration_result.migrations_applied)
        .with_schema_version(&identity.schema_version);

        Ok(LoadResult {
            identity,
            diagnostics,
            migrated,
        })
    }

    /// Check whether a persisted identity exists.
    pub fn identity_exists(&self) -> bool {
        self.storage.identity_exists()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (ProjectIdentityLoader, TempDir) {
        let tmp = TempDir::new().expect("temp dir");
        let loader = ProjectIdentityLoader::new(tmp.path());
        (loader, tmp)
    }

    #[test]
    fn test_load_missing_identity() {
        let (loader, _tmp) = setup();
        let result = loader.load();
        assert!(result.is_err());
        match result.unwrap_err() {
            LoadError::Storage(StorageError::NotFound(_)) => {}
            other => panic!("Expected NotFound error, got {:?}", other),
        }
    }

    #[test]
    fn test_load_existing_identity() {
        let (loader, _tmp) = setup();
        let identity = ProjectIdentity::new("loaded-proj", "rust");
        loader.storage().save_identity(&identity).expect("save");
        let result = loader.load().expect("load");
        assert_eq!(result.identity.name, "loaded-proj");
        assert!(!result.migrated);
        assert_eq!(
            result.diagnostics.source,
            IdentitySource::Loaded
        );
    }

    #[test]
    fn test_load_with_old_schema_triggers_migration() {
        let (loader, _tmp) = setup();
        // Simulate an old schema identity.
        let identity = ProjectIdentity::new("old-proj", "go")
            .with_workspace_root("/tmp/old");
        // Manually set an old schema version.
        let old_identity = ProjectIdentity {
            schema_version: "0.9.0".to_string(),
            ..identity
        };
        loader.storage().save_identity(&old_identity).expect("save");
        let result = loader.load().expect("load");
        // After migration, schema version should be current.
        assert_eq!(
            result.identity.schema_version,
            super::super::identity::CURRENT_SCHEMA_VERSION
        );
        assert!(result.migrated);
        assert_eq!(
            result.diagnostics.source,
            IdentitySource::Migrated
        );
    }

    #[test]
    fn test_load_unknown_schema_version_rejected() {
        let (loader, _tmp) = setup();
        let identity = ProjectIdentity::new("unknown-proj", "rust");
        let bad_identity = ProjectIdentity {
            schema_version: "9.9.9".to_string(),
            ..identity
        };
        loader.storage().save_identity(&bad_identity).expect("save");
        let result = loader.load();
        assert!(result.is_err());
        match result.unwrap_err() {
            LoadError::UnknownSchemaVersion(_) => {}
            other => panic!("Expected UnknownSchemaVersion error, got {:?}", other),
        }
    }

    #[test]
    fn test_load_invalid_identity_fails_validation() {
        let (loader, _tmp) = setup();
        let mut identity = ProjectIdentity::new("invalid-proj", "rust");
        identity.name = String::new(); // empty name is invalid
        loader.storage().save_identity(&identity).expect("save");
        let result = loader.load();
        assert!(result.is_err());
        match result.unwrap_err() {
            LoadError::ValidationFailed(_) => {}
            other => panic!("Expected ValidationFailed error, got {:?}", other),
        }
    }

    #[test]
    fn test_identity_exists_false_when_missing() {
        let (loader, _tmp) = setup();
        assert!(!loader.identity_exists());
    }

    #[test]
    fn test_identity_exists_true_when_present() {
        let (loader, _tmp) = setup();
        let identity = ProjectIdentity::new("exists-proj", "rust");
        loader.storage().save_identity(&identity).expect("save");
        assert!(loader.identity_exists());
    }
}

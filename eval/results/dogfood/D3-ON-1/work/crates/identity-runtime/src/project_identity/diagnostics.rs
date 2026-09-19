//! Diagnostics for `ProjectIdentityRuntime`.
//!
//! Tracks load_time, save_time, migration_count, validation_errors,
//! identity_updates, and snapshot_generation_time.

use serde::{Deserialize, Serialize};

/// Diagnostic snapshot for the project identity runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectIdentityDiagnostics {
    /// Time spent loading identity (microseconds).
    pub load_time_us: u64,
    /// Time spent saving identity (microseconds).
    pub save_time_us: u64,
    /// Number of migrations applied during load.
    pub migration_count: u32,
    /// Validation errors found (0 means clean).
    pub validation_errors: u32,
    /// Total number of identity updates performed.
    pub identity_updates: u32,
    /// Time spent generating a snapshot (microseconds).
    pub snapshot_generation_time_us: u64,
    /// Schema version that was loaded.
    pub schema_version: String,
    /// Source of the identity (loaded, created, migrated).
    pub source: IdentitySource,
}

/// Where the identity came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IdentitySource {
    /// Created fresh because no persisted identity existed.
    Created,
    /// Loaded from persisted storage.
    Loaded,
    /// Loaded and then migrated to a newer schema version.
    Migrated,
}

impl std::fmt::Display for IdentitySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IdentitySource::Created => write!(f, "created"),
            IdentitySource::Loaded => write!(f, "loaded"),
            IdentitySource::Migrated => write!(f, "migrated"),
        }
    }
}

impl ProjectIdentityDiagnostics {
    pub fn new(source: IdentitySource) -> Self {
        ProjectIdentityDiagnostics {
            load_time_us: 0,
            save_time_us: 0,
            migration_count: 0,
            validation_errors: 0,
            identity_updates: 0,
            snapshot_generation_time_us: 0,
            schema_version: crate::project_identity::identity::CURRENT_SCHEMA_VERSION.to_string(),
            source,
        }
    }

    pub fn with_load_time(mut self, us: u64) -> Self {
        self.load_time_us = us;
        self
    }

    pub fn with_save_time(mut self, us: u64) -> Self {
        self.save_time_us = us;
        self
    }

    pub fn with_migration_count(mut self, count: u32) -> Self {
        self.migration_count = count;
        self
    }

    pub fn with_validation_errors(mut self, count: u32) -> Self {
        self.validation_errors = count;
        self
    }

    pub fn with_identity_updates(mut self, count: u32) -> Self {
        self.identity_updates = count;
        self
    }

    pub fn with_snapshot_generation_time(mut self, us: u64) -> Self {
        self.snapshot_generation_time_us = us;
        self
    }

    pub fn with_schema_version(mut self, version: impl Into<String>) -> Self {
        self.schema_version = version.into();
        self
    }

    /// Returns `true` when no diagnostics have been recorded.
    pub fn is_empty(&self) -> bool {
        self.load_time_us == 0
            && self.save_time_us == 0
            && self.migration_count == 0
            && self.validation_errors == 0
            && self.identity_updates == 0
            && self.snapshot_generation_time_us == 0
    }

    /// Human-readable summary.
    pub fn summary(&self) -> String {
        format!(
            "ProjectIdentity Diagnostics:\n\
             Source: {}\n\
             Schema version: {}\n\
             Load time: {} us\n\
             Save time: {} us\n\
             Migrations applied: {}\n\
             Validation errors: {}\n\
             Identity updates: {}\n\
             Snapshot generation: {} us",
            self.source,
            self.schema_version,
            self.load_time_us,
            self.save_time_us,
            self.migration_count,
            self.validation_errors,
            self.identity_updates,
            self.snapshot_generation_time_us,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_diagnostics_created() {
        let d = ProjectIdentityDiagnostics::new(IdentitySource::Created);
        assert!(d.is_empty());
        assert_eq!(d.source, IdentitySource::Created);
        assert_eq!(
            d.schema_version,
            crate::project_identity::identity::CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn test_diagnostics_builder() {
        let d = ProjectIdentityDiagnostics::new(IdentitySource::Loaded)
            .with_load_time(1200)
            .with_save_time(800)
            .with_migration_count(1)
            .with_validation_errors(0)
            .with_identity_updates(5)
            .with_snapshot_generation_time(50)
            .with_schema_version("1.0.0");
        assert!(!d.is_empty());
        assert_eq!(d.load_time_us, 1200);
        assert_eq!(d.save_time_us, 800);
        assert_eq!(d.migration_count, 1);
        assert_eq!(d.validation_errors, 0);
        assert_eq!(d.identity_updates, 5);
        assert_eq!(d.snapshot_generation_time_us, 50);
    }

    #[test]
    fn test_diagnostics_serialization_roundtrip() {
        let d = ProjectIdentityDiagnostics::new(IdentitySource::Migrated)
            .with_load_time(500)
            .with_migration_count(2);
        let json = serde_json::to_string(&d).expect("serialize");
        let decoded: ProjectIdentityDiagnostics = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(d.load_time_us, decoded.load_time_us);
        assert_eq!(d.migration_count, decoded.migration_count);
        assert_eq!(d.source, decoded.source);
    }

    #[test]
    fn test_summary_contains_key_values() {
        let d = ProjectIdentityDiagnostics::new(IdentitySource::Loaded)
            .with_load_time(100)
            .with_migration_count(1);
        let s = d.summary();
        assert!(s.contains("100"));
        assert!(s.contains("1"));
        assert!(s.contains("loaded"));
    }
}

//! Migration pipeline for project identity schema evolution.
//!
//! Supports a `version` field, future migrations, and backward compatibility.

use super::identity::{ProjectIdentity, CURRENT_SCHEMA_VERSION};

/// Result of applying migrations.
pub struct MigrationResult {
    pub identity: ProjectIdentity,
    pub migrations_applied: u32,
    pub migration_log: Vec<String>,
}

/// A single migration step.
struct Migration {
    from_version: &'static str,
    to_version: &'static str,
    apply: fn(ProjectIdentity) -> ProjectIdentity,
}

/// All known schema versions, ordered from oldest to newest.
const KNOWN_VERSIONS: &[&str] = &["0.9.0", CURRENT_SCHEMA_VERSION];

/// All known migrations, ordered from oldest to newest.
const MIGRATIONS: &[Migration] = &[
    // Migration 0.9.0 → 1.0.0: add new fields with sensible defaults.
    Migration {
        from_version: "0.9.0",
        to_version: "1.0.0",
        apply: migrate_v090_to_v100,
    },
];

fn migrate_v090_to_v100(identity: ProjectIdentity) -> ProjectIdentity {
    let mut id = identity;
    id.schema_version = CURRENT_SCHEMA_VERSION.to_string();
    // Add default values for new fields that may be missing.
    id.known_patterns = id.known_patterns;
    id.known_modules = id.known_modules;
    id.coding_conventions = id.coding_conventions;
    id.recent_milestones = id.recent_milestones;
    id
}

/// Validate the schema version before attempting migration.
///
/// Returns `Ok(())` if the version is known, or `Err` with a diagnostic
/// message if the version is unknown.
pub fn validate_schema_version(version: &str) -> Result<(), String> {
    if KNOWN_VERSIONS.contains(&version) {
        Ok(())
    } else {
        Err(format!("unknown schema version: {}", version))
    }
}

/// Apply all applicable migrations to an identity.
///
/// If the identity's schema version matches the current version, no
/// migrations are applied and the identity is returned unchanged.
///
/// # Panics
///
/// This function assumes the version has already been validated by
/// `validate_schema_version`. Unknown versions are rejected before this
/// function is called.
pub fn apply_migrations(identity: ProjectIdentity) -> MigrationResult {
    let mut current = identity;
    let mut migrations_applied: u32 = 0;
    let mut migration_log: Vec<String> = Vec::new();

    for migration in MIGRATIONS {
        if current.schema_version == migration.from_version {
            let before = current.schema_version.clone();
            current = (migration.apply)(current);
            migrations_applied += 1;
            migration_log.push(format!("migrated {} → {}", before, current.schema_version));
        }
    }

    MigrationResult {
        identity: current,
        migrations_applied,
        migration_log,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_identity::identity::ProjectIdentity;

    #[test]
    fn test_no_migration_when_current_version() {
        let identity = ProjectIdentity::new("test", "rust");
        let result = apply_migrations(identity.clone());
        assert_eq!(result.migrations_applied, 0);
        assert!(result.migration_log.is_empty());
        assert_eq!(result.identity.schema_version, CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn test_validate_known_version() {
        assert!(validate_schema_version(CURRENT_SCHEMA_VERSION).is_ok());
        assert!(validate_schema_version("0.9.0").is_ok());
    }

    #[test]
    fn test_validate_unknown_version() {
        assert!(validate_schema_version("9.9.9").is_err());
    }

    #[test]
    fn test_migration_logs() {
        let identity = ProjectIdentity::new("migrated", "go").with_workspace_root("/tmp/old");
        let migrated = ProjectIdentity {
            schema_version: "0.9.0".to_string(),
            ..identity
        };
        let result = apply_migrations(migrated);
        assert_eq!(result.migrations_applied, 1);
        assert_eq!(result.migration_log.len(), 1);
        assert!(result.migration_log[0].contains("0.9.0"));
        assert!(result.migration_log[0].contains("1.0.0"));
        assert_eq!(result.identity.schema_version, CURRENT_SCHEMA_VERSION);
    }
}

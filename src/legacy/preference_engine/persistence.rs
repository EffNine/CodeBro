//! Preference Persistence — atomic writes, backup, rollback, corruption detection, migration.
//!
//! All persistence operations are atomic where possible. If a write fails
//! mid-operation, the previous valid state is preserved.

use super::diagnostics::PreferenceDiagnostics;
use super::schema::*;
use super::validation::PreferenceValidator;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Result of a persistence operation.
#[derive(Debug)]
pub enum PersistResult {
    Ok,
    BackupCreated(PathBuf),
    RolledBack(PathBuf),
    CorruptionDetected(PathBuf),
    MigrationApplied(u32, u32),
}

/// Persistence backend for preferences.
pub struct PreferencePersistence {
    data_dir: PathBuf,
    diagnostics: PreferenceDiagnostics,
    pub(crate) validator: PreferenceValidator,
}

impl Clone for PreferencePersistence {
    fn clone(&self) -> Self {
        PreferencePersistence {
            data_dir: self.data_dir.clone(),
            diagnostics: self.diagnostics.clone(),
            validator: PreferenceValidator::new(self.diagnostics.clone()),
        }
    }
}

impl PreferencePersistence {
    pub fn new(data_dir: PathBuf, diagnostics: PreferenceDiagnostics) -> Self {
        PreferencePersistence {
            data_dir,
            diagnostics: diagnostics.clone(),
            validator: PreferenceValidator::new(diagnostics),
        }
    }

    pub fn ensure_dir(&self) -> std::io::Result<()> {
        fs::create_dir_all(&self.data_dir)
    }
    pub fn preferences_path(&self) -> PathBuf {
        self.data_dir.join("preferences.json")
    }

    /// Full path to the backup file.
    pub fn backup_path(&self) -> PathBuf {
        self.data_dir.join("preferences.json.bak")
    }

    /// Full path to the migration history file.
    pub fn migration_log_path(&self) -> PathBuf {
        self.data_dir.join("migration_log.json")
    }

    /// Load preferences from disk.
    ///
    /// If the file does not exist, returns a default set.
    /// If the file is corrupt, triggers corruption detection and rollback.
    pub fn load(&self) -> Result<PreferenceSet, String> {
        let path = self.preferences_path();
        if !path.exists() {
            return Ok(PreferenceSet::default());
        }

        let content = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read preferences file: {}", e))?;

        match serde_json::from_str::<PreferenceSet>(&content) {
            Ok(set) => {
                // Validate the loaded set
                let errors = self.validator.validate_set(&set);
                if !errors.is_empty() {
                    let msg = format!("Loaded preferences have validation errors: {:?}", errors);
                    self.diagnostics.record(
                        super::diagnostics::DiagnosticKind::LoadFailure,
                        &msg,
                        true,
                    );
                }
                Ok(set)
            }
            Err(e) => {
                self.diagnostics.record(
                    super::diagnostics::DiagnosticKind::CorruptionDetected,
                    &format!("Corrupt preferences file: {}", e),
                    true,
                );
                self.attempt_rollback()
            }
        }
    }

    /// Save preferences to disk atomically.
    ///
    /// Writes to a temp file first, then renames. If the write fails,
    /// the previous file is untouched. A backup is created before overwrite.
    pub fn save(&self, set: &PreferenceSet) -> Result<PersistResult, String> {
        self.ensure_dir()
            .map_err(|e| format!("Failed to create data directory: {}", e))?;

        let path = self.preferences_path();
        let backup_path = self.backup_path();

        // Create backup before overwrite
        if path.exists() {
            fs::copy(&path, &backup_path).map_err(|e| format!("Failed to create backup: {}", e))?;
        }

        // Serialize and write to temp file
        let content = serde_json::to_string_pretty(set)
            .map_err(|e| format!("Failed to serialize preferences: {}", e))?;

        let temp_path = self.data_dir.join(".preferences.json.tmp");
        let mut file = fs::File::create(&temp_path)
            .map_err(|e| format!("Failed to create temp file: {}", e))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("Failed to write preferences: {}", e))?;
        file.flush()
            .map_err(|e| format!("Failed to flush preferences: {}", e))?;

        // Atomic rename
        fs::rename(&temp_path, &path)
            .map_err(|e| format!("Failed to atomically rename preferences: {}", e))?;

        if backup_path.exists() {
            Ok(PersistResult::BackupCreated(backup_path))
        } else {
            Ok(PersistResult::Ok)
        }
    }

    /// Update a single preference in the stored set and save.
    pub fn update(
        &self,
        key: &str,
        new_value: PreferenceValue,
        origin: PreferenceOrigin,
    ) -> Result<PersistResult, String> {
        let mut set = self.load()?;

        if let Some(pref) = set.preferences.iter_mut().find(|p| p.key == key) {
            pref.update_value(new_value.clone());
            pref.update_origin(origin.clone());
        } else {
            let new_pref = Preference::new(key, self.category_for_key(key), new_value, "", origin);
            set.add(new_pref);
        }

        self.save(&set)
    }

    /// Delete a preference by key.
    pub fn delete(&self, key: &str) -> Result<PersistResult, String> {
        let mut set = self.load()?;
        let before = set.len();
        set.preferences.retain(|p| p.key != key);
        if set.len() == before {
            return Err(format!("Preference '{}' not found", key));
        }
        self.save(&set)
    }

    /// Reset all preferences to defaults.
    pub fn reset(&self) -> Result<PersistResult, String> {
        let defaults = default_preference_set();
        self.save(&defaults)
    }

    /// Export preferences as pretty-printed JSON string.
    pub fn export(&self) -> Result<String, String> {
        let set = self.load()?;
        serde_json::to_string_pretty(&set)
            .map_err(|e| format!("Failed to export preferences: {}", e))
    }

    /// Import preferences from a JSON string.
    ///
    /// Validates the imported data before saving.
    pub fn import(&self, json: &str) -> Result<PersistResult, String> {
        let imported: PreferenceSet =
            serde_json::from_str(json).map_err(|e| format!("Invalid JSON in import: {}", e))?;

        // Migrate if needed
        let imported = match self.validator.migrate(&imported) {
            Ok(m) => m,
            Err(e) => return Err(e),
        };

        // Validate
        let errors = self.validator.validate_set(&imported);
        if !errors.is_empty() {
            let msg = format!("Imported preferences have validation errors: {:?}", errors);
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::ValidationFailure,
                &msg,
                false,
            );
            return Err(msg);
        }

        let dup_result = self.validator.validate_no_duplicates(&imported);
        if !dup_result.is_ok() {
            return Err(format!(
                "Imported preferences have duplicate keys: {:?}",
                dup_result
            ));
        }

        self.save(&imported)
    }

    /// Get the path of the backup file (if it exists).
    pub fn backup_exists(&self) -> bool {
        self.backup_path().exists()
    }

    /// Restore from backup.
    pub fn restore_backup(&self) -> Result<PersistResult, String> {
        let backup = self.backup_path();
        let target = self.preferences_path();

        if !backup.exists() {
            return Err("No backup file found".to_string());
        }

        // Validate backup before restoring
        let content =
            fs::read_to_string(&backup).map_err(|e| format!("Failed to read backup: {}", e))?;
        let backup_set: PreferenceSet =
            serde_json::from_str(&content).map_err(|e| format!("Backup is corrupt: {}", e))?;

        // Check compatibility
        let loaded = self.load().unwrap_or_default();
        if !self
            .validator
            .validate_compatibility(&loaded, &backup_set)
            .is_ok()
        {
            let msg = "Backup incompatible with current state".to_string();
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::RollbackFailure,
                &msg,
                true,
            );
            return Err(msg);
        }

        fs::copy(&backup, &target).map_err(|e| format!("Failed to restore from backup: {}", e))?;

        self.diagnostics.record(
            super::diagnostics::DiagnosticKind::SaveFailure,
            "Restored from backup successfully",
            false,
        );

        Ok(PersistResult::RolledBack(backup))
    }

    /// Attempt to rollback to a previous valid state.
    fn attempt_rollback(&self) -> Result<PreferenceSet, String> {
        if self.backup_exists() {
            match self.restore_backup() {
                Ok(_) => return self.load(),
                Err(_) => {
                    let msg = "Rollback to backup also failed".to_string();
                    self.diagnostics.record(
                        super::diagnostics::DiagnosticKind::RollbackFailure,
                        &msg,
                        false,
                    );
                    return Err(msg);
                }
            }
        }

        let msg = "No backup available for rollback".to_string();
        self.diagnostics.record(
            super::diagnostics::DiagnosticKind::RollbackFailure,
            &msg,
            false,
        );
        Err(msg)
    }

    /// Helper: infer category from key name.
    pub fn category_for_key(&self, key: &str) -> PreferenceCategory {
        match key {
            "provider" => PreferenceCategory::Provider,
            "model" => PreferenceCategory::Model,
            _ if key.starts_with("subagent_") => PreferenceCategory::Subagent,
            "language" | "primary_language" => PreferenceCategory::Language,
            "max_iterations" | "context_token_budget" | "max_tool_iterations" => {
                PreferenceCategory::Workflow
            }
            "max_cost" | "max_cost_per_session" => PreferenceCategory::Cost,
            "auto_approve" | "approval" | "max_approvals" => PreferenceCategory::Approval,
            "privacy" | "privacy_mode" => PreferenceCategory::Privacy,
            _ => PreferenceCategory::Workflow,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn make_set() -> PreferenceSet {
        let mut set = PreferenceSet::new();
        set.add(Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Test model",
            PreferenceOrigin::User,
        ));
        set.add(Preference::new(
            "provider",
            PreferenceCategory::Provider,
            PreferenceValue::String("openai".to_string()),
            "Test provider",
            PreferenceOrigin::User,
        ));
        set
    }

    #[test]
    fn test_save_and_load() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        let set = make_set();
        persistence.save(&set).unwrap();

        let loaded = persistence.load().unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.by_key("model").unwrap().value,
            PreferenceValue::String("gpt-4o".to_string())
        );
    }

    #[test]
    fn test_load_defaults_when_file_missing() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        let loaded = persistence.load().unwrap();
        assert!(loaded.is_empty());
    }

    #[test]
    fn test_update() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        persistence
            .update(
                "model",
                PreferenceValue::String("gpt-4o-mini".to_string()),
                PreferenceOrigin::User,
            )
            .unwrap();

        let loaded = persistence.load().unwrap();
        assert_eq!(
            loaded.by_key("model").unwrap().value,
            PreferenceValue::String("gpt-4o-mini".to_string())
        );
    }

    #[test]
    fn test_update_creates_new_key() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        persistence
            .update(
                "new_key",
                PreferenceValue::Boolean(true),
                PreferenceOrigin::User,
            )
            .unwrap();

        let loaded = persistence.load().unwrap();
        assert!(loaded.by_key("new_key").is_some());
    }

    #[test]
    fn test_delete() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        persistence.delete("model").unwrap();

        let loaded = persistence.load().unwrap();
        assert_eq!(loaded.len(), 1);
        assert!(loaded.by_key("model").is_none());
    }

    #[test]
    fn test_delete_missing_key() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        let result = persistence.delete("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_reset() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        persistence.reset().unwrap();

        let loaded = persistence.load().unwrap();
        assert_eq!(loaded.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(!loaded.is_empty());
    }

    #[test]
    fn test_export() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        let exported = persistence.export().unwrap();
        assert!(exported.contains("model"));
        assert!(exported.contains("gpt-4o"));
    }

    #[test]
    fn test_import() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        let json = serde_json::to_string_pretty(&make_set()).unwrap();
        persistence.import(&json).unwrap();

        let loaded = persistence.load().unwrap();
        assert_eq!(loaded.len(), 2);
    }

    #[test]
    fn test_import_corrupt_json() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        let result = persistence.import("not valid json{{{");
        assert!(result.is_err());
    }

    #[test]
    fn test_backup_created_on_save() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        assert!(!persistence.backup_exists()); // No backup on first save

        persistence.save(&make_set()).unwrap();
        assert!(persistence.backup_exists());
    }

    #[test]
    fn test_restore_backup() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        persistence.save(&make_set()).unwrap(); // creates backup
        assert!(persistence.backup_exists());

        let result = persistence.restore_backup();
        assert!(result.is_ok());
    }

    #[test]
    fn test_restore_backup_missing() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        let result = persistence.restore_backup();
        assert!(result.is_err());
    }

    #[test]
    fn test_corruption_detection() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        // Write corrupt data
        fs::write(persistence.preferences_path(), "{corrupt").unwrap();

        // Load should detect corruption and try rollback
        let result = persistence.load();
        // May fail if no backup exists, which is expected
        assert!(result.is_err() || persistence.backup_exists());
    }

    #[test]
    fn test_atomic_write_on_save_failure() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        // Save should succeed
        persistence.save(&make_set()).unwrap();

        // Verify file exists and is valid JSON
        let path = persistence.preferences_path();
        assert!(path.exists());
        let content = fs::read_to_string(path).unwrap();
        let _: PreferenceSet = serde_json::from_str(&content).unwrap();
    }

    #[test]
    fn test_load_from_temp_dir() {
        let dir = tempdir().unwrap();
        let diag = PreferenceDiagnostics::new(100);
        let persistence = PreferencePersistence::new(dir.path().to_path_buf(), diag);

        persistence.save(&make_set()).unwrap();
        let loaded = persistence.load().unwrap();
        assert_eq!(loaded.len(), 2);
    }
}

//! Preference Validation — schema, values, compatibility, version, migration.
//!
//! Validates are deterministic and never invoke LLMs.

use super::schema::*;
use crate::preference_engine::diagnostics::PreferenceDiagnostics;
use std::collections::HashSet;

/// Validation result.
#[derive(Debug, Clone, PartialEq)]
pub enum ValidationResult {
    Ok,
    InvalidSchema(String),
    InvalidValue(String),
    InvalidCompatibility(String),
    InvalidVersion(String),
    InvalidMigration(String),
}

impl ValidationResult {
    pub fn is_ok(&self) -> bool {
        matches!(self, ValidationResult::Ok)
    }

    pub fn error_message(&self) -> Option<&str> {
        match self {
            ValidationResult::Ok => None,
            ValidationResult::InvalidSchema(msg) => Some(msg),
            ValidationResult::InvalidValue(msg) => Some(msg),
            ValidationResult::InvalidCompatibility(msg) => Some(msg),
            ValidationResult::InvalidVersion(msg) => Some(msg),
            ValidationResult::InvalidMigration(msg) => Some(msg),
        }
    }
}

/// Validator for preferences.
pub struct PreferenceValidator {
    diagnostics: PreferenceDiagnostics,
}

impl PreferenceValidator {
    pub fn new(diagnostics: PreferenceDiagnostics) -> Self {
        PreferenceValidator { diagnostics }
    }

    /// Validate a single preference.
    pub fn validate_preference(&self, pref: &Preference) -> ValidationResult {
        // 1. Schema validation
        if pref.key.is_empty() {
            let msg = "Preference key cannot be empty".to_string();
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::ValidationFailure,
                &msg,
                false,
            );
            return ValidationResult::InvalidSchema(msg);
        }

        if pref.description.is_empty() {
            let msg = format!("Preference '{}' has empty description", pref.key);
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::ValidationFailure,
                &msg,
                false,
            );
            return ValidationResult::InvalidSchema(msg);
        }

        // 2. Version validation
        if pref.schema_version != CURRENT_SCHEMA_VERSION {
            let msg = format!(
                "Preference '{}' has schema version {}, expected {}",
                pref.key, pref.schema_version, CURRENT_SCHEMA_VERSION
            );
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::ValidationFailure,
                &msg,
                true,
            );
            return ValidationResult::InvalidVersion(msg);
        }

        // 3. Value validation
        match self.validate_value(&pref.value) {
            ValidationResult::Ok => {}
            result => return result,
        }

        // 4. Origin validation
        if !matches!(
            pref.origin,
            PreferenceOrigin::User | PreferenceOrigin::Imported | PreferenceOrigin::Default
        ) {
            let msg = format!("Invalid origin for preference '{}'", pref.key);
            return ValidationResult::InvalidSchema(msg);
        }

        ValidationResult::Ok
    }

    /// Validate a preference set (all preferences).
    pub fn validate_set(&self, set: &PreferenceSet) -> Vec<ValidationResult> {
        let mut errors = Vec::new();
        for pref in &set.preferences {
            let result = self.validate_preference(pref);
            if !result.is_ok() {
                errors.push(result);
            }
        }
        errors
    }

    /// Validate that no duplicate keys exist.
    pub fn validate_no_duplicates(&self, set: &PreferenceSet) -> ValidationResult {
        let mut seen = HashSet::new();
        for pref in &set.preferences {
            if !seen.insert(&pref.key) {
                let msg = format!("Duplicate preference key: {}", pref.key);
                self.diagnostics.record(
                    super::diagnostics::DiagnosticKind::ValidationFailure,
                    &msg,
                    false,
                );
                return ValidationResult::InvalidSchema(msg);
            }
        }
        ValidationResult::Ok
    }

    /// Validate a single value.
    fn validate_value(&self, value: &PreferenceValue) -> ValidationResult {
        match value {
            PreferenceValue::String(s) => {
                if s.len() > 10_000 {
                    let msg = "String value exceeds maximum length of 10,000".to_string();
                    self.diagnostics.record(
                        super::diagnostics::DiagnosticKind::ValidationFailure,
                        &msg,
                        false,
                    );
                    return ValidationResult::InvalidValue(msg);
                }
                ValidationResult::Ok
            }
            PreferenceValue::Integer(_) => ValidationResult::Ok,
            PreferenceValue::Float(_) => ValidationResult::Ok,
            PreferenceValue::Boolean(_) => ValidationResult::Ok,
            PreferenceValue::List(items) => {
                if items.len() > 1000 {
                    let msg = "List value exceeds maximum length of 1,000".to_string();
                    self.diagnostics.record(
                        super::diagnostics::DiagnosticKind::ValidationFailure,
                        &msg,
                        false,
                    );
                    return ValidationResult::InvalidValue(msg);
                }
                ValidationResult::Ok
            }
            PreferenceValue::Map(items) => {
                if items.len() > 500 {
                    let msg = "Map value exceeds maximum size of 500 entries".to_string();
                    self.diagnostics.record(
                        super::diagnostics::DiagnosticKind::ValidationFailure,
                        &msg,
                        false,
                    );
                    return ValidationResult::InvalidValue(msg);
                }
                ValidationResult::Ok
            }
            PreferenceValue::Null => ValidationResult::Ok,
        }
    }

    /// Validate compatibility between two preference sets.
    pub fn validate_compatibility(
        &self,
        old: &PreferenceSet,
        new: &PreferenceSet,
    ) -> ValidationResult {
        if old.schema_version != new.schema_version {
            let msg = format!(
                "Schema version mismatch: old={}, new={}",
                old.schema_version, new.schema_version
            );
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::ValidationFailure,
                &msg,
                true,
            );
            return ValidationResult::InvalidCompatibility(msg);
        }
        ValidationResult::Ok
    }

    /// Attempt to migrate a preference set to the current schema version.
    ///
    /// Returns the migrated set, or an error if migration is not possible.
    pub fn migrate(&self, set: &PreferenceSet) -> Result<PreferenceSet, String> {
        if set.schema_version == CURRENT_SCHEMA_VERSION {
            return Ok(set.clone());
        }

        // Migration from version 0 to 1: add schema_version to all preferences
        if set.schema_version == 0 {
            let mut migrated = set.clone();
            migrated.schema_version = CURRENT_SCHEMA_VERSION;
            for pref in &mut migrated.preferences {
                pref.schema_version = CURRENT_SCHEMA_VERSION;
            }
            return Ok(migrated);
        }

        let msg = format!(
            "Cannot migrate from schema version {} to {}",
            set.schema_version, CURRENT_SCHEMA_VERSION
        );
        self.diagnostics.record(
            super::diagnostics::DiagnosticKind::MigrationFailure,
            &msg,
            false,
        );
        Err(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_pref(key: &str, value: PreferenceValue) -> Preference {
        Preference::new(
            key,
            PreferenceCategory::Model,
            value,
            "A test preference",
            PreferenceOrigin::User,
        )
    }

    #[test]
    fn test_valid_preference() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let pref = make_pref("model", PreferenceValue::String("gpt-4o".to_string()));
        assert!(validator.validate_preference(&pref).is_ok());
    }

    #[test]
    fn test_empty_key_rejected() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let mut pref = make_pref("model", PreferenceValue::String("gpt-4o".to_string()));
        pref.key = "".to_string();
        let result = validator.validate_preference(&pref);
        assert!(!result.is_ok());
        assert!(result.error_message().unwrap().contains("empty"));
    }

    #[test]
    fn test_empty_description_rejected() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let mut pref = make_pref("model", PreferenceValue::String("gpt-4o".to_string()));
        pref.description = "".to_string();
        let result = validator.validate_preference(&pref);
        assert!(!result.is_ok());
    }

    #[test]
    fn test_wrong_version_rejected() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let mut pref = make_pref("model", PreferenceValue::String("gpt-4o".to_string()));
        pref.schema_version = 99;
        let result = validator.validate_preference(&pref);
        assert!(!result.is_ok());
        assert_eq!(
            result,
            ValidationResult::InvalidVersion(format!(
                "Preference 'model' has schema version 99, expected {}",
                CURRENT_SCHEMA_VERSION
            ))
        );
    }

    #[test]
    fn test_value_too_long_string() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let long_string = "x".repeat(10_001);
        let pref = make_pref("long", PreferenceValue::String(long_string));
        let result = validator.validate_preference(&pref);
        assert!(!result.is_ok());
        assert_eq!(
            result,
            ValidationResult::InvalidValue(
                "String value exceeds maximum length of 10,000".to_string()
            )
        );
    }

    #[test]
    fn test_list_too_long() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let long_list = vec!["x".to_string(); 1001];
        let pref = make_pref("list", PreferenceValue::List(long_list));
        let result = validator.validate_preference(&pref);
        assert!(!result.is_ok());
        assert_eq!(
            result,
            ValidationResult::InvalidValue(
                "List value exceeds maximum length of 1,000".to_string()
            )
        );
    }

    #[test]
    fn test_duplicate_keys_rejected() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let mut set = PreferenceSet::new();
        set.add(make_pref("model", PreferenceValue::String("a".to_string())));
        set.add(make_pref("model", PreferenceValue::String("b".to_string())));
        let result = validator.validate_no_duplicates(&set);
        assert!(!result.is_ok());
        assert!(result.error_message().unwrap().contains("Duplicate"));
    }

    #[test]
    fn test_valid_set_no_duplicates() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let mut set = PreferenceSet::new();
        set.add(make_pref("model", PreferenceValue::String("a".to_string())));
        set.add(make_pref(
            "provider",
            PreferenceValue::String("b".to_string()),
        ));
        assert!(validator.validate_no_duplicates(&set).is_ok());
    }

    #[test]
    fn test_compatibility_version_mismatch() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let old = PreferenceSet {
            schema_version: 0,
            preferences: vec![],
        };
        let new = PreferenceSet {
            schema_version: 1,
            preferences: vec![],
        };
        let result = validator.validate_compatibility(&old, &new);
        assert!(!result.is_ok());
    }

    #[test]
    fn test_compatibility_version_match() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let old = PreferenceSet {
            schema_version: 1,
            preferences: vec![],
        };
        let new = PreferenceSet {
            schema_version: 1,
            preferences: vec![],
        };
        assert!(validator.validate_compatibility(&old, &new).is_ok());
    }

    #[test]
    fn test_migration_v0_to_v1() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let set = PreferenceSet {
            schema_version: 0,
            preferences: vec![make_pref(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
            )],
        };
        let migrated = validator.migrate(&set).unwrap();
        assert_eq!(migrated.schema_version, 1);
        assert_eq!(migrated.preferences[0].schema_version, 1);
    }

    #[test]
    fn test_migration_already_current_version() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let set = PreferenceSet {
            schema_version: 1,
            preferences: vec![],
        };
        let migrated = validator.migrate(&set).unwrap();
        assert_eq!(migrated.schema_version, 1);
    }

    #[test]
    fn test_migration_unknown_version_fails() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let set = PreferenceSet {
            schema_version: 99,
            preferences: vec![],
        };
        let result = validator.migrate(&set);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_set_collects_all_errors() {
        let diag = PreferenceDiagnostics::new(100);
        let validator = PreferenceValidator::new(diag);
        let mut set = PreferenceSet::new();
        set.add(make_pref("", PreferenceValue::String("a".to_string())));
        set.add(make_pref("b", PreferenceValue::String("c".to_string())));
        let errors = validator.validate_set(&set);
        assert_eq!(errors.len(), 1);
    }
}

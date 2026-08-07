//! Preference Schema — strongly typed preference model.
//!
//! Every preference belongs to a category and carries a version, timestamp,
//! and origin (so we know who changed it and when).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;
use uuid::Uuid;

/// Current schema version. Increment when the model changes incompatibly.
pub const CURRENT_SCHEMA_VERSION: u32 = 1;

/// Unique identifier for a preference.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PreferenceId(pub String);

impl fmt::Display for PreferenceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl PreferenceId {
    pub fn new() -> Self {
        PreferenceId(Uuid::new_v4().to_string())
    }
}

impl Default for PreferenceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Category that a preference belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PreferenceCategory {
    Provider,
    Model,
    Subagent,
    Language,
    Workflow,
    Cost,
    Approval,
    Privacy,
}

impl fmt::Display for PreferenceCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreferenceCategory::Provider => write!(f, "provider"),
            PreferenceCategory::Model => write!(f, "model"),
            PreferenceCategory::Subagent => write!(f, "subagent"),
            PreferenceCategory::Language => write!(f, "language"),
            PreferenceCategory::Workflow => write!(f, "workflow"),
            PreferenceCategory::Cost => write!(f, "cost"),
            PreferenceCategory::Approval => write!(f, "approval"),
            PreferenceCategory::Privacy => write!(f, "privacy"),
        }
    }
}

/// The concrete value stored in a preference.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PreferenceValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    List(Vec<String>),
    Map(HashMap<String, String>),
    Null,
}

impl fmt::Display for PreferenceValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreferenceValue::String(s) => write!(f, "{}", s),
            PreferenceValue::Integer(i) => write!(f, "{}", i),
            PreferenceValue::Float(fl) => write!(f, "{}", fl),
            PreferenceValue::Boolean(b) => write!(f, "{}", b),
            PreferenceValue::List(items) => write!(f, "[{}]", items.join(", ")),
            PreferenceValue::Map(m) => write!(
                f,
                "{{{}}}",
                m.iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            PreferenceValue::Null => write!(f, "null"),
        }
    }
}

/// Source of a preference change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PreferenceOrigin {
    /// Explicitly set by the user via the API.
    User,
    /// Loaded from an import file.
    Imported,
    /// Reset to defaults.
    Default,
}

impl fmt::Display for PreferenceOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PreferenceOrigin::User => write!(f, "user"),
            PreferenceOrigin::Imported => write!(f, "imported"),
            PreferenceOrigin::Default => write!(f, "default"),
        }
    }
}

/// A single preference entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preference {
    pub id: PreferenceId,
    pub key: String,
    pub category: PreferenceCategory,
    pub value: PreferenceValue,
    pub description: String,
    pub schema_version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub origin: PreferenceOrigin,
}

impl Preference {
    pub fn new(
        key: &str,
        category: PreferenceCategory,
        value: PreferenceValue,
        description: &str,
        origin: PreferenceOrigin,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Preference {
            id: PreferenceId::new(),
            key: key.to_string(),
            category,
            value,
            description: description.to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: now.clone(),
            updated_at: now,
            origin,
        }
    }

    pub fn with_id(
        id: PreferenceId,
        key: &str,
        category: PreferenceCategory,
        value: PreferenceValue,
        description: &str,
        origin: PreferenceOrigin,
    ) -> Self {
        let now = chrono::Utc::now().to_rfc3339();
        Preference {
            id,
            key: key.to_string(),
            category,
            value,
            description: description.to_string(),
            schema_version: CURRENT_SCHEMA_VERSION,
            created_at: now.clone(),
            updated_at: now,
            origin,
        }
    }

    pub fn update_value(&mut self, new_value: PreferenceValue) {
        self.value = new_value;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }

    pub fn update_origin(&mut self, origin: PreferenceOrigin) {
        self.origin = origin;
        self.updated_at = chrono::Utc::now().to_rfc3339();
    }
}

/// The complete preference store as a single serializable document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PreferenceSet {
    pub schema_version: u32,
    pub preferences: Vec<Preference>,
}

impl PreferenceSet {
    pub fn new() -> Self {
        PreferenceSet {
            schema_version: CURRENT_SCHEMA_VERSION,
            preferences: Vec::new(),
        }
    }

    pub fn add(&mut self, preference: Preference) {
        self.preferences.push(preference);
    }

    pub fn by_key(&self, key: &str) -> Option<&Preference> {
        self.preferences.iter().find(|p| p.key == key)
    }

    pub fn by_category(&self, category: &PreferenceCategory) -> Vec<Preference> {
        self.preferences
            .iter()
            .filter(|p| &p.category == category)
            .cloned()
            .collect()
    }

    pub fn len(&self) -> usize {
        self.preferences.len()
    }

    pub fn is_empty(&self) -> bool {
        self.preferences.is_empty()
    }
}

/// Default preferences for each category.
///
/// These are the baseline values used on reset and first-run import.
pub fn default_preferences() -> Vec<Preference> {
    vec![
        Preference::new(
            "provider",
            PreferenceCategory::Provider,
            PreferenceValue::String("openai".to_string()),
            "Default AI provider",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "Default model",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "max_iterations",
            PreferenceCategory::Workflow,
            PreferenceValue::Integer(10),
            "Maximum tool iterations per task",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "auto_approve_safe_ops",
            PreferenceCategory::Approval,
            PreferenceValue::Boolean(false),
            "Auto-approve safe operations",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "max_cost_per_session",
            PreferenceCategory::Cost,
            PreferenceValue::Float(5.0),
            "Maximum cost per session in USD",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "primary_language",
            PreferenceCategory::Language,
            PreferenceValue::String("en".to_string()),
            "Primary interface language",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "subagent_coding",
            PreferenceCategory::Subagent,
            PreferenceValue::Boolean(true),
            "Enable coding subagent",
            PreferenceOrigin::Default,
        ),
        Preference::new(
            "privacy_mode",
            PreferenceCategory::Privacy,
            PreferenceValue::Boolean(false),
            "Enable privacy mode (no external data)",
            PreferenceOrigin::Default,
        ),
    ]
}

/// Build a default PreferenceSet.
pub fn default_preference_set() -> PreferenceSet {
    let mut set = PreferenceSet::new();
    for pref in default_preferences() {
        set.add(pref);
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preference_creation() {
        let p = Preference::new(
            "test_key",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "A test preference",
            PreferenceOrigin::User,
        );
        assert_eq!(p.key, "test_key");
        assert_eq!(p.category, PreferenceCategory::Model);
        assert_eq!(p.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(p.id.0.len() > 0);
    }

    #[test]
    fn test_preference_update_value() {
        let mut p = Preference::new(
            "key",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "",
            PreferenceOrigin::User,
        );
        let old_updated = p.updated_at.clone();
        std::thread::sleep(std::time::Duration::from_millis(10));
        p.update_value(PreferenceValue::String("gpt-4o-mini".to_string()));
        assert_eq!(p.value, PreferenceValue::String("gpt-4o-mini".to_string()));
        assert!(p.updated_at > old_updated);
    }

    #[test]
    fn test_preference_set_by_key() {
        let mut set = PreferenceSet::new();
        set.add(Preference::new(
            "provider",
            PreferenceCategory::Provider,
            PreferenceValue::String("openai".to_string()),
            "",
            PreferenceOrigin::Default,
        ));
        set.add(Preference::new(
            "model",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "",
            PreferenceOrigin::Default,
        ));

        assert!(set.by_key("provider").is_some());
        assert!(set.by_key("model").is_some());
        assert!(set.by_key("nonexistent").is_none());
    }

    #[test]
    fn test_preference_set_by_category() {
        let mut set = PreferenceSet::new();
        set.add(Preference::new(
            "a",
            PreferenceCategory::Model,
            PreferenceValue::String("x".to_string()),
            "",
            PreferenceOrigin::Default,
        ));
        set.add(Preference::new(
            "b",
            PreferenceCategory::Model,
            PreferenceValue::String("y".to_string()),
            "",
            PreferenceOrigin::Default,
        ));
        set.add(Preference::new(
            "c",
            PreferenceCategory::Provider,
            PreferenceValue::String("z".to_string()),
            "",
            PreferenceOrigin::Default,
        ));

        let models = set.by_category(&PreferenceCategory::Model);
        assert_eq!(models.len(), 2);
        let providers = set.by_category(&PreferenceCategory::Provider);
        assert_eq!(providers.len(), 1);
    }

    #[test]
    fn test_default_preferences() {
        let prefs = default_preferences();
        assert!(!prefs.is_empty());
        let providers: Vec<_> = prefs
            .iter()
            .filter(|p| p.category == PreferenceCategory::Provider)
            .collect();
        assert!(!providers.is_empty());
        let all_have_version = prefs
            .iter()
            .all(|p| p.schema_version == CURRENT_SCHEMA_VERSION);
        assert!(all_have_version);
    }

    #[test]
    fn test_default_preference_set() {
        let set = default_preference_set();
        assert_eq!(set.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(!set.is_empty());
    }

    #[test]
    fn test_preference_display() {
        let p = Preference::new(
            "test",
            PreferenceCategory::Model,
            PreferenceValue::String("gpt-4o".to_string()),
            "desc",
            PreferenceOrigin::User,
        );
        let display = format!("{}", p.value);
        assert_eq!(display, "gpt-4o");
    }

    #[test]
    fn test_preference_origin_display() {
        assert_eq!(format!("{}", PreferenceOrigin::User), "user");
        assert_eq!(format!("{}", PreferenceOrigin::Imported), "imported");
        assert_eq!(format!("{}", PreferenceOrigin::Default), "default");
    }
}

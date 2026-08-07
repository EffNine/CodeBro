//! Preference Store — persistent storage abstraction.
//!
//! Provides load, save, update, delete, reset, export, and import operations
//! backed by JSON files with atomic writes and backup support.

use super::diagnostics::PreferenceDiagnostics;
use super::events::{
    EventLog, EventTimestamp, PreferenceEvent, PreferenceSubscriber, TestSubscriber,
};
use super::persistence::{PersistResult, PreferencePersistence};
use super::schema::*;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// The public Preference Store.
///
/// All external access must go through this type. No other code touches
/// the persistence layer directly.
#[derive(Clone)]
pub struct PreferenceStore {
    persistence: PreferencePersistence,
    diagnostics: PreferenceDiagnostics,
    event_log: Arc<Mutex<EventLog>>,
    subscribers: Arc<Mutex<Vec<Box<dyn PreferenceSubscriber>>>>,
}

impl PreferenceStore {
    /// Create a new PreferenceStore backed by the given directory.
    pub fn new(data_dir: PathBuf) -> Self {
        let diagnostics = PreferenceDiagnostics::new(1000);
        let persistence = PreferencePersistence::new(data_dir, diagnostics.clone());
        PreferenceStore {
            persistence,
            diagnostics,
            event_log: Arc::new(Mutex::new(EventLog::new(1000))),
            subscribers: Arc::new(Mutex::new(Vec::new())),
        }
    }

    // ─── Load ─────────────────────────────────────────────────────────────

    /// Load all preferences from disk.
    pub fn load(&self) -> Result<PreferenceSet, String> {
        match self.persistence.load() {
            Ok(set) => {
                self.fire_event(PreferenceEvent::PreferenceExported {
                    count: set.len(),
                    timestamp: EventTimestamp::now(),
                });
                Ok(set)
            }
            Err(e) => {
                self.diagnostics
                    .record(super::diagnostics::DiagnosticKind::LoadFailure, &e, true);
                Err(e)
            }
        }
    }

    // ─── Save ─────────────────────────────────────────────────────────────

    /// Save a complete PreferenceSet to disk.
    pub fn save(&self, set: &PreferenceSet) -> Result<PersistResult, String> {
        match self.persistence.save(set) {
            Ok(result) => {
                self.fire_event(PreferenceEvent::PreferenceExported {
                    count: set.len(),
                    timestamp: EventTimestamp::now(),
                });
                Ok(result)
            }
            Err(e) => {
                self.diagnostics
                    .record(super::diagnostics::DiagnosticKind::SaveFailure, &e, true);
                Err(e)
            }
        }
    }

    // ─── Update ───────────────────────────────────────────────────────────

    /// Update or create a single preference.
    pub fn update(
        &self,
        key: &str,
        value: PreferenceValue,
        description: &str,
        origin: PreferenceOrigin,
    ) -> Result<PersistResult, String> {
        let mut set = self.load()?;

        if let Some(pref) = set.preferences.iter_mut().find(|p| p.key == key) {
            pref.update_value(value.clone());
            pref.update_origin(origin.clone());
            pref.description = description.to_string();
            self.fire_event(PreferenceEvent::PreferenceUpdated {
                id: pref.id.clone(),
                key: key.to_string(),
                new_value: value.to_string(),
                timestamp: EventTimestamp::now(),
            });
        } else {
            let category = self.infer_category(key);
            let new_pref =
                Preference::new(key, category, value.clone(), description, origin.clone());
            let category_str = new_pref.category.to_string();
            let id = new_pref.id.clone();
            set.add(new_pref);
            self.fire_event(PreferenceEvent::PreferenceCreated {
                id,
                key: key.to_string(),
                category: category_str,
                timestamp: EventTimestamp::now(),
            });
        }

        self.save(&set)
    }

    // ─── Delete ───────────────────────────────────────────────────────────

    /// Delete a preference by key.
    pub fn delete(&self, key: &str) -> Result<PersistResult, String> {
        let mut set = self.load()?;
        let before = set.len();
        set.preferences.retain(|p| p.key != key);
        if set.len() == before {
            return Err(format!("Preference '{}' not found", key));
        }
        self.save(&set)?;
        self.fire_event(PreferenceEvent::PreferenceDeleted {
            id: PreferenceId::new(),
            key: key.to_string(),
            timestamp: EventTimestamp::now(),
        });
        Ok(PersistResult::Ok)
    }

    // ─── Reset ────────────────────────────────────────────────────────────

    /// Reset all preferences to defaults.
    pub fn reset(&self) -> Result<PersistResult, String> {
        let defaults = default_preference_set();
        let count = defaults.len();
        self.save(&defaults)?;
        self.fire_event(PreferenceEvent::PreferenceReset {
            count,
            timestamp: EventTimestamp::now(),
        });
        Ok(PersistResult::Ok)
    }

    // ─── Export ───────────────────────────────────────────────────────────

    /// Export all preferences as a JSON string.
    pub fn export(&self) -> Result<String, String> {
        let set = self.load()?;
        let json = serde_json::to_string_pretty(&set)
            .map_err(|e| format!("Failed to serialize preferences: {}", e))?;
        self.fire_event(PreferenceEvent::PreferenceExported {
            count: set.len(),
            timestamp: EventTimestamp::now(),
        });
        Ok(json)
    }

    // ─── Import ───────────────────────────────────────────────────────────

    /// Import preferences from a JSON string.
    pub fn import(&self, json: &str) -> Result<usize, String> {
        let imported: PreferenceSet =
            serde_json::from_str(json).map_err(|e| format!("Invalid JSON in import: {}", e))?;

        // Migrate if needed
        let mut migrated = match self.persistence.validator.migrate(&imported) {
            Ok(m) => m,
            Err(e) => return Err(e),
        };

        // Validate
        let errors = self.persistence.validator.validate_set(&migrated);
        if !errors.is_empty() {
            let msg = format!("Imported preferences have validation errors: {:?}", errors);
            self.diagnostics.record(
                super::diagnostics::DiagnosticKind::ValidationFailure,
                &msg,
                false,
            );
            return Err(msg);
        }

        let dup_result = self.persistence.validator.validate_no_duplicates(&migrated);
        if !dup_result.is_ok() {
            return Err(format!(
                "Imported preferences have duplicate keys: {:?}",
                dup_result
            ));
        }

        // Update origins to Imported
        for pref in &mut migrated.preferences {
            pref.update_origin(PreferenceOrigin::Imported);
        }

        self.save(&migrated)?;
        let count = migrated.len();
        self.fire_event(PreferenceEvent::PreferenceImported {
            count,
            timestamp: EventTimestamp::now(),
        });
        Ok(count)
    }

    // ─── Query ────────────────────────────────────────────────────────────

    /// Get a single preference by key.
    pub fn get(&self, key: &str) -> Result<Option<Preference>, String> {
        let set = self.load()?;
        Ok(set.by_key(key).cloned())
    }

    /// Get all preferences in a category.
    pub fn get_by_category(
        &self,
        category: &PreferenceCategory,
    ) -> Result<Vec<Preference>, String> {
        let set = self.load()?;
        Ok(set.by_category(category))
    }

    /// Get the count of all preferences.
    pub fn count(&self) -> Result<usize, String> {
        Ok(self.load()?.len())
    }

    // ─── Diagnostics ─────────────────────────────────────────────────────

    /// Get diagnostics records.
    pub fn diagnostics(&self) -> Vec<super::diagnostics::DiagnosticRecord> {
        self.diagnostics.records()
    }

    /// Get event log.
    pub fn event_log(&self) -> Vec<PreferenceEvent> {
        let log = self.event_log.lock().unwrap();
        log.events().to_vec()
    }

    /// Get recent events.
    pub fn recent_events(&self, n: usize) -> Vec<PreferenceEvent> {
        let log = self.event_log.lock().unwrap();
        log.recent(n).to_vec()
    }

    // ─── Subscribers ─────────────────────────────────────────────────────

    /// Register a subscriber for preference events.
    pub fn subscribe(&self, subscriber: Box<dyn PreferenceSubscriber>) {
        let mut subs = self.subscribers.lock().unwrap();
        subs.push(subscriber);
    }

    // ─── Internal ─────────────────────────────────────────────────────────

    fn fire_event(&self, event: PreferenceEvent) {
        {
            let mut log = self.event_log.lock().unwrap();
            log.record(event.clone());
        }
        let subs = self.subscribers.lock().unwrap();
        for sub in subs.iter() {
            sub.on_event(&event);
        }
    }

    fn infer_category(&self, key: &str) -> PreferenceCategory {
        self.persistence.category_for_key(key)
    }
}

/// Convenience constructor for tests.
pub fn test_store() -> (PreferenceStore, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let store = PreferenceStore::new(dir.path().to_path_buf());
    (store, dir.path().to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_store_load_empty() {
        let (store, _dir) = test_store();
        let set = store.load().unwrap();
        assert!(set.is_empty());
    }

    #[test]
    fn test_store_update_and_get() {
        let (store, _dir) = test_store();
        store
            .update(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
                "Default model",
                PreferenceOrigin::User,
            )
            .unwrap();

        let pref = store.get("model").unwrap().unwrap();
        assert_eq!(pref.value, PreferenceValue::String("gpt-4o".to_string()));
        assert_eq!(pref.origin, PreferenceOrigin::User);
    }

    #[test]
    fn test_store_delete() {
        let (store, _dir) = test_store();
        store
            .update(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
                "Default model",
                PreferenceOrigin::User,
            )
            .unwrap();
        assert_eq!(store.count().unwrap(), 1);

        store.delete("model").unwrap();
        assert_eq!(store.count().unwrap(), 0);
        assert!(store.get("model").unwrap().is_none());
    }

    #[test]
    fn test_store_delete_missing() {
        let (store, _dir) = test_store();
        let result = store.delete("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_store_reset() {
        let (store, _dir) = test_store();
        store
            .update(
                "model",
                PreferenceValue::String("custom".to_string()),
                "Custom",
                PreferenceOrigin::User,
            )
            .unwrap();
        assert_eq!(store.count().unwrap(), 1);

        store.reset().unwrap();
        let set = store.load().unwrap();
        assert_eq!(set.schema_version, CURRENT_SCHEMA_VERSION);
        assert!(!set.is_empty());
    }

    #[test]
    fn test_store_export_import() {
        let (store, _dir) = test_store();
        store
            .update(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
                "Default model",
                PreferenceOrigin::User,
            )
            .unwrap();

        let exported = store.export().unwrap();
        assert!(exported.contains("gpt-4o"));

        // Create a new store and import
        let (store2, _dir2) = test_store();
        let count = store2.import(&exported).unwrap();
        assert_eq!(count, 1);
        let pref = store2.get("model").unwrap().unwrap();
        assert_eq!(pref.origin, PreferenceOrigin::Imported);
    }

    #[test]
    fn test_store_events() {
        let (store, _dir) = test_store();
        store
            .update(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
                "Default model",
                PreferenceOrigin::User,
            )
            .unwrap();

        let events = store.event_log();
        assert!(!events.is_empty());
        assert!(events
            .iter()
            .any(|e| matches!(e, PreferenceEvent::PreferenceCreated { .. })));
    }

    #[test]
    fn test_store_subscriber() {
        let (store, _dir) = test_store();
        let sub = TestSubscriber::new();
        store.subscribe(Box::new(sub));

        store
            .update(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
                "Default model",
                PreferenceOrigin::User,
            )
            .unwrap();

        let events = store.event_log();
        assert!(!events.is_empty());
        assert!(events
            .iter()
            .any(|e| matches!(e, PreferenceEvent::PreferenceCreated { .. })));
    }

    #[test]
    fn test_store_category_query() {
        let (store, _dir) = test_store();
        store
            .update(
                "provider",
                PreferenceValue::String("openai".to_string()),
                "P",
                PreferenceOrigin::User,
            )
            .unwrap();
        store
            .update(
                "model",
                PreferenceValue::String("gpt-4o".to_string()),
                "M",
                PreferenceOrigin::User,
            )
            .unwrap();
        store
            .update(
                "max_cost",
                PreferenceValue::Float(5.0),
                "C",
                PreferenceOrigin::User,
            )
            .unwrap();

        let models = store.get_by_category(&PreferenceCategory::Model).unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].key, "model");

        let costs = store.get_by_category(&PreferenceCategory::Cost).unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].key, "max_cost");
    }

    #[test]
    fn test_store_import_validates() {
        let (store, _dir) = test_store();
        let invalid_json = "{model\": \"gpt-4o\""; // missing opening brace
        let result = store.import(invalid_json);
        assert!(result.is_err());
    }

    #[test]
    fn test_store_import_with_duplicates() {
        let (store, _dir) = test_store();
        let json = r#"{
            "schema_version": 1,
            "preferences": [
                {"id": "a", "key": "model", "category": "model", "value": {"String": "gpt-4o"}, "description": "d", "schema_version": 1, "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z", "origin": "imported"},
                {"id": "b", "key": "model", "category": "model", "value": {"String": "gpt-4o-mini"}, "description": "d", "schema_version": 1, "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z", "origin": "imported"}
            ]
        }"#;
        let result = store.import(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_store_diagnostics_track_failures() {
        let (store, _dir) = test_store();
        // Trigger a validation failure by importing a set with empty keys
        let json = r#"{
            "schema_version": 1,
            "preferences": [
                {"id": "a", "key": "", "category": "Model", "value": {"String": "x"}, "description": "d", "schema_version": 1, "created_at": "2026-01-01T00:00:00Z", "updated_at": "2026-01-01T00:00:00Z", "origin": "Imported"}
            ]
        }"#;
        let _ = store.import(json);
        let diags = store.diagnostics();
        assert!(diags.iter().any(|d| matches!(
            d.kind,
            crate::preference_engine::DiagnosticKind::ValidationFailure
        )));
    }
}

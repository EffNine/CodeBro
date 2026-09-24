use super::types::{MemoryEntry, MemoryEvent, MemoryQuery, MemoryResolution, MemoryTier};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::Instant;

/// Memory lifecycle manager.
///
/// Owns the state of memory entries and coordinates their creation,
/// access, and deletion. Does NOT persist them.
#[derive(Debug)]
pub struct MemoryLifecycle {
    entries: Arc<RwLock<HashMap<String, MemoryEntry>>>,
    tier_index: Arc<RwLock<HashMap<MemoryTier, HashSet<String>>>>,
    events: Arc<RwLock<Vec<MemoryEvent>>>,
    max_events: usize,
}

impl MemoryLifecycle {
    fn poison_err(lock: &str) -> super::types::MemoryRuntimeError {
        super::types::MemoryRuntimeError::Generic(format!("lock poisoned: {}", lock))
    }

    pub fn new(max_events: usize) -> Self {
        MemoryLifecycle {
            entries: Arc::new(RwLock::new(HashMap::new())),
            tier_index: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(Vec::new())),
            max_events,
        }
    }

    /// Create a new memory entry.
    pub fn create(&self, entry: MemoryEntry) -> super::types::MemoryRuntimeResult<String> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| Self::poison_err("entries"))?;
        let mut tier_index = self
            .tier_index
            .write()
            .map_err(|_| Self::poison_err("tier_index"))?;

        if entries.contains_key(&entry.id) {
            return Err(super::types::MemoryRuntimeError::Conflict(format!(
                "Entry {} already exists",
                entry.id
            )));
        }

        let id = entry.id.clone();
        let tier = entry.tier;
        entries.insert(id.clone(), entry);
        tier_index.entry(tier).or_default().insert(id.clone());

        self.record_event(MemoryEvent::PolicyApplied {
            event_id: uuid::Uuid::new_v4().to_string(),
            policy_name: "lifecycle.create".to_string(),
            action: "created".to_string(),
            affected_count: 1,
            timestamp: 0,
        });

        Ok(id)
    }

    /// Get a memory entry by ID.
    pub fn get(&self, id: &str) -> Option<MemoryEntry> {
        let mut entries = match self.entries.write() {
            Ok(guard) => guard,
            Err(_) => return None,
        };
        if let Some(entry) = entries.get_mut(id) {
            entry.record_access();
            Some(entry.clone())
        } else {
            None
        }
    }

    /// Update a memory entry.
    pub fn update(
        &self,
        id: &str,
        value: impl Into<String>,
    ) -> super::types::MemoryRuntimeResult<()> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| Self::poison_err("entries"))?;
        if let Some(entry) = entries.get_mut(id) {
            entry.value = value.into();
            entry.record_access();
            Ok(())
        } else {
            Err(super::types::MemoryRuntimeError::EntryNotFound(
                id.to_string(),
            ))
        }
    }

    /// Delete a memory entry.
    pub fn delete(&self, id: &str) -> super::types::MemoryRuntimeResult<()> {
        let mut entries = self
            .entries
            .write()
            .map_err(|_| Self::poison_err("entries"))?;
        let mut tier_index = self
            .tier_index
            .write()
            .map_err(|_| Self::poison_err("tier_index"))?;

        if let Some(entry) = entries.remove(id) {
            if let Some(tier_set) = tier_index.get_mut(&entry.tier) {
                tier_set.remove(id);
            }
            Ok(())
        } else {
            Err(super::types::MemoryRuntimeError::EntryNotFound(
                id.to_string(),
            ))
        }
    }

    /// List entries in a tier.
    pub fn list_by_tier(&self, tier: MemoryTier) -> Vec<MemoryEntry> {
        let entries = match self.entries.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };
        let tier_index = match self.tier_index.read() {
            Ok(guard) => guard,
            Err(_) => return Vec::new(),
        };

        tier_index
            .get(&tier)
            .map(|ids| {
                ids.iter()
                    .filter_map(|id| entries.get(id).cloned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// List all entries.
    pub fn list_all(&self) -> Vec<MemoryEntry> {
        match self.entries.read() {
            Ok(guard) => guard.values().cloned().collect(),
            Err(_) => Vec::new(),
        }
    }

    /// Count entries in a tier.
    pub fn count_by_tier(&self, tier: MemoryTier) -> usize {
        match self.tier_index.read() {
            Ok(guard) => guard.get(&tier).map(|s| s.len()).unwrap_or(0),
            Err(_) => 0,
        }
    }

    /// Check if an entry exists.
    pub fn contains(&self, id: &str) -> bool {
        match self.entries.read() {
            Ok(guard) => guard.contains_key(id),
            Err(_) => false,
        }
    }

    /// Clear all entries.
    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.write() {
            entries.clear();
        }
        if let Ok(mut tier_index) = self.tier_index.write() {
            tier_index.clear();
        }
    }

    /// Record a memory event.
    pub fn record_event(&self, event: MemoryEvent) {
        if let Ok(mut events) = self.events.write() {
            if events.len() >= self.max_events {
                events.remove(0);
            }
            events.push(event);
        }
    }

    /// Get memory events.
    pub fn events(&self) -> Vec<MemoryEvent> {
        match self.events.read() {
            Ok(guard) => guard.clone(),
            Err(_) => Vec::new(),
        }
    }

    /// Get entry count.
    pub fn entry_count(&self) -> usize {
        match self.entries.read() {
            Ok(guard) => guard.len(),
            Err(_) => 0,
        }
    }
}

impl Default for MemoryLifecycle {
    fn default() -> Self {
        MemoryLifecycle::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_runtime::types::MemoryRuntimeError;

    fn test_entry(id: &str) -> MemoryEntry {
        MemoryEntry::new(id, MemoryTier::Session, "key", "value")
    }

    fn poison_entries(lifecycle: &MemoryLifecycle) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lifecycle.entries.write().unwrap();
            panic!("intentional poison for test");
        }));
    }

    fn poison_tier_index(lifecycle: &MemoryLifecycle) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lifecycle.tier_index.write().unwrap();
            panic!("intentional poison for test");
        }));
    }

    fn poison_events(lifecycle: &MemoryLifecycle) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = lifecycle.events.write().unwrap();
            panic!("intentional poison for test");
        }));
    }

    #[test]
    fn poisoned_entries_lock_returns_errors_not_panics() {
        let lifecycle = MemoryLifecycle::new(10);
        lifecycle.create(test_entry("e1")).unwrap();
        poison_entries(&lifecycle);
        poison_tier_index(&lifecycle);

        let create_result = lifecycle.create(test_entry("e2"));
        assert!(create_result.is_err());
        match create_result {
            Err(MemoryRuntimeError::Generic(msg)) => {
                assert!(msg.contains("lock poisoned"), "unexpected message: {}", msg)
            }
            other => panic!("expected Generic poison error, got {:?}", other),
        }

        assert!(lifecycle.get("e1").is_none());
        assert!(lifecycle.update("e1", "v").is_err());
        assert!(lifecycle.delete("e1").is_err());
        assert!(lifecycle.list_by_tier(MemoryTier::Session).is_empty());
        assert!(lifecycle.list_all().is_empty());
        assert_eq!(lifecycle.count_by_tier(MemoryTier::Session), 0);
        assert_eq!(lifecycle.entry_count(), 0);
        assert!(!lifecycle.contains("e1"));
        // Must not panic.
        lifecycle.clear();
    }

    #[test]
    fn poisoned_events_lock_degrades_gracefully() {
        let lifecycle = MemoryLifecycle::new(10);
        poison_events(&lifecycle);

        // Must not panic; poison is swallowed.
        lifecycle.record_event(MemoryEvent::PolicyApplied {
            event_id: "e".to_string(),
            policy_name: "p".to_string(),
            action: "a".to_string(),
            affected_count: 1,
            timestamp: 0,
        });
        assert!(lifecycle.events().is_empty());
    }
}

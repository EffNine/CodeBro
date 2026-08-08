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
        let mut entries = self.entries.write().unwrap();
        let mut tier_index = self.tier_index.write().unwrap();

        if entries.contains_key(&entry.id) {
            return Err(super::types::MemoryRuntimeError::Conflict(format!(
                "Entry {} already exists",
                entry.id
            )));
        }

        let id = entry.id.clone();
        let tier = entry.tier;
        entries.insert(id.clone(), entry);
        tier_index
            .entry(tier)
            .or_insert_with(|| HashSet::new())
            .insert(id.clone());

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
        let mut entries = self.entries.write().unwrap();
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
        let mut entries = self.entries.write().unwrap();
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
        let mut entries = self.entries.write().unwrap();
        let mut tier_index = self.tier_index.write().unwrap();

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
        let entries = self.entries.read().unwrap();
        let tier_index = self.tier_index.read().unwrap();

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
        self.entries.read().unwrap().values().cloned().collect()
    }

    /// Count entries in a tier.
    pub fn count_by_tier(&self, tier: MemoryTier) -> usize {
        self.tier_index
            .read()
            .unwrap()
            .get(&tier)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    /// Check if an entry exists.
    pub fn contains(&self, id: &str) -> bool {
        self.entries.read().unwrap().contains_key(id)
    }

    /// Clear all entries.
    pub fn clear(&self) {
        self.entries.write().unwrap().clear();
        self.tier_index.write().unwrap().clear();
    }

    /// Record a memory event.
    pub fn record_event(&self, event: MemoryEvent) {
        let mut events = self.events.write().unwrap();
        if events.len() >= self.max_events {
            events.remove(0);
        }
        events.push(event);
    }

    /// Get memory events.
    pub fn events(&self) -> Vec<MemoryEvent> {
        self.events.read().unwrap().clone()
    }

    /// Get entry count.
    pub fn entry_count(&self) -> usize {
        self.entries.read().unwrap().len()
    }
}

impl Default for MemoryLifecycle {
    fn default() -> Self {
        MemoryLifecycle::new(1000)
    }
}

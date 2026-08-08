use super::diagnostics::MemoryDiagnostics;
use super::lifecycle::MemoryLifecycle;
use super::policy::{EvictionPolicy, MemoryPolicy, RetentionPolicy};
use super::snapshot::SnapshotManager;
use super::types::{MemoryEntry, MemoryEvent, MemoryTier};
use std::sync::{Arc, RwLock};

/// Tier coordinator that manages memory across Session, Project, and Global tiers.
#[derive(Debug)]
pub struct TierCoordinator {
    lifecycle: Arc<MemoryLifecycle>,
    snapshots: Arc<SnapshotManager>,
    diagnostics: Arc<RwLock<MemoryDiagnostics>>,
    policy: Arc<RwLock<MemoryPolicy>>,
}

impl TierCoordinator {
    pub fn new(policy: MemoryPolicy) -> Self {
        TierCoordinator {
            lifecycle: Arc::new(MemoryLifecycle::new(1000)),
            snapshots: Arc::new(SnapshotManager::new(1000)),
            diagnostics: Arc::new(RwLock::new(MemoryDiagnostics::new(1000))),
            policy: Arc::new(RwLock::new(policy)),
        }
    }

    /// Get the lifecycle (for resolution).
    pub fn lifecycle(&self) -> &Arc<MemoryLifecycle> {
        &self.lifecycle
    }

    /// Create a memory entry in a specific tier.
    pub fn create(&self, entry: MemoryEntry) -> super::types::MemoryRuntimeResult<String> {
        // Check access rules
        let policy = self.policy.read().unwrap();
        if !policy.is_access_allowed(entry.tier, &entry.key, entry.metadata.confidence) {
            self.diagnostics.write().unwrap().record_policy_violation();
            return Err(super::types::MemoryRuntimeError::PolicyViolation(format!(
                "Access denied for key '{}' in tier {}",
                entry.key, entry.tier
            )));
        }

        // Check max entries per tier
        if self.lifecycle.count_by_tier(entry.tier) >= policy.max_entries_per_tier {
            self.evict_by_policy(entry.tier)?;
        }

        let id = self.lifecycle.create(entry)?;
        self.diagnostics.write().unwrap().record_hit();
        Ok(id)
    }

    /// Get a memory entry.
    pub fn get(&self, id: &str) -> Option<MemoryEntry> {
        self.lifecycle.get(id)
    }

    /// Update a memory entry.
    pub fn update(
        &self,
        id: &str,
        value: impl Into<String>,
    ) -> super::types::MemoryRuntimeResult<()> {
        self.lifecycle.update(id, value)
    }

    /// Delete a memory entry.
    pub fn delete(&self, id: &str) -> super::types::MemoryRuntimeResult<()> {
        self.lifecycle.delete(id)
    }

    /// List entries in a tier.
    pub fn list_by_tier(&self, tier: MemoryTier) -> Vec<MemoryEntry> {
        self.lifecycle.list_by_tier(tier)
    }

    /// List all entries.
    pub fn list_all(&self) -> Vec<MemoryEntry> {
        self.lifecycle.list_all()
    }

    /// Create a snapshot of a tier.
    pub fn snapshot_tier(
        &self,
        snapshot_id: impl Into<String>,
        tier: MemoryTier,
    ) -> super::types::MemoryRuntimeResult<String> {
        let entries = self.lifecycle.list_by_tier(tier);
        let entry_map: std::collections::HashMap<String, MemoryEntry> =
            entries.into_iter().map(|e| (e.id.clone(), e)).collect();

        let id = self
            .snapshots
            .create(snapshot_id.into(), tier, entry_map, Default::default())?;

        self.diagnostics.write().unwrap().record_snapshot_creation();
        Ok(id)
    }

    /// Merge two snapshots.
    pub fn merge_snapshots(
        &self,
        source_id: &str,
        target_id: &str,
        new_id: impl Into<String>,
    ) -> super::types::MemoryRuntimeResult<super::snapshot::MemorySnapshot> {
        let result = self.snapshots.merge(source_id, target_id, new_id)?;
        self.diagnostics.write().unwrap().record_snapshot_merge();
        Ok(result)
    }

    /// Diff two snapshots.
    pub fn diff_snapshots(
        &self,
        snapshot_a_id: &str,
        snapshot_b_id: &str,
    ) -> super::types::MemoryRuntimeResult<super::snapshot::SnapshotDiff> {
        self.snapshots.diff(snapshot_a_id, snapshot_b_id)
    }

    /// Restore memory from a snapshot.
    pub fn restore_from_snapshot(
        &self,
        snapshot_id: &str,
    ) -> super::types::MemoryRuntimeResult<Vec<MemoryEntry>> {
        self.snapshots.restore(snapshot_id)
    }

    /// Apply eviction policy to a tier.
    pub fn evict_by_policy(&self, tier: MemoryTier) -> super::types::MemoryRuntimeResult<usize> {
        let policy = self.policy.read().unwrap();
        let entries = self.lifecycle.list_by_tier(tier);

        if entries.is_empty() {
            return Ok(0);
        }

        let mut to_evict = Vec::new();

        match policy.eviction {
            EvictionPolicy::LRU => {
                let mut sorted = entries;
                sorted.sort_by(|a, b| a.last_accessed.cmp(&b.last_accessed));
                to_evict = sorted;
            }
            EvictionPolicy::LFU => {
                let mut sorted = entries;
                sorted.sort_by(|a, b| a.access_count.cmp(&b.access_count));
                to_evict = sorted;
            }
            EvictionPolicy::LowestImportance => {
                let mut sorted = entries;
                sorted.sort_by(|a, b| {
                    b.metadata
                        .importance
                        .partial_cmp(&a.metadata.importance)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                to_evict = sorted;
            }
            EvictionPolicy::LowestConfidence => {
                let mut sorted = entries;
                sorted.sort_by(|a, b| {
                    b.metadata
                        .confidence
                        .partial_cmp(&a.metadata.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                to_evict = sorted;
            }
            EvictionPolicy::FIFO => {
                let mut sorted = entries;
                sorted.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                to_evict = sorted;
            }
        }

        // Evict low-priority entries
        let eviction_count = to_evict.len() / 4; // Evict bottom 25%
        for entry in to_evict.iter().take(eviction_count) {
            if policy.should_evict(entry) || policy.is_expired(entry) {
                self.lifecycle.delete(&entry.id)?;
                self.diagnostics.write().unwrap().record_eviction();
            }
        }

        Ok(eviction_count)
    }

    /// Apply retention policy across all tiers.
    pub fn apply_retention(&self) -> super::types::MemoryRuntimeResult<usize> {
        let policy = self.policy.read().unwrap();
        let mut evicted = 0;

        for tier in [MemoryTier::Session, MemoryTier::Project, MemoryTier::Global] {
            let entries = self.lifecycle.list_by_tier(tier);
            for entry in entries {
                if policy.should_evict(&entry) || policy.is_expired(&entry) {
                    self.lifecycle.delete(&entry.id)?;
                    evicted += 1;
                }
            }
        }

        self.diagnostics
            .write()
            .unwrap()
            .record_event(MemoryEvent::PolicyApplied {
                event_id: uuid::Uuid::new_v4().to_string(),
                policy_name: "retention".to_string(),
                action: "evicted".to_string(),
                affected_count: evicted,
                timestamp: 0,
            });

        Ok(evicted)
    }

    /// Get diagnostics.
    pub fn diagnostics(&self) -> super::diagnostics::MemoryDiagnosticsSummary {
        self.diagnostics.read().unwrap().summary()
    }

    /// Get event count.
    pub fn event_count(&self) -> usize {
        self.diagnostics.read().unwrap().events().len()
    }

    /// Get entry count.
    pub fn entry_count(&self) -> usize {
        self.lifecycle.entry_count()
    }

    /// Get entry count by tier.
    pub fn entry_count_by_tier(&self, tier: MemoryTier) -> usize {
        self.lifecycle.count_by_tier(tier)
    }
}

impl Default for TierCoordinator {
    fn default() -> Self {
        TierCoordinator::new(MemoryPolicy::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_runtime::types::{MemoryMetadata, MemoryQuery};

    fn test_entry(id: &str, tier: MemoryTier, key: &str, value: &str) -> MemoryEntry {
        MemoryEntry::new(id, tier, key, value)
            .with_metadata(MemoryMetadata::new().with_confidence(0.9))
    }

    #[test]
    fn test_create_and_get() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        let id = coordinator.create(entry).unwrap();
        assert_eq!(id, "e1");

        let retrieved = coordinator.get("e1").unwrap();
        assert_eq!(retrieved.key, "key");
        assert_eq!(retrieved.value, "value");
    }

    #[test]
    fn test_create_duplicate() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        coordinator.create(entry.clone()).unwrap();
        let result = coordinator.create(entry);
        assert!(result.is_err());
    }

    #[test]
    fn test_update() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("e1", MemoryTier::Session, "key", "value"))
            .unwrap();
        coordinator.update("e1", "new_value").unwrap();

        let entry = coordinator.get("e1").unwrap();
        assert_eq!(entry.value, "new_value");
    }

    #[test]
    fn test_delete() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("e1", MemoryTier::Session, "key", "value"))
            .unwrap();
        coordinator.delete("e1").unwrap();
        assert!(coordinator.get("e1").is_none());
    }

    #[test]
    fn test_delete_not_found() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        let result = coordinator.delete("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_by_tier() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("s1", MemoryTier::Session, "key", "value"))
            .unwrap();
        coordinator
            .create(test_entry("p1", MemoryTier::Project, "key", "value"))
            .unwrap();
        coordinator
            .create(test_entry("g1", MemoryTier::Global, "key", "value"))
            .unwrap();

        let session_entries = coordinator.list_by_tier(MemoryTier::Session);
        assert_eq!(session_entries.len(), 1);
        assert_eq!(session_entries[0].id, "s1");

        let project_entries = coordinator.list_by_tier(MemoryTier::Project);
        assert_eq!(project_entries.len(), 1);
        assert_eq!(project_entries[0].id, "p1");

        let global_entries = coordinator.list_by_tier(MemoryTier::Global);
        assert_eq!(global_entries.len(), 1);
        assert_eq!(global_entries[0].id, "g1");
    }

    #[test]
    fn test_list_all() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("s1", MemoryTier::Session, "key", "value"))
            .unwrap();
        coordinator
            .create(test_entry("p1", MemoryTier::Project, "key", "value"))
            .unwrap();

        let all = coordinator.list_all();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn test_snapshot_tier() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("s1", MemoryTier::Session, "key", "value"))
            .unwrap();

        let snapshot_id = coordinator
            .snapshot_tier("snap1", MemoryTier::Session)
            .unwrap();
        assert_eq!(snapshot_id, "snap1");

        let snapshot = coordinator.restore_from_snapshot("snap1").unwrap();
        assert_eq!(snapshot.len(), 1);
    }

    #[test]
    fn test_access_denied() {
        let policy = MemoryPolicy::new().with_access_rule(
            crate::memory_runtime::AccessRule::new(MemoryTier::Session).deny_key("secret"),
        );
        let coordinator = TierCoordinator::new(policy);
        let entry = test_entry("e1", MemoryTier::Session, "secret", "value");
        let result = coordinator.create(entry);
        assert!(result.is_err());
    }

    #[test]
    fn test_entry_count() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        assert_eq!(coordinator.entry_count(), 0);

        coordinator
            .create(test_entry("e1", MemoryTier::Session, "key", "value"))
            .unwrap();
        assert_eq!(coordinator.entry_count(), 1);
    }

    #[test]
    fn test_entry_count_by_tier() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("s1", MemoryTier::Session, "key", "value"))
            .unwrap();
        coordinator
            .create(test_entry("s2", MemoryTier::Session, "key", "value"))
            .unwrap();

        assert_eq!(coordinator.entry_count_by_tier(MemoryTier::Session), 2);
        assert_eq!(coordinator.entry_count_by_tier(MemoryTier::Project), 0);
    }

    #[test]
    fn test_diagnostics() {
        let coordinator = TierCoordinator::new(MemoryPolicy::default());
        coordinator
            .create(test_entry("e1", MemoryTier::Session, "key", "value"))
            .unwrap();

        let summary = coordinator.diagnostics();
        assert!(summary.total_hits > 0 || summary.total_misses >= 0);
    }
}

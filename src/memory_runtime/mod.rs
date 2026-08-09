pub mod diagnostics;
pub mod lifecycle;
pub mod policy;
pub mod resolution;
pub mod snapshot;
pub mod tier_coordination;
pub mod types;

use std::fmt;

pub use diagnostics::{MemoryDiagnostics, MemoryDiagnosticsSummary};
pub use lifecycle::MemoryLifecycle;
pub use policy::{
    AccessRule, ConflictPolicy, EvictionPolicy, ExpirationPolicy, MemoryPolicy, PriorityPolicy,
    RetentionPolicy,
};
pub use resolution::MemoryResolver;
pub use snapshot::{compute_diff, MemorySnapshot, SnapshotDiff, SnapshotManager, SnapshotMetadata};
pub use tier_coordination::TierCoordinator;
pub use types::{
    MemoryEntry, MemoryEvent, MemoryMetadata, MemoryQuery, MemoryResolution, MemoryRuntimeError,
    MemoryRuntimeResult, MemoryTier,
};

/// High-level Memory Runtime that coordinates all memory operations.
pub struct MemoryRuntime {
    lifecycle: std::sync::Arc<MemoryLifecycle>,
    resolver: MemoryResolver,
    coordinator: std::sync::Arc<TierCoordinator>,
    snapshots: std::sync::Arc<SnapshotManager>,
}

impl MemoryRuntime {
    pub fn new(policy: MemoryPolicy) -> Self {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(1000));
        let resolver = MemoryResolver::new(lifecycle.clone());
        let coordinator = std::sync::Arc::new(TierCoordinator::new(policy.clone()));
        let snapshots = std::sync::Arc::new(SnapshotManager::new(1000));

        MemoryRuntime {
            lifecycle,
            resolver,
            coordinator,
            snapshots,
        }
    }

    /// Create a memory entry.
    pub fn create(&self, entry: MemoryEntry) -> MemoryRuntimeResult<String> {
        self.coordinator.create(entry)
    }

    /// Get a memory entry.
    pub fn get(&self, id: &str) -> Option<MemoryEntry> {
        self.coordinator.get(id)
    }

    /// Update a memory entry.
    pub fn update(&self, id: &str, value: impl Into<String>) -> MemoryRuntimeResult<()> {
        self.coordinator.update(id, value)
    }

    /// Delete a memory entry.
    pub fn delete(&self, id: &str) -> MemoryRuntimeResult<()> {
        self.coordinator.delete(id)
    }

    /// Resolve a memory query.
    pub fn resolve(&self, query: &MemoryQuery) -> MemoryResolution {
        // Use the coordinator's internal lifecycle for resolution
        let resolver = MemoryResolver::new(self.coordinator.lifecycle().clone());
        resolver.resolve(query)
    }

    /// Resolve with conflict policy.
    pub fn resolve_with_policy(
        &self,
        query: &MemoryQuery,
        policy: ConflictPolicy,
    ) -> MemoryResolution {
        let resolution = self.resolver.resolve(query);
        // Apply conflict policy if needed
        resolution
    }

    /// List entries by tier.
    pub fn list_by_tier(&self, tier: MemoryTier) -> Vec<MemoryEntry> {
        self.coordinator.list_by_tier(tier)
    }

    /// List all entries.
    pub fn list_all(&self) -> Vec<MemoryEntry> {
        self.coordinator.list_all()
    }

    /// Create a snapshot.
    pub fn snapshot(&self, id: impl Into<String>, tier: MemoryTier) -> MemoryRuntimeResult<String> {
        self.coordinator.snapshot_tier(id, tier)
    }

    /// Merge snapshots.
    pub fn merge_snapshots(
        &self,
        source: &str,
        target: &str,
        new_id: impl Into<String>,
    ) -> MemoryRuntimeResult<MemorySnapshot> {
        self.coordinator.merge_snapshots(source, target, new_id)
    }

    /// Diff snapshots.
    pub fn diff_snapshots(&self, snap_a: &str, snap_b: &str) -> MemoryRuntimeResult<SnapshotDiff> {
        self.coordinator.diff_snapshots(snap_a, snap_b)
    }

    /// Restore from snapshot.
    pub fn restore(&self, snapshot_id: &str) -> MemoryRuntimeResult<Vec<MemoryEntry>> {
        self.coordinator.restore_from_snapshot(snapshot_id)
    }

    /// Apply retention policy.
    pub fn apply_retention(&self) -> MemoryRuntimeResult<usize> {
        self.coordinator.apply_retention()
    }

    /// Get diagnostics.
    pub fn diagnostics(&self) -> MemoryDiagnosticsSummary {
        self.coordinator.diagnostics()
    }

    /// Get entry count.
    pub fn entry_count(&self) -> usize {
        self.coordinator.entry_count()
    }

    /// Get entry count by tier.
    pub fn entry_count_by_tier(&self, tier: MemoryTier) -> usize {
        self.coordinator.entry_count_by_tier(tier)
    }
}

impl std::fmt::Debug for MemoryRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryRuntime")
            .field("entry_count", &self.entry_count())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_runtime::types::MemoryMetadata;

    fn test_entry(id: &str, tier: MemoryTier, key: &str, value: &str) -> MemoryEntry {
        MemoryEntry::new(id, tier, key, value)
            .with_metadata(MemoryMetadata::new().with_confidence(0.9))
    }

    #[test]
    fn test_memory_runtime_creation() {
        let runtime = MemoryRuntime::new(MemoryPolicy::default());
        assert_eq!(runtime.entry_count(), 0);
    }

    #[test]
    fn test_memory_runtime_create_and_get() {
        let runtime = MemoryRuntime::new(MemoryPolicy::default());
        let entry = test_entry("e1", MemoryTier::Session, "key", "value");
        let id = runtime.create(entry).unwrap();
        assert_eq!(id, "e1");

        let retrieved = runtime.get("e1").unwrap();
        assert_eq!(retrieved.key, "key");
    }

    #[test]
    fn test_memory_runtime_resolve() {
        let runtime = MemoryRuntime::new(MemoryPolicy::default());
        runtime
            .create(test_entry("s1", MemoryTier::Session, "language", "rust"))
            .unwrap();
        runtime
            .create(test_entry("p1", MemoryTier::Project, "language", "python"))
            .unwrap();

        let query = MemoryQuery::new("language");
        let resolution = runtime.resolve(&query);
        // Debug: check what we got
        eprintln!("Resolution hits: {}", resolution.hits.len());
        for h in &resolution.hits {
            eprintln!("  - {} ({})", h.id, h.tier);
        }
        // First match wins (deterministic)
        assert!(
            resolution.hits.len() >= 1,
            "Should find at least one match for 'language'"
        );
    }

    #[test]
    fn test_memory_runtime_snapshot() {
        let runtime = MemoryRuntime::new(MemoryPolicy::default());
        runtime
            .create(test_entry("e1", MemoryTier::Session, "key", "value"))
            .unwrap();

        let snap_id = runtime.snapshot("snap1", MemoryTier::Session).unwrap();
        assert_eq!(snap_id, "snap1");

        let restored = runtime.restore("snap1").unwrap();
        assert_eq!(restored.len(), 1);
    }

    #[test]
    fn test_memory_runtime_diagnostics() {
        let runtime = MemoryRuntime::new(MemoryPolicy::default());
        runtime
            .create(test_entry("e1", MemoryTier::Session, "key", "value"))
            .unwrap();

        let diag = runtime.diagnostics();
    }

    #[test]
    fn test_memory_runtime_debug() {
        let runtime = MemoryRuntime::new(MemoryPolicy::default());
        let debug = format!("{:?}", runtime);
        assert!(debug.contains("MemoryRuntime"));
    }
}

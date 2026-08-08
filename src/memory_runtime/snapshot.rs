use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::types::{MemoryEntry, MemoryEvent, MemoryTier};

/// An immutable snapshot of memory at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemorySnapshot {
    pub id: String,
    pub created_at: u64,
    pub tier: MemoryTier,
    pub entries: HashMap<String, MemoryEntry>,
    pub metadata: SnapshotMetadata,
}

impl MemorySnapshot {
    pub fn new(
        id: impl Into<String>,
        tier: MemoryTier,
        entries: HashMap<String, MemoryEntry>,
    ) -> Self {
        MemorySnapshot {
            id: id.into(),
            created_at: chrono::Utc::now().timestamp() as u64,
            tier,
            entries,
            metadata: SnapshotMetadata::default(),
        }
    }

    pub fn with_metadata(mut self, metadata: SnapshotMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn contains(&self, id: &str) -> bool {
        self.entries.contains_key(id)
    }

    pub fn get(&self, id: &str) -> Option<&MemoryEntry> {
        self.entries.get(id)
    }
}

/// Metadata for a snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub source: Option<String>,
}

impl SnapshotMetadata {
    pub fn new() -> Self {
        SnapshotMetadata::default()
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag_str = tag.into();
        if !self.tags.contains(&tag_str) {
            self.tags.push(tag_str);
        }
        self
    }

    pub fn with_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }
}

/// Snapshot diff showing differences between two snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub snapshot_a_id: String,
    pub snapshot_b_id: String,
    pub added: Vec<MemoryEntry>,
    pub removed: Vec<String>,
    pub modified: Vec<(String, MemoryEntry, MemoryEntry)>,
}

impl SnapshotDiff {
    pub fn new(
        snapshot_a_id: impl Into<String>,
        snapshot_b_id: impl Into<String>,
        added: Vec<MemoryEntry>,
        removed: Vec<String>,
        modified: Vec<(String, MemoryEntry, MemoryEntry)>,
    ) -> Self {
        SnapshotDiff {
            snapshot_a_id: snapshot_a_id.into(),
            snapshot_b_id: snapshot_b_id.into(),
            added,
            removed,
            modified,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty() && self.modified.is_empty()
    }

    pub fn summary(&self) -> String {
        format!(
            "Diff({}, {}): +{} entries, -{} entries, ~{} modified",
            self.snapshot_a_id,
            self.snapshot_b_id,
            self.added.len(),
            self.removed.len(),
            self.modified.len(),
        )
    }
}

/// Compute diff between two snapshots.
pub fn compute_diff(snapshot_a: &MemorySnapshot, snapshot_b: &MemorySnapshot) -> SnapshotDiff {
    let mut added = Vec::new();
    let mut removed = Vec::new();
    let mut modified = Vec::new();

    // Find added and modified entries
    for (id, entry_b) in &snapshot_b.entries {
        if let Some(entry_a) = snapshot_a.entries.get(id) {
            if entry_a.value != entry_b.value || entry_a.metadata != entry_b.metadata {
                modified.push((id.clone(), entry_a.clone(), entry_b.clone()));
            }
        } else {
            added.push(entry_b.clone());
        }
    }

    // Find removed entries
    for id in snapshot_a.entries.keys() {
        if !snapshot_b.entries.contains_key(id) {
            removed.push(id.clone());
        }
    }

    SnapshotDiff::new(&snapshot_a.id, &snapshot_b.id, added, removed, modified)
}

/// Snapshot manager for creating, storing, and merging snapshots.
///
/// Snapshots are immutable. No mutable global memory.
#[derive(Debug)]
pub struct SnapshotManager {
    snapshots: Arc<RwLock<HashMap<String, MemorySnapshot>>>,
    events: Arc<RwLock<Vec<MemoryEvent>>>,
    max_events: usize,
}

impl SnapshotManager {
    pub fn new(max_events: usize) -> Self {
        SnapshotManager {
            snapshots: Arc::new(RwLock::new(HashMap::new())),
            events: Arc::new(RwLock::new(Vec::new())),
            max_events,
        }
    }

    /// Create a new snapshot from current memory state.
    pub fn create(
        &self,
        id: impl Into<String>,
        tier: MemoryTier,
        entries: HashMap<String, MemoryEntry>,
        metadata: SnapshotMetadata,
    ) -> super::types::MemoryRuntimeResult<String> {
        let id = id.into();
        let snapshot = MemorySnapshot::new(id.clone(), tier, entries).with_metadata(metadata);

        let mut snapshots = self.snapshots.write().unwrap();
        snapshots.insert(snapshot.id.clone(), snapshot);

        self.record_event(MemoryEvent::SnapshotCreated {
            event_id: uuid::Uuid::new_v4().to_string(),
            snapshot_id: id.clone(),
            entry_count: snapshots.len(),
            timestamp: 0,
        });

        Ok(id)
    }

    /// Get a snapshot by ID.
    pub fn get(&self, id: &str) -> Option<MemorySnapshot> {
        self.snapshots.read().unwrap().get(id).cloned()
    }

    /// List all snapshots.
    pub fn list(&self) -> Vec<MemorySnapshot> {
        self.snapshots.read().unwrap().values().cloned().collect()
    }

    /// Delete a snapshot.
    pub fn delete(&self, id: &str) -> super::types::MemoryRuntimeResult<()> {
        let mut snapshots = self.snapshots.write().unwrap();
        if snapshots.remove(id).is_some() {
            Ok(())
        } else {
            Err(super::types::MemoryRuntimeError::SnapshotError(format!(
                "Snapshot {} not found",
                id
            )))
        }
    }

    /// Merge two snapshots into a new snapshot.
    pub fn merge(
        &self,
        source_id: &str,
        target_id: &str,
        new_id: impl Into<String>,
    ) -> super::types::MemoryRuntimeResult<MemorySnapshot> {
        // First, read snapshots
        let (source_entries, target_entries, target_tier) = {
            let snapshots = self.snapshots.read().unwrap();

            let source = snapshots.get(source_id).ok_or_else(|| {
                super::types::MemoryRuntimeError::SnapshotError(format!(
                    "Source snapshot {} not found",
                    source_id
                ))
            })?;
            let target = snapshots.get(target_id).ok_or_else(|| {
                super::types::MemoryRuntimeError::SnapshotError(format!(
                    "Target snapshot {} not found",
                    target_id
                ))
            })?;

            (source.entries.clone(), target.entries.clone(), target.tier)
        };

        // Merge: target entries take precedence, but include source additions
        let mut merged_entries = target_entries;
        let mut merged_count = 0;

        for (id, entry) in &source_entries {
            if !merged_entries.contains_key(id) {
                merged_entries.insert(id.clone(), entry.clone());
                merged_count += 1;
            }
        }

        let new_id_str = new_id.into();
        let merged = MemorySnapshot::new(new_id_str.clone(), target_tier, merged_entries);

        // Now write the merged snapshot
        {
            let mut snapshots = self.snapshots.write().unwrap();
            snapshots.insert(new_id_str, merged.clone());
        }

        self.record_event(MemoryEvent::SnapshotMerged {
            event_id: uuid::Uuid::new_v4().to_string(),
            source_snapshot: source_id.to_string(),
            target_snapshot: target_id.to_string(),
            entries_merged: merged_count,
            timestamp: 0,
        });

        Ok(merged)
    }

    /// Compute diff between two snapshots.
    pub fn diff(
        &self,
        snapshot_a_id: &str,
        snapshot_b_id: &str,
    ) -> super::types::MemoryRuntimeResult<SnapshotDiff> {
        let snapshots = self.snapshots.read().unwrap();

        let snapshot_a = snapshots.get(snapshot_a_id).ok_or_else(|| {
            super::types::MemoryRuntimeError::SnapshotError(format!(
                "Snapshot {} not found",
                snapshot_a_id
            ))
        })?;
        let snapshot_b = snapshots.get(snapshot_b_id).ok_or_else(|| {
            super::types::MemoryRuntimeError::SnapshotError(format!(
                "Snapshot {} not found",
                snapshot_b_id
            ))
        })?;

        Ok(compute_diff(snapshot_a, snapshot_b))
    }

    /// Restore memory state from a snapshot.
    ///
    /// Returns the entries from the snapshot (does not modify lifecycle).
    pub fn restore(
        &self,
        snapshot_id: &str,
    ) -> super::types::MemoryRuntimeResult<Vec<MemoryEntry>> {
        let snapshot = self.get(snapshot_id).ok_or_else(|| {
            super::types::MemoryRuntimeError::SnapshotError(format!(
                "Snapshot {} not found",
                snapshot_id
            ))
        })?;

        Ok(snapshot.entries.values().cloned().collect())
    }

    /// Get snapshot count.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.read().unwrap().len()
    }

    /// Record a snapshot event.
    pub fn record_event(&self, event: MemoryEvent) {
        let mut events = self.events.write().unwrap();
        if events.len() >= self.max_events {
            events.remove(0);
        }
        events.push(event);
    }

    /// Get snapshot events.
    pub fn events(&self) -> Vec<MemoryEvent> {
        self.events.read().unwrap().clone()
    }
}

impl Default for SnapshotManager {
    fn default() -> Self {
        SnapshotManager::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_runtime::types::MemoryMetadata;

    fn test_entry(id: &str, tier: MemoryTier, key: &str, value: &str) -> MemoryEntry {
        MemoryEntry::new(id, tier, key, value).with_metadata(MemoryMetadata::new())
    }

    #[test]
    fn test_snapshot_create_and_get() {
        let manager = SnapshotManager::new(100);
        let mut entries = HashMap::new();
        entries.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1"),
        );
        entries.insert(
            "e2".to_string(),
            test_entry("e2", MemoryTier::Session, "key", "value2"),
        );

        let id = manager
            .create(
                "snap1".to_string().to_string(),
                MemoryTier::Session,
                entries.clone(),
                SnapshotMetadata::new(),
            )
            .unwrap();
        assert_eq!(id, "snap1");

        let snapshot = manager.get("snap1").unwrap();
        assert_eq!(snapshot.entry_count(), 2);
        assert!(snapshot.contains("e1"));
        assert!(snapshot.contains("e2"));
    }

    #[test]
    fn test_snapshot_list() {
        let manager = SnapshotManager::new(100);
        manager
            .create(
                "snap1".to_string().to_string(),
                MemoryTier::Session,
                HashMap::new(),
                SnapshotMetadata::new(),
            )
            .unwrap();
        manager
            .create(
                "snap2".to_string().to_string(),
                MemoryTier::Project,
                HashMap::new(),
                SnapshotMetadata::new(),
            )
            .unwrap();

        let snapshots = manager.list();
        assert_eq!(snapshots.len(), 2);
    }

    #[test]
    fn test_snapshot_delete() {
        let manager = SnapshotManager::new(100);
        manager
            .create(
                "snap1".to_string(),
                MemoryTier::Session,
                HashMap::new(),
                SnapshotMetadata::new(),
            )
            .unwrap();

        manager.delete("snap1").unwrap();
        assert!(manager.get("snap1").is_none());
    }

    #[test]
    fn test_snapshot_delete_not_found() {
        let manager = SnapshotManager::new(100);
        let result = manager.delete("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_snapshot_merge() {
        let manager = SnapshotManager::new(100);

        let mut entries1 = HashMap::new();
        entries1.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1"),
        );
        entries1.insert(
            "e2".to_string(),
            test_entry("e2", MemoryTier::Session, "key", "value2"),
        );

        let mut entries2 = HashMap::new();
        entries2.insert(
            "e2".to_string(),
            test_entry("e2", MemoryTier::Session, "key", "value2_modified"),
        );
        entries2.insert(
            "e3".to_string(),
            test_entry("e3", MemoryTier::Session, "key", "value3"),
        );

        manager
            .create(
                "snap1".to_string(),
                MemoryTier::Session,
                entries1,
                SnapshotMetadata::new(),
            )
            .unwrap();
        manager
            .create(
                "snap2".to_string(),
                MemoryTier::Session,
                entries2,
                SnapshotMetadata::new(),
            )
            .unwrap();

        let merged = manager.merge("snap1", "snap2", "snap_merged").unwrap();
        assert_eq!(merged.entry_count(), 3);
        assert!(merged.contains("e1"));
        assert!(merged.contains("e2"));
        assert!(merged.contains("e3"));
    }

    #[test]
    fn test_snapshot_diff() {
        let manager = SnapshotManager::new(100);

        let mut entries1 = HashMap::new();
        entries1.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1"),
        );
        entries1.insert(
            "e2".to_string(),
            test_entry("e2", MemoryTier::Session, "key", "value2"),
        );

        let mut entries2 = HashMap::new();
        entries2.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1_modified"),
        );
        entries2.insert(
            "e3".to_string(),
            test_entry("e3", MemoryTier::Session, "key", "value3"),
        );

        manager
            .create(
                "snap1".to_string(),
                MemoryTier::Session,
                entries1,
                SnapshotMetadata::new(),
            )
            .unwrap();
        manager
            .create(
                "snap2".to_string(),
                MemoryTier::Session,
                entries2,
                SnapshotMetadata::new(),
            )
            .unwrap();

        let diff = manager.diff("snap1", "snap2").unwrap();
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 1);
    }

    #[test]
    fn test_snapshot_diff_empty() {
        let manager = SnapshotManager::new(100);

        let entries = HashMap::new();
        manager
            .create(
                "snap1".to_string(),
                MemoryTier::Session,
                entries.clone(),
                SnapshotMetadata::new(),
            )
            .unwrap();
        manager
            .create(
                "snap2".to_string(),
                MemoryTier::Session,
                entries,
                SnapshotMetadata::new(),
            )
            .unwrap();

        let diff = manager.diff("snap1", "snap2").unwrap();
        assert!(diff.is_empty());
    }

    #[test]
    fn test_snapshot_restore() {
        let manager = SnapshotManager::new(100);

        let mut entries = HashMap::new();
        entries.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1"),
        );

        manager
            .create(
                "snap1".to_string(),
                MemoryTier::Session,
                entries,
                SnapshotMetadata::new(),
            )
            .unwrap();

        let restored = manager.restore("snap1").unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, "e1");
    }

    #[test]
    fn test_snapshot_restore_not_found() {
        let manager = SnapshotManager::new(100);
        let result = manager.restore("nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_compute_diff() {
        let mut entries_a = HashMap::new();
        entries_a.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1"),
        );
        entries_a.insert(
            "e2".to_string(),
            test_entry("e2", MemoryTier::Session, "key", "value2"),
        );

        let mut entries_b = HashMap::new();
        entries_b.insert(
            "e1".to_string(),
            test_entry("e1", MemoryTier::Session, "key", "value1_modified"),
        );
        entries_b.insert(
            "e3".to_string(),
            test_entry("e3", MemoryTier::Session, "key", "value3"),
        );

        let snap_a = MemorySnapshot::new("a", MemoryTier::Session, entries_a);
        let snap_b = MemorySnapshot::new("b", MemoryTier::Session, entries_b);

        let diff = compute_diff(&snap_a, &snap_b);
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.modified.len(), 1);
    }

    #[test]
    fn test_snapshot_metadata() {
        let metadata = SnapshotMetadata::new()
            .with_description("Test snapshot")
            .with_tag("important")
            .with_source("manual");

        assert_eq!(metadata.description, Some("Test snapshot".to_string()));
        assert_eq!(metadata.tags, vec!["important"]);
        assert_eq!(metadata.source, Some("manual".to_string()));
    }

    #[test]
    fn test_snapshot_diff_summary() {
        let diff = SnapshotDiff::new("a", "b", vec![], vec!["e1".to_string()], vec![]);
        let summary = diff.summary();
        assert!(summary.contains("a"));
        assert!(summary.contains("b"));
        assert!(summary.contains("-1 entries"));
    }
}

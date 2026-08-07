use super::lifecycle::MemoryLifecycle;
use super::policy::{ConflictPolicy, EvictionPolicy, ExpirationPolicy, MemoryPolicy, PriorityPolicy, RetentionPolicy};
use super::resolution::MemoryResolver;
use super::snapshot::{MemorySnapshot, SnapshotDiff, SnapshotManager, SnapshotMetadata};
use super::tier_coordination::TierCoordinator;
use super::types::{MemoryEntry, MemoryEvent, MemoryMetadata, MemoryQuery, MemoryResolution, MemoryTier};
use super::MemoryRuntime;

// =============================================================================
// Types tests
// =============================================================================

#[test]
fn test_memory_tier_display() {
    assert_eq!(format!("{}", MemoryTier::Session), "session");
    assert_eq!(format!("{}", MemoryTier::Project), "project");
    assert_eq!(format!("{}", MemoryTier::Global), "global");
}

#[test]
fn test_memory_tier_from_str() {
    assert_eq!(MemoryTier::from_str("session"), Some(MemoryTier::Session));
    assert_eq!(MemoryTier::from_str("project"), Some(MemoryTier::Project));
    assert_eq!(MemoryTier::from_str("global"), Some(MemoryTier::Global));
    assert_eq!(MemoryTier::from_str("invalid"), None);
}

#[test]
fn test_memory_tier_resolution_order() {
    assert_eq!(MemoryTier::Session.resolution_order(), 0);
    assert_eq!(MemoryTier::Project.resolution_order(), 1);
    assert_eq!(MemoryTier::Global.resolution_order(), 2);
}

#[test]
fn test_memory_entry_creation() {
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    assert_eq!(entry.id, "e1");
    assert_eq!(entry.tier, MemoryTier::Session);
    assert_eq!(entry.key, "key");
    assert_eq!(entry.value, "value");
    assert_eq!(entry.access_count, 0);
}

#[test]
fn test_memory_entry_with_metadata() {
    let metadata = MemoryMetadata::new()
        .with_importance(0.9)
        .with_confidence(0.8)
        .with_tag("important")
        .with_source("test")
        .with_context("ctx");

    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value")
        .with_metadata(metadata);

    assert_eq!(entry.metadata.importance, 0.9);
    assert_eq!(entry.metadata.confidence, 0.8);
    assert_eq!(entry.metadata.tags, vec!["important"]);
    assert_eq!(entry.metadata.source, Some("test".to_string()));
    assert_eq!(entry.metadata.context, Some("ctx".to_string()));
}

#[test]
fn test_memory_entry_record_access() {
    let mut entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    assert_eq!(entry.access_count, 0);
    entry.record_access();
    assert_eq!(entry.access_count, 1);
    entry.record_access();
    assert_eq!(entry.access_count, 2);
}

#[test]
fn test_memory_entry_matches_key() {
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "language", "rust");
    assert!(entry.matches_key("language"));
    assert!(entry.matches_key("rust"));
    assert!(!entry.matches_key("python"));
}

#[test]
fn test_memory_metadata_default() {
    let metadata = MemoryMetadata::default();
    assert_eq!(metadata.importance, 0.5);
    assert_eq!(metadata.confidence, 0.5);
    assert!(metadata.tags.is_empty());
    assert!(metadata.source.is_none());
    assert!(metadata.context.is_none());
}

#[test]
fn test_memory_metadata_clamp_importance() {
    let metadata = MemoryMetadata::new().with_importance(1.5);
    assert_eq!(metadata.importance, 1.0);

    let metadata = MemoryMetadata::new().with_importance(-0.5);
    assert_eq!(metadata.importance, 0.0);
}

#[test]
fn test_memory_query_new() {
    let query = MemoryQuery::new("language");
    assert_eq!(query.key, "language");
    assert!(query.tier.is_none());
    assert_eq!(query.max_results, 10);
    assert!(query.require_confidence.is_none());
    assert!(query.tags.is_empty());
}

#[test]
fn test_memory_query_builder() {
    let query = MemoryQuery::new("key")
        .in_tier(MemoryTier::Project)
        .limit(5)
        .require_confidence(0.7)
        .with_tag("important");

    assert_eq!(query.tier, Some(MemoryTier::Project));
    assert_eq!(query.max_results, 5);
    assert_eq!(query.require_confidence, Some(0.7));
    assert_eq!(query.tags, vec!["important"]);
}

// =============================================================================
// Lifecycle tests
// =============================================================================

#[test]
fn test_lifecycle_create_and_get() {
    let lifecycle = MemoryLifecycle::new(100);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    let id = lifecycle.create(entry).unwrap();
    assert_eq!(id, "e1");

    let retrieved = lifecycle.get("e1").unwrap();
    assert_eq!(retrieved.id, "e1");
    assert_eq!(retrieved.access_count, 1);
}

#[test]
fn test_lifecycle_create_duplicate() {
    let lifecycle = MemoryLifecycle::new(100);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    lifecycle.create(entry.clone()).unwrap();
    let result = lifecycle.create(entry);
    assert!(result.is_err());
}

#[test]
fn test_lifecycle_update() {
    let lifecycle = MemoryLifecycle::new(100);
    lifecycle.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    lifecycle.update("e1", "new_value").unwrap();

    let entry = lifecycle.get("e1").unwrap();
    assert_eq!(entry.value, "new_value");
}

#[test]
fn test_lifecycle_update_not_found() {
    let lifecycle = MemoryLifecycle::new(100);
    let result = lifecycle.update("nonexistent", "value");
    assert!(result.is_err());
}

#[test]
fn test_lifecycle_delete() {
    let lifecycle = MemoryLifecycle::new(100);
    lifecycle.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    lifecycle.delete("e1").unwrap();
    assert!(lifecycle.get("e1").is_none());
}

#[test]
fn test_lifecycle_delete_not_found() {
    let lifecycle = MemoryLifecycle::new(100);
    let result = lifecycle.delete("nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_lifecycle_list_by_tier() {
    let lifecycle = MemoryLifecycle::new(100);
    lifecycle.create(MemoryEntry::new("s1", MemoryTier::Session, "k1", "v1")).unwrap();
    lifecycle.create(MemoryEntry::new("s2", MemoryTier::Session, "k2", "v2")).unwrap();
    lifecycle.create(MemoryEntry::new("p1", MemoryTier::Project, "k1", "v1")).unwrap();

    let session_entries = lifecycle.list_by_tier(MemoryTier::Session);
    assert_eq!(session_entries.len(), 2);

    let project_entries = lifecycle.list_by_tier(MemoryTier::Project);
    assert_eq!(project_entries.len(), 1);
}

#[test]
fn test_lifecycle_list_all() {
    let lifecycle = MemoryLifecycle::new(100);
    lifecycle.create(MemoryEntry::new("s1", MemoryTier::Session, "k1", "v1")).unwrap();
    lifecycle.create(MemoryEntry::new("p1", MemoryTier::Project, "k1", "v1")).unwrap();

    let all = lifecycle.list_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_lifecycle_count_by_tier() {
    let lifecycle = MemoryLifecycle::new(100);
    assert_eq!(lifecycle.count_by_tier(MemoryTier::Session), 0);

    lifecycle.create(MemoryEntry::new("s1", MemoryTier::Session, "k1", "v1")).unwrap();
    lifecycle.create(MemoryEntry::new("s2", MemoryTier::Session, "k2", "v2")).unwrap();
    assert_eq!(lifecycle.count_by_tier(MemoryTier::Session), 2);
}

#[test]
fn test_lifecycle_contains() {
    let lifecycle = MemoryLifecycle::new(100);
    assert!(!lifecycle.contains("e1"));

    lifecycle.create(MemoryEntry::new("e1", MemoryTier::Session, "k1", "v1")).unwrap();
    assert!(lifecycle.contains("e1"));
}

#[test]
fn test_lifecycle_clear() {
    let lifecycle = MemoryLifecycle::new(100);
    lifecycle.create(MemoryEntry::new("e1", MemoryTier::Session, "k1", "v1")).unwrap();
    lifecycle.create(MemoryEntry::new("e2", MemoryTier::Project, "k1", "v1")).unwrap();

    lifecycle.clear();
    assert_eq!(lifecycle.entry_count(), 0);
}

#[test]
fn test_lifecycle_record_event() {
    let lifecycle = MemoryLifecycle::new(100);
    lifecycle.record_event(MemoryEvent::MemoryResolved {
        event_id: "e1".to_string(),
        query_key: "key".to_string(),
        tier: MemoryTier::Session,
        hit_count: 1,
        timestamp: 0,
    });
    assert_eq!(lifecycle.events().len(), 1);
}

#[test]
fn test_lifecycle_event_limit() {
    let lifecycle = MemoryLifecycle::new(5);
    for i in 0..10 {
        lifecycle.record_event(MemoryEvent::MemoryResolved {
            event_id: format!("e{}", i),
            query_key: "key".to_string(),
            tier: MemoryTier::Session,
            hit_count: 1,
            timestamp: 0,
        });
    }
    assert_eq!(lifecycle.events().len(), 5);
}

// =============================================================================
// Resolution tests
// =============================================================================

#[test]
fn test_resolver_session_priority() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "session_val")).unwrap();
    lifecycle.create(MemoryEntry::new("p1", MemoryTier::Project, "key", "project_val")).unwrap();
    lifecycle.create(MemoryEntry::new("g1", MemoryTier::Global, "key", "global_val")).unwrap();

    let query = MemoryQuery::new("key");
    let resolution = resolver.resolve(&query);

    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].id, "s1");
}

#[test]
fn test_resolver_falls_back_to_project() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(MemoryEntry::new("p1", MemoryTier::Project, "key", "project_val")).unwrap();
    lifecycle.create(MemoryEntry::new("g1", MemoryTier::Global, "key", "global_val")).unwrap();

    let query = MemoryQuery::new("key");
    let resolution = resolver.resolve(&query);

    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].id, "p1");
}

#[test]
fn test_resolver_falls_back_to_global() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(MemoryEntry::new("g1", MemoryTier::Global, "key", "global_val")).unwrap();

    let query = MemoryQuery::new("key");
    let resolution = resolver.resolve(&query);

    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].id, "g1");
}

#[test]
fn test_resolver_no_match() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(MemoryEntry::new("g1", MemoryTier::Global, "key", "value")).unwrap();

    let query = MemoryQuery::new("nonexistent");
    let resolution = resolver.resolve(&query);

    assert!(resolution.is_empty());
    assert_eq!(resolution.misses.len(), 3);
}

#[test]
fn test_resolver_specific_tier() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "session_val")).unwrap();
    lifecycle.create(MemoryEntry::new("g1", MemoryTier::Global, "key", "global_val")).unwrap();

    let query = MemoryQuery::new("key").in_tier(MemoryTier::Global);
    let resolution = resolver.resolve(&query);

    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].tier, MemoryTier::Global);
}

#[test]
fn test_resolver_confidence_filter() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(
        MemoryEntry::new("low", MemoryTier::Session, "key", "value")
            .with_metadata(MemoryMetadata::new().with_confidence(0.3)),
    ).unwrap();
    lifecycle.create(
        MemoryEntry::new("high", MemoryTier::Session, "key", "value")
            .with_metadata(MemoryMetadata::new().with_confidence(0.9)),
    ).unwrap();

    let query = MemoryQuery::new("key").require_confidence(0.5);
    let resolution = resolver.resolve(&query);

    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].id, "high");
}

#[test]
fn test_resolver_tag_filter() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    lifecycle.create(
        MemoryEntry::new("tagged", MemoryTier::Session, "key", "value")
            .with_metadata(MemoryMetadata::new().with_tag("important")),
    ).unwrap();
    lifecycle.create(MemoryEntry::new("untagged", MemoryTier::Session, "key", "value")).unwrap();

    let query = MemoryQuery::new("key").with_tag("important");
    let resolution = resolver.resolve(&query);

    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].id, "tagged");
}

#[test]
fn test_resolver_max_results() {
    let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
    let resolver = MemoryResolver::new(lifecycle.clone());

    for i in 0..5 {
        lifecycle.create(MemoryEntry::new(&format!("e{}", i), MemoryTier::Session, "prefix", &format!("value_{}", i))).unwrap();
    }

    let query = MemoryQuery::new("prefix").limit(2);
    let resolution = resolver.resolve(&query);

    // First match wins (deterministic)
    assert_eq!(resolution.hits.len(), 1);
    assert_eq!(resolution.hits[0].id, "e0");
}

#[test]
fn test_resolver_deterministic_order() {
    let order = MemoryResolver::resolution_order();
    assert_eq!(order, vec![
        MemoryTier::Session,
        MemoryTier::Project,
        MemoryTier::Global,
    ]);
}

// =============================================================================
// Policy tests
// =============================================================================

#[test]
fn test_retention_policy_infinite() {
    let policy = MemoryPolicy::new().with_retention(RetentionPolicy::Infinite);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    assert!(!policy.should_evict(&entry));
}

#[test]
fn test_retention_policy_duration() {
    let policy = MemoryPolicy::new().with_retention(RetentionPolicy::Duration(
        std::time::Duration::from_secs(1),
    ));
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    assert!(!policy.should_evict(&entry));
}

#[test]
fn test_retention_policy_importance_threshold() {
    let policy = MemoryPolicy::new().with_retention(RetentionPolicy::ImportanceThreshold {
        threshold: 0.5,
    });
    let high_importance = MemoryEntry::new("e1", MemoryTier::Session, "key", "value")
        .with_metadata(MemoryMetadata::new().with_importance(0.8));
    let low_importance = MemoryEntry::new("e2", MemoryTier::Session, "key", "value")
        .with_metadata(MemoryMetadata::new().with_importance(0.3));

    assert!(!policy.should_evict(&high_importance));
    assert!(policy.should_evict(&low_importance));
}

#[test]
fn test_expiration_policy_none() {
    let policy = MemoryPolicy::new().with_expiration(ExpirationPolicy::None);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    assert!(!policy.is_expired(&entry));
}

#[test]
fn test_priority_policy_importance() {
    let policy = MemoryPolicy::new().with_priority(PriorityPolicy::Importance);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value")
        .with_metadata(MemoryMetadata::new().with_importance(0.8));
    assert_eq!(policy.priority_score(&entry), 0.8);
}

#[test]
fn test_priority_policy_recency() {
    let policy = MemoryPolicy::new().with_priority(PriorityPolicy::Recency);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    let score = policy.priority_score(&entry);
    assert!(score > 0.9);
}

#[test]
fn test_priority_policy_frequency() {
    let policy = MemoryPolicy::new().with_priority(PriorityPolicy::Frequency);
    let mut entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    entry.access_count = 5;
    assert_eq!(policy.priority_score(&entry), 0.5);
}

#[test]
fn test_access_rule_allow() {
    let rule = super::policy::AccessRule::new(MemoryTier::Session)
        .allow_key("language")
        .allow_key("framework");
    assert!(rule.matches("language", 0.9));
    assert!(rule.matches("framework", 0.9));
    assert!(!rule.matches("other", 0.9));
}

#[test]
fn test_access_rule_deny() {
    let rule = super::policy::AccessRule::new(MemoryTier::Session).deny_key("secret");
    assert!(!rule.matches("secret", 0.9));
    assert!(rule.matches("other", 0.9));
}

#[test]
fn test_access_rule_confidence() {
    let rule = super::policy::AccessRule::new(MemoryTier::Session)
        .with_min_confidence(0.5);
    assert!(!rule.matches("key", 0.3));
    assert!(rule.matches("key", 0.6));
}

#[test]
fn test_conflict_policy_first_match() {
    assert_eq!(ConflictPolicy::FirstMatch, ConflictPolicy::default());
}

#[test]
fn test_memory_policy_default() {
    let policy = MemoryPolicy::default();
    assert!(matches!(policy.retention, RetentionPolicy::Duration(_)));
    assert_eq!(policy.max_entries_per_tier, 1000);
    assert!(!policy.auto_consolidate);
}

#[test]
fn test_memory_policy_builder() {
    let policy = MemoryPolicy::new()
        .with_retention(RetentionPolicy::Infinite)
        .with_eviction(EvictionPolicy::LRU)
        .with_expiration(ExpirationPolicy::None)
        .with_priority(PriorityPolicy::Importance)
        .with_conflict_resolution(ConflictPolicy::HighestImportance)
        .with_max_entries(500)
        .with_auto_consolidate(true);

    assert!(matches!(policy.retention, RetentionPolicy::Infinite));
    assert_eq!(policy.max_entries_per_tier, 500);
    assert!(policy.auto_consolidate);
}

// =============================================================================
// Snapshot tests
// =============================================================================

#[test]
fn test_snapshot_create_and_get() {
    let manager = SnapshotManager::new(100);
    let mut entries = std::collections::HashMap::new();
    entries.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1"));
    entries.insert("e2".to_string(), MemoryEntry::new("e2", MemoryTier::Session, "key", "value2"));

    let id = manager
        .create("snap1", MemoryTier::Session, entries.clone(), SnapshotMetadata::new())
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
        .create("snap1", MemoryTier::Session, std::collections::HashMap::new(), SnapshotMetadata::new())
        .unwrap();
    manager
        .create("snap2", MemoryTier::Project, std::collections::HashMap::new(), SnapshotMetadata::new())
        .unwrap();

    let snapshots = manager.list();
    assert_eq!(snapshots.len(), 2);
}

#[test]
fn test_snapshot_delete() {
    let manager = SnapshotManager::new(100);
    manager
        .create("snap1", MemoryTier::Session, std::collections::HashMap::new(), SnapshotMetadata::new())
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

    let mut entries1 = std::collections::HashMap::new();
    entries1.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1"));
    entries1.insert("e2".to_string(), MemoryEntry::new("e2", MemoryTier::Session, "key", "value2"));

    let mut entries2 = std::collections::HashMap::new();
    entries2.insert("e2".to_string(), MemoryEntry::new("e2", MemoryTier::Session, "key", "value2_modified"));
    entries2.insert("e3".to_string(), MemoryEntry::new("e3", MemoryTier::Session, "key", "value3"));

    manager
        .create("snap1", MemoryTier::Session, entries1, SnapshotMetadata::new())
        .unwrap();
    manager
        .create("snap2", MemoryTier::Session, entries2, SnapshotMetadata::new())
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

    let mut entries1 = std::collections::HashMap::new();
    entries1.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1"));
    entries1.insert("e2".to_string(), MemoryEntry::new("e2", MemoryTier::Session, "key", "value2"));

    let mut entries2 = std::collections::HashMap::new();
    entries2.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1_modified"));
    entries2.insert("e3".to_string(), MemoryEntry::new("e3", MemoryTier::Session, "key", "value3"));

    manager
        .create("snap1", MemoryTier::Session, entries1, SnapshotMetadata::new())
        .unwrap();
    manager
        .create("snap2", MemoryTier::Session, entries2, SnapshotMetadata::new())
        .unwrap();

    let diff = manager.diff("snap1", "snap2").unwrap();
    assert_eq!(diff.added.len(), 1);
    assert_eq!(diff.removed.len(), 1);
    assert_eq!(diff.modified.len(), 1);
}

#[test]
fn test_snapshot_diff_empty() {
    let manager = SnapshotManager::new(100);

    let entries = std::collections::HashMap::new();
    manager
        .create("snap1", MemoryTier::Session, entries.clone(), SnapshotMetadata::new())
        .unwrap();
    manager
        .create("snap2", MemoryTier::Session, entries, SnapshotMetadata::new())
        .unwrap();

    let diff = manager.diff("snap1", "snap2").unwrap();
    assert!(diff.is_empty());
}

#[test]
fn test_snapshot_restore() {
    let manager = SnapshotManager::new(100);

    let mut entries = std::collections::HashMap::new();
    entries.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1"));

    manager
        .create("snap1", MemoryTier::Session, entries, SnapshotMetadata::new())
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
    let mut entries_a = std::collections::HashMap::new();
    entries_a.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1"));
    entries_a.insert("e2".to_string(), MemoryEntry::new("e2", MemoryTier::Session, "key", "value2"));

    let mut entries_b = std::collections::HashMap::new();
    entries_b.insert("e1".to_string(), MemoryEntry::new("e1", MemoryTier::Session, "key", "value1_modified"));
    entries_b.insert("e3".to_string(), MemoryEntry::new("e3", MemoryTier::Session, "key", "value3"));

    let snap_a = MemorySnapshot::new("a", MemoryTier::Session, entries_a);
    let snap_b = MemorySnapshot::new("b", MemoryTier::Session, entries_b);

    let diff = super::snapshot::compute_diff(&snap_a, &snap_b);
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

// =============================================================================
// Diagnostics tests
// =============================================================================

#[test]
fn test_diagnostics_record_hit() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_hit();
    assert_eq!(diag.summary().total_hits, 1);
}

#[test]
fn test_diagnostics_record_miss() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_miss();
    assert_eq!(diag.summary().total_misses, 1);
}

#[test]
fn test_diagnostics_hit_rate() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_hit();
    diag.record_hit();
    diag.record_hit();
    diag.record_miss();
    assert_eq!(diag.hit_rate(), 0.75);
}

#[test]
fn test_diagnostics_hit_rate_empty() {
    let diag = super::diagnostics::MemoryDiagnostics::new(100);
    assert_eq!(diag.hit_rate(), 0.0);
}

#[test]
fn test_diagnostics_eviction() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_eviction();
    assert_eq!(diag.summary().total_evictions, 1);
}

#[test]
fn test_diagnostics_snapshot_creation() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_snapshot_creation();
    assert_eq!(diag.summary().total_snapshot_creations, 1);
}

#[test]
fn test_diagnostics_snapshot_merge() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_snapshot_merge();
    assert_eq!(diag.summary().total_snapshot_merges, 1);
}

#[test]
fn test_diagnostics_policy_violation() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_policy_violation();
    assert_eq!(diag.summary().total_policy_violations, 1);
}

#[test]
fn test_diagnostics_resolution_latency() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_resolution_latency(10);
    diag.record_resolution_latency(20);
    diag.record_resolution_latency(30);
    assert_eq!(diag.avg_resolution_latency(), 20);
}

#[test]
fn test_diagnostics_avg_latency_empty() {
    let diag = super::diagnostics::MemoryDiagnostics::new(100);
    assert_eq!(diag.avg_resolution_latency(), 0);
}

#[test]
fn test_diagnostics_p95_latency() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    for i in 1..=100 {
        diag.record_resolution_latency(i);
    }
    let p95 = diag.p95_resolution_latency();
    assert!(p95 >= 90 && p95 <= 100);
}

#[test]
fn test_diagnostics_summary_healthy() {
    let diag = super::diagnostics::MemoryDiagnostics::new(100);
    assert!(diag.summary().is_healthy());
}

#[test]
fn test_diagnostics_summary_unhealthy() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_policy_violation();
    assert!(!diag.summary().is_healthy());
}

#[test]
fn test_diagnostics_clear() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_hit();
    diag.record_miss();
    diag.clear();
    assert_eq!(diag.summary().total_hits, 0);
    assert_eq!(diag.summary().total_misses, 0);
}

#[test]
fn test_diagnostics_record_event() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(100);
    diag.record_event(MemoryEvent::MemoryResolved {
        event_id: "e1".to_string(),
        query_key: "key".to_string(),
        tier: MemoryTier::Session,
        hit_count: 1,
        timestamp: 0,
    });
    assert_eq!(diag.events().len(), 1);
}

#[test]
fn test_diagnostics_event_limit() {
    let mut diag = super::diagnostics::MemoryDiagnostics::new(5);
    for i in 0..10 {
        diag.record_event(MemoryEvent::MemoryResolved {
            event_id: format!("e{}", i),
            query_key: "key".to_string(),
            tier: MemoryTier::Session,
            hit_count: 1,
            timestamp: 0,
        });
    }
    assert_eq!(diag.events().len(), 5);
}

// =============================================================================
// Tier Coordination tests
// =============================================================================

#[test]
fn test_tier_coordinator_create_and_get() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    let id = coordinator.create(entry).unwrap();
    assert_eq!(id, "e1");

    let retrieved = coordinator.get("e1").unwrap();
    assert_eq!(retrieved.key, "key");
    assert_eq!(retrieved.value, "value");
}

#[test]
fn test_tier_coordinator_create_duplicate() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    coordinator.create(entry.clone()).unwrap();
    let result = coordinator.create(entry);
    assert!(result.is_err());
}

#[test]
fn test_tier_coordinator_update() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    coordinator.update("e1", "new_value").unwrap();

    let entry = coordinator.get("e1").unwrap();
    assert_eq!(entry.value, "new_value");
}

#[test]
fn test_tier_coordinator_delete() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    coordinator.delete("e1").unwrap();
    assert!(coordinator.get("e1").is_none());
}

#[test]
fn test_tier_coordinator_delete_not_found() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    let result = coordinator.delete("nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_tier_coordinator_list_by_tier() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "value")).unwrap();
    coordinator.create(MemoryEntry::new("p1", MemoryTier::Project, "key", "value")).unwrap();
    coordinator.create(MemoryEntry::new("g1", MemoryTier::Global, "key", "value")).unwrap();

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
fn test_tier_coordinator_list_all() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "value")).unwrap();
    coordinator.create(MemoryEntry::new("p1", MemoryTier::Project, "key", "value")).unwrap();

    let all = coordinator.list_all();
    assert_eq!(all.len(), 2);
}

#[test]
fn test_tier_coordinator_snapshot() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "value")).unwrap();

    let snapshot_id = coordinator.snapshot_tier("snap1", MemoryTier::Session).unwrap();
    assert_eq!(snapshot_id, "snap1");

    let snapshot = coordinator.restore_from_snapshot("snap1").unwrap();
    assert_eq!(snapshot.len(), 1);
}

#[test]
fn test_tier_coordinator_access_denied() {
    let policy = MemoryPolicy::new().with_access_rule(
        super::policy::AccessRule::new(MemoryTier::Session).deny_key("secret"),
    );
    let coordinator = TierCoordinator::new(policy);
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "secret", "value");
    let result = coordinator.create(entry);
    assert!(result.is_err());
}

#[test]
fn test_tier_coordinator_entry_count() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    assert_eq!(coordinator.entry_count(), 0);

    coordinator.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    assert_eq!(coordinator.entry_count(), 1);
}

#[test]
fn test_tier_coordinator_entry_count_by_tier() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "value")).unwrap();
    coordinator.create(MemoryEntry::new("s2", MemoryTier::Session, "key", "value")).unwrap();

    assert_eq!(coordinator.entry_count_by_tier(MemoryTier::Session), 2);
    assert_eq!(coordinator.entry_count_by_tier(MemoryTier::Project), 0);
}

#[test]
fn test_tier_coordinator_diagnostics() {
    let coordinator = TierCoordinator::new(MemoryPolicy::default());
    coordinator.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();

    let summary = coordinator.diagnostics();
    assert!(summary.total_hits >= 0);
}

// =============================================================================
// MemoryRuntime integration tests
// =============================================================================

#[test]
fn test_memory_runtime_creation() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    assert_eq!(runtime.entry_count(), 0);
}

#[test]
fn test_memory_runtime_create_and_get() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    let entry = MemoryEntry::new("e1", MemoryTier::Session, "key", "value");
    let id = runtime.create(entry).unwrap();
    assert_eq!(id, "e1");

    let retrieved = runtime.get("e1").unwrap();
    assert_eq!(retrieved.key, "key");
}

#[test]
fn test_memory_runtime_resolve() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("s1", MemoryTier::Session, "language", "rust")).unwrap();
    runtime.create(MemoryEntry::new("p1", MemoryTier::Project, "language", "python")).unwrap();

    let query = MemoryQuery::new("language");
    let resolution = runtime.resolve(&query);
    // First match wins (deterministic)
    assert_eq!(resolution.hits.len(), 1);
}

#[test]
fn test_memory_runtime_snapshot() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();

    let snap_id = runtime.snapshot("snap1", MemoryTier::Session).unwrap();
    assert_eq!(snap_id, "snap1");

    let restored = runtime.restore("snap1").unwrap();
    assert_eq!(restored.len(), 1);
}

#[test]
fn test_memory_runtime_diagnostics() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();

    let diag = runtime.diagnostics();
    assert!(diag.total_hits >= 0);
}

#[test]
fn test_memory_runtime_debug() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    let debug = format!("{:?}", runtime);
    assert!(debug.contains("MemoryRuntime"));
}

#[test]
fn test_memory_runtime_list_by_tier() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "value")).unwrap();
    runtime.create(MemoryEntry::new("p1", MemoryTier::Project, "key", "value")).unwrap();

    let session = runtime.list_by_tier(MemoryTier::Session);
    assert_eq!(session.len(), 1);

    let project = runtime.list_by_tier(MemoryTier::Project);
    assert_eq!(project.len(), 1);
}

#[test]
fn test_memory_runtime_update() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    runtime.update("e1", "new_value").unwrap();

    let entry = runtime.get("e1").unwrap();
    assert_eq!(entry.value, "new_value");
}

#[test]
fn test_memory_runtime_delete() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();
    runtime.delete("e1").unwrap();
    assert!(runtime.get("e1").is_none());
}

#[test]
fn test_memory_runtime_resolve_no_match() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    let query = MemoryQuery::new("nonexistent");
    let resolution = runtime.resolve(&query);
    assert!(resolution.is_empty());
}

#[test]
fn test_memory_runtime_snapshot_diff() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value1")).unwrap();
    runtime.snapshot("snap1", MemoryTier::Session).unwrap();

    runtime.update("e1", "value2").unwrap();
    runtime.create(MemoryEntry::new("e2", MemoryTier::Session, "key2", "value")).unwrap();
    runtime.snapshot("snap2", MemoryTier::Session).unwrap();

    let diff = runtime.diff_snapshots("snap1", "snap2").unwrap();
    assert!(!diff.is_empty());
}

#[test]
fn test_memory_runtime_apply_retention() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("e1", MemoryTier::Session, "key", "value")).unwrap();

    let evicted = runtime.apply_retention().unwrap();
    assert_eq!(evicted, 0);
}

#[test]
fn test_memory_runtime_entry_count_by_tier() {
    let runtime = MemoryRuntime::new(MemoryPolicy::default());
    runtime.create(MemoryEntry::new("s1", MemoryTier::Session, "key", "value")).unwrap();
    runtime.create(MemoryEntry::new("s2", MemoryTier::Session, "key", "value")).unwrap();

    assert_eq!(runtime.entry_count_by_tier(MemoryTier::Session), 2);
    assert_eq!(runtime.entry_count_by_tier(MemoryTier::Project), 0);
}

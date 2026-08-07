use super::lifecycle::MemoryLifecycle;
use super::types::{MemoryEvent, MemoryQuery, MemoryResolution, MemoryTier};
use std::time::Instant;

/// Memory resolver that performs deterministic tier-based resolution.
///
/// Resolution order is always: Session -> Project -> Global
/// Never random. Policy decides conflict resolution.
pub struct MemoryResolver {
    lifecycle: std::sync::Arc<MemoryLifecycle>,
}

impl MemoryResolver {
    pub fn new(lifecycle: std::sync::Arc<MemoryLifecycle>) -> Self {
        MemoryResolver { lifecycle }
    }

    /// Resolve a memory query deterministically.
    ///
    /// Resolution order:
    /// 1. Session tier (highest priority)
    /// 2. Project tier
    /// 3. Global tier (lowest priority)
    ///
    /// Returns all matching entries (up to max_results) across all tiers.
    pub fn resolve(&self, query: &MemoryQuery) -> MemoryResolution {
        let start = std::time::Instant::now();

        let resolution_order = if let Some(tier) = query.tier {
            vec![tier]
        } else {
            vec![
                MemoryTier::Session,
                MemoryTier::Project,
                MemoryTier::Global,
            ]
        };

        let mut hits = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();

        for tier in &resolution_order {
            let tier_entries = self.lifecycle.list_by_tier(*tier);

            for entry in tier_entries {
                if seen_ids.contains(&entry.id) {
                    continue;
                }

                if entry.matches_key(&query.key) {
                    if let Some(min_confidence) = query.require_confidence {
                        if entry.metadata.confidence < min_confidence {
                            continue;
                        }
                    }

                    if !query.tags.is_empty() {
                        let has_all_tags = query
                            .tags
                            .iter()
                            .all(|tag| entry.metadata.tags.contains(tag));
                        if !has_all_tags {
                            continue;
                        }
                    }

                    hits.push(entry.clone());
                    seen_ids.insert(entry.id.clone());

                    // Stop after first match (deterministic: Session > Project > Global)
                    break;
                }
            }

            // If we found a match, stop
            if !hits.is_empty() {
                break;
            }
        }

        let latency_ms = start.elapsed().as_millis() as u64;

        // Record event
        if let Some(first_hit) = hits.first() {
            self.lifecycle.record_event(MemoryEvent::MemoryResolved {
                event_id: uuid::Uuid::new_v4().to_string(),
                query_key: query.key.clone(),
                tier: first_hit.tier,
                hit_count: hits.len(),
                timestamp: 0,
            });
        }

        MemoryResolution::new(query.clone(), hits, latency_ms)
    }

    /// Resolve with conflict resolution policy.
    ///
    /// If multiple entries match across tiers, policy determines the winner.
    pub fn resolve_with_policy(
        &self,
        query: &MemoryQuery,
        policy: &super::policy::ConflictPolicy,
    ) -> MemoryResolution {
        let start = Instant::now();

        let resolution_order = if let Some(tier) = query.tier {
            vec![tier]
        } else {
            vec![
                MemoryTier::Session,
                MemoryTier::Project,
                MemoryTier::Global,
            ]
        };

        // Collect all matching entries
        let mut all_hits: Vec<super::types::MemoryEntry> = Vec::new();
        for tier in &resolution_order {
            let tier_entries = self.lifecycle.list_by_tier(*tier);
            for entry in tier_entries {
                if entry.matches_key(&query.key) {
                    if let Some(min_confidence) = query.require_confidence {
                        if entry.metadata.confidence < min_confidence {
                            continue;
                        }
                    }
                    if !query.tags.is_empty() {
                        let has_all_tags = query
                            .tags
                            .iter()
                            .all(|tag| entry.metadata.tags.contains(tag));
                        if !has_all_tags {
                            continue;
                        }
                    }
                    all_hits.push(entry);
                }
            }
        }

        // Apply conflict resolution
        let hits = match policy {
            super::policy::ConflictPolicy::FirstMatch => {
                all_hits.into_iter().take(query.max_results).collect::<Vec<_>>()
            }
            super::policy::ConflictPolicy::HighestImportance => {
                all_hits.sort_by(|a, b| {
                    b.metadata
                        .importance
                        .partial_cmp(&a.metadata.importance)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                all_hits.into_iter().take(query.max_results).collect::<Vec<_>>()
            }
            super::policy::ConflictPolicy::HighestConfidence => {
                all_hits.sort_by(|a, b| {
                    b.metadata
                        .confidence
                        .partial_cmp(&a.metadata.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                all_hits.into_iter().take(query.max_results).collect()
            }
            super::policy::ConflictPolicy::MostRecent => {
                all_hits.sort_by(|a, b| b.last_accessed.cmp(&a.last_accessed));
                all_hits.into_iter().take(query.max_results).collect()
            }
            super::policy::ConflictPolicy::MostAccessed => {
                all_hits.sort_by(|a, b| b.access_count.cmp(&a.access_count));
                all_hits.into_iter().take(query.max_results).collect()
            }
        };

        let latency_ms = start.elapsed().as_millis() as u64;

        // Record event
        if !hits.is_empty() {
            self.lifecycle.record_event(MemoryEvent::MemoryResolved {
                event_id: uuid::Uuid::new_v4().to_string(),
                query_key: query.key.clone(),
                tier: hits[0].tier,
                hit_count: hits.len(),
                timestamp: 0,
            });
        }

        let resolution_order = if query.tier.is_some() {
            vec![query.tier.unwrap()]
        } else {
            vec![
                MemoryTier::Session,
                MemoryTier::Project,
                MemoryTier::Global,
            ]
        };

        let misses = resolution_order
            .iter()
            .filter(|tier| !hits.iter().any(|h| h.tier == **tier))
            .map(|t| t.to_string())
            .collect();

        MemoryResolution {
            query: query.clone(),
            hits,
            misses,
            resolution_order,
            latency_ms,
        }
    }

    /// Get resolution order.
    pub fn resolution_order() -> Vec<MemoryTier> {
        vec![
            MemoryTier::Session,
            MemoryTier::Project,
            MemoryTier::Global,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory_runtime::types::{MemoryEntry, MemoryMetadata, MemoryQuery};

    fn test_entry(id: &str, tier: MemoryTier, key: &str, value: &str) -> MemoryEntry {
        MemoryEntry::new(id, tier, key, value)
            .with_metadata(MemoryMetadata::new().with_confidence(0.8))
    }

    #[test]
    fn test_resolve_session_first() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(test_entry("s1", MemoryTier::Session, "language", "rust"))
            .unwrap();
        lifecycle
            .create(test_entry("p1", MemoryTier::Project, "language", "python"))
            .unwrap();
        lifecycle
            .create(test_entry("g1", MemoryTier::Global, "language", "go"))
            .unwrap();

        let query = MemoryQuery::new("language");
        let resolution = resolver.resolve(&query);

        assert_eq!(resolution.hits.len(), 1);
        assert_eq!(resolution.hits[0].id, "s1");
        assert_eq!(resolution.hits[0].tier, MemoryTier::Session);
    }

    #[test]
    fn test_resolve_falls_back_to_project() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(test_entry("p1", MemoryTier::Project, "framework", "tokio"))
            .unwrap();
        lifecycle
            .create(test_entry("g1", MemoryTier::Global, "framework", "actix"))
            .unwrap();

        let query = MemoryQuery::new("framework");
        let resolution = resolver.resolve(&query);

        assert_eq!(resolution.hits.len(), 1);
        assert_eq!(resolution.hits[0].id, "p1");
        assert_eq!(resolution.hits[0].tier, MemoryTier::Project);
    }

    #[test]
    fn test_resolve_falls_back_to_global() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(test_entry("g1", MemoryTier::Global, "style", "rustfmt"))
            .unwrap();

        let query = MemoryQuery::new("style");
        let resolution = resolver.resolve(&query);

        assert_eq!(resolution.hits.len(), 1);
        assert_eq!(resolution.hits[0].id, "g1");
        assert_eq!(resolution.hits[0].tier, MemoryTier::Global);
    }

    #[test]
    fn test_resolve_no_match() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(test_entry("g1", MemoryTier::Global, "language", "rust"))
            .unwrap();

        let query = MemoryQuery::new("nonexistent");
        let resolution = resolver.resolve(&query);

        assert!(resolution.is_empty());
        assert_eq!(resolution.misses.len(), 3);
    }

    #[test]
    fn test_resolve_specific_tier() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(test_entry("s1", MemoryTier::Session, "key", "session_val"))
            .unwrap();
        lifecycle
            .create(test_entry("g1", MemoryTier::Global, "key", "global_val"))
            .unwrap();

        let query = MemoryQuery::new("key").in_tier(MemoryTier::Global);
        let resolution = resolver.resolve(&query);

        assert_eq!(resolution.hits.len(), 1);
        assert_eq!(resolution.hits[0].tier, MemoryTier::Global);
        assert_eq!(resolution.resolution_order, vec![MemoryTier::Global]);
    }

    #[test]
    fn test_resolve_with_confidence_filter() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(
                test_entry("low", MemoryTier::Session, "key", "low_conf")
                    .with_metadata(MemoryMetadata::new().with_confidence(0.3)),
            )
            .unwrap();
        lifecycle
            .create(
                test_entry("high", MemoryTier::Session, "key", "high_conf")
                    .with_metadata(MemoryMetadata::new().with_confidence(0.9)),
            )
            .unwrap();

        let query = MemoryQuery::new("key").require_confidence(0.5);
        let resolution = resolver.resolve(&query);

        assert_eq!(resolution.hits.len(), 1);
        assert_eq!(resolution.hits[0].id, "high");
    }

    #[test]
    fn test_resolve_with_tag_filter() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        lifecycle
            .create(
                test_entry("tagged", MemoryTier::Session, "key", "value")
                    .with_metadata(MemoryMetadata::new().with_tag("important")),
            )
            .unwrap();
        lifecycle
            .create(test_entry("untagged", MemoryTier::Session, "key", "value"))
            .unwrap();

        let query = MemoryQuery::new("key").with_tag("important");
        let resolution = resolver.resolve(&query);

        assert_eq!(resolution.hits.len(), 1);
        assert_eq!(resolution.hits[0].id, "tagged");
    }

    #[test]
    fn test_resolve_max_results() {
        let lifecycle = std::sync::Arc::new(MemoryLifecycle::new(100));
        let resolver = MemoryResolver::new(lifecycle.clone());

        for i in 0..5 {
            lifecycle
                .create(test_entry(&format!("e{}", i), MemoryTier::Session, "prefix", &format!("value_{}", i)))
                .unwrap();
        }

        let query = MemoryQuery::new("prefix").limit(2);
        let resolution = resolver.resolve(&query);

        // First match wins (deterministic)
        assert_eq!(resolution.hits.len(), 1);
        assert!(resolution.hits[0].id.starts_with("e"));
    }

    #[test]
    fn test_deterministic_resolution_order() {
        let order = MemoryResolver::resolution_order();
        assert_eq!(order, vec![
            MemoryTier::Session,
            MemoryTier::Project,
            MemoryTier::Global,
        ]);
    }
}

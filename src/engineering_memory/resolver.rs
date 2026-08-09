//! Deterministic resolver for engineering memory.
//!
//! Given a set of project-tier entries, produces a ranked, filtered,
//! budget-capped `EngineeringMemoryContext` suitable for injection into
//! `EngineeringContext`.

use super::types::{
    EngineeringMemoryEntry, EngineeringMemoryMetadata, EngineeringMemoryResolveError,
    EngineeringMemoryResolveResult,
};
use crate::engineering_context::memory::{
    EngineeringMemoryContext, MemoryEntry as ContextMemoryEntry, MemoryTier as ContextMemoryTier,
};

/// Fixed budgets for memory resolution.
pub const DEFAULT_MAX_ENTRIES: usize = 20;
pub const DEFAULT_TOKEN_BUDGET: usize = 500;
pub const DEFAULT_MIN_CONFIDENCE: f64 = 0.3;

/// Deterministic resolver for engineering memory entries.
#[derive(Debug, Clone)]
pub struct EngineeringMemoryResolver {
    max_entries: usize,
    token_budget: usize,
    min_confidence: f64,
}

impl Default for EngineeringMemoryResolver {
    fn default() -> Self {
        EngineeringMemoryResolver {
            max_entries: DEFAULT_MAX_ENTRIES,
            token_budget: DEFAULT_TOKEN_BUDGET,
            min_confidence: DEFAULT_MIN_CONFIDENCE,
        }
    }
}

impl EngineeringMemoryResolver {
    /// Create a resolver with explicit budgets.
    pub fn new(max_entries: usize, token_budget: usize, min_confidence: f64) -> Self {
        EngineeringMemoryResolver {
            max_entries,
            token_budget,
            min_confidence,
        }
    }

    /// Resolve a task query against a set of project-tier entries.
    ///
    /// Resolution pipeline:
    /// 1. Filter entries whose key or value matches any task keyword.
    /// 2. Filter entries that carry at least one active-file tag.
    /// 3. Filter entries below `min_confidence`.
    /// 4. Rank by importance desc, confidence desc, key asc, id asc.
    /// 5. Enforce entry budget (`max_entries`) and token budget.
    /// 6. Map selected entries into `EngineeringMemoryContext`.
    pub fn resolve(
        &self,
        entries: &[EngineeringMemoryEntry],
        task_keywords: &[String],
        active_file_tags: &[String],
    ) -> EngineeringMemoryResolveResult<EngineeringMemoryContext> {
        // Step 1: filter by task keywords.
        let keyword_matches: Vec<&EngineeringMemoryEntry> = if task_keywords.is_empty() {
            entries.iter().collect()
        } else {
            entries
                .iter()
                .filter(|e| task_keywords.iter().any(|kw| e.matches_keyword(kw)))
                .collect()
        };

        // Step 2: filter by active-file tags.
        let tag_matches: Vec<&EngineeringMemoryEntry> = if active_file_tags.is_empty() {
            keyword_matches
        } else {
            keyword_matches
                .into_iter()
                .filter(|e| e.matches_tags(active_file_tags))
                .collect()
        };

        // Step 3: filter by minimum confidence.
        let confidence_matches: Vec<&EngineeringMemoryEntry> = tag_matches
            .into_iter()
            .filter(|e| e.metadata.confidence >= self.min_confidence)
            .collect();

        if confidence_matches.is_empty() {
            return Err(EngineeringMemoryResolveError::NoMatches);
        }

        // Step 4: rank deterministically.
        let mut ranked: Vec<&EngineeringMemoryEntry> = confidence_matches;
        ranked.sort_by(|a, b| {
            b.metadata
                .importance
                .partial_cmp(&a.metadata.importance)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    b.metadata
                        .confidence
                        .partial_cmp(&a.metadata.confidence)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.id.cmp(&b.id))
        });

        // Step 5: enforce entry budget.
        let budgeted: Vec<&EngineeringMemoryEntry> =
            ranked.into_iter().take(self.max_entries).collect();

        // Step 6: enforce token budget and map.
        let mut selected = Vec::new();
        let mut tokens_used: usize = 0;

        for entry in budgeted {
            let text = format!("{}: {}", entry.key, entry.value);
            let entry_tokens = text.len() / 4;
            if tokens_used + entry_tokens > self.token_budget {
                continue;
            }
            tokens_used += entry_tokens;
            selected.push(ContextMemoryEntry {
                key: entry.key.clone(),
                value: entry.value.clone(),
                confidence: entry.metadata.confidence,
                tier: ContextMemoryTier::Project,
            });
        }

        if selected.is_empty() {
            return Err(EngineeringMemoryResolveError::NoMatches);
        }

        let budget_remaining = self.token_budget.saturating_sub(tokens_used);

        Ok(EngineeringMemoryContext::new()
            .with_entries(selected)
            .with_budget(budget_remaining))
    }

    /// Resolve with default budgets.
    pub fn resolve_default(
        &self,
        entries: &[EngineeringMemoryEntry],
        task_keywords: &[String],
        active_file_tags: &[String],
    ) -> EngineeringMemoryResolveResult<EngineeringMemoryContext> {
        self.resolve(entries, task_keywords, active_file_tags)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_memory::types::{EngineeringMemoryEntry, EngineeringMemoryMetadata};

    fn entry(
        id: &str,
        key: &str,
        value: &str,
        confidence: f64,
        importance: f64,
        tags: &[String],
    ) -> EngineeringMemoryEntry {
        let mut meta = EngineeringMemoryMetadata::new()
            .with_confidence(confidence)
            .with_importance(importance);
        for tag in tags {
            meta = meta.with_tag(tag.as_str());
        }
        EngineeringMemoryEntry::new(id, key, value).with_metadata(meta)
    }

    #[test]
    fn test_resolve_empty_entries() {
        let resolver = EngineeringMemoryResolver::default();
        let result = resolver.resolve(&[], &["auth".to_string()], &[]);
        assert!(matches!(
            result,
            Err(EngineeringMemoryResolveError::NoMatches)
        ));
    }

    #[test]
    fn test_resolve_no_keywords_returns_all_matching_confidence() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e1", "language", "rust", 0.9, 0.8, &[]),
            entry("e2", "framework", "axum", 0.7, 0.6, &[]),
        ];
        let result = resolver.resolve(&entries, &[], &[]).expect("resolve");
        assert_eq!(result.entries.len(), 2);
        // Sorted by key ascending.
        assert_eq!(result.entries[0].key, "framework");
        assert_eq!(result.entries[1].key, "language");
    }

    #[test]
    fn test_resolve_filters_by_keyword() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e1", "language", "rust", 0.9, 0.8, &[]),
            entry("e2", "auth_module", "jwt based", 0.8, 0.7, &[]),
            entry("e3", "database", "postgres", 0.85, 0.9, &[]),
        ];
        let result = resolver
            .resolve(&entries, &["auth".to_string()], &[])
            .expect("resolve");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].key, "auth_module");
    }

    #[test]
    fn test_resolve_filters_by_tag() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e1", "lang", "rust", 0.9, 0.8, &["frontend".to_string()]),
            entry("e2", "auth", "jwt", 0.8, 0.7, &["backend".to_string()]),
        ];
        // Only entries tagged "backend" should match.
        let result = resolver
            .resolve(&entries, &[], &["backend".to_string()])
            .expect("resolve");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].key, "auth");
    }

    #[test]
    fn test_resolve_filters_by_confidence() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e1", "low", "value", 0.2, 0.5, &[]),
            entry("e2", "high", "value", 0.9, 0.8, &[]),
        ];
        let result = resolver.resolve(&entries, &[], &[]).expect("resolve");
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].key, "high");
    }

    #[test]
    fn test_resolve_ranking_order() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e_b", "b_key", "val", 0.8, 0.5, &[]),
            entry("e_a", "a_key", "val", 0.9, 0.5, &[]),
            entry("e_c", "c_key", "val", 0.9, 0.9, &[]),
        ];
        let result = resolver.resolve(&entries, &[], &[]).expect("resolve");
        // with_entries sorts by key ascending for deterministic output.
        assert_eq!(result.entries[0].key, "a_key");
        assert_eq!(result.entries[1].key, "b_key");
        assert_eq!(result.entries[2].key, "c_key");
    }

    #[test]
    fn test_resolve_enforces_entry_budget() {
        let resolver = EngineeringMemoryResolver::new(2, 10000, 0.0);
        let entries = vec![
            entry("e1", "a", "v1", 0.9, 0.9, &[]),
            entry("e2", "b", "v2", 0.9, 0.8, &[]),
            entry("e3", "c", "v3", 0.9, 0.7, &[]),
        ];
        let result = resolver.resolve(&entries, &[], &[]).expect("resolve");
        assert_eq!(result.entries.len(), 2);
    }

    #[test]
    fn test_resolve_enforces_token_budget() {
        let resolver = EngineeringMemoryResolver::new(100, 10, 0.0);
        let entries = vec![
            entry("e1", "short", "x", 0.9, 0.9, &[]),
            entry("e2", "very_long_key", &"y".repeat(100), 0.9, 0.8, &[]),
        ];
        let result = resolver.resolve(&entries, &[], &[]).expect("resolve");
        // "short: x" = 6 chars = 1 token; "very_long_key: yyy..." >> 10 tokens
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].key, "short");
    }

    #[test]
    fn test_resolve_deterministic_same_inputs() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e2", "b_key", "val", 0.8, 0.5, &[]),
            entry("e1", "a_key", "val", 0.9, 0.5, &[]),
        ];
        let r1 = resolver.resolve(&entries, &[], &[]).expect("resolve");
        let r2 = resolver.resolve(&entries, &[], &[]).expect("resolve");
        assert_eq!(r1.entries, r2.entries);
    }

    #[test]
    fn test_resolve_with_both_keyword_and_tag_filters() {
        let resolver = EngineeringMemoryResolver::default();
        let entries = vec![
            entry("e1", "auth", "jwt", 0.9, 0.8, &["security".to_string()]),
            entry("e2", "auth", "oauth", 0.8, 0.7, &["security".to_string()]),
            entry("e3", "auth", "basic", 0.9, 0.9, &["legacy".to_string()]),
        ];
        let result = resolver
            .resolve(&entries, &["jwt".to_string()], &["security".to_string()])
            .expect("resolve");
        // Only e1 matches keyword "jwt" AND tag "security".
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].key, "auth");
        assert_eq!(result.entries[0].value, "jwt");
    }
}

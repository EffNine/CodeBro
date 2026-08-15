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

/// Marker appended to a value that was shortened to fit the response budget.
pub const TRUNCATION_MARKER: &str = "\n…[truncated for memory budget]";

/// Hard cap on a single excerpt (chars). Guards against pathological inputs;
/// the effective cap is `min(remaining_budget * 4, EXCERPT_MAX_CHARS)`.
const EXCERPT_MAX_CHARS: usize = 2000;

/// Minimum remaining budget (chars) required before we emit an excerpt for an
/// oversized entry. Below this the fragment would be too short to be useful,
/// so resolution stops instead.
const MIN_EXCERPT_CHARS: usize = 64;

/// Deterministically shorten `text` to at most `max_chars` bytes/chars,
/// preferring a paragraph/sentence boundary so a recorded decision stays
/// readable instead of being cut mid-word.
///
/// Returns `(excerpt, was_truncated)`. The excerpt is a prefix of `text`; when
/// truncated it is a prefix ending at the last sentence/line boundary found
/// before the cap, or the hard cap itself if no boundary exists.
fn bounded_excerpt(text: &str, max_chars: usize) -> (String, bool) {
    let max = max_chars.min(EXCERPT_MAX_CHARS);
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let safe_max = text.floor_char_boundary(max);
    let head = &text[..safe_max];

    // Prefer the last paragraph/line/sentence boundary within the head so the
    // excerpt ends cleanly. Longer boundaries are preferred over shorter ones.
    const BOUNDARIES: [&str; 6] = ["\n\n", "\n", ". ", ".\n", "; ", ", "];
    let mut best: Option<usize> = None;
    for b in BOUNDARIES {
        if let Some(pos) = head.rfind(b) {
            let end = pos + b.len();
            if end <= safe_max {
                best = Some(best.map_or(end, |cur| end.max(cur)));
            }
        }
    }

    // Only use the boundary if it lets us keep a meaningful portion of the
    // excerpt; otherwise the head is one long sentence and a hard cut (with the
    // explicit marker) is the deterministic fallback.
    match best {
        Some(end) if end >= safe_max.saturating_div(2) => {
            (text[..end].trim_end().to_string(), true)
        }
        _ => (text[..safe_max].trim_end().to_string(), true),
    }
}

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
        //
        // The token budget is a *total response budget* shared by all selected
        // entries. Entries that fit are included in full; an entry that does
        // not fit is never silently dropped — it is included as a bounded,
        // deterministic excerpt so highly relevant memory does not disappear
        // merely because one entry is larger than the budget.
        let mut selected = Vec::new();
        let mut tokens_used: usize = 0;

        for entry in budgeted {
            let text = format!("{}: {}", entry.key, entry.value);
            let entry_tokens = text.len() / 4;
            let remaining = self.token_budget.saturating_sub(tokens_used);

            let fits = entry_tokens <= remaining;
            if fits {
                tokens_used += entry_tokens;
                selected.push(ContextMemoryEntry {
                    key: entry.key.clone(),
                    value: entry.value.clone(),
                    confidence: entry.metadata.confidence,
                    tier: ContextMemoryTier::Project,
                });
                continue;
            }

            // Oversized entry: keep metadata, bound the content.
            if remaining >= MIN_EXCERPT_CHARS {
                let (excerpt, truncated) = bounded_excerpt(&entry.value, remaining * 4);
                let mut value = excerpt;
                if truncated {
                    value.push_str(TRUNCATION_MARKER);
                }
                selected.push(ContextMemoryEntry {
                    key: entry.key.clone(),
                    value,
                    confidence: entry.metadata.confidence,
                    tier: ContextMemoryTier::Project,
                });
                // The excerpt consumed the remaining budget.
                tokens_used = self.token_budget;
                break;
            }

            // Not enough budget left for a meaningful fragment — stop.
            break;
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

    // ── P1.3 regression: oversized entries must not silently disappear ──

    #[test]
    fn test_resolve_oversized_entry_is_returned_bounded() {
        // Regression for the P1.2 blocking defect: an entry whose estimated
        // token cost exceeds the 500-token budget used to be skipped entirely,
        // so Session 2 received `entries: []` despite an exact keyword match.
        let resolver = EngineeringMemoryResolver::default();
        let long_value = "decision: ".to_string() + &"x".repeat(3000);
        let entries = vec![entry(
            "e1",
            "architecture:mutation-boundary",
            &long_value,
            0.9,
            0.8,
            &[],
        )];

        let result = resolver
            .resolve(&entries, &["mutation".to_string()], &[])
            .expect("oversized entry must resolve");

        assert_eq!(
            result.entries.len(),
            1,
            "oversized entry must not disappear"
        );
        let got = &result.entries[0];
        assert_eq!(got.key, "architecture:mutation-boundary");
        assert_eq!(got.confidence, 0.9);
        // Bounded: excerpt (≤ 500 tokens ≈ 2000 chars) + explicit marker.
        assert!(
            got.value.len() < 2000 + super::TRUNCATION_MARKER.len() + 32,
            "value must be bounded, got {} chars",
            got.value.len()
        );
        assert!(
            got.value.ends_with(super::TRUNCATION_MARKER),
            "truncation must be explicit"
        );
        // The decision's leading content is preserved (prefix excerpt).
        assert!(got.value.starts_with("decision: "));
        // Budget semantics: the oversized entry consumed the whole budget.
        assert_eq!(result.budget_remaining, 0);
    }

    #[test]
    fn test_resolve_oversized_entry_with_exact_keyword() {
        // Exact-keyword retrieval (as attempted repeatedly in P1.2 Session 2).
        let resolver = EngineeringMemoryResolver::default();
        let long_value = "canonical boundary: ".to_string() + &"y".repeat(2900);
        let entries = vec![entry(
            "e1",
            "architecture:mutation-boundary",
            &long_value,
            0.9,
            0.85,
            &[],
        )];
        let result = resolver
            .resolve(
                &entries,
                &["architecture:mutation-boundary".to_string()],
                &[],
            )
            .expect("exact keyword must resolve");
        assert_eq!(result.entries.len(), 1);
        assert!(result.entries[0].value.ends_with(super::TRUNCATION_MARKER));
    }

    #[test]
    fn test_resolve_mixed_sizes_ranks_and_bounds_deterministically() {
        // One oversized lower-importance entry plus several small
        // higher-importance entries: small relevant entries fit fully, the
        // oversized one is included as a bounded excerpt, and the outcome is
        // deterministic.
        let resolver = EngineeringMemoryResolver::default();
        let big_value = "big ".to_string() + &"z".repeat(2900);
        let entries = vec![
            entry("e_big", "mutation", &big_value, 0.9, 0.4, &[]),
            entry("e_a", "mutation", "small a", 0.9, 0.9, &[]),
            entry("e_c", "mutation", "small c", 0.9, 0.8, &[]),
        ];

        let r1 = resolver
            .resolve(&entries, &["mutation".to_string()], &[])
            .expect("resolve");
        let r2 = resolver
            .resolve(&entries, &["mutation".to_string()], &[])
            .expect("resolve");

        // Deterministic across runs.
        assert_eq!(r1.entries, r2.entries);
        assert_eq!(r1.budget_remaining, r2.budget_remaining);

        // Ranking preserved before budget mapping: small_a, small_c ranked
        // above big (importance 0.9/0.8 vs 0.4); final order is key-ascending
        // per the context contract, but all three must be present.
        let keys: Vec<&str> = r1.entries.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(keys, vec!["mutation", "mutation", "mutation"]);
        assert_eq!(r1.entries.len(), 3);

        // The two small entries are included in full; the big one is bounded.
        let big = r1
            .entries
            .iter()
            .find(|e| e.value.contains("zzz"))
            .expect("big entry present");
        assert!(big.value.ends_with(super::TRUNCATION_MARKER));
        assert!(big.value.len() < 2000 + super::TRUNCATION_MARKER.len() + 32);
        let smalls: Vec<&str> = r1
            .entries
            .iter()
            .filter(|e| e.value == "small a" || e.value == "small c")
            .map(|e| e.value.as_str())
            .collect();
        assert_eq!(smalls.len(), 2);
        // Budget fully consumed.
        assert_eq!(r1.budget_remaining, 0);
    }

    #[test]
    fn test_resolve_oversized_low_confidence_still_excluded() {
        // The confidence filter applies before the budget: a low-confidence
        // oversized entry is still excluded (confidence is a trust gate, not a
        // size question).
        let resolver = EngineeringMemoryResolver::default();
        let long_value = "d".repeat(3000);
        let entries = vec![entry("e1", "mutation", &long_value, 0.2, 0.8, &[])];
        let result = resolver.resolve(&entries, &["mutation".to_string()], &[]);
        assert!(matches!(
            result,
            Err(EngineeringMemoryResolveError::NoMatches)
        ));
    }

    #[test]
    fn test_bounded_excerpt_prefers_sentence_boundary() {
        let text =
            "First sentence about the boundary. Second sentence is long and keeps going. Third";
        let (excerpt, truncated) = super::bounded_excerpt(text, 40);
        assert!(truncated);
        assert!(
            excerpt.ends_with("."),
            "excerpt should end at a sentence boundary: {excerpt:?}"
        );
        assert!(excerpt.len() <= 40);
        // No mid-word cuts: the cut lands on the last sentence end within 40.
        assert!(text.starts_with(&excerpt));
    }

    #[test]
    fn test_bounded_excerpt_short_text_unchanged() {
        let (excerpt, truncated) = super::bounded_excerpt("short", 100);
        assert_eq!(excerpt, "short");
        assert!(!truncated);
    }
}

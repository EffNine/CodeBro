use serde::{Deserialize, Serialize};

/// Statistics collected during a single assembly run.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssemblyStatistics {
    /// Total fragments produced before any filtering.
    pub total_fragments: usize,
    /// Fragments remaining after ranking, deduplication, and budgeting.
    pub selected_fragments: usize,
    /// Fragments discarded during deduplication.
    pub duplicate_count: usize,
    /// Fragments removed because they would exceed the token budget.
    pub discarded_fragments: usize,
    /// Estimated total tokens in the final package.
    pub estimated_tokens: usize,
    /// Highest ranking score observed across all fragments.
    pub max_score: f64,
    /// Lowest ranking score among selected fragments.
    pub min_score: f64,
    /// Per-source fragment counts in the final package.
    pub per_source: std::collections::HashMap<String, usize>,
    /// Wall-clock assembly duration in milliseconds.
    pub elapsed_ms: u64,
}

impl AssemblyStatistics {
    pub fn new() -> Self {
        AssemblyStatistics::default()
    }

    /// True when no fragments were selected.
    pub fn is_empty(&self) -> bool {
        self.selected_fragments == 0
    }

    /// True when the budget was the limiting factor.
    pub fn budget_limited(&self) -> bool {
        self.discarded_fragments > 0
    }

    /// True when deduplication removed any fragments.
    pub fn had_duplicates(&self) -> bool {
        self.duplicate_count > 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_stats() {
        let s = AssemblyStatistics::default();
        assert_eq!(s.total_fragments, 0);
        assert!(s.is_empty());
        assert!(!s.budget_limited());
        assert!(!s.had_duplicates());
    }

    #[test]
    fn test_stats_after_assembly() {
        let mut s = AssemblyStatistics::new();
        s.total_fragments = 100;
        s.selected_fragments = 20;
        s.duplicate_count = 5;
        s.discarded_fragments = 10;
        s.estimated_tokens = 4000;
        s.max_score = 1.0;
        s.min_score = 0.3;
        s.per_source.insert("user_request".to_string(), 1);
        s.elapsed_ms = 12;

        assert!(!s.is_empty());
        assert!(s.budget_limited());
        assert!(s.had_duplicates());
        assert_eq!(s.per_source.len(), 1);
    }
}

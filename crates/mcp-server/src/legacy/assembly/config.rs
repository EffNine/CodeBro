use serde::{Deserialize, Serialize};

use super::budget::TokenBudget;

/// TOML-parseable configuration for the Context Assembly Engine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssemblyConfig {
    /// Default token budget envelope.
    #[serde(default = "default_budget")]
    pub default_budget: TokenBudget,

    /// Ranking weight factors (must sum > 0 for normalised scores).
    #[serde(default)]
    pub ranking_weights: RankingWeights,

    /// Hard cap on how many source files are included.
    #[serde(default = "default_max_files")]
    pub max_files: usize,

    /// Hard cap on how many symbol facts are included.
    #[serde(default = "default_max_symbols")]
    pub max_symbols: usize,

    /// Hard cap on how many memory entries are included.
    #[serde(default = "default_max_memories")]
    pub max_memories: usize,

    /// Whether to deduplicate fragments by source+content fingerprint.
    #[serde(default = "default_deduplicate")]
    pub deduplicate: bool,

    /// Whether to surface diagnostics before lower-priority facts.
    #[serde(default = "default_prioritize_diagnostics")]
    pub prioritize_diagnostics: bool,
}

impl Default for AssemblyConfig {
    fn default() -> Self {
        AssemblyConfig {
            default_budget: default_budget(),
            ranking_weights: RankingWeights::default(),
            max_files: default_max_files(),
            max_symbols: default_max_symbols(),
            max_memories: default_max_memories(),
            deduplicate: default_deduplicate(),
            prioritize_diagnostics: default_prioritize_diagnostics(),
        }
    }
}

fn default_budget() -> TokenBudget {
    TokenBudget::Medium
}

fn default_max_files() -> usize {
    20
}

fn default_max_symbols() -> usize {
    50
}

fn default_max_memories() -> usize {
    10
}

fn default_deduplicate() -> bool {
    true
}

fn default_prioritize_diagnostics() -> bool {
    true
}

/// Relative importance weights used by the ranking pass.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankingWeights {
    pub relevance: f64,
    pub recency: f64,
    pub dependency_distance: f64,
    pub symbol_proximity: f64,
    pub user_focus: f64,
    pub active_workspace: f64,
    pub diagnostics_priority: f64,
}

impl Default for RankingWeights {
    fn default() -> Self {
        RankingWeights {
            relevance: 1.0,
            recency: 0.5,
            dependency_distance: 0.8,
            symbol_proximity: 0.7,
            user_focus: 1.0,
            active_workspace: 0.6,
            diagnostics_priority: 0.9,
        }
    }
}

impl RankingWeights {
    /// Sum of all weights.
    pub fn total(&self) -> f64 {
        self.relevance
            + self.recency
            + self.dependency_distance
            + self.symbol_proximity
            + self.user_focus
            + self.active_workspace
            + self.diagnostics_priority
    }

    /// Normalise every weight to [0, 1].
    pub fn normalised(&self) -> Self {
        let total = self.total().max(0.001);
        RankingWeights {
            relevance: self.relevance / total,
            recency: self.recency / total,
            dependency_distance: self.dependency_distance / total,
            symbol_proximity: self.symbol_proximity / total,
            user_focus: self.user_focus / total,
            active_workspace: self.active_workspace / total,
            diagnostics_priority: self.diagnostics_priority / total,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let cfg = AssemblyConfig::default();
        assert_eq!(cfg.default_budget, TokenBudget::Medium);
        assert_eq!(cfg.max_files, 20);
        assert!(cfg.deduplicate);
        assert!(cfg.prioritize_diagnostics);
    }

    #[test]
    fn test_ranking_weights_total() {
        let w = RankingWeights::default();
        assert!((w.total() - 5.5).abs() < 0.001);
    }

    #[test]
    fn test_ranking_weights_normalised() {
        let w = RankingWeights {
            relevance: 2.0,
            recency: 1.0,
            dependency_distance: 1.0,
            symbol_proximity: 0.0,
            user_focus: 0.0,
            active_workspace: 0.0,
            diagnostics_priority: 0.0,
        };
        let n = w.normalised();
        assert!((n.relevance - 0.5).abs() < 0.01);
        assert!((n.recency - 0.25).abs() < 0.01);
        assert!((n.dependency_distance - 0.25).abs() < 0.01);
        assert_eq!(n.symbol_proximity, 0.0);
    }

    #[test]
    fn test_budget_enums() {
        use super::super::ContextBudget;
        let small: ContextBudget = TokenBudget::Small.into();
        assert_eq!(small.max_tokens, 2000);
        let medium: ContextBudget = TokenBudget::Medium.into();
        assert_eq!(medium.max_tokens, 8000);
        let large: ContextBudget = TokenBudget::Large.into();
        assert_eq!(large.max_tokens, 16000);
        let unlimited: ContextBudget = TokenBudget::Unlimited.into();
        assert_eq!(unlimited.max_tokens, usize::MAX);
    }
}

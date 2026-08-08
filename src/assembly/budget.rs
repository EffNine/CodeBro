use serde::{Deserialize, Serialize};
use std::time::SystemTime;

use crate::error::Result;
use crate::intelligence::CodeIndexer;
use crate::memory_runtime::{MemoryEntry, MemoryRuntime, MemoryTier};
use crate::workspace_runtime::{Change, WorkspaceRuntime};

/// A configurable token budget for context assembly.
///
/// Budgets are expressed in estimated tokens (chars / 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextBudget {
    pub max_tokens: usize,
    pub max_files: usize,
    pub max_symbols: usize,
    pub max_memories: usize,
}

impl Default for ContextBudget {
    fn default() -> Self {
        ContextBudget {
            max_tokens: 8000,
            max_files: 20,
            max_symbols: 50,
            max_memories: 10,
        }
    }
}

impl ContextBudget {
    pub fn small() -> Self {
        ContextBudget {
            max_tokens: 2000,
            max_files: 5,
            max_symbols: 10,
            max_memories: 3,
        }
    }

    pub fn medium() -> Self {
        ContextBudget::default()
    }

    pub fn large() -> Self {
        ContextBudget {
            max_tokens: 16000,
            max_files: 50,
            max_symbols: 200,
            max_memories: 20,
        }
    }

    pub fn unlimited() -> Self {
        ContextBudget {
            max_tokens: usize::MAX,
            max_files: usize::MAX,
            max_symbols: usize::MAX,
            max_memories: usize::MAX,
        }
    }

    /// True when the budget would allow no context at all.
    pub fn is_empty(&self) -> bool {
        self.max_tokens == 0
    }
}

/// A simple enum wrapper around ContextBudget for TOML parsing convenience.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenBudget {
    Small,
    Medium,
    Large,
    Unlimited,
}

impl From<TokenBudget> for ContextBudget {
    fn from(value: TokenBudget) -> Self {
        match value {
            TokenBudget::Small => ContextBudget::small(),
            TokenBudget::Medium => ContextBudget::medium(),
            TokenBudget::Large => ContextBudget::large(),
            TokenBudget::Unlimited => ContextBudget::unlimited(),
        }
    }
}

impl TokenBudget {
    pub fn into_budget(self) -> ContextBudget {
        self.into()
    }
}

/// Token budgeting utilities applied after ranking and deduplication.
pub mod budget {
    use super::ContextBudget;
    use crate::assembly::sources::ContextFragment;

    /// Apply `budget` to `fragments` in-place. Returns the number of
    /// fragments removed.
    pub fn apply(fragments: &mut Vec<ContextFragment>, budget: &ContextBudget) -> usize {
        let before = fragments.len();
        let mut used = 0usize;
        fragments.retain(|f| {
            if used + f.estimated_tokens <= budget.max_tokens {
                used += f.estimated_tokens;
                true
            } else {
                false
            }
        });
        before - fragments.len()
    }

    /// Compute the total estimated tokens in a fragment list.
    pub fn total_tokens(fragments: &[ContextFragment]) -> usize {
        fragments.iter().map(|f| f.estimated_tokens).sum()
    }

    /// Check whether `fragments` fits within `budget`.
    pub fn fits(fragments: &[ContextFragment], budget: &ContextBudget) -> bool {
        total_tokens(fragments) <= budget.max_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assembly::sources::ContextFragment;
    use crate::assembly::sources::ContextPriority;
    use crate::assembly::sources::ContextSource;

    #[test]
    fn test_budget_sizes() {
        assert_eq!(ContextBudget::small().max_tokens, 2000);
        assert_eq!(ContextBudget::medium().max_tokens, 8000);
        assert_eq!(ContextBudget::large().max_tokens, 16000);
        assert_eq!(ContextBudget::unlimited().max_tokens, usize::MAX);
    }

    #[test]
    fn test_token_budget_from_enum() {
        let b: ContextBudget = TokenBudget::Small.into();
        assert_eq!(b.max_tokens, 2000);
        let b: ContextBudget = TokenBudget::Unlimited.into();
        assert_eq!(b.max_tokens, usize::MAX);
    }

    #[test]
    fn test_budget_apply_trims() {
        let mut frags = vec![
            ContextFragment::new(
                super::super::ContextSource::UserRequest,
                super::super::ContextPriority::Critical,
                "a".repeat(800),
                1.0,
            ),
            ContextFragment::new(
                super::super::ContextSource::UserRequest,
                super::super::ContextPriority::High,
                "b".repeat(800),
                0.9,
            ),
            ContextFragment::new(
                super::super::ContextSource::UserRequest,
                super::super::ContextPriority::Medium,
                "c".repeat(800),
                0.8,
            ),
        ];
        let budget = ContextBudget {
            max_tokens: 300,
            ..Default::default()
        };
        let removed = budget::apply(&mut frags, &budget);
        assert_eq!(removed, 2);
        assert_eq!(frags.len(), 1);
    }

    #[test]
    fn test_budget_total_tokens() {
        let frags = vec![ContextFragment::new(
            super::super::ContextSource::UserRequest,
            super::super::ContextPriority::Critical,
            "hello".to_string(),
            1.0,
        )];
        let total = budget::total_tokens(&frags);
        assert!(total >= 1);
    }

    #[test]
    fn test_budget_fits() {
        let frags = vec![ContextFragment::new(
            super::super::ContextSource::UserRequest,
            super::super::ContextPriority::Critical,
            "x".repeat(100),
            1.0,
        )];
        let budget = ContextBudget {
            max_tokens: 1000,
            ..Default::default()
        };
        assert!(budget::fits(&frags, &budget));

        let big = vec![ContextFragment::new(
            super::super::ContextSource::UserRequest,
            super::super::ContextPriority::Critical,
            "x".repeat(10_000),
            1.0,
        )];
        assert!(!budget::fits(&big, &budget));
    }
}

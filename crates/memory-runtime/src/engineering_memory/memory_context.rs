//! Engineering memory — resolved memory fragments relevant to the
//! current task, with budget tracking.

use serde::{Deserialize, Serialize};

/// A single memory fragment: a key-value pair with confidence metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub tier: MemoryTier,
}

/// Tier of a memory entry, indicating how permanent it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemoryTier {
    Ephemeral,
    Session,
    Project,
    Persistent,
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryTier::Ephemeral => write!(f, "ephemeral"),
            MemoryTier::Session => write!(f, "session"),
            MemoryTier::Project => write!(f, "project"),
            MemoryTier::Persistent => write!(f, "persistent"),
        }
    }
}

/// Immutable collection of resolved engineering memory.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EngineeringMemoryContext {
    pub entries: Vec<MemoryEntry>,
    pub budget_remaining: usize,
}

impl EngineeringMemoryContext {
    pub fn new() -> Self {
        EngineeringMemoryContext {
            entries: Vec::new(),
            budget_remaining: 0,
        }
    }

    pub fn with_entries(mut self, entries: Vec<MemoryEntry>) -> Self {
        self.entries = entries;
        self.entries.sort_by(|a, b| a.key.cmp(&b.key));
        self
    }

    pub fn with_budget(mut self, budget: usize) -> Self {
        self.budget_remaining = budget;
        self
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_memory() {
        let mem = EngineeringMemoryContext::new();
        assert!(mem.is_empty());
        assert_eq!(mem.entry_count(), 0);
    }

    #[test]
    fn test_memory_with_entries() {
        let entries = vec![
            MemoryEntry {
                key: "language".to_string(),
                value: "rust".to_string(),
                confidence: 0.95,
                tier: MemoryTier::Project,
            },
            MemoryEntry {
                key: "framework".to_string(),
                value: "axum".to_string(),
                confidence: 0.88,
                tier: MemoryTier::Session,
            },
        ];
        let mem = EngineeringMemoryContext::new()
            .with_entries(entries)
            .with_budget(500);

        assert!(!mem.is_empty());
        assert_eq!(mem.entry_count(), 2);
        assert_eq!(mem.budget_remaining, 500);
        assert_eq!(mem.entries[0].key, "framework");
        assert_eq!(mem.entries[1].key, "language");
    }

    #[test]
    fn test_serialization_roundtrip() {
        let mem = EngineeringMemoryContext::new()
            .with_entries(vec![MemoryEntry {
                key: "k1".to_string(),
                value: "v1".to_string(),
                confidence: 0.9,
                tier: MemoryTier::Project,
            }])
            .with_budget(100);
        let json = serde_json::to_string(&mem).expect("serialize");
        let decoded: EngineeringMemoryContext = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(mem, decoded);
    }
}

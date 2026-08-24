//! Statistics for EngineeringContext — aggregate metrics after construction.

use serde::{Deserialize, Serialize};

/// Compile-time and construction statistics for an `EngineeringContext`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineeringContextStatistics {
    pub file_count: usize,
    pub memory_entries: usize,
    pub constraint_entries: usize,
    pub workspace_size: usize,
    pub context_fragments: usize,
    pub estimated_tokens: usize,
    pub compile_time: u64,
}

impl EngineeringContextStatistics {
    pub fn new() -> Self {
        EngineeringContextStatistics {
            file_count: 0,
            memory_entries: 0,
            constraint_entries: 0,
            workspace_size: 0,
            context_fragments: 0,
            estimated_tokens: 0,
            compile_time: 0,
        }
    }

    pub fn with_file_count(mut self, count: usize) -> Self {
        self.file_count = count;
        self
    }

    pub fn with_memory_entries(mut self, count: usize) -> Self {
        self.memory_entries = count;
        self
    }

    pub fn with_constraint_entries(mut self, count: usize) -> Self {
        self.constraint_entries = count;
        self
    }

    pub fn with_workspace_size(mut self, size: usize) -> Self {
        self.workspace_size = size;
        self
    }

    pub fn with_context_fragments(mut self, count: usize) -> Self {
        self.context_fragments = count;
        self
    }

    pub fn with_estimated_tokens(mut self, tokens: usize) -> Self {
        self.estimated_tokens = tokens;
        self
    }

    pub fn with_compile_time(mut self, time: u64) -> Self {
        self.compile_time = time;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.file_count == 0
            && self.memory_entries == 0
            && self.constraint_entries == 0
            && self.workspace_size == 0
            && self.context_fragments == 0
            && self.estimated_tokens == 0
    }
}

impl Default for EngineeringContextStatistics {
    fn default() -> Self {
        EngineeringContextStatistics::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_statistics() {
        let s = EngineeringContextStatistics::default();
        assert!(s.is_empty());
        assert_eq!(s.file_count, 0);
        assert_eq!(s.estimated_tokens, 0);
    }

    #[test]
    fn test_statistics_builder() {
        let s = EngineeringContextStatistics::new()
            .with_file_count(10)
            .with_memory_entries(5)
            .with_constraint_entries(3)
            .with_workspace_size(4096)
            .with_context_fragments(8)
            .with_estimated_tokens(2000)
            .with_compile_time(15);

        assert!(!s.is_empty());
        assert_eq!(s.file_count, 10);
        assert_eq!(s.memory_entries, 5);
        assert_eq!(s.constraint_entries, 3);
        assert_eq!(s.workspace_size, 4096);
        assert_eq!(s.context_fragments, 8);
        assert_eq!(s.estimated_tokens, 2000);
        assert_eq!(s.compile_time, 15);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let s = EngineeringContextStatistics::new()
            .with_file_count(7)
            .with_estimated_tokens(1400)
            .with_compile_time(20);
        let json = serde_json::to_string(&s).expect("serialize");
        let decoded: EngineeringContextStatistics =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(s, decoded);
    }
}

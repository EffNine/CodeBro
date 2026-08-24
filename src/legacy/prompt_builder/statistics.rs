//! Statistics for prompt compilation.
//!
//! Lightweight, aggregate metrics exposed after compilation.

use super::template::PromptTemplate;
use serde::{Deserialize, Serialize};

/// Compile-time statistics for a compiled prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptStatistics {
    pub section_count: usize,
    pub estimated_tokens: usize,
    pub compile_time_ns: u64,
    pub memory_fragments: usize,
    pub context_fragments: usize,
    pub template: String,
}

impl PromptStatistics {
    pub fn new(template: PromptTemplate, compile_time_ns: u64) -> Self {
        PromptStatistics {
            section_count: 0,
            estimated_tokens: 0,
            compile_time_ns,
            memory_fragments: 0,
            context_fragments: 0,
            template: template.as_str().to_string(),
        }
    }

    pub fn with_section_count(mut self, count: usize) -> Self {
        self.section_count = count;
        self
    }

    pub fn with_estimated_tokens(mut self, tokens: usize) -> Self {
        self.estimated_tokens = tokens;
        self
    }

    pub fn with_memory_fragments(mut self, count: usize) -> Self {
        self.memory_fragments = count;
        self
    }

    pub fn with_context_fragments(mut self, count: usize) -> Self {
        self.context_fragments = count;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_statistics_creation() {
        let stats = PromptStatistics::new(PromptTemplate::Engineering, 1_000_000);
        assert_eq!(stats.template, "engineering");
        assert_eq!(stats.compile_time_ns, 1_000_000);
        assert_eq!(stats.section_count, 0);
    }

    #[test]
    fn test_statistics_builder() {
        let stats = PromptStatistics::new(PromptTemplate::Default, 500_000)
            .with_section_count(9)
            .with_estimated_tokens(2000)
            .with_memory_fragments(3)
            .with_context_fragments(5);

        assert_eq!(stats.section_count, 9);
        assert_eq!(stats.estimated_tokens, 2000);
        assert_eq!(stats.memory_fragments, 3);
        assert_eq!(stats.context_fragments, 5);
    }
}

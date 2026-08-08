//! Diagnostics for prompt compilation.
//!
//! Tracks section sizes, dropped sections, compile duration, and
//! other observability metrics.

use std::time::Instant;

use super::template::PromptTemplate;
use serde::{Deserialize, Serialize};

/// Diagnostic snapshot of a single prompt compilation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromptDiagnostics {
    pub total_length: usize,
    pub section_sizes: Vec<(String, usize)>,
    pub template_used: String,
    pub estimated_tokens: usize,
    pub dropped_sections: Vec<String>,
    pub compile_duration_ms: u64,
}

impl PromptDiagnostics {
    pub fn new(template: PromptTemplate, compile_duration: Instant) -> Self {
        PromptDiagnostics {
            total_length: 0,
            section_sizes: Vec::new(),
            template_used: template.as_str().to_string(),
            estimated_tokens: 0,
            dropped_sections: Vec::new(),
            compile_duration_ms: compile_duration.elapsed().as_millis() as u64,
        }
    }

    pub fn add_section(&mut self, label: &str, length: usize, tokens: usize) {
        self.section_sizes.push((label.to_string(), length));
        self.total_length += length;
        self.estimated_tokens += tokens;
    }

    pub fn drop_section(&mut self, label: &str) {
        self.dropped_sections.push(label.to_string());
    }

    pub fn is_empty(&self) -> bool {
        self.total_length == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_creation() {
        let start = Instant::now();
        let diag = PromptDiagnostics::new(PromptTemplate::Engineering, start);
        assert_eq!(diag.template_used, "engineering");
        assert_eq!(diag.total_length, 0);
        assert!(diag.dropped_sections.is_empty());
    }

    #[test]
    fn test_diagnostics_add_section() {
        let start = Instant::now();
        let mut diag = PromptDiagnostics::new(PromptTemplate::Default, start);
        diag.add_section("system_identity", 200, 50);
        diag.add_section("user_request", 100, 25);
        assert_eq!(diag.total_length, 300);
        assert_eq!(diag.estimated_tokens, 75);
        assert_eq!(diag.section_sizes.len(), 2);
    }

    #[test]
    fn test_diagnostics_drop_section() {
        let start = Instant::now();
        let mut diag = PromptDiagnostics::new(PromptTemplate::Default, start);
        diag.drop_section("engineering_memory");
        assert_eq!(diag.dropped_sections.len(), 1);
        assert_eq!(diag.dropped_sections[0], "engineering_memory");
    }

    #[test]
    fn test_diagnostics_is_empty() {
        let start = Instant::now();
        let mut diag = PromptDiagnostics::new(PromptTemplate::Default, start);
        assert!(diag.is_empty());
        diag.add_section("test", 10, 2);
        assert!(!diag.is_empty());
    }
}

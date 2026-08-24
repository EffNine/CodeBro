//! Diagnostics for EngineeringContext construction and lifecycle.

use serde::{Deserialize, Serialize};

/// Diagnostic snapshot captured at build time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineeringContextDiagnostics {
    pub creation_time: String,
    pub build_duration_ms: u64,
    pub fragment_count: usize,
    pub memory_count: usize,
    pub constraint_count: usize,
    pub workspace_files: usize,
    pub estimated_tokens: usize,
    pub provider: Option<String>,
    pub template: Option<String>,
}

impl EngineeringContextDiagnostics {
    pub fn new(
        creation_time: String,
        build_duration_ms: u64,
        fragment_count: usize,
        memory_count: usize,
        constraint_count: usize,
        workspace_files: usize,
        estimated_tokens: usize,
    ) -> Self {
        EngineeringContextDiagnostics {
            creation_time,
            build_duration_ms,
            fragment_count,
            memory_count,
            constraint_count,
            workspace_files,
            estimated_tokens,
            provider: None,
            template: None,
        }
    }

    pub fn with_provider(mut self, provider: Option<String>) -> Self {
        self.provider = provider;
        self
    }

    pub fn with_template(mut self, template: Option<String>) -> Self {
        self.template = template;
        self
    }

    pub fn summary(&self) -> String {
        format!(
            "EngineeringContext Diagnostics:\n\
             Created: {}\n\
             Build duration: {} ms\n\
             Fragments: {}\n\
             Memory entries: {}\n\
             Constraints: {}\n\
             Workspace files: {}\n\
             Estimated tokens: {}\n\
             Provider: {:?}\n\
             Template: {:?}",
            self.creation_time,
            self.build_duration_ms,
            self.fragment_count,
            self.memory_count,
            self.constraint_count,
            self.workspace_files,
            self.estimated_tokens,
            self.provider,
            self.template,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_diagnostics_creation() {
        let d = EngineeringContextDiagnostics::new(
            "2026-08-09T00:00:00Z".to_string(),
            5,
            10,
            3,
            2,
            5,
            2000,
        );
        assert_eq!(d.fragment_count, 10);
        assert_eq!(d.memory_count, 3);
        assert_eq!(d.constraint_count, 2);
        assert_eq!(d.workspace_files, 5);
        assert_eq!(d.estimated_tokens, 2000);
        assert!(d.provider.is_none());
        assert!(d.template.is_none());
    }

    #[test]
    fn test_diagnostics_with_provider_and_template() {
        let d = EngineeringContextDiagnostics::new(
            "2026-08-09T00:00:00Z".to_string(),
            1,
            0,
            0,
            0,
            0,
            0,
        )
        .with_provider(Some("openai".to_string()))
        .with_template(Some("engineering".to_string()));
        assert_eq!(d.provider, Some("openai".to_string()));
        assert_eq!(d.template, Some("engineering".to_string()));
    }

    #[test]
    fn test_diagnostics_summary() {
        let d = EngineeringContextDiagnostics::new(
            "2026-08-09T00:00:00Z".to_string(),
            3,
            7,
            2,
            1,
            3,
            1500,
        );
        let s = d.summary();
        assert!(s.contains("7"));
        assert!(s.contains("1500"));
    }

    #[test]
    fn test_diagnostics_serialization_roundtrip() {
        let d = EngineeringContextDiagnostics::new(
            "2026-08-09T00:00:00Z".to_string(),
            2,
            5,
            1,
            1,
            2,
            800,
        )
        .with_provider(Some("claude".to_string()))
        .with_template(Some("debugging".to_string()));
        let json = serde_json::to_string(&d).expect("serialize");
        let decoded: EngineeringContextDiagnostics =
            serde_json::from_str(&json).expect("deserialize");
        assert_eq!(d.fragment_count, decoded.fragment_count);
        assert_eq!(d.provider, decoded.provider);
        assert_eq!(d.template, decoded.template);
    }
}

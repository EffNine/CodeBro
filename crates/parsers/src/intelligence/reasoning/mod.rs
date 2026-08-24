#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Reasoning Engine — Code reasoning abstraction layer.
/// Provides a trait-based interface for analyzing code and generating
/// reasoning results for agent consumption.
pub mod engine;

use anyhow::Result;

pub use engine::{AgentReasoningEngine, ReasoningResult, ReasoningStep};

// =========================================================================
// Trait Definitions
// =========================================================================

/// Trait for code reasoning engines.
///
/// Implementations analyze code before modification and provide
/// structured reasoning results with confidence scores.
///
/// Note: SQLite connections are not thread-safe; this trait is not Send-bound.
pub trait ReasoningEngineTrait {
    /// Analyze the codebase before a modification request.
    fn analyze_before_modification(&self, request: &str) -> Result<ReasoningResult>;

    /// Analyze a specific file for understanding.
    fn analyze_for_code_understanding(&self, file_path: &str) -> Result<ReasoningResult>;

    /// Find existing patterns matching a name.
    fn find_existing_patterns(&self, pattern_name: &str) -> Result<Vec<String>>;

    /// Suggest implementation approaches based on existing code.
    fn suggest_implementation_approach(&self, request: &str) -> Result<Vec<String>>;
}

impl ReasoningEngineTrait for AgentReasoningEngine {
    fn analyze_before_modification(&self, request: &str) -> Result<ReasoningResult> {
        AgentReasoningEngine::analyze_before_modification(self, request)
    }

    fn analyze_for_code_understanding(&self, file_path: &str) -> Result<ReasoningResult> {
        AgentReasoningEngine::analyze_for_code_understanding(self, file_path)
    }

    fn find_existing_patterns(&self, pattern_name: &str) -> Result<Vec<String>> {
        AgentReasoningEngine::find_existing_patterns(self, pattern_name)
    }

    fn suggest_implementation_approach(&self, request: &str) -> Result<Vec<String>> {
        AgentReasoningEngine::suggest_implementation_approach(self, request)
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentCapability {
    pub name: String,
    pub description: String,
    pub tools_required: Vec<String>,
    pub context_needed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentContext {
    pub task_description: String,
    pub project_root: String,
    pub relevant_files: Vec<String>,
    pub related_symbols: Vec<String>,
    pub dependencies: Vec<String>,
    pub previous_results: Vec<SubAgentResult>,
    pub memory_entries: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAgentResult {
    pub agent_name: String,
    pub success: bool,
    pub output: String,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<String>,
    pub duration_ms: u64,
    pub errors: Vec<String>,
    pub recommendations: Vec<String>,
}

pub trait SubAgent: Send + Sync {
    fn name(&self) -> &str;
    fn purpose(&self) -> &str;
    fn capabilities(&self) -> Vec<SubAgentCapability>;
    fn required_tools(&self) -> Vec<&str>;

    fn can_handle(&self, task: &str) -> bool;
    fn required_context(&self) -> Vec<&str>;

    fn execute(&self, context: &SubAgentContext) -> SubAgentResult;

    fn estimate_effort(&self, context: &SubAgentContext) -> f32 {
        context.relevant_files.len() as f32 * 0.1 + context.related_symbols.len() as f32 * 0.05
    }

    fn validate_prerequisites(&self, context: &SubAgentContext) -> bool {
        !context.task_description.is_empty()
    }
}

pub struct SubAgentStats {
    pub total_executions: u32,
    pub successful_executions: u32,
    pub failed_executions: u32,
    pub average_duration_ms: u64,
    pub total_duration_ms: u64,
    pub tools_used: std::collections::HashMap<String, u32>,
    pub common_errors: std::collections::HashMap<String, u32>,
}

impl SubAgentStats {
    pub fn new() -> Self {
        SubAgentStats {
            total_executions: 0,
            successful_executions: 0,
            failed_executions: 0,
            average_duration_ms: 0,
            total_duration_ms: 0,
            tools_used: std::collections::HashMap::new(),
            common_errors: std::collections::HashMap::new(),
        }
    }

    pub fn record_execution(&mut self, result: &SubAgentResult) {
        self.total_executions += 1;
        if result.success {
            self.successful_executions += 1;
        } else {
            self.failed_executions += 1;
        }
        self.total_duration_ms += result.duration_ms;
        self.average_duration_ms = self.total_duration_ms / self.total_executions as u64;

        for tool in &result.tools_used {
            *self.tools_used.entry(tool.clone()).or_insert(0) += 1;
        }

        for error in &result.errors {
            *self.common_errors.entry(error.clone()).or_insert(0) += 1;
        }
    }

    pub fn success_rate(&self) -> f32 {
        if self.total_executions == 0 {
            return 0.0;
        }
        self.successful_executions as f32 / self.total_executions as f32
    }
}

impl Default for SubAgentStats {
    fn default() -> Self {
        Self::new()
    }
}

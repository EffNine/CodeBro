#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use crate::config::Config;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskMetrics {
    pub task: String,
    pub total_duration_ms: u64,
    pub agent_durations: HashMap<String, u64>,
    pub tool_durations: HashMap<String, u64>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub context_size_tokens: u64,
    pub files_modified: Vec<String>,
    pub retry_count: u32,
    pub tools_run: u32,
    pub started_at: String,
    pub completed_at: Option<String>,
}

impl TaskMetrics {
    pub fn new(task: impl Into<String>) -> Self {
        TaskMetrics {
            task: task.into(),
            started_at: chrono::Local::now().to_rfc3339(),
            ..Default::default()
        }
    }

    pub fn record_agent_duration(&mut self, agent: &str, duration_ms: u64) {
        let entry = self.agent_durations.entry(agent.to_string()).or_insert(0);
        *entry += duration_ms;
        self.total_duration_ms += duration_ms;
    }

    pub fn record_tool_duration(&mut self, tool: &str, duration_ms: u64) {
        let entry = self.tool_durations.entry(tool.to_string()).or_insert(0);
        *entry += duration_ms;
        self.tools_run += 1;
    }

    pub fn record_tokens(&mut self, input: u64, output: u64) {
        self.input_tokens += input;
        self.output_tokens += output;
    }

    pub fn add_file(&mut self, file: impl Into<String>) {
        let file = file.into();
        if !self.files_modified.contains(&file) {
            self.files_modified.push(file);
        }
    }

    pub fn increment_retries(&mut self) {
        self.retry_count += 1;
    }

    pub fn complete(&mut self) {
        self.completed_at = Some(chrono::Local::now().to_rfc3339());
    }

    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens
    }

    pub fn estimated_cost_usd(&self, model: &str) -> f64 {
        cost_for_tokens(model, self.input_tokens, self.output_tokens)
    }

    pub fn agent_count(&self) -> usize {
        self.agent_durations.len()
    }

    pub fn file_count(&self) -> usize {
        self.files_modified.len()
    }
}

pub fn cost_for_tokens(model: &str, input_tokens: u64, output_tokens: u64) -> f64 {
    let (input_per_m, output_per_m) = model_rates(model);
    (input_tokens as f64 / 1_000_000.0) * input_per_m
        + (output_tokens as f64 / 1_000_000.0) * output_per_m
}

pub fn model_rates(model: &str) -> (f64, f64) {
    let m = model.to_lowercase();
    if m.contains("claude") && m.contains("opus") {
        (15.0, 75.0)
    } else if m.contains("claude") && m.contains("sonnet") {
        (3.0, 15.0)
    } else if m.contains("claude") && m.contains("haiku") {
        (0.25, 1.25)
    } else if m.contains("gpt-4o") {
        (2.5, 10.0)
    } else if m.contains("gpt-4") {
        (30.0, 60.0)
    } else if m.contains("gpt-3.5") {
        (0.5, 1.5)
    } else if m.contains("deepseek") && m.contains("reasoner") {
        (0.55, 2.19)
    } else if m.contains("deepseek") {
        (0.14, 0.28)
    } else if m.contains("gemini") && m.contains("ultra") {
        (1.25, 5.0)
    } else if m.contains("gemini") {
        (0.35, 1.05)
    } else {
        (1.0, 2.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageRecord {
    pub timestamp: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub estimated_cost: f64,
    pub context_size_tokens: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageHistory {
    pub records: Vec<UsageRecord>,
}

impl UsageHistory {
    pub fn record(
        &mut self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        context_size_tokens: u64,
    ) {
        let cost = cost_for_tokens(model, input_tokens, output_tokens);
        self.records.push(UsageRecord {
            timestamp: chrono::Local::now().to_rfc3339(),
            model: model.to_string(),
            input_tokens,
            output_tokens,
            estimated_cost: cost,
            context_size_tokens,
        });
    }

    pub fn total_cost(&self) -> f64 {
        self.records.iter().map(|r| r.estimated_cost).sum()
    }

    pub fn total_tokens(&self) -> u64 {
        self.records
            .iter()
            .map(|r| r.input_tokens + r.output_tokens)
            .sum()
    }

    pub fn record_count(&self) -> usize {
        self.records.len()
    }

    pub fn last(&self) -> Option<&UsageRecord> {
        self.records.last()
    }
}

pub struct CostTracker {
    history: UsageHistory,
    usage_path: PathBuf,
}

impl CostTracker {
    pub fn new() -> Result<Self> {
        let config_dir = Config::config_dir();
        let usage_path = config_dir.join("usage.json");

        let history = if usage_path.exists() {
            let content = fs::read_to_string(&usage_path)
                .with_context(|| format!("Failed to read usage file: {:?}", usage_path))?;
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse usage file: {:?}", usage_path))?
        } else {
            UsageHistory::default()
        };

        Ok(CostTracker {
            history,
            usage_path,
        })
    }

    pub fn with_usage_path(usage_path: PathBuf) -> Result<Self> {
        let history = if usage_path.exists() {
            let content = fs::read_to_string(&usage_path)
                .with_context(|| format!("Failed to read usage file: {:?}", usage_path))?;
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse usage file: {:?}", usage_path))?
        } else {
            UsageHistory::default()
        };

        Ok(CostTracker {
            history,
            usage_path,
        })
    }

    pub fn track_usage(
        &mut self,
        model: &str,
        input_tokens: u64,
        output_tokens: u64,
        context_size_tokens: u64,
    ) -> Result<()> {
        self.history
            .record(model, input_tokens, output_tokens, context_size_tokens);
        self.save()?;
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.usage_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(&self.history)?;
        fs::write(&self.usage_path, content)?;
        Ok(())
    }

    pub fn history(&self) -> &UsageHistory {
        &self.history
    }

    pub fn total_cost(&self) -> f64 {
        self.history.total_cost()
    }
}

pub struct MetricsRegistry {
    pub current: Option<TaskMetrics>,
    pub completed: Vec<TaskMetrics>,
    pub cost_tracker: CostTracker,
    max_completed: usize,
}

impl MetricsRegistry {
    pub fn new() -> Result<Self> {
        Ok(MetricsRegistry {
            current: None,
            completed: Vec::new(),
            cost_tracker: CostTracker::new()?,
            max_completed: 50,
        })
    }

    pub fn begin_task(&mut self, task: impl Into<String>) {
        self.current = Some(TaskMetrics::new(task));
    }

    pub fn end_task(&mut self, model: &str) {
        if let Some(mut metrics) = self.current.take() {
            metrics.complete();
            let _cost = metrics.estimated_cost_usd(model);
            self.cost_tracker
                .track_usage(
                    model,
                    metrics.input_tokens,
                    metrics.output_tokens,
                    metrics.context_size_tokens,
                )
                .ok();
            self.completed.push(metrics);
            while self.completed.len() > self.max_completed {
                self.completed.remove(0);
            }
        }
    }

    pub fn metrics(&self) -> Option<&TaskMetrics> {
        self.current.as_ref()
    }

    pub fn metrics_mut(&mut self) -> Option<&mut TaskMetrics> {
        self.current.as_mut()
    }
}

pub fn format_token_count(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1000 {
        format!("{:.1}k", tokens as f64 / 1000.0)
    } else {
        tokens.to_string()
    }
}

pub fn format_cost_usd(cost: f64) -> String {
    format!("${:.4}", cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cost_calculation() {
        let cost = cost_for_tokens("gpt-4o", 1000000, 500000);
        assert!((cost - (2.5 + 5.0)).abs() < 0.001);
    }

    #[test]
    fn test_model_rates() {
        let (input, output) = model_rates("claude-sonnet-4");
        assert_eq!(input, 3.0);
        assert_eq!(output, 15.0);

        let (input, output) = model_rates("gpt-4o");
        assert_eq!(input, 2.5);
        assert_eq!(output, 10.0);

        let (input, output) = model_rates("deepseek-chat");
        assert_eq!(input, 0.14);
        assert_eq!(output, 0.28);
    }

    #[test]
    fn test_task_metrics() {
        let mut metrics = TaskMetrics::new("Add caching");
        metrics.record_agent_duration("research", 1000);
        metrics.record_agent_duration("coding", 2000);
        metrics.record_tool_duration("edit_file", 500);
        metrics.record_tokens(1000, 500);
        metrics.add_file("auth.rs");
        metrics.add_file("auth.rs");

        assert_eq!(metrics.agent_count(), 2);
        assert_eq!(metrics.file_count(), 1);
        assert_eq!(metrics.total_tokens(), 1500);
        assert_eq!(metrics.tools_run, 1);
    }

    #[test]
    fn test_usage_history() {
        let mut history = UsageHistory::default();
        history.record("gpt-4o", 1000000, 500000, 200000);
        assert_eq!(history.record_count(), 1);
        assert!((history.total_cost() - 7.5).abs() < 0.001);
        assert_eq!(history.total_tokens(), 1500000);
    }

    #[test]
    fn test_token_format() {
        assert_eq!(format_token_count(500), "500");
        assert_eq!(format_token_count(1500), "1.5k");
        assert_eq!(format_token_count(1500000), "1.5M");
    }

    #[test]
    fn test_cost_format() {
        assert_eq!(format_cost_usd(0.08), "$0.0800");
        assert_eq!(format_cost_usd(1.5), "$1.5000");
    }
}

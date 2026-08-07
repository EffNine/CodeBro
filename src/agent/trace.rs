#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::config::Config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationTrace {
    pub task_id: String,
    pub timestamp: String,
    pub user_request: String,
    pub plan_summary: String,
    pub tools_executed: Vec<String>,
    pub files_changed: Vec<String>,
    pub commands_executed: Vec<String>,
    pub result: String,
    pub lesson_learned: Option<String>,
    pub memory_influence: Vec<String>,
    pub skill_used: Option<String>,
}

pub struct TraceStore {
    traces_dir: PathBuf,
}

impl TraceStore {
    pub fn new(traces_dir: PathBuf) -> Result<Self> {
        if !traces_dir.exists() {
            fs::create_dir_all(&traces_dir)
                .with_context(|| format!("Failed to create traces directory: {:?}", traces_dir))?;
        }

        Ok(TraceStore { traces_dir })
    }

    pub fn record(&self, trace: &OperationTrace) -> Result<()> {
        let path = self.traces_dir.join(format!("{}.json", trace.task_id));
        let content =
            serde_json::to_string_pretty(trace).with_context(|| "Failed to serialize trace")?;
        fs::write(&path, content).with_context(|| "Failed to write trace file")?;
        Ok(())
    }

    pub fn load(&self, task_id: &str) -> Result<OperationTrace> {
        let path = self.traces_dir.join(format!("{}.json", task_id));
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read trace file: {:?}", path))?;
        let trace: OperationTrace = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse trace file: {:?}", path))?;
        Ok(trace)
    }

    pub fn list_traces(&self) -> Vec<OperationTrace> {
        let mut traces = Vec::new();
        if !self.traces_dir.exists() {
            return traces;
        }

        if let Ok(entries) = fs::read_dir(&self.traces_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map(|e| e == "json").unwrap_or(false) {
                    if let Ok(content) = fs::read_to_string(&path) {
                        if let Ok(trace) = serde_json::from_str::<OperationTrace>(&content) {
                            traces.push(trace);
                        }
                    }
                }
            }
        }

        traces
    }

    pub fn recent_traces(&self, count: usize) -> Vec<OperationTrace> {
        let mut traces = self.list_traces();
        traces.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        traces.into_iter().take(count).collect()
    }
}

pub fn create_trace(
    task_id: &str,
    user_request: &str,
    plan_summary: &str,
    tools_executed: &[String],
    files_changed: &[String],
    commands_executed: &[String],
    result: &str,
    lesson_learned: Option<String>,
    memory_influence: &[String],
    skill_used: Option<&str>,
) -> OperationTrace {
    OperationTrace {
        task_id: task_id.to_string(),
        timestamp: chrono::Local::now().to_rfc3339(),
        user_request: user_request.to_string(),
        plan_summary: plan_summary.to_string(),
        tools_executed: tools_executed.to_vec(),
        files_changed: files_changed.to_vec(),
        commands_executed: commands_executed.to_vec(),
        result: result.to_string(),
        lesson_learned,
        memory_influence: memory_influence.to_vec(),
        skill_used: skill_used.map(|s| s.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trace_creation() {
        let trace = create_trace(
            "task-1",
            "Add API endpoint",
            "Read files, patch, test",
            &["read_file".to_string(), "patch_file".to_string()],
            &["src/main.rs".to_string()],
            &["cargo test".to_string()],
            "success",
            None,
            &[],
            None,
        );

        assert_eq!(trace.task_id, "task-1");
        assert_eq!(trace.result, "success");
        assert_eq!(trace.tools_executed.len(), 2);
    }

    #[test]
    fn test_trace_with_lesson() {
        let trace = create_trace(
            "task-2",
            "Fix build error",
            "Read, edit, test",
            &["read_file".to_string(), "edit_file".to_string()],
            &["src/lib.rs".to_string()],
            &["cargo build".to_string()],
            "success",
            Some("Always check Cargo.toml dependencies first".to_string()),
            &["Project uses cargo".to_string()],
            Some("rust_build"),
        );

        assert!(trace.lesson_learned.is_some());
        assert_eq!(trace.skill_used, Some("rust_build".to_string()));
        assert_eq!(trace.memory_influence.len(), 1);
    }

    #[test]
    fn test_trace_store_record_and_load() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TraceStore::new(dir.path().join("traces")).unwrap();

        let trace = create_trace(
            "task-3",
            "Test request",
            "Test plan",
            &["run_command".to_string()],
            &[],
            &["echo test".to_string()],
            "success",
            None,
            &[],
            None,
        );

        store.record(&trace).unwrap();
        let loaded = store.load("task-3").unwrap();
        assert_eq!(loaded.task_id, "task-3");
        assert_eq!(loaded.result, "success");
    }

    #[test]
    fn test_trace_store_list_traces() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TraceStore::new(dir.path().join("traces")).unwrap();

        let trace1 = create_trace(
            "task-4",
            "Request 1",
            "Plan 1",
            &["read_file".to_string()],
            &[],
            &[],
            "success",
            None,
            &[],
            None,
        );
        let trace2 = create_trace(
            "task-5",
            "Request 2",
            "Plan 2",
            &["run_command".to_string()],
            &[],
            &[],
            "success",
            None,
            &[],
            None,
        );

        store.record(&trace1).unwrap();
        store.record(&trace2).unwrap();

        let traces = store.list_traces();
        assert_eq!(traces.len(), 2);
    }
}

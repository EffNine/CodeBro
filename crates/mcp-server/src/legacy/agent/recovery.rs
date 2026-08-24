#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::config::Config;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FailureType {
    ToolError,
    CompileError,
    TestFailure,
    ProviderError,
    PermissionDenied,
    Timeout,
    Unknown,
}

impl FailureType {
    pub fn classify(error: &str) -> Self {
        let e = error.to_lowercase();
        if e.contains("permission") || e.contains("denied") {
            FailureType::PermissionDenied
        } else if e.contains("timeout") || e.contains("timed out") {
            FailureType::Timeout
        } else if e.contains("compile")
            || e.contains("borrow")
            || e.contains("type error")
            || e.contains("cannot find")
        {
            FailureType::CompileError
        } else if e.contains("test") || e.contains("failed") || e.contains("assertion") {
            FailureType::TestFailure
        } else if e.contains("provider")
            || e.contains("api")
            || e.contains("request")
            || e.contains("http")
        {
            FailureType::ProviderError
        } else if e.contains("tool") || e.contains("io") || e.contains("file") {
            FailureType::ToolError
        } else {
            FailureType::Unknown
        }
    }

    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            FailureType::Timeout | FailureType::ProviderError | FailureType::Unknown
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FailureType::ToolError => "tool_error",
            FailureType::CompileError => "compile_error",
            FailureType::TestFailure => "test_failure",
            FailureType::ProviderError => "provider_error",
            FailureType::PermissionDenied => "permission_denied",
            FailureType::Timeout => "timeout",
            FailureType::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RecoveryAction {
    Retry,
    AnalyzeAndFix,
    AskCodingAgent,
    AskUser,
    Skip,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureEvent {
    pub id: String,
    pub agent: String,
    pub task: String,
    pub error: String,
    pub failure_type: FailureType,
    pub timestamp: String,
    pub retry_count: u32,
    pub action_taken: Option<RecoveryAction>,
    pub resolved: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveryPolicy {
    pub max_retries: u32,
    pub retry_delay_ms: u64,
    pub escalate_after_retries: u32,
    pub analyze_on_first_failure: bool,
}

impl Default for RecoveryPolicy {
    fn default() -> Self {
        RecoveryPolicy {
            max_retries: 3,
            retry_delay_ms: 1000,
            escalate_after_retries: 2,
            analyze_on_first_failure: true,
        }
    }
}

pub struct RecoveryEngine {
    policy: RecoveryPolicy,
    failures: Vec<FailureEvent>,
    storage_path: PathBuf,
}

impl RecoveryEngine {
    pub fn new() -> Result<Self> {
        let config_dir = Config::config_dir();
        let storage_path = config_dir.join("recovery.json");

        let failures = if storage_path.exists() {
            let content = fs::read_to_string(&storage_path)
                .with_context(|| format!("Failed to read recovery file: {:?}", storage_path))?;
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse recovery file: {:?}", storage_path))?
        } else {
            Vec::new()
        };

        Ok(RecoveryEngine {
            policy: RecoveryPolicy::default(),
            failures,
            storage_path,
        })
    }

    pub fn with_policy(mut self, policy: RecoveryPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn with_storage_path(path: PathBuf) -> Result<Self> {
        let failures = if path.exists() {
            let content = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read recovery file: {:?}", path))?;
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse recovery file: {:?}", path))?
        } else {
            Vec::new()
        };

        Ok(RecoveryEngine {
            policy: RecoveryPolicy::default(),
            failures,
            storage_path: path,
        })
    }

    pub fn handle_failure(&mut self, agent: &str, task: &str, error: &str) -> Result<RecoveryPlan> {
        let failure_type = FailureType::classify(error);
        let existing = self
            .failures
            .iter()
            .filter(|f| f.agent == agent && f.task == task && !f.resolved)
            .count() as u32;

        let retry_count = existing;
        let event = FailureEvent {
            id: uuid::Uuid::new_v4().to_string(),
            agent: agent.to_string(),
            task: task.to_string(),
            error: error.to_string(),
            failure_type: failure_type.clone(),
            timestamp: chrono::Local::now().to_rfc3339(),
            retry_count,
            action_taken: None,
            resolved: false,
        };

        let action = self.determine_action(&event);

        let mut event = event;
        event.action_taken = Some(action.clone());
        self.failures.push(event.clone());
        self.save()?;

        Ok(RecoveryPlan {
            failure: event,
            action: action.clone(),
            should_retry: action == RecoveryAction::Retry && retry_count < self.policy.max_retries,
            should_escalate: retry_count >= self.policy.escalate_after_retries,
            suggested_agent: self.suggest_agent(&failure_type),
            retry_delay_ms: self.policy.retry_delay_ms,
        })
    }

    fn determine_action(&self, event: &FailureEvent) -> RecoveryAction {
        if event.failure_type.is_retryable() {
            return RecoveryAction::Retry;
        }

        if self.policy.analyze_on_first_failure {
            match event.failure_type {
                FailureType::CompileError | FailureType::TestFailure => {
                    RecoveryAction::AskCodingAgent
                }
                FailureType::PermissionDenied => RecoveryAction::AskUser,
                _ => RecoveryAction::AnalyzeAndFix,
            }
        } else {
            RecoveryAction::Retry
        }
    }

    fn suggest_agent(&self, failure_type: &FailureType) -> String {
        match failure_type {
            FailureType::CompileError | FailureType::TestFailure => "coding".to_string(),
            FailureType::ProviderError | FailureType::Timeout => "main".to_string(),
            FailureType::PermissionDenied => "main".to_string(),
            _ => "research".to_string(),
        }
    }

    pub fn mark_resolved(&mut self, failure_id: &str) -> Result<()> {
        if let Some(failure) = self.failures.iter_mut().find(|f| f.id == failure_id) {
            failure.resolved = true;
            self.save()?;
        }
        Ok(())
    }

    pub fn record_retry(&mut self, failure_id: &str) -> Result<()> {
        if let Some(failure) = self.failures.iter_mut().find(|f| f.id == failure_id) {
            failure.retry_count += 1;
            self.save()?;
        }
        Ok(())
    }

    pub fn get_failures(&self) -> &[FailureEvent] {
        &self.failures
    }

    pub fn unresolved_failures(&self) -> Vec<&FailureEvent> {
        self.failures.iter().filter(|f| !f.resolved).collect()
    }

    pub fn retry_stats(&self) -> RetryStats {
        let total = self.failures.len();
        let resolved = self.failures.iter().filter(|f| f.resolved).count();
        let escalated = self
            .failures
            .iter()
            .filter(|f| f.retry_count >= self.policy.escalate_after_retries)
            .count();
        RetryStats {
            total_failures: total,
            resolved,
            escalated,
            unresolved: total - resolved,
            total_retries: self.failures.iter().map(|f| f.retry_count as u64).sum(),
        }
    }

    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.storage_path.parent() {
            if !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }
        let content = serde_json::to_string_pretty(&self.failures)?;
        fs::write(&self.storage_path, content)?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RecoveryPlan {
    pub failure: FailureEvent,
    pub action: RecoveryAction,
    pub should_retry: bool,
    pub should_escalate: bool,
    pub suggested_agent: String,
    pub retry_delay_ms: u64,
}

#[derive(Debug, Clone, Default)]
pub struct RetryStats {
    pub total_failures: usize,
    pub resolved: usize,
    pub escalated: usize,
    pub unresolved: usize,
    pub total_retries: u64,
}

pub fn default_recovery_policy() -> RecoveryPolicy {
    RecoveryPolicy::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_failure_classification() {
        assert_eq!(
            FailureType::classify("cargo test failed: assertion failed"),
            FailureType::TestFailure
        );
        assert_eq!(
            FailureType::classify("cannot find function `foo`"),
            FailureType::CompileError
        );
        assert_eq!(
            FailureType::classify("permission denied: Operation not permitted"),
            FailureType::PermissionDenied
        );
        assert_eq!(
            FailureType::classify("request timed out"),
            FailureType::Timeout
        );
    }

    #[test]
    fn test_retryable() {
        assert!(FailureType::Timeout.is_retryable());
        assert!(FailureType::ProviderError.is_retryable());
        assert!(!FailureType::CompileError.is_retryable());
        assert!(!FailureType::PermissionDenied.is_retryable());
    }

    #[test]
    fn test_recovery_plan_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine =
            RecoveryEngine::with_storage_path(dir.path().join("recovery.json")).unwrap();
        let plan = engine
            .handle_failure("testing", "run tests", "request timed out")
            .unwrap();
        assert_eq!(plan.action, RecoveryAction::Retry);
        assert!(plan.should_retry);
        assert_eq!(plan.suggested_agent, "main");
    }

    #[test]
    fn test_recovery_plan_test_failure() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine =
            RecoveryEngine::with_storage_path(dir.path().join("recovery.json")).unwrap();
        let plan = engine
            .handle_failure("testing", "run tests", "cargo test failed: assertion")
            .unwrap();
        assert_eq!(plan.action, RecoveryAction::AskCodingAgent);
        assert!(!plan.should_retry);
        assert_eq!(plan.suggested_agent, "coding");
    }

    #[test]
    fn test_recovery_escalation() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine =
            RecoveryEngine::with_storage_path(dir.path().join("recovery.json")).unwrap();
        for _ in 0..3 {
            engine
                .handle_failure("testing", "run tests", "request timed out")
                .unwrap();
        }
        let stats = engine.retry_stats();
        assert_eq!(stats.total_failures, 3);
        assert!(stats.escalated >= 1);
    }

    #[test]
    fn test_recovery_mark_resolved() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine =
            RecoveryEngine::with_storage_path(dir.path().join("recovery.json")).unwrap();
        let plan = engine
            .handle_failure("coding", "implement", "compile error: cannot find")
            .unwrap();
        let failure_id = plan.failure.id.clone();
        engine.mark_resolved(&failure_id).unwrap();
        assert_eq!(engine.unresolved_failures().len(), 0);
    }

    #[test]
    fn test_recovery_stats() {
        let dir = tempfile::tempdir().unwrap();
        let mut engine =
            RecoveryEngine::with_storage_path(dir.path().join("recovery.json")).unwrap();
        engine
            .handle_failure("testing", "t1", "cargo test failed")
            .unwrap();
        engine
            .handle_failure("testing", "t2", "cargo test failed")
            .unwrap();
        let stats = engine.retry_stats();
        assert_eq!(stats.total_failures, 2);
        assert_eq!(stats.resolved, 0);
    }
}

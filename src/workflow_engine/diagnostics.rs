//! Workflow Diagnostics — failure tracking and observability.
//!
/// Tracks workflow planning metrics, failures, and statistics.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Type of diagnostic event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    /// A workflow was planned.
    WorkflowPlanned,
    /// A workflow planning failed.
    PlanningFailure,
    /// A dependency analysis failed.
    DependencyFailure,
    /// A validation failed.
    ValidationFailure,
    /// A cycle was detected.
    CycleDetected,
    /// A conflict was detected.
    ConflictDetected,
}

impl DiagnosticKind {
    pub fn label(&self) -> &str {
        match self {
            DiagnosticKind::WorkflowPlanned => "workflow_planned",
            DiagnosticKind::PlanningFailure => "planning_failure",
            DiagnosticKind::DependencyFailure => "dependency_failure",
            DiagnosticKind::ValidationFailure => "validation_failure",
            DiagnosticKind::CycleDetected => "cycle_detected",
            DiagnosticKind::ConflictDetected => "conflict_detected",
        }
    }
}

/// A single diagnostic record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticRecord {
    pub kind: DiagnosticKind,
    pub message: String,
    pub timestamp: String,
    pub recovery_suggested: bool,
}

impl DiagnosticRecord {
    pub fn new(kind: DiagnosticKind, message: &str, recovery_suggested: bool) -> Self {
        DiagnosticRecord {
            kind,
            message: message.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            recovery_suggested,
        }
    }
}

/// Thread-safe diagnostic logger for the Workflow Engine.
#[derive(Debug, Default)]
pub struct WorkflowDiagnostics {
    records: Arc<Mutex<Vec<DiagnosticRecord>>>,
    max_records: usize,
}

impl Clone for WorkflowDiagnostics {
    fn clone(&self) -> Self {
        WorkflowDiagnostics {
            records: self.records.clone(),
            max_records: self.max_records,
        }
    }
}

impl WorkflowDiagnostics {
    pub fn new(max_records: usize) -> Self {
        WorkflowDiagnostics {
            records: Arc::new(Mutex::new(Vec::new())),
            max_records,
        }
    }

    /// Record a diagnostic event.
    pub fn record(&self, kind: DiagnosticKind, message: &str, recovery_suggested: bool) {
        let mut inner = self.records.lock().unwrap();
        inner.push(DiagnosticRecord::new(kind, message, recovery_suggested));
        if inner.len() > self.max_records {
            let retain_from = inner.len() - self.max_records;
            inner.drain(..retain_from);
        }
    }

    /// Record that planning started.
    pub fn record_planning_started(&self) {
        self.record(DiagnosticKind::WorkflowPlanned, "Planning started", false);
    }

    /// Record that planning completed.
    pub fn record_planning_completed(&self, plan: &super::types::WorkflowPlan) {
        self.record(
            DiagnosticKind::WorkflowPlanned,
            &format!(
                "Planning completed: {} steps, valid={}",
                plan.total_steps, plan.is_valid
            ),
            false,
        );
    }

    /// Record a validation failure.
    pub fn record_validation_failure(&self, issues: &[super::types::WorkflowIssue]) {
        for issue in issues {
            self.record(
                DiagnosticKind::ValidationFailure,
                &format!("Validation issue: {}", issue),
                true,
            );
        }
    }

    /// Get all diagnostic records.
    pub fn records(&self) -> Vec<DiagnosticRecord> {
        self.records.lock().unwrap().clone()
    }

    /// Count records by kind.
    pub fn count_by_kind(&self, kind: &DiagnosticKind) -> usize {
        let inner = self.records.lock().unwrap();
        inner.iter().filter(|r| &r.kind == kind).count()
    }

    /// Total count of all records.
    pub fn total_count(&self) -> usize {
        let inner = self.records.lock().unwrap();
        inner.len()
    }

    /// Check if there are any failure records.
    pub fn has_failures(&self) -> bool {
        let inner = self.records.lock().unwrap();
        inner.iter().any(|r| {
            matches!(
                r.kind,
                DiagnosticKind::PlanningFailure
                    | DiagnosticKind::DependencyFailure
                    | DiagnosticKind::ValidationFailure
                    | DiagnosticKind::CycleDetected
                    | DiagnosticKind::ConflictDetected
            )
        })
    }

    /// Get recent records (last `n`).
    pub fn recent(&self, n: usize) -> Vec<DiagnosticRecord> {
        let inner = self.records.lock().unwrap();
        let start = inner.len().saturating_sub(n);
        inner[start..].to_vec()
    }

    /// Clear all records.
    pub fn clear(&self) {
        let mut inner = self.records.lock().unwrap();
        inner.clear();
    }

    /// Get summary statistics.
    pub fn summary(&self) -> Vec<(DiagnosticKind, usize)> {
        let inner = self.records.lock().unwrap();
        let mut counts: Vec<(DiagnosticKind, usize)> = Vec::new();
        for kind in [
            DiagnosticKind::WorkflowPlanned,
            DiagnosticKind::PlanningFailure,
            DiagnosticKind::DependencyFailure,
            DiagnosticKind::ValidationFailure,
            DiagnosticKind::CycleDetected,
            DiagnosticKind::ConflictDetected,
        ] {
            let count = inner.iter().filter(|r| r.kind == kind).count();
            if count > 0 {
                counts.push((kind, count));
            }
        }
        counts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_retrieve() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "test", false);
        assert_eq!(diag.total_count(), 1);
        let records = diag.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "test");
    }

    #[test]
    fn test_count_by_kind() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "w1", false);
        diag.record(DiagnosticKind::WorkflowPlanned, "w2", false);
        diag.record(DiagnosticKind::PlanningFailure, "f1", false);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::WorkflowPlanned), 2);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::PlanningFailure), 1);
    }

    #[test]
    fn test_has_failures() {
        let diag = WorkflowDiagnostics::new(100);
        assert!(!diag.has_failures());
        diag.record(DiagnosticKind::PlanningFailure, "err", false);
        assert!(diag.has_failures());
    }

    #[test]
    fn test_max_size_eviction() {
        let diag = WorkflowDiagnostics::new(5);
        for i in 0..10 {
            diag.record(DiagnosticKind::WorkflowPlanned, &format!("w{}", i), false);
        }
        assert_eq!(diag.total_count(), 5);
    }

    #[test]
    fn test_clear() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "test", false);
        diag.clear();
        assert_eq!(diag.total_count(), 0);
    }

    #[test]
    fn test_clone_shares_state() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "test", false);
        let cloned = diag.clone();
        assert_eq!(cloned.total_count(), 1);
        cloned.record(DiagnosticKind::PlanningFailure, "test2", false);
        assert_eq!(diag.total_count(), 2);
    }

    #[test]
    fn test_kind_labels() {
        assert_eq!(DiagnosticKind::WorkflowPlanned.label(), "workflow_planned");
        assert_eq!(DiagnosticKind::PlanningFailure.label(), "planning_failure");
        assert_eq!(
            DiagnosticKind::DependencyFailure.label(),
            "dependency_failure"
        );
        assert_eq!(
            DiagnosticKind::ValidationFailure.label(),
            "validation_failure"
        );
        assert_eq!(DiagnosticKind::CycleDetected.label(), "cycle_detected");
        assert_eq!(
            DiagnosticKind::ConflictDetected.label(),
            "conflict_detected"
        );
    }

    #[test]
    fn test_recent() {
        let diag = WorkflowDiagnostics::new(100);
        for i in 0..5 {
            diag.record(DiagnosticKind::WorkflowPlanned, &format!("w{}", i), false);
        }
        let recent = diag.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].message.contains("w2"));
        assert!(recent[2].message.contains("w4"));
    }

    #[test]
    fn test_summary() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "w1", false);
        diag.record(DiagnosticKind::WorkflowPlanned, "w2", false);
        diag.record(DiagnosticKind::PlanningFailure, "f1", false);
        let summary = diag.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2);
        assert_eq!(summary[1].1, 1);
    }

    #[test]
    fn test_summary_empty() {
        let diag = WorkflowDiagnostics::new(100);
        let summary = diag.summary();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_serializable() {
        let diag = WorkflowDiagnostics::new(100);
        diag.record(DiagnosticKind::WorkflowPlanned, "test", false);
        let records = diag.records();
        let json = serde_json::to_string(&records).expect("serialize");
        let _parsed: Vec<DiagnosticRecord> = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn test_record_planning_completed() {
        let diag = WorkflowDiagnostics::new(100);
        let plan = super::super::types::WorkflowPlan::new(
            "p1".to_string(),
            "i1",
            vec![],
            vec![],
            super::super::types::ExecutionStrategy::Sequential,
            vec![],
            vec![],
        );
        diag.record_planning_completed(&plan);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::WorkflowPlanned), 1);
    }
}

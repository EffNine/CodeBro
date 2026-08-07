//! Adaptive Validation Diagnostics — failure tracking and observability.
//!
/// Tracks validation metrics, failures, and statistics.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Type of diagnostic event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    /// A validation was started.
    ValidationStarted,
    /// A validation completed.
    ValidationCompleted,
    /// A policy failure occurred.
    PolicyFailure,
    /// A rule failure occurred.
    RuleFailure,
    /// A confidence threshold was breached.
    ConfidenceFailure,
    /// A risk threshold was breached.
    RiskFailure,
}

impl DiagnosticKind {
    pub fn label(&self) -> &str {
        match self {
            DiagnosticKind::ValidationStarted => "validation_started",
            DiagnosticKind::ValidationCompleted => "validation_completed",
            DiagnosticKind::PolicyFailure => "policy_failure",
            DiagnosticKind::RuleFailure => "rule_failure",
            DiagnosticKind::ConfidenceFailure => "confidence_failure",
            DiagnosticKind::RiskFailure => "risk_failure",
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

/// Thread-safe diagnostic logger for Adaptive Validation.
#[derive(Debug, Default)]
pub struct AdaptiveDiagnostics {
    records: Arc<Mutex<Vec<DiagnosticRecord>>>,
    max_records: usize,
}

impl Clone for AdaptiveDiagnostics {
    fn clone(&self) -> Self {
        AdaptiveDiagnostics {
            records: self.records.clone(),
            max_records: self.max_records,
        }
    }
}

impl AdaptiveDiagnostics {
    pub fn new(max_records: usize) -> Self {
        AdaptiveDiagnostics {
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

    /// Record that validation started.
    pub fn record_validation_started(&self) {
        self.record(
            DiagnosticKind::ValidationStarted,
            "Validation started",
            false,
        );
    }

    /// Record that validation completed.
    pub fn record_validation_completed(&self, report: &super::types::ValidationReport) {
        self.record(
            DiagnosticKind::ValidationCompleted,
            &format!(
                "Validation completed: {} issues, {} warnings, result={}",
                report.issues.len(),
                report.warnings.len(),
                report.result
            ),
            false,
        );
    }

    /// Record a policy failure.
    pub fn record_policy_failure(&self, policy_name: &str) {
        self.record(
            DiagnosticKind::PolicyFailure,
            &format!("Policy '{}' failed", policy_name),
            true,
        );
    }

    /// Record a rule failure.
    pub fn record_rule_failure(&self, rule_id: &str) {
        self.record(
            DiagnosticKind::RuleFailure,
            &format!("Rule '{}' failed", rule_id),
            true,
        );
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
            DiagnosticKind::ValidationStarted,
            DiagnosticKind::ValidationCompleted,
            DiagnosticKind::PolicyFailure,
            DiagnosticKind::RuleFailure,
            DiagnosticKind::ConfidenceFailure,
            DiagnosticKind::RiskFailure,
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
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "test", false);
        assert_eq!(diag.total_count(), 1);
        let records = diag.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "test");
    }

    #[test]
    fn test_count_by_kind() {
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "v1", false);
        diag.record(DiagnosticKind::ValidationStarted, "v2", false);
        diag.record(DiagnosticKind::PolicyFailure, "p1", false);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::ValidationStarted), 2);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::PolicyFailure), 1);
    }

    #[test]
    fn test_max_size_eviction() {
        let diag = AdaptiveDiagnostics::new(5);
        for i in 0..10 {
            diag.record(DiagnosticKind::ValidationStarted, &format!("v{}", i), false);
        }
        assert_eq!(diag.total_count(), 5);
    }

    #[test]
    fn test_clear() {
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "test", false);
        diag.clear();
        assert_eq!(diag.total_count(), 0);
    }

    #[test]
    fn test_clone_shares_state() {
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "test", false);
        let cloned = diag.clone();
        assert_eq!(cloned.total_count(), 1);
        cloned.record(DiagnosticKind::PolicyFailure, "test2", false);
        assert_eq!(diag.total_count(), 2);
    }

    #[test]
    fn test_kind_labels() {
        assert_eq!(
            DiagnosticKind::ValidationStarted.label(),
            "validation_started"
        );
        assert_eq!(
            DiagnosticKind::ValidationCompleted.label(),
            "validation_completed"
        );
        assert_eq!(DiagnosticKind::PolicyFailure.label(), "policy_failure");
        assert_eq!(DiagnosticKind::RuleFailure.label(), "rule_failure");
        assert_eq!(
            DiagnosticKind::ConfidenceFailure.label(),
            "confidence_failure"
        );
        assert_eq!(DiagnosticKind::RiskFailure.label(), "risk_failure");
    }

    #[test]
    fn test_recent() {
        let diag = AdaptiveDiagnostics::new(100);
        for i in 0..5 {
            diag.record(DiagnosticKind::ValidationStarted, &format!("v{}", i), false);
        }
        let recent = diag.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].message.contains("v2"));
        assert!(recent[2].message.contains("v4"));
    }

    #[test]
    fn test_summary() {
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "v1", false);
        diag.record(DiagnosticKind::ValidationStarted, "v2", false);
        diag.record(DiagnosticKind::PolicyFailure, "p1", false);
        let summary = diag.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2);
        assert_eq!(summary[1].1, 1);
    }

    #[test]
    fn test_summary_empty() {
        let diag = AdaptiveDiagnostics::new(100);
        let summary = diag.summary();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_serializable() {
        let diag = AdaptiveDiagnostics::new(100);
        diag.record(DiagnosticKind::ValidationStarted, "test", false);
        let records = diag.records();
        let json = serde_json::to_string(&records).expect("serialize");
        let _parsed: Vec<DiagnosticRecord> = serde_json::from_str(&json).expect("deserialize");
    }

    #[test]
    fn test_record_validation_completed() {
        let diag = AdaptiveDiagnostics::new(100);
        let report = super::super::types::ValidationReport::new(
            "r1".to_string(),
            super::super::types::ValidationResult::Pass,
        );
        diag.record_validation_completed(&report);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::ValidationCompleted), 1);
    }
}

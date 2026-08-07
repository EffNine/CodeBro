//! Recommendation Diagnostics — metrics and observability.
//!
/// Tracks recommendation production, filtering, and rule hit statistics.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Type of diagnostic event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    /// A recommendation was produced.
    RecommendationProduced,
    /// A recommendation was filtered out.
    RecommendationFiltered,
    /// A duplicate recommendation was removed.
    DuplicateRemoved,
    /// A conflicting recommendation was removed.
    ConflictRemoved,
    /// A rule was matched.
    RuleMatched,
    /// A rule was not matched.
    RuleNotMatched,
}

impl DiagnosticKind {
    pub fn label(&self) -> &str {
        match self {
            DiagnosticKind::RecommendationProduced => "recommendation_produced",
            DiagnosticKind::RecommendationFiltered => "recommendation_filtered",
            DiagnosticKind::DuplicateRemoved => "duplicate_removed",
            DiagnosticKind::ConflictRemoved => "conflict_removed",
            DiagnosticKind::RuleMatched => "rule_matched",
            DiagnosticKind::RuleNotMatched => "rule_not_matched",
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

/// Thread-safe diagnostic logger for the Recommendation Engine.
#[derive(Debug, Default)]
pub struct RecommendationDiagnostics {
    records: Arc<Mutex<Vec<DiagnosticRecord>>>,
    max_records: usize,
}

impl Clone for RecommendationDiagnostics {
    fn clone(&self) -> Self {
        RecommendationDiagnostics {
            records: self.records.clone(),
            max_records: self.max_records,
        }
    }
}

impl RecommendationDiagnostics {
    pub fn new(max_records: usize) -> Self {
        RecommendationDiagnostics {
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
            DiagnosticKind::RecommendationProduced,
            DiagnosticKind::RecommendationFiltered,
            DiagnosticKind::DuplicateRemoved,
            DiagnosticKind::ConflictRemoved,
            DiagnosticKind::RuleMatched,
            DiagnosticKind::RuleNotMatched,
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
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        assert_eq!(diag.total_count(), 1);
        let records = diag.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "test");
    }

    #[test]
    fn test_count_by_kind() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "r1", false);
        diag.record(DiagnosticKind::RecommendationProduced, "r2", false);
        diag.record(DiagnosticKind::RecommendationFiltered, "f1", false);
        assert_eq!(
            diag.count_by_kind(&DiagnosticKind::RecommendationProduced),
            2
        );
        assert_eq!(
            diag.count_by_kind(&DiagnosticKind::RecommendationFiltered),
            1
        );
    }

    #[test]
    fn test_max_size_eviction() {
        let diag = RecommendationDiagnostics::new(5);
        for i in 0..10 {
            diag.record(
                DiagnosticKind::RecommendationProduced,
                &format!("r{}", i),
                false,
            );
        }
        assert_eq!(diag.total_count(), 5);
    }

    #[test]
    fn test_clear() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        diag.clear();
        assert_eq!(diag.total_count(), 0);
    }

    #[test]
    fn test_clone_shares_state() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        let cloned = diag.clone();
        assert_eq!(cloned.total_count(), 1);
        cloned.record(DiagnosticKind::RecommendationFiltered, "test2", false);
        assert_eq!(diag.total_count(), 2);
    }

    #[test]
    fn test_kind_labels() {
        assert_eq!(
            DiagnosticKind::RecommendationProduced.label(),
            "recommendation_produced"
        );
        assert_eq!(
            DiagnosticKind::RecommendationFiltered.label(),
            "recommendation_filtered"
        );
        assert_eq!(
            DiagnosticKind::DuplicateRemoved.label(),
            "duplicate_removed"
        );
        assert_eq!(DiagnosticKind::ConflictRemoved.label(), "conflict_removed");
        assert_eq!(DiagnosticKind::RuleMatched.label(), "rule_matched");
        assert_eq!(DiagnosticKind::RuleNotMatched.label(), "rule_not_matched");
    }

    #[test]
    fn test_recent() {
        let diag = RecommendationDiagnostics::new(100);
        for i in 0..5 {
            diag.record(
                DiagnosticKind::RecommendationProduced,
                &format!("r{}", i),
                false,
            );
        }
        let recent = diag.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].message.contains("r2"));
        assert!(recent[2].message.contains("r4"));
    }

    #[test]
    fn test_summary() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "r1", false);
        diag.record(DiagnosticKind::RecommendationProduced, "r2", false);
        diag.record(DiagnosticKind::RecommendationFiltered, "f1", false);
        let summary = diag.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2);
        assert_eq!(summary[1].1, 1);
    }

    #[test]
    fn test_summary_empty() {
        let diag = RecommendationDiagnostics::new(100);
        let summary = diag.summary();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_serializable() {
        let diag = RecommendationDiagnostics::new(100);
        diag.record(DiagnosticKind::RecommendationProduced, "test", false);
        let records = diag.records();
        let json = serde_json::to_string(&records).expect("serialize");
        let _parsed: Vec<DiagnosticRecord> = serde_json::from_str(&json).expect("deserialize");
    }
}

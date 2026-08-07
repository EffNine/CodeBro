//! Intent Engine Diagnostics — failure tracking and observability.
//!
/// Tracks classification failures, ambiguity detections, resolver failures,
/// and command generation failures.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Type of intent diagnostic event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    /// A classification rule failed to match or produced an error.
    ClassificationFailure,
    /// An ambiguous input was detected.
    AmbiguityDetected,
    /// A resolver failed to generate commands.
    ResolverFailure,
    /// A command generation failed.
    CommandGenerationFailure,
    /// A preview generation failed.
    PreviewGenerationFailure,
    /// An ambiguity detection failed.
    AmbiguityDetectionFailure,
}

impl DiagnosticKind {
    pub fn label(&self) -> &str {
        match self {
            DiagnosticKind::ClassificationFailure => "classification_failure",
            DiagnosticKind::AmbiguityDetected => "ambiguity_detected",
            DiagnosticKind::ResolverFailure => "resolver_failure",
            DiagnosticKind::CommandGenerationFailure => "command_generation_failure",
            DiagnosticKind::PreviewGenerationFailure => "preview_generation_failure",
            DiagnosticKind::AmbiguityDetectionFailure => "ambiguity_detection_failure",
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

/// Thread-safe diagnostic logger for the Intent Engine.
#[derive(Debug, Default)]
pub struct IntentDiagnostics {
    records: Arc<Mutex<Vec<DiagnosticRecord>>>,
    max_records: usize,
}

impl Clone for IntentDiagnostics {
    fn clone(&self) -> Self {
        IntentDiagnostics {
            records: self.records.clone(),
            max_records: self.max_records,
        }
    }
}

impl IntentDiagnostics {
    pub fn new(max_records: usize) -> Self {
        IntentDiagnostics {
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

    /// Check if there are any failure records.
    pub fn has_failures(&self) -> bool {
        let inner = self.records.lock().unwrap();
        inner.iter().any(|r| {
            matches!(
                r.kind,
                DiagnosticKind::ClassificationFailure
                    | DiagnosticKind::ResolverFailure
                    | DiagnosticKind::CommandGenerationFailure
                    | DiagnosticKind::PreviewGenerationFailure
                    | DiagnosticKind::AmbiguityDetectionFailure
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
            DiagnosticKind::ClassificationFailure,
            DiagnosticKind::AmbiguityDetected,
            DiagnosticKind::ResolverFailure,
            DiagnosticKind::CommandGenerationFailure,
            DiagnosticKind::PreviewGenerationFailure,
            DiagnosticKind::AmbiguityDetectionFailure,
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
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "test error", true);
        assert_eq!(diag.total_count(), 1);
        let records = diag.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "test error");
        assert_eq!(records[0].kind, DiagnosticKind::ClassificationFailure);
    }

    #[test]
    fn test_count_by_kind() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "err1", false);
        diag.record(DiagnosticKind::ClassificationFailure, "err2", false);
        diag.record(DiagnosticKind::AmbiguityDetected, "amb1", false);
        assert_eq!(
            diag.count_by_kind(&DiagnosticKind::ClassificationFailure),
            2
        );
        assert_eq!(diag.count_by_kind(&DiagnosticKind::AmbiguityDetected), 1);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::ResolverFailure), 0);
    }

    #[test]
    fn test_has_failures() {
        let diag = IntentDiagnostics::new(100);
        assert!(!diag.has_failures());
        diag.record(DiagnosticKind::ClassificationFailure, "err", false);
        assert!(diag.has_failures());
        diag.record(DiagnosticKind::AmbiguityDetected, "amb", false);
        // Ambiguity detected is not a failure
        assert!(diag.has_failures());
    }

    #[test]
    fn test_max_size_eviction() {
        let diag = IntentDiagnostics::new(5);
        for i in 0..10 {
            diag.record(
                DiagnosticKind::ClassificationFailure,
                &format!("err{}", i),
                false,
            );
        }
        assert_eq!(diag.total_count(), 5);
    }

    #[test]
    fn test_clear() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "err", false);
        diag.clear();
        assert_eq!(diag.total_count(), 0);
    }

    #[test]
    fn test_clone_shares_state() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "err", false);
        let cloned = diag.clone();
        assert_eq!(cloned.total_count(), 1);
        cloned.record(DiagnosticKind::AmbiguityDetected, "amb", false);
        assert_eq!(diag.total_count(), 2);
    }

    #[test]
    fn test_kind_labels() {
        assert_eq!(
            DiagnosticKind::ClassificationFailure.label(),
            "classification_failure"
        );
        assert_eq!(
            DiagnosticKind::AmbiguityDetected.label(),
            "ambiguity_detected"
        );
        assert_eq!(DiagnosticKind::ResolverFailure.label(), "resolver_failure");
        assert_eq!(
            DiagnosticKind::CommandGenerationFailure.label(),
            "command_generation_failure"
        );
        assert_eq!(
            DiagnosticKind::PreviewGenerationFailure.label(),
            "preview_generation_failure"
        );
        assert_eq!(
            DiagnosticKind::AmbiguityDetectionFailure.label(),
            "ambiguity_detection_failure"
        );
    }

    #[test]
    fn test_recent() {
        let diag = IntentDiagnostics::new(100);
        for i in 0..5 {
            diag.record(
                DiagnosticKind::ClassificationFailure,
                &format!("err{}", i),
                false,
            );
        }
        let recent = diag.recent(3);
        assert_eq!(recent.len(), 3);
        assert!(recent[0].message.contains("err2"));
        assert!(recent[2].message.contains("err4"));
    }

    #[test]
    fn test_summary() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "err1", false);
        diag.record(DiagnosticKind::ClassificationFailure, "err2", false);
        diag.record(DiagnosticKind::AmbiguityDetected, "amb1", false);
        let summary = diag.summary();
        assert_eq!(summary.len(), 2);
        assert_eq!(summary[0].1, 2);
        assert_eq!(summary[1].1, 1);
    }

    #[test]
    fn test_summary_empty() {
        let diag = IntentDiagnostics::new(100);
        let summary = diag.summary();
        assert!(summary.is_empty());
    }

    #[test]
    fn test_serializable() {
        let diag = IntentDiagnostics::new(100);
        diag.record(DiagnosticKind::ClassificationFailure, "test", false);
        let records = diag.records();
        let json = serde_json::to_string(&records).expect("should serialize");
        let deserialized: Vec<DiagnosticRecord> =
            serde_json::from_str(&json).expect("should deserialize");
        assert_eq!(deserialized.len(), 1);
        assert_eq!(deserialized[0].kind, DiagnosticKind::ClassificationFailure);
    }
}

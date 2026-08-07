//! Preference Diagnostics — failure tracking and observability.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// Type of diagnostic failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiagnosticKind {
    LoadFailure,
    SaveFailure,
    MigrationFailure,
    ValidationFailure,
    CorruptionDetected,
    BackupFailure,
    RollbackFailure,
}

impl DiagnosticKind {
    pub fn label(&self) -> &str {
        match self {
            DiagnosticKind::LoadFailure => "load_failure",
            DiagnosticKind::SaveFailure => "save_failure",
            DiagnosticKind::MigrationFailure => "migration_failure",
            DiagnosticKind::ValidationFailure => "validation_failure",
            DiagnosticKind::CorruptionDetected => "corruption_detected",
            DiagnosticKind::BackupFailure => "backup_failure",
            DiagnosticKind::RollbackFailure => "rollback_failure",
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

/// Thread-safe diagnostic logger.
#[derive(Debug, Default)]
pub struct PreferenceDiagnostics {
    records: Arc<Mutex<Vec<DiagnosticRecord>>>,
    max_records: usize,
}

impl Clone for PreferenceDiagnostics {
    fn clone(&self) -> Self {
        PreferenceDiagnostics {
            records: self.records.clone(),
            max_records: self.max_records,
        }
    }
}

impl PreferenceDiagnostics {
    pub fn new(max_records: usize) -> Self {
        PreferenceDiagnostics {
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
                DiagnosticKind::LoadFailure
                    | DiagnosticKind::SaveFailure
                    | DiagnosticKind::MigrationFailure
                    | DiagnosticKind::ValidationFailure
                    | DiagnosticKind::CorruptionDetected
                    | DiagnosticKind::BackupFailure
                    | DiagnosticKind::RollbackFailure
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_and_retrieve() {
        let diag = PreferenceDiagnostics::new(100);
        diag.record(DiagnosticKind::LoadFailure, "test error", true);
        assert_eq!(diag.total_count(), 1);
        let records = diag.records();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].message, "test error");
    }

    #[test]
    fn test_count_by_kind() {
        let diag = PreferenceDiagnostics::new(100);
        diag.record(DiagnosticKind::LoadFailure, "err1", false);
        diag.record(DiagnosticKind::LoadFailure, "err2", false);
        diag.record(DiagnosticKind::SaveFailure, "err3", true);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::LoadFailure), 2);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::SaveFailure), 1);
        assert_eq!(diag.count_by_kind(&DiagnosticKind::MigrationFailure), 0);
    }

    #[test]
    fn test_has_failures() {
        let diag = PreferenceDiagnostics::new(100);
        assert!(!diag.has_failures());
        diag.record(DiagnosticKind::LoadFailure, "err", false);
        assert!(diag.has_failures());
    }

    #[test]
    fn test_max_size_eviction() {
        let diag = PreferenceDiagnostics::new(5);
        for i in 0..10 {
            diag.record(DiagnosticKind::LoadFailure, &format!("err{}", i), false);
        }
        assert_eq!(diag.total_count(), 5);
    }

    #[test]
    fn test_clear() {
        let diag = PreferenceDiagnostics::new(100);
        diag.record(DiagnosticKind::LoadFailure, "err", false);
        diag.clear();
        assert_eq!(diag.total_count(), 0);
    }

    #[test]
    fn test_clone_shares_state() {
        let diag = PreferenceDiagnostics::new(100);
        diag.record(DiagnosticKind::LoadFailure, "err", false);
        let cloned = diag.clone();
        assert_eq!(cloned.total_count(), 1);
        cloned.record(DiagnosticKind::SaveFailure, "err2", true);
        assert_eq!(diag.total_count(), 2);
    }

    #[test]
    fn test_kind_labels() {
        assert_eq!(DiagnosticKind::LoadFailure.label(), "load_failure");
        assert_eq!(DiagnosticKind::SaveFailure.label(), "save_failure");
        assert_eq!(
            DiagnosticKind::MigrationFailure.label(),
            "migration_failure"
        );
        assert_eq!(
            DiagnosticKind::ValidationFailure.label(),
            "validation_failure"
        );
        assert_eq!(
            DiagnosticKind::CorruptionDetected.label(),
            "corruption_detected"
        );
        assert_eq!(DiagnosticKind::BackupFailure.label(), "backup_failure");
        assert_eq!(DiagnosticKind::RollbackFailure.label(), "rollback_failure");
    }
}

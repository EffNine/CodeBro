#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Diagnostics for the Service Registry.
//!
//! Tracks:
//! - Registry statistics
//! - Failed lookups
//! - Permission violations
//! - Lifecycle events

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use super::types::*;

struct DiagnosticsInner {
    stats: RegistryStatistics,
    failed_lookups: VecDeque<ResolutionFailureRecord>,
    permission_violations: VecDeque<PermissionViolationRecord>,
    lifecycle_events: VecDeque<RegistryDiagnosticEvent>,
    max_history: usize,
}

/// Diagnostic observer for the service registry.
#[derive(Clone)]
pub struct ServiceDiagnostics {
    inner: Arc<Mutex<DiagnosticsInner>>,
}

impl ServiceDiagnostics {
    pub fn new() -> Self {
        ServiceDiagnostics {
            inner: Arc::new(Mutex::new(DiagnosticsInner {
                stats: RegistryStatistics::new(),
                failed_lookups: VecDeque::new(),
                permission_violations: VecDeque::new(),
                lifecycle_events: VecDeque::new(),
                max_history: 1000,
            })),
        }
    }

    pub fn with_capacity(max_history: usize) -> Self {
        ServiceDiagnostics {
            inner: Arc::new(Mutex::new(DiagnosticsInner {
                stats: RegistryStatistics::new(),
                failed_lookups: VecDeque::new(),
                permission_violations: VecDeque::new(),
                lifecycle_events: VecDeque::new(),
                max_history,
            })),
        }
    }

    /// Record a service registration.
    pub fn record_registration(&self, event: &RegistryDiagnosticEvent) {
        let mut inner = self.inner.lock().unwrap();
        if let RegistryDiagnosticEvent::ServiceRegistered { .. } = event {
            inner.stats.total_registered += 1;
        }
        inner.lifecycle_events.push_back(event.clone());
        if inner.lifecycle_events.len() > inner.max_history {
            inner.lifecycle_events.pop_front();
        }
    }

    /// Record a service resolution.
    pub fn record_resolution(&self, event: &RegistryDiagnosticEvent) {
        let mut inner = self.inner.lock().unwrap();
        inner.stats.resolution_count += 1;
        if let RegistryDiagnosticEvent::ServiceResolved { .. } = event {
            inner.stats.resolution_success_count += 1;
        }
        inner.lifecycle_events.push_back(event.clone());
        if inner.lifecycle_events.len() > inner.max_history {
            inner.lifecycle_events.pop_front();
        }
    }

    /// Record a resolution failure.
    pub fn record_failure(&self, query_name: &str, reason: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.stats.resolution_failure_count += 1;
        inner.stats.total_errors += 1;
        let record = ResolutionFailureRecord {
            query_name: query_name.to_string(),
            reason: reason.to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
        };
        inner.failed_lookups.push_back(record.clone());
        if inner.failed_lookups.len() > inner.max_history {
            inner.failed_lookups.pop_front();
        }
        inner.stats.recent_failures.push(record);
        if inner.stats.recent_failures.len() > 50 {
            inner.stats.recent_failures.remove(0);
        }
    }

    /// Record a permission violation.
    pub fn record_permission_violation(&self, event: &RegistryDiagnosticEvent) {
        let mut inner = self.inner.lock().unwrap();
        if let RegistryDiagnosticEvent::PermissionDenied { .. } = event {
            inner.stats.permission_violations += 1;
        }
        if let RegistryDiagnosticEvent::DependencyViolation { .. } = event {
            inner.stats.dependency_violations += 1;
        }
        inner.lifecycle_events.push_back(event.clone());
        if inner.lifecycle_events.len() > inner.max_history {
            inner.lifecycle_events.pop_front();
        }
    }

    /// Record a lifecycle event.
    pub fn record_lifecycle(&self, event: &RegistryDiagnosticEvent) {
        let mut inner = self.inner.lock().unwrap();
        match event {
            RegistryDiagnosticEvent::ServiceActivated { .. } => {
                inner.stats.total_activated += 1;
            }
            RegistryDiagnosticEvent::ServiceDeactivated { .. } => {
                inner.stats.total_deactivated += 1;
            }
            RegistryDiagnosticEvent::ServiceRegistered { .. } => {
                inner.stats.total_registered += 1;
            }
            RegistryDiagnosticEvent::ServiceUnregistered { .. } => {
                inner.stats.total_registered = inner.stats.total_registered.saturating_sub(1);
            }
            _ => {}
        }
        inner.lifecycle_events.push_back(event.clone());
        if inner.lifecycle_events.len() > inner.max_history {
            inner.lifecycle_events.pop_front();
        }
    }

    /// Get current statistics.
    pub fn statistics(&self) -> RegistryStatistics {
        let inner = self.inner.lock().unwrap();
        inner.stats.clone()
    }

    /// Get recent failed lookups.
    pub fn recent_failures(&self) -> Vec<ResolutionFailureRecord> {
        let inner = self.inner.lock().unwrap();
        inner.failed_lookups.iter().cloned().collect()
    }

    /// Get recent permission violations.
    pub fn recent_violations(&self) -> Vec<RegistryDiagnosticEvent> {
        let inner = self.inner.lock().unwrap();
        inner
            .lifecycle_events
            .iter()
            .filter(|e| matches!(e, RegistryDiagnosticEvent::PermissionDenied { .. }))
            .cloned()
            .collect()
    }

    /// Get recent lifecycle events.
    pub fn recent_events(&self, limit: usize) -> Vec<RegistryDiagnosticEvent> {
        let inner = self.inner.lock().unwrap();
        inner
            .lifecycle_events
            .iter()
            .rev()
            .take(limit)
            .cloned()
            .rev()
            .collect()
    }

    /// Get a snapshot of all diagnostics.
    pub fn snapshot(&self) -> DiagnosticSnapshot {
        let inner = self.inner.lock().unwrap();
        DiagnosticSnapshot {
            timestamp: chrono::Local::now().to_rfc3339(),
            stats: inner.stats.clone(),
            recent_failures: inner.failed_lookups.iter().cloned().collect(),
            recent_events: inner
                .lifecycle_events
                .iter()
                .rev()
                .take(20)
                .cloned()
                .rev()
                .collect(),
        }
    }

    /// Reset all diagnostics.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.stats = RegistryStatistics::new();
        inner.failed_lookups.clear();
        inner.permission_violations.clear();
        inner.lifecycle_events.clear();
    }
}

impl Default for ServiceDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

/// A snapshot of registry diagnostics at a point in time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSnapshot {
    pub timestamp: String,
    pub stats: RegistryStatistics,
    pub recent_failures: Vec<ResolutionFailureRecord>,
    pub recent_events: Vec<RegistryDiagnosticEvent>,
}

impl DiagnosticSnapshot {
    pub fn summary(&self) -> String {
        format!(
            "DiagnosticSnapshot@{}\n{}",
            self.timestamp,
            self.stats.summary()
        )
    }
}

/// Permission violation record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionViolationRecord {
    pub requester: String,
    pub service_id: ServiceId,
    pub required_access: AccessLevel,
    pub timestamp: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_record_registration() {
        let diag = ServiceDiagnostics::new();
        diag.record_lifecycle(&RegistryDiagnosticEvent::ServiceRegistered {
            service_id: ServiceId::new("s1").unwrap(),
            version: ServiceVersion::new("1.0.0").unwrap(),
            provider: "plugin-a".to_string(),
        });
        let stats = diag.statistics();
        assert_eq!(stats.total_registered, 1);
    }

    #[test]
    fn test_record_resolution() {
        let diag = ServiceDiagnostics::new();
        diag.record_resolution(&RegistryDiagnosticEvent::ServiceResolved {
            service_id: ServiceId::new("s1").unwrap(),
            version: ServiceVersion::new("1.0.0").unwrap(),
            requester: "plugin-a".to_string(),
            resolution_time_ms: 0.5,
        });
        let stats = diag.statistics();
        assert_eq!(stats.resolution_count, 1);
        assert_eq!(stats.resolution_success_count, 1);
    }

    #[test]
    fn test_record_failure() {
        let diag = ServiceDiagnostics::new();
        diag.record_failure("data", "not found");
        let stats = diag.statistics();
        assert_eq!(stats.resolution_failure_count, 1);
        assert_eq!(stats.total_errors, 1);
        let failures = diag.recent_failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].query_name, "data");
    }

    #[test]
    fn test_record_permission_violation() {
        let diag = ServiceDiagnostics::new();
        diag.record_permission_violation(&RegistryDiagnosticEvent::PermissionDenied {
            requester: "bad-plugin".to_string(),
            service_id: ServiceId::new("s1").unwrap(),
            required_access: AccessLevel::Write,
        });
        let stats = diag.statistics();
        assert_eq!(stats.permission_violations, 1);
        let violations = diag.recent_violations();
        assert_eq!(violations.len(), 1);
    }

    #[test]
    fn test_record_lifecycle_events() {
        let diag = ServiceDiagnostics::new();
        diag.record_lifecycle(&RegistryDiagnosticEvent::ServiceActivated {
            service_id: ServiceId::new("s1").unwrap(),
        });
        diag.record_lifecycle(&RegistryDiagnosticEvent::ServiceDeactivated {
            service_id: ServiceId::new("s1").unwrap(),
        });
        let stats = diag.statistics();
        assert_eq!(stats.total_activated, 1);
        assert_eq!(stats.total_deactivated, 1);
    }

    #[test]
    fn test_snapshot() {
        let diag = ServiceDiagnostics::new();
        diag.record_registration(&RegistryDiagnosticEvent::ServiceRegistered {
            service_id: ServiceId::new("s1").unwrap(),
            version: ServiceVersion::new("1.0.0").unwrap(),
            provider: "p".to_string(),
        });
        let snap = diag.snapshot();
        assert_eq!(snap.stats.total_registered, 1);
        assert!(!snap.timestamp.is_empty());
    }

    #[test]
    fn test_clear() {
        let diag = ServiceDiagnostics::new();
        diag.record_failure("data", "not found");
        diag.record_registration(&RegistryDiagnosticEvent::ServiceRegistered {
            service_id: ServiceId::new("s1").unwrap(),
            version: ServiceVersion::new("1.0.0").unwrap(),
            provider: "p".to_string(),
        });
        diag.clear();
        let stats = diag.statistics();
        assert_eq!(stats.total_registered, 0);
        assert_eq!(stats.total_errors, 0);
        assert!(diag.recent_failures().is_empty());
    }

    #[test]
    fn test_history_limit() {
        let diag = ServiceDiagnostics::with_capacity(5);
        for i in 0..10 {
            diag.record_failure(&format!("query_{i}"), "error");
        }
        let failures = diag.recent_failures();
        assert!(failures.len() <= 5);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let diag = ServiceDiagnostics::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let d = diag.clone();
                thread::spawn(move || {
                    for j in 0..100 {
                        d.record_failure(&format!("q_{i}_{j}"), "err");
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        let stats = diag.statistics();
        assert_eq!(stats.resolution_failure_count, 1000);
    }

    #[test]
    fn test_dependency_violation_recording() {
        let diag = ServiceDiagnostics::new();
        diag.record_permission_violation(&RegistryDiagnosticEvent::DependencyViolation {
            service_id: ServiceId::new("s1").unwrap(),
            missing_dependency: ServiceId::new("dep1").unwrap(),
        });
        let stats = diag.statistics();
        assert_eq!(stats.dependency_violations, 1);
    }
}

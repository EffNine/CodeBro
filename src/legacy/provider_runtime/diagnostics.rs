#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider Diagnostics for the Provider Runtime (P10.3).
//!
//! Tracks selection decisions, capability mismatches, health
//! transitions, retry history, failover history, and per-provider
//! statistics. All diagnostics are observational.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::capabilities::Capability;
use super::circuit_breaker::CircuitBreakerState;
use super::types::{HealthState, ProviderId};

/// Diagnostic events emitted by the Provider Runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderEvent {
    ProviderSelected {
        provider: ProviderId,
        reason: String,
        correlation_id: String,
    },
    ProviderRejected {
        provider: ProviderId,
        reason: String,
        correlation_id: String,
    },
    ProviderUnavailable {
        provider: ProviderId,
        state: HealthState,
    },
    RetryStarted {
        provider: ProviderId,
        attempt: usize,
    },
    RetryCompleted {
        provider: ProviderId,
        attempts_used: usize,
    },
    FailoverTriggered {
        from: ProviderId,
        to: ProviderId,
        reason: String,
    },
    CostRecorded {
        provider: ProviderId,
        estimated: f64,
    },
    ProviderRecovered {
        provider: ProviderId,
        from: HealthState,
        to: HealthState,
    },
    CircuitBreakerOpened {
        provider: ProviderId,
        failure_count: u32,
        correlation_id: String,
    },
    CircuitBreakerClosed {
        provider: ProviderId,
        correlation_id: String,
    },
    CircuitBreakerHalfOpened {
        provider: ProviderId,
        correlation_id: String,
    },
    CircuitBreakerRequestRejected {
        provider: ProviderId,
        correlation_id: String,
    },
    CircuitBreakerRecoverySucceeded {
        provider: ProviderId,
        correlation_id: String,
    },
    CircuitBreakerRecoveryFailed {
        provider: ProviderId,
        correlation_id: String,
    },
}

impl ProviderEvent {
    /// Short summary for observers.
    pub fn summary(&self) -> String {
        match self {
            ProviderEvent::ProviderSelected { provider, .. } => {
                format!("selected {provider}")
            }
            ProviderEvent::ProviderRejected {
                provider, reason, ..
            } => {
                format!("rejected {provider}: {reason}")
            }
            ProviderEvent::ProviderUnavailable { provider, state } => {
                format!("{provider} unavailable ({state:?})")
            }
            ProviderEvent::RetryStarted { provider, attempt } => {
                format!("retry {provider} attempt {attempt}")
            }
            ProviderEvent::RetryCompleted {
                provider,
                attempts_used,
            } => {
                format!("retries done for {provider} ({attempts_used})")
            }
            ProviderEvent::FailoverTriggered { from, to, reason } => {
                format!("failover {from} -> {to}: {reason}")
            }
            ProviderEvent::CostRecorded {
                provider,
                estimated,
            } => {
                format!("cost {provider} = {estimated:.6}")
            }
            ProviderEvent::ProviderRecovered { provider, from, to } => {
                format!("recovered {provider} {from:?}->{to:?}")
            }
            ProviderEvent::CircuitBreakerOpened {
                provider,
                failure_count,
                ..
            } => {
                format!("breaker opened for {provider} (failures={failure_count})")
            }
            ProviderEvent::CircuitBreakerClosed { provider, .. } => {
                format!("breaker closed for {provider}")
            }
            ProviderEvent::CircuitBreakerHalfOpened { provider, .. } => {
                format!("breaker half-open for {provider}")
            }
            ProviderEvent::CircuitBreakerRequestRejected { provider, .. } => {
                format!("request rejected for {provider}")
            }
            ProviderEvent::CircuitBreakerRecoverySucceeded { provider, .. } => {
                format!("recovery succeeded for {provider}")
            }
            ProviderEvent::CircuitBreakerRecoveryFailed { provider, .. } => {
                format!("recovery failed for {provider}")
            }
        }
    }
}

/// A recorded selection decision (for the diagnostics report).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionRecord {
    pub correlation_id: String,
    pub selected: ProviderId,
    pub rejected: Vec<(ProviderId, String)>,
    pub considered: usize,
}

/// A recorded capability mismatch.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MismatchRecord {
    pub provider: ProviderId,
    pub missing: Vec<Capability>,
    pub correlation_id: String,
}

/// A recorded health transition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthTransitionRecord {
    pub provider: ProviderId,
    pub from: HealthState,
    pub to: HealthState,
}

/// A recorded retry episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryRecord {
    pub provider: ProviderId,
    pub correlation_id: String,
    pub attempts: usize,
    pub succeeded: bool,
}

/// A recorded failover episode.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailoverRecord {
    pub correlation_id: String,
    pub from: ProviderId,
    pub to: ProviderId,
    pub reason: String,
}

/// Per-provider statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderStatistics {
    pub selections: usize,
    pub rejections: usize,
    pub retries: usize,
    pub failovers_from: usize,
    pub cost_records: usize,
}

/// The diagnostics collector.
#[derive(Clone, Default)]
pub struct ProviderDiagnostics {
    inner: Arc<RwLock<DiagnosticsInner>>,
}

#[derive(Default)]
struct DiagnosticsInner {
    events: Vec<ProviderEvent>,
    selections: Vec<SelectionRecord>,
    mismatches: Vec<MismatchRecord>,
    health_transitions: Vec<HealthTransitionRecord>,
    retries: Vec<RetryRecord>,
    failovers: Vec<FailoverRecord>,
    stats: HashMap<ProviderId, ProviderStatistics>,
    circuit_breaker_events: Vec<ProviderEvent>,
    max: usize,
}

impl ProviderDiagnostics {
    pub fn new() -> Self {
        ProviderDiagnostics {
            inner: Arc::new(RwLock::new(DiagnosticsInner {
                max: 10_000,
                ..Default::default()
            })),
        }
    }

    fn push<T>(vec: &mut Vec<T>, item: T, max: usize) {
        vec.push(item);
        while vec.len() > max {
            vec.remove(0);
        }
    }

    fn bump(&self, id: &ProviderId, f: impl Fn(&mut ProviderStatistics)) {
        let mut inner = self.inner.write().unwrap();
        let stat = inner.stats.entry(id.clone()).or_default();
        f(stat);
    }

    pub fn record_selected(&self, provider: &ProviderId, reason: &str, correlation_id: &str) {
        self.emit(ProviderEvent::ProviderSelected {
            provider: provider.clone(),
            reason: reason.to_string(),
            correlation_id: correlation_id.to_string(),
        });
        {
            let mut inner = self.inner.write().unwrap();
            let max = inner.max;
            Self::push(
                &mut inner.selections,
                SelectionRecord {
                    correlation_id: correlation_id.to_string(),
                    selected: provider.clone(),
                    rejected: Vec::new(),
                    considered: 0,
                },
                max,
            );
        }
        self.bump(provider, |s| s.selections += 1);
    }

    pub fn record_rejected(&self, provider: &ProviderId, reason: &str, correlation_id: &str) {
        self.emit(ProviderEvent::ProviderRejected {
            provider: provider.clone(),
            reason: reason.to_string(),
            correlation_id: correlation_id.to_string(),
        });
        self.bump(provider, |s| s.rejections += 1);
    }

    pub fn record_unavailable(&self, provider: &ProviderId, state: HealthState) {
        self.emit(ProviderEvent::ProviderUnavailable {
            provider: provider.clone(),
            state,
        });
    }

    pub fn record_health_transition(
        &self,
        provider: &ProviderId,
        from: HealthState,
        to: HealthState,
    ) {
        if from == to {
            return;
        }
        self.emit(ProviderEvent::ProviderRecovered {
            provider: provider.clone(),
            from,
            to,
        });
        let mut inner = self.inner.write().unwrap();
        let max = inner.max;
        Self::push(
            &mut inner.health_transitions,
            HealthTransitionRecord {
                provider: provider.clone(),
                from,
                to,
            },
            max,
        );
    }

    pub fn record_retry(&self, provider: &ProviderId, attempt: usize, correlation_id: &str) {
        self.emit(ProviderEvent::RetryStarted {
            provider: provider.clone(),
            attempt,
        });
        let mut inner = self.inner.write().unwrap();
        let max = inner.max;
        Self::push(
            &mut inner.retries,
            RetryRecord {
                provider: provider.clone(),
                correlation_id: correlation_id.to_string(),
                attempts: attempt,
                succeeded: false,
            },
            max,
        );
        let stat = inner.stats.entry(provider.clone()).or_default();
        stat.retries += 1;
    }

    pub fn record_retry_done(&self, provider: &ProviderId, attempts_used: usize) {
        self.emit(ProviderEvent::RetryCompleted {
            provider: provider.clone(),
            attempts_used,
        });
        let mut inner = self.inner.write().unwrap();
        if let Some(rec) = inner
            .retries
            .iter_mut()
            .rev()
            .find(|r| r.provider == *provider)
        {
            rec.attempts = attempts_used;
            rec.succeeded = true;
        }
    }

    pub fn record_failover(
        &self,
        correlation_id: &str,
        from: &ProviderId,
        to: &ProviderId,
        reason: &str,
    ) {
        self.emit(ProviderEvent::FailoverTriggered {
            from: from.clone(),
            to: to.clone(),
            reason: reason.to_string(),
        });
        let mut inner = self.inner.write().unwrap();
        let max = inner.max;
        Self::push(
            &mut inner.failovers,
            FailoverRecord {
                correlation_id: correlation_id.to_string(),
                from: from.clone(),
                to: to.clone(),
                reason: reason.to_string(),
            },
            max,
        );
        let stat = inner.stats.entry(from.clone()).or_default();
        stat.failovers_from += 1;
    }

    pub fn record_cost(&self, provider: &ProviderId, estimated: f64) {
        self.emit(ProviderEvent::CostRecorded {
            provider: provider.clone(),
            estimated,
        });
        let mut inner = self.inner.write().unwrap();
        let stat = inner.stats.entry(provider.clone()).or_default();
        stat.cost_records += 1;
    }

    pub fn record_mismatch(
        &self,
        provider: &ProviderId,
        missing: Vec<Capability>,
        correlation_id: &str,
    ) {
        let mut inner = self.inner.write().unwrap();
        let max = inner.max;
        Self::push(
            &mut inner.mismatches,
            MismatchRecord {
                provider: provider.clone(),
                missing,
                correlation_id: correlation_id.to_string(),
            },
            max,
        );
    }

    pub fn record_breaker_opened(
        &self,
        provider: &ProviderId,
        failure_count: u32,
        correlation_id: &str,
    ) {
        let event = ProviderEvent::CircuitBreakerOpened {
            provider: provider.clone(),
            failure_count,
            correlation_id: correlation_id.to_string(),
        };
        self.emit(event.clone());
        let max = self.inner.read().unwrap().max;
        {
            let mut inner = self.inner.write().unwrap();
            Self::push(&mut inner.circuit_breaker_events, event, max);
        }
    }

    pub fn record_breaker_closed(&self, provider: &ProviderId, correlation_id: &str) {
        let event = ProviderEvent::CircuitBreakerClosed {
            provider: provider.clone(),
            correlation_id: correlation_id.to_string(),
        };
        self.emit(event.clone());
        let max = self.inner.read().unwrap().max;
        {
            let mut inner = self.inner.write().unwrap();
            Self::push(&mut inner.circuit_breaker_events, event, max);
        }
    }

    pub fn record_breaker_half_opened(&self, provider: &ProviderId, correlation_id: &str) {
        let event = ProviderEvent::CircuitBreakerHalfOpened {
            provider: provider.clone(),
            correlation_id: correlation_id.to_string(),
        };
        self.emit(event.clone());
        let max = self.inner.read().unwrap().max;
        {
            let mut inner = self.inner.write().unwrap();
            Self::push(&mut inner.circuit_breaker_events, event, max);
        }
    }

    pub fn record_breaker_rejected(&self, provider: &ProviderId, correlation_id: &str) {
        let event = ProviderEvent::CircuitBreakerRequestRejected {
            provider: provider.clone(),
            correlation_id: correlation_id.to_string(),
        };
        self.emit(event.clone());
        let max = self.inner.read().unwrap().max;
        {
            let mut inner = self.inner.write().unwrap();
            Self::push(&mut inner.circuit_breaker_events, event, max);
        }
    }

    pub fn record_breaker_recovery_succeeded(&self, provider: &ProviderId, correlation_id: &str) {
        let event = ProviderEvent::CircuitBreakerRecoverySucceeded {
            provider: provider.clone(),
            correlation_id: correlation_id.to_string(),
        };
        self.emit(event.clone());
        let max = self.inner.read().unwrap().max;
        {
            let mut inner = self.inner.write().unwrap();
            Self::push(&mut inner.circuit_breaker_events, event, max);
        }
    }

    pub fn record_breaker_recovery_failed(&self, provider: &ProviderId, correlation_id: &str) {
        let event = ProviderEvent::CircuitBreakerRecoveryFailed {
            provider: provider.clone(),
            correlation_id: correlation_id.to_string(),
        };
        self.emit(event.clone());
        let max = self.inner.read().unwrap().max;
        {
            let mut inner = self.inner.write().unwrap();
            Self::push(&mut inner.circuit_breaker_events, event, max);
        }
    }

    fn emit(&self, event: ProviderEvent) {
        let mut inner = self.inner.write().unwrap();
        let max = inner.max;
        Self::push(&mut inner.events, event, max);
    }

    pub fn events(&self) -> Vec<ProviderEvent> {
        self.inner.read().unwrap().events.clone()
    }

    pub fn selections(&self) -> Vec<SelectionRecord> {
        self.inner.read().unwrap().selections.clone()
    }

    pub fn mismatches(&self) -> Vec<MismatchRecord> {
        self.inner.read().unwrap().mismatches.clone()
    }

    pub fn health_transitions(&self) -> Vec<HealthTransitionRecord> {
        self.inner.read().unwrap().health_transitions.clone()
    }

    pub fn retries(&self) -> Vec<RetryRecord> {
        self.inner.read().unwrap().retries.clone()
    }

    pub fn failovers(&self) -> Vec<FailoverRecord> {
        self.inner.read().unwrap().failovers.clone()
    }

    pub fn statistics(&self, provider: &ProviderId) -> ProviderStatistics {
        self.inner
            .read()
            .unwrap()
            .stats
            .get(provider)
            .cloned()
            .unwrap_or_default()
    }

    pub fn summary(&self) -> DiagnosticsSummary {
        let inner = self.inner.read().unwrap();
        DiagnosticsSummary {
            events: inner.events.len(),
            selections: inner.selections.len(),
            mismatches: inner.mismatches.len(),
            health_transitions: inner.health_transitions.len(),
            retries: inner.retries.len(),
            failovers: inner.failovers.len(),
            providers_tracked: inner.stats.len(),
            circuit_breaker_events: inner.circuit_breaker_events.len(),
        }
    }
}

/// Diagnostics summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DiagnosticsSummary {
    pub events: usize,
    pub selections: usize,
    pub mismatches: usize,
    pub health_transitions: usize,
    pub retries: usize,
    pub failovers: usize,
    pub providers_tracked: usize,
    pub circuit_breaker_events: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_selection_records() {
        let d = ProviderDiagnostics::new();
        d.record_selected(&ProviderId::new("p"), "cheap", "c1");
        let stats = d.statistics(&ProviderId::new("p"));
        assert_eq!(stats.selections, 1);
        assert!(d
            .events()
            .iter()
            .any(|e| matches!(e, ProviderEvent::ProviderSelected { .. })));
    }

    #[test]
    fn test_rejection_records() {
        let d = ProviderDiagnostics::new();
        d.record_rejected(&ProviderId::new("p"), "unhealthy", "c2");
        assert_eq!(d.statistics(&ProviderId::new("p")).rejections, 1);
    }

    #[test]
    fn test_unavailable_event() {
        let d = ProviderDiagnostics::new();
        d.record_unavailable(&ProviderId::new("p"), HealthState::Unavailable);
        assert!(d
            .events()
            .iter()
            .any(|e| matches!(e, ProviderEvent::ProviderUnavailable { .. })));
    }

    #[test]
    fn test_health_transition_recorded() {
        let d = ProviderDiagnostics::new();
        d.record_health_transition(
            &ProviderId::new("p"),
            HealthState::Healthy,
            HealthState::Degraded,
        );
        assert_eq!(d.health_transitions().len(), 1);
    }

    #[test]
    fn test_health_transition_noop_ignored() {
        let d = ProviderDiagnostics::new();
        d.record_health_transition(
            &ProviderId::new("p"),
            HealthState::Healthy,
            HealthState::Healthy,
        );
        assert_eq!(d.health_transitions().len(), 0);
    }

    #[test]
    fn test_retry_records() {
        let d = ProviderDiagnostics::new();
        d.record_retry(&ProviderId::new("p"), 1, "c3");
        assert_eq!(d.retries().len(), 1);
        assert_eq!(d.statistics(&ProviderId::new("p")).retries, 1);
    }

    #[test]
    fn test_retry_done_marks_success() {
        let d = ProviderDiagnostics::new();
        d.record_retry(&ProviderId::new("p"), 1, "c4");
        d.record_retry_done(&ProviderId::new("p"), 3);
        let rec = d.retries().pop().unwrap();
        assert!(rec.succeeded);
        assert_eq!(rec.attempts, 3);
    }

    #[test]
    fn test_failover_recorded() {
        let d = ProviderDiagnostics::new();
        d.record_failover(
            "c5",
            &ProviderId::new("a"),
            &ProviderId::new("b"),
            "unhealthy",
        );
        assert_eq!(d.failovers().len(), 1);
        assert_eq!(d.statistics(&ProviderId::new("a")).failovers_from, 1);
        assert!(d
            .events()
            .iter()
            .any(|e| matches!(e, ProviderEvent::FailoverTriggered { .. })));
    }

    #[test]
    fn test_cost_recorded() {
        let d = ProviderDiagnostics::new();
        d.record_cost(&ProviderId::new("p"), 0.01);
        assert_eq!(d.statistics(&ProviderId::new("p")).cost_records, 1);
    }

    #[test]
    fn test_mismatch_recorded() {
        let d = ProviderDiagnostics::new();
        d.record_mismatch(&ProviderId::new("p"), vec![Capability::Vision], "c6");
        assert_eq!(d.mismatches().len(), 1);
        assert_eq!(d.mismatches()[0].missing, vec![Capability::Vision]);
    }

    #[test]
    fn test_event_summaries() {
        assert!(ProviderEvent::ProviderSelected {
            provider: ProviderId::new("p"),
            reason: "x".into(),
            correlation_id: "c".into(),
        }
        .summary()
        .contains("p"));
        assert!(ProviderEvent::RetryStarted {
            provider: ProviderId::new("p"),
            attempt: 2,
        }
        .summary()
        .contains("2"));
    }

    #[test]
    fn test_summary_counts() {
        let d = ProviderDiagnostics::new();
        d.record_selected(&ProviderId::new("p"), "r", "c");
        d.record_failover("c", &ProviderId::new("a"), &ProviderId::new("b"), "r");
        let s = d.summary();
        assert!(s.providers_tracked >= 2);
        assert_eq!(s.failovers, 1);
    }

    #[test]
    fn test_multiple_selections_across_providers() {
        let d = ProviderDiagnostics::new();
        d.record_selected(&ProviderId::new("a"), "r", "c");
        d.record_selected(&ProviderId::new("b"), "r", "c");
        d.record_selected(&ProviderId::new("a"), "r", "c");
        assert_eq!(d.statistics(&ProviderId::new("a")).selections, 2);
        assert_eq!(d.statistics(&ProviderId::new("b")).selections, 1);
    }

    #[test]
    fn test_diagnostics_are_observational() {
        // Recording diagnostics must not change routing behaviour.
        let d = ProviderDiagnostics::new();
        let before = d.summary().events;
        d.record_unavailable(&ProviderId::new("p"), HealthState::Cooldown);
        d.record_cost(&ProviderId::new("p"), 0.5);
        assert!(d.summary().events > before);
    }
}

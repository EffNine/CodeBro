#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Circuit Breaker Metrics for the Provider Runtime (P17.0).
//!
//! Bridges the per-breaker metrics into the existing observability
//! infrastructure (`EventBus`, `MetricRecorder`, `Logger`).

use crate::observability::{
    types::{CorrelationId, Dimension, EventType, MetricName, Severity},
    EventBus, MetricRecorder,
};
use std::sync::{Arc, Mutex};

use super::circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerMetrics, CircuitBreakerState,
};
use super::circuit_breaker_registry::CircuitBreakerRegistry;
use super::diagnostics::ProviderEvent;
use super::types::ProviderId;

/// Aggregated circuit breaker metrics across all providers.
#[derive(Debug, Clone, Default)]
pub struct CircuitBreakerMetricsView {
    /// Per-provider breaker metrics.
    pub by_provider: Vec<(ProviderId, CircuitBreakerMetrics)>,
    /// Totals across all providers.
    pub total_requests: u64,
    pub total_successful: u64,
    pub total_failed: u64,
    pub total_rejected: u64,
    pub total_open_count: u64,
    pub total_half_open_transitions: u64,
}

impl CircuitBreakerMetricsView {
    pub fn overall_success_rate(&self) -> f64 {
        let completed = self.total_successful + self.total_failed;
        if completed == 0 {
            1.0
        } else {
            self.total_successful as f64 / completed as f64
        }
    }

    pub fn overall_rejection_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.total_rejected as f64 / self.total_requests as f64
        }
    }
}

/// Collects per-breaker metrics and publishes them to the observability
/// layer. Thread-safe.
#[derive(Clone)]
pub struct CircuitBreakerMetricsCollector {
    registry: CircuitBreakerRegistry,
    event_bus: Option<Arc<Mutex<EventBus>>>,
    metric_recorder: Option<Arc<MetricRecorder>>,
}

impl CircuitBreakerMetricsCollector {
    /// Creates a collector without observability integration.
    pub fn new(registry: CircuitBreakerRegistry) -> Self {
        CircuitBreakerMetricsCollector {
            registry,
            event_bus: None,
            metric_recorder: None,
        }
    }

    /// Creates a collector wired to the observability layer.
    pub fn with_observability(
        registry: CircuitBreakerRegistry,
        event_bus: Arc<Mutex<EventBus>>,
        metric_recorder: Arc<MetricRecorder>,
    ) -> Self {
        CircuitBreakerMetricsCollector {
            registry,
            event_bus: Some(event_bus),
            metric_recorder: Some(metric_recorder),
        }
    }

    /// Returns the registry.
    pub fn registry(&self) -> &CircuitBreakerRegistry {
        &self.registry
    }

    /// Records a successful invocation for a provider.
    pub fn record_success(&self, provider: &ProviderId, correlation_id: &str) {
        if let Some(cb) = self.registry.get(provider) {
            cb.record_success();
        }
        self.emit_event(provider, correlation_id, "success");
        self.emit_metric(provider, "success");
    }

    /// Records a failed invocation for a provider.
    pub fn record_failure(&self, provider: &ProviderId, correlation_id: &str) {
        if let Some(cb) = self.registry.get(provider) {
            cb.record_failure();
        }
        self.emit_event(provider, correlation_id, "failure");
        self.emit_metric(provider, "failure");
    }

    /// Records a rejected request (breaker was open).
    pub fn record_rejected(&self, provider: &ProviderId, correlation_id: &str) {
        self.emit_event(provider, correlation_id, "rejected");
        self.emit_metric(provider, "rejected");
    }

    /// Returns aggregated metrics across all providers.
    pub fn snapshot(&self) -> CircuitBreakerMetricsView {
        let mut view = CircuitBreakerMetricsView::default();
        for id in self.registry.providers() {
            if let Some(cb) = self.registry.get(&id) {
                let m = cb.metrics();
                view.by_provider.push((id.clone(), m.clone()));
                view.total_requests += m.total_requests;
                view.total_successful += m.successful_requests;
                view.total_failed += m.failed_requests;
                view.total_rejected += m.rejected_requests;
                view.total_open_count += m.open_count;
                view.total_half_open_transitions += m.half_open_transitions;
            }
        }
        view
    }

    /// Emits a circuit breaker state transition event to the diagnostic
    /// event stream.
    pub fn emit_state_event(
        &self,
        provider: &ProviderId,
        from: CircuitBreakerState,
        to: CircuitBreakerState,
        correlation_id: &str,
    ) {
        match (from, to) {
            (CircuitBreakerState::Closed, CircuitBreakerState::Open) => {
                if let Some(ref eb) = self.event_bus {
                    let bus = eb.lock().unwrap();
                    bus.emit(&crate::observability::event::error_event(
                        CorrelationId::new(),
                        "CircuitBreakerOpened",
                        &format!("Circuit breaker opened for {provider}"),
                        true,
                    ));
                }
            }
            (CircuitBreakerState::Open, CircuitBreakerState::HalfOpen) => {
                if let Some(ref eb) = self.event_bus {
                    let bus = eb.lock().unwrap();
                    bus.emit(&crate::observability::event::pipeline_completed(
                        CorrelationId::new(),
                        0,
                        "breaker_half_open",
                        1,
                    ));
                }
            }
            (CircuitBreakerState::HalfOpen, CircuitBreakerState::Closed) => {
                if let Some(ref eb) = self.event_bus {
                    let bus = eb.lock().unwrap();
                    bus.emit(&crate::observability::event::pipeline_completed(
                        CorrelationId::new(),
                        0,
                        "breaker_closed",
                        1,
                    ));
                }
            }
            _ => {}
        }
    }

    fn emit_event(&self, provider: &ProviderId, correlation_id: &str, outcome: &str) {
        if let Some(ref eb) = self.event_bus {
            let bus = eb.lock().unwrap();
            let severity = match outcome {
                "failure" => Severity::Error,
                "rejected" => Severity::Warn,
                _ => Severity::Info,
            };
            let event = crate::observability::event::error_event(
                CorrelationId::new(),
                "CircuitBreaker",
                &format!(
                    "provider={} outcome={} corr={}",
                    provider, outcome, correlation_id
                ),
                outcome == "failure",
            )
            .with_severity(severity)
            .with_attribute("provider", provider.as_str())
            .with_attribute("outcome", outcome)
            .with_attribute("correlation_id", correlation_id);
            bus.emit(&event);
        }
    }

    fn emit_metric(&self, provider: &ProviderId, outcome: &str) {
        if let Some(ref mr) = self.metric_recorder {
            let metric = match outcome {
                "success" => MetricName::Custom(format!("cb.{}.success", provider)),
                "failure" => MetricName::Custom(format!("cb.{}.failure", provider)),
                "rejected" => MetricName::Custom(format!("cb.{}.rejected", provider)),
                _ => return,
            };
            mr.increment(metric, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::{EventBus, MetricRecorder};
    use std::sync::Arc;
    use std::time::Duration;

    fn make_registry_with_breakers() -> (
        CircuitBreakerRegistry,
        Arc<Mutex<EventBus>>,
        Arc<MetricRecorder>,
    ) {
        let reg = CircuitBreakerRegistry::new();
        let eb = Arc::new(Mutex::new(EventBus::new()));
        let mr = Arc::new(MetricRecorder::new());
        let collector =
            CircuitBreakerMetricsCollector::with_observability(reg, eb.clone(), mr.clone());
        (collector.registry().clone(), eb, mr)
    }

    #[test]
    fn test_record_success_and_failure() {
        let reg = CircuitBreakerRegistry::new();
        let id = ProviderId::new("p");
        reg.register(
            &id,
            CircuitBreakerConfig {
                failure_threshold: 2,
                success_threshold: 1,
                cooldown_duration: Duration::from_secs(1),
                ..Default::default()
            },
        );

        let eb = Arc::new(Mutex::new(EventBus::new()));
        let mr = Arc::new(MetricRecorder::new());
        let collector = CircuitBreakerMetricsCollector::with_observability(reg.clone(), eb, mr);

        collector.record_success(&id, "c1");
        collector.record_failure(&id, "c2");
        collector.record_failure(&id, "c3");

        let snapshot = collector.snapshot();
        assert_eq!(snapshot.total_successful, 1);
        assert_eq!(snapshot.total_failed, 2);
    }

    #[test]
    fn test_record_rejected() {
        let reg = CircuitBreakerRegistry::new();
        let id = ProviderId::new("p");
        reg.register(
            &id,
            CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_duration: Duration::from_secs(1),
                ..Default::default()
            },
        );

        let eb = Arc::new(Mutex::new(EventBus::new()));
        let mr = Arc::new(MetricRecorder::new());
        let collector = CircuitBreakerMetricsCollector::with_observability(reg.clone(), eb, mr);

        collector.record_failure(&id, "c1");
        // The breaker is now open; a subsequent can_execute() would be
        // a rejection, but record_rejected is a separate diagnostic call.
        collector.record_rejected(&id, "c2");

        // The breaker itself tracked the failure; rejected is tracked
        // by the collector's diagnostic path (not the breaker's internal
        // metrics). Verify the breaker state is open.
        let cb = reg.get(&id).unwrap();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
        assert_eq!(cb.metrics().failed_requests, 1);
    }

    #[test]
    fn test_snapshot_aggregation() {
        let reg = CircuitBreakerRegistry::new();
        let a = ProviderId::new("a");
        let b = ProviderId::new("b");
        reg.register(&a, CircuitBreakerConfig::default());
        reg.register(&b, CircuitBreakerConfig::default());

        let eb = Arc::new(Mutex::new(EventBus::new()));
        let mr = Arc::new(MetricRecorder::new());
        let collector = CircuitBreakerMetricsCollector::with_observability(reg.clone(), eb, mr);

        collector.record_success(&a, "c1");
        collector.record_success(&a, "c2");
        collector.record_failure(&b, "c3");

        let snap = collector.snapshot();
        assert_eq!(snap.by_provider.len(), 2);
        assert_eq!(snap.total_successful, 2);
        assert_eq!(snap.total_failed, 1);
    }

    #[test]
    fn test_state_transition_events() {
        let reg = CircuitBreakerRegistry::new();
        let id = ProviderId::new("p");
        reg.register(
            &id,
            CircuitBreakerConfig {
                failure_threshold: 1,
                success_threshold: 1,
                cooldown_duration: Duration::from_millis(10),
                ..Default::default()
            },
        );

        let eb = Arc::new(Mutex::new(EventBus::new()));
        let mr = Arc::new(MetricRecorder::new());
        let collector = CircuitBreakerMetricsCollector::with_observability(reg, eb.clone(), mr);

        collector.record_failure(&id, "c1");
        // Emit state transition Closed -> Open
        collector.emit_state_event(
            &id,
            CircuitBreakerState::Closed,
            CircuitBreakerState::Open,
            "c1",
        );

        let bus = eb.lock().unwrap();
        let events = bus.buffer();
        assert!(!events.is_empty());
    }
}

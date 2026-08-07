#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Diagnostics — debug snapshots and aggregate observability health.

use std::sync::{Arc, Mutex};

use super::event_bus::EventBus;
use super::logger::{LogEntry, Logger, MemoryLogSink};
use super::metrics::MetricRecorder;
use super::tracing::TraceContext;
use super::types::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DebugSnapshot {
    pub snapshot_id: String,
    pub timestamp: String,
    pub correlation_id: CorrelationId,
    pub trace_id: Option<TraceId>,
    pub event_count: usize,
    pub metric_summary: String,
    pub trace_summary: String,
    pub recent_logs: Vec<LogEntry>,
    pub active_spans: usize,
    pub error_count: u64,
    pub pipeline_latencies: Vec<f64>,
}

impl DebugSnapshot {
    pub fn new(
        correlation_id: CorrelationId,
        trace_context: Option<&TraceContext>,
        event_bus: &EventBus,
        metrics: &MetricRecorder,
        log_sink: &MemoryLogSink,
    ) -> Self {
        let trace_id = trace_context.map(|t| t.trace_id());
        let trace_summary = trace_context.map(|t| t.summary()).unwrap_or_default();
        let metric_summary = metrics.summary();
        let pipeline_latencies = metrics
            .histogram(&MetricName::PipelineLatency)
            .iter()
            .cloned()
            .collect();

        DebugSnapshot {
            snapshot_id: uuid::Uuid::new_v4().to_string(),
            timestamp: chrono::Local::now().to_rfc3339(),
            correlation_id,
            trace_id,
            event_count: event_bus.buffer_len(),
            metric_summary,
            trace_summary,
            recent_logs: log_sink.entries(),
            active_spans: trace_context
                .map(|t| {
                    t.spans()
                        .iter()
                        .filter(|s| s.phase == TracePhase::Start)
                        .count()
                })
                .unwrap_or(0),
            error_count: metrics.counter(&MetricName::ErrorCount),
            pipeline_latencies,
        }
    }
}

struct DiagnosticsInner {
    event_bus: EventBus,
    metrics: MetricRecorder,
    log_sink: MemoryLogSink,
}

#[derive(Clone)]
pub struct Diagnostics {
    inner: Arc<Mutex<DiagnosticsInner>>,
}

impl Diagnostics {
    pub fn new() -> Self {
        Diagnostics {
            inner: Arc::new(Mutex::new(DiagnosticsInner {
                event_bus: EventBus::new(),
                metrics: MetricRecorder::new(),
                log_sink: MemoryLogSink::new(500),
            })),
        }
    }

    pub fn with_log_capacity(capacity: usize) -> Self {
        Diagnostics {
            inner: Arc::new(Mutex::new(DiagnosticsInner {
                event_bus: EventBus::new(),
                metrics: MetricRecorder::new(),
                log_sink: MemoryLogSink::new(capacity),
            })),
        }
    }

    pub fn event_bus(&self) -> EventBus {
        let inner = self.inner.lock().unwrap();
        inner.event_bus.clone()
    }

    pub fn metrics(&self) -> MetricRecorder {
        let inner = self.inner.lock().unwrap();
        inner.metrics.clone()
    }

    pub fn log_sink(&self) -> MemoryLogSink {
        let inner = self.inner.lock().unwrap();
        inner.log_sink.clone()
    }

    pub fn logger(&self, correlation_id: CorrelationId, target: &str) -> Logger {
        let log_sink = self.log_sink();
        let mut logger = Logger::new(correlation_id, target);
        logger.add_sink(Box::new(log_sink));
        logger
    }

    pub fn snapshot(&self, correlation_id: CorrelationId) -> DebugSnapshot {
        let inner = self.inner.lock().unwrap();
        DebugSnapshot::new(
            correlation_id,
            None,
            &inner.event_bus,
            &inner.metrics,
            &inner.log_sink,
        )
    }

    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        lines.push("=== Observability Diagnostics ===".to_string());
        lines.push(format!("Events buffered: {}", inner.event_bus.buffer_len()));
        lines.push(format!(
            "Event observers: {}",
            inner.event_bus.observer_count()
        ));
        lines.push(inner.metrics.summary());
        lines.push(format!("Log entries: {}", inner.log_sink.count()));
        lines.join("\n")
    }

    pub fn clear(&self) {
        let inner = self.inner.lock().unwrap();
        inner.event_bus.clear_buffer();
        inner.metrics.clear();
    }
}

impl Default for Diagnostics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observability::event;

    #[test]
    fn test_diagnostics_creation() {
        let diag = Diagnostics::new();
        let summary = diag.summary();
        assert!(summary.contains("Observability Diagnostics"));
    }

    #[test]
    fn test_emit_and_observe() {
        let diag = Diagnostics::new();
        let corr = CorrelationId::new();
        let ev = event::intent_resolved(corr.clone(), "preference", "change model", 0.9);
        diag.event_bus().emit(&ev);
        assert_eq!(diag.event_bus().buffer_len(), 1);
    }

    #[test]
    fn test_metrics_recording() {
        let diag = Diagnostics::new();
        diag.metrics().increment(MetricName::ErrorCount, 1);
        diag.metrics()
            .record_histogram(MetricName::PipelineLatency, 150.0);
        assert_eq!(diag.metrics().counter(&MetricName::ErrorCount), 1);
        assert_eq!(
            diag.metrics().histogram(&MetricName::PipelineLatency),
            vec![150.0]
        );
    }

    #[test]
    fn test_logger() {
        let diag = Diagnostics::new();
        let corr = CorrelationId::new();
        let logger = diag.logger(corr.clone(), "test-target");
        logger.info("hello world");
        assert_eq!(diag.log_sink().count(), 1);
    }

    #[test]
    fn test_snapshot() {
        let diag = Diagnostics::new();
        diag.metrics().increment(MetricName::ErrorCount, 3);
        let snap = diag.snapshot(CorrelationId::new());
        assert_eq!(snap.error_count, 3);
        assert!(snap.metric_summary.contains("error.count"));
    }

    #[test]
    fn test_clear() {
        let diag = Diagnostics::new();
        diag.event_bus().emit(&Event::new(
            EventType::Error,
            CorrelationId::new(),
            "test",
            "x",
        ));
        diag.metrics().increment(MetricName::ErrorCount, 1);
        diag.clear();
        assert_eq!(diag.event_bus().buffer_len(), 0);
        assert_eq!(diag.metrics().counter(&MetricName::ErrorCount), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let diag = Diagnostics::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let d = diag.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        d.metrics().increment(MetricName::ErrorCount, 1);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(diag.metrics().counter(&MetricName::ErrorCount), 1000);
    }

    #[test]
    fn test_with_log_capacity() {
        let diag = Diagnostics::with_log_capacity(50);
        let logger = diag.logger(CorrelationId::new(), "test");
        for i in 0..100 {
            logger.info(&format!("msg {i}"));
        }
        assert_eq!(diag.log_sink().count(), 50);
    }
}

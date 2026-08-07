#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Tracing — span-based request lifecycle observability.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::observability::types::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    pub span_id: SpanId,
    pub parent_span_id: Option<SpanId>,
    pub trace_id: TraceId,
    pub name: String,
    pub phase: TracePhase,
    pub start_duration: Duration,
    pub end: Option<Duration>,
    pub attributes: Vec<Dimension>,
    pub events: Vec<TraceEvent>,
}

impl Span {
    pub fn start(
        name: &str,
        trace_id: TraceId,
        parent_span_id: Option<SpanId>,
    ) -> (Self, std::time::Instant) {
        let span = Span {
            span_id: SpanId::new(),
            parent_span_id,
            trace_id: trace_id.clone(),
            name: name.to_string(),
            phase: TracePhase::Start,
            start_duration: Duration::ZERO,
            end: None,
            attributes: Vec::new(),
            events: Vec::new(),
        };
        (span, std::time::Instant::now())
    }

    pub fn with_attribute(mut self, key: &str, value: &str) -> Self {
        self.attributes.push(Dimension::new(key, value));
        self
    }

    pub fn with_attributes(mut self, attrs: Vec<(&str, &str)>) -> Self {
        for (k, v) in attrs {
            self.attributes.push(Dimension::new(k, v));
        }
        self
    }

    pub fn add_event(mut self, event: TraceEvent) -> Self {
        self.events.push(event);
        self
    }

    pub fn end(self, end_instant: std::time::Instant) -> Self {
        Span {
            phase: TracePhase::End,
            end: Some(end_instant.elapsed()),
            ..self
        }
    }

    pub fn duration(&self) -> Option<Duration> {
        self.end
    }
}

#[derive(Debug)]
struct TraceContextInner {
    trace_id: TraceId,
    correlation_id: CorrelationId,
    spans: Vec<Span>,
    active_span_ids: Vec<SpanId>,
}

#[derive(Clone)]
pub struct TraceContext {
    inner: Arc<Mutex<TraceContextInner>>,
}

impl TraceContext {
    pub fn new(correlation_id: CorrelationId) -> Self {
        TraceContext {
            inner: Arc::new(Mutex::new(TraceContextInner {
                trace_id: TraceId::new(),
                correlation_id,
                spans: Vec::new(),
                active_span_ids: Vec::new(),
            })),
        }
    }

    pub fn trace_id(&self) -> TraceId {
        let inner = self.inner.lock().unwrap();
        inner.trace_id.clone()
    }

    pub fn correlation_id(&self) -> CorrelationId {
        let inner = self.inner.lock().unwrap();
        inner.correlation_id.clone()
    }

    pub fn begin_span(&self, name: &str) -> (Span, std::time::Instant) {
        let mut inner = self.inner.lock().unwrap();
        let parent_id = inner.active_span_ids.last().cloned();
        let (span, instant) = Span::start(name, inner.trace_id.clone(), parent_id);
        inner.spans.push(span.clone());
        inner.active_span_ids.push(span.span_id.clone());
        drop(inner);
        (span, instant)
    }

    pub fn end_span(&self, end_instant: std::time::Instant) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(span_id) = inner.active_span_ids.pop() {
            if let Some(span) = inner
                .spans
                .iter_mut()
                .rev()
                .find(|s| s.span_id == span_id && s.phase == TracePhase::Start)
            {
                span.phase = TracePhase::End;
                span.end = Some(end_instant.elapsed());
            }
        }
    }

    pub fn spans(&self) -> Vec<Span> {
        let inner = self.inner.lock().unwrap();
        inner.spans.clone()
    }

    pub fn span_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.spans.len()
    }

    pub fn total_duration(&self) -> Option<Duration> {
        let inner = self.inner.lock().unwrap();
        let started = inner.spans.iter().map(|s| s.start_duration).min()?;
        let ended = inner.spans.iter().filter_map(|s| s.end).max()?;
        Some(ended - started)
    }

    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        lines.push(format!("Trace: {}", inner.trace_id));
        lines.push(format!("Correlation: {}", inner.correlation_id));
        lines.push(format!("Spans: {}", inner.spans.len()));
        for span in &inner.spans {
            let dur = span
                .end
                .map(|d| format!("{}ms", d.as_millis()))
                .unwrap_or_else(|| "active".to_string());
            lines.push(format!("  [{}] {} — {}", span.span_id, span.name, dur));
            for attr in &span.attributes {
                lines.push(format!("    {} = {}", attr.key, attr.value));
            }
        }
        lines.join("\n")
    }

    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.spans.clear();
        inner.active_span_ids.clear();
    }
}

pub fn trace_span<F, R>(ctx: &TraceContext, name: &str, f: F) -> R
where
    F: FnOnce() -> R,
{
    let (span, instant) = ctx.begin_span(name);
    let result = f();
    ctx.end_span(instant);
    drop(span);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_begin_end_span() {
        let ctx = TraceContext::new(CorrelationId::new());
        let (span, instant) = ctx.begin_span("classification");
        assert_eq!(span.name, "classification");
        assert_eq!(span.phase, TracePhase::Start);
        ctx.end_span(instant);
        let spans = ctx.spans();
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].phase, TracePhase::End);
        assert!(spans[0].end.is_some());
    }

    #[test]
    fn test_nested_spans() {
        let ctx = TraceContext::new(CorrelationId::new());
        let (parent, p_start) = ctx.begin_span("pipeline");
        let (child, c_start) = ctx.begin_span("classification");
        ctx.end_span(c_start);
        ctx.end_span(p_start);
        drop(parent);
        drop(child);
        let spans = ctx.spans();
        assert_eq!(spans.len(), 2);
        assert_eq!(spans[0].parent_span_id, None);
        assert_eq!(spans[1].parent_span_id, Some(spans[0].span_id.clone()));
    }

    #[test]
    fn test_trace_id_persistence() {
        let ctx = TraceContext::new(CorrelationId::new());
        let tid1 = ctx.trace_id();
        let _span = ctx.begin_span("s");
        let tid2 = ctx.trace_id();
        assert_eq!(tid1, tid2);
    }

    #[test]
    fn test_summary() {
        let ctx = TraceContext::new(CorrelationId::new());
        let (_span, instant) = ctx.begin_span("test");
        ctx.end_span(instant);
        drop(_span);
        let summary = ctx.summary();
        assert!(summary.contains("Trace:"));
        assert!(summary.contains("Spans: 1"));
    }

    #[test]
    fn test_clear() {
        let ctx = TraceContext::new(CorrelationId::new());
        let (_span, instant) = ctx.begin_span("test");
        ctx.end_span(instant);
        drop(_span);
        ctx.clear();
        assert_eq!(ctx.span_count(), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let ctx = TraceContext::new(CorrelationId::new());
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let c = ctx.clone();
                thread::spawn(move || {
                    for _ in 0..50 {
                        let (s, inst) = c.begin_span(&format!("span-{i}"));
                        c.end_span(inst);
                        drop(s);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(ctx.span_count(), 500);
    }

    #[test]
    fn test_trace_span_helper() {
        let ctx = TraceContext::new(CorrelationId::new());
        let result = trace_span(&ctx, "computed", || 42);
        assert_eq!(result, 42);
        assert_eq!(ctx.span_count(), 1);
    }
}

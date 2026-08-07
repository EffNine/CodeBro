#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Observability Platform for CodeBro.
//!
//! First-class observability layer — structured events, metrics, tracing,
//! correlation IDs, and diagnostics. Observational only; never influences
//! deterministic business logic.
//!
//! # Architecture
//!
//! ```text
//! IntegrationPipeline
//!   ├─ EventBus     — pub/sub for structured domain events
//!   ├─ Metrics      — counters, histograms, timing
//!   ├─ Tracing      — span-based request lifecycle
//!   ├─ Logger       — structured log sink with correlation IDs
//!   └─ Diagnostics  — debug snapshots and aggregate health
//! ```
//!
//! # Design Rules
//!
//! - **Stateless**: No external telemetry services; all data stays in-process.
//! - **Thread-safe**: `Arc<Mutex<>>` for all shared state; `Clone` via `Arc`.
//! - **Deterministic**: Observability never mutates pipeline state or outputs.
//! - **Optional**: No observability code runs unless explicitly enabled.
//! - **Non-invasive**: Zero changes to existing engine traits or types.
//!
//! # Thread Safety
//!
//! Every public type implements `Send + Sync + Clone`.

pub mod diagnostics;
pub mod event;
pub mod event_bus;
pub mod logger;
pub mod metrics;
pub mod tracing;
pub mod types;

pub use diagnostics::{DebugSnapshot, Diagnostics};
pub use event_bus::EventBus;
pub use logger::{LogEntry, LogLevel, Logger};
pub use metrics::MetricRecorder;
pub use tracing::{Span, TraceContext};
pub use types::{
    CorrelationId, Dimension, Event, EventPayload, EventType, MetricKind, MetricName, MetricUnit,
    MetricValue, PipelineStage, Severity, SpanId, TraceEvent, TraceId, TracePhase,
};

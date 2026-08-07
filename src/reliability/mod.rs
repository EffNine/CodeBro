#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Reliability Layer for CodeBro.
//!
//! Provides structured error classification, timeout management, health
//! monitoring, circuit breaking, resource guarding, diagnostics, and
//! structured logging — all without adding new dependencies.
//!
//! # Architecture
//!
//! The reliability layer sits beneath the runtime pipeline and provides:
//!
//! - **Error Classification** (`error`): Structured error categories with
//!   retryability and escalation metadata.
//! - **Timeout Management** (`timeout`): Centralized timeout handling with
//!   per-provider and per-tool configuration.
//! - **Health Monitoring** (`health`): Tracks health of providers, tools,
//!   runtime, and resources with degradation thresholds.
//! - **Circuit Breaker** (`circuit_breaker`): Prevents cascading failures
//!   with closed → open → half-open state transitions.
//! - **Resource Guard** (`resource_guard`): Enforces memory and operation
//!   limits with graceful shutdown support.
//! - **Diagnostics** (`diagnostics`): Structured failure and recovery traces
//!   with correlation IDs for post-mortem analysis.
//! - **Structured Logging** (`logging`): Consistent logging with correlation
//!   IDs and pluggable log sinks.
//!
//! # Thread Safety
//!
//! All components are `Clone` (via `Arc<Mutex<>>`) and safe to share across
//! tasks. No component requires `UnsafeCell` or raw pointers.
//!
//! # Integration
//!
//! The reliability layer is intentionally additive. It does not modify
//! existing traits (`Provider`, `Tool`, `AgentEvent`) or the state machine
//! (`RuntimeState`). Instead, it wraps and observes existing operations.

pub mod circuit_breaker;
pub mod diagnostics;
pub mod error;
pub mod health;
pub mod logging;
pub mod resource_guard;
pub mod timeout;

pub use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitState};
pub use diagnostics::{Diagnostics, FailureTrace, RecoveryTrace};
pub use error::{classify_error, from_message, RuntimeError, RuntimeErrorCategory};
pub use health::{HealthMonitor, HealthStatus, HealthTarget};
pub use logging::{ConsoleLogSink, LogEntry, LogLevel, LogSink, MemoryLogSink, StructuredLogger};
pub use resource_guard::{ResourceGuard, ResourceGuardConfig, ResourceStatus};
pub use timeout::{TimeoutConfig, TimeoutKind, TimeoutManager};

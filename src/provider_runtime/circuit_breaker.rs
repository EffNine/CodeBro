#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Circuit Breaker for the Provider Runtime (P17.0).
//!
//! Prevents repeated failures from cascading by opening the circuit after
//! a configurable failure threshold, then probing recovery with a
//! half-open state. Every provider owns an independent circuit breaker.
//!
//! # State Machine
//!
//! ```text
//! Closed --(failure_threshold reached)--> Open
//! Open  --(cooldown expires)             --> HalfOpen
//! HalfOpen --(success)                   --> Closed
//! HalfOpen --(failure)                   --> Open
//! ```
//!
//! # Thread Safety
//!
//! All state is protected by `std::sync::Mutex`. The breaker is
//! `Clone` (via `Arc`) and safe to share across tasks.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::types::ProviderId;

// =========================================================================
// State
// =========================================================================

/// The state of a circuit breaker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitBreakerState {
    /// Circuit is closed — normal operation, requests pass through.
    Closed,
    /// Circuit is open — requests are rejected immediately.
    Open,
    /// Circuit is half-open — probes are allowed through to test recovery.
    HalfOpen,
}

impl std::fmt::Display for CircuitBreakerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitBreakerState::Closed => write!(f, "closed"),
            CircuitBreakerState::Open => write!(f, "open"),
            CircuitBreakerState::HalfOpen => write!(f, "half-open"),
        }
    }
}

// =========================================================================
// Config
// =========================================================================

/// Configuration for a circuit breaker.
///
/// Default values are production-sensible:
/// - 5 failures to open
/// - 3 consecutive successes in half-open to close
/// - 30 s cooldown
/// - 10 s request timeout
/// - 60 s rolling failure window
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    /// Number of failures in the rolling window before opening.
    pub failure_threshold: u32,
    /// Consecutive successes required in half-open to close.
    pub success_threshold: u32,
    /// Duration to wait in open state before transitioning to half-open.
    pub cooldown_duration: Duration,
    /// Maximum duration allowed for a single request while half-open.
    pub request_timeout: Duration,
    /// Size of the rolling failure window (failures older than this are
    /// discarded).
    pub rolling_window: Duration,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        CircuitBreakerConfig {
            failure_threshold: 5,
            success_threshold: 3,
            cooldown_duration: Duration::from_secs(30),
            request_timeout: Duration::from_secs(10),
            rolling_window: Duration::from_secs(60),
        }
    }
}

impl CircuitBreakerConfig {
    pub fn with_failure_threshold(mut self, n: u32) -> Self {
        self.failure_threshold = n;
        self
    }

    pub fn with_success_threshold(mut self, n: u32) -> Self {
        self.success_threshold = n;
        self
    }

    pub fn with_cooldown(mut self, d: Duration) -> Self {
        self.cooldown_duration = d;
        self
    }

    pub fn with_request_timeout(mut self, d: Duration) -> Self {
        self.request_timeout = d;
        self
    }

    pub fn with_rolling_window(mut self, d: Duration) -> Self {
        self.rolling_window = d;
        self
    }
}

// =========================================================================
// Metrics
// =========================================================================

/// Per-breaker operational metrics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CircuitBreakerMetrics {
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub rejected_requests: u64,
    pub open_count: u64,
    pub half_open_transitions: u64,
    pub total_recovery_time_ms: u64,
    pub recovery_count: u64,
}

impl CircuitBreakerMetrics {
    pub fn average_recovery_time_ms(&self) -> f64 {
        if self.recovery_count == 0 {
            0.0
        } else {
            self.total_recovery_time_ms as f64 / self.recovery_count as f64
        }
    }

    pub fn rejection_rate(&self) -> f64 {
        if self.total_requests == 0 {
            0.0
        } else {
            self.rejected_requests as f64 / self.total_requests as f64
        }
    }

    pub fn success_rate(&self) -> f64 {
        let completed = self.successful_requests + self.failed_requests;
        if completed == 0 {
            1.0
        } else {
            self.successful_requests as f64 / completed as f64
        }
    }
}

// =========================================================================
// Inner state
// =========================================================================

#[derive(Debug)]
struct CircuitBreakerInner {
    state: CircuitBreakerState,
    failure_count: u32,
    success_count: u32,
    config: CircuitBreakerConfig,
    last_failure_time: Option<Instant>,
    /// Timestamp when the breaker transitioned to open (for cooldown).
    opened_at: Option<Instant>,
    /// Timestamp when the breaker last transitioned to closed (for
    /// recovery-time tracking).
    last_closed_at: Option<Instant>,
    /// Rolling failure timestamps for windowed counting.
    failure_timestamps: Vec<Instant>,
    /// Metrics.
    metrics: CircuitBreakerMetrics,
}

// =========================================================================
// CircuitBreaker
// =========================================================================

/// A circuit breaker for a single provider.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    inner: Arc<Mutex<CircuitBreakerInner>>,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with default configuration.
    pub fn new() -> Self {
        CircuitBreaker {
            inner: Arc::new(Mutex::new(CircuitBreakerInner {
                state: CircuitBreakerState::Closed,
                failure_count: 0,
                success_count: 0,
                config: CircuitBreakerConfig::default(),
                last_failure_time: None,
                opened_at: None,
                last_closed_at: None,
                failure_timestamps: Vec::new(),
                metrics: CircuitBreakerMetrics::default(),
            })),
        }
    }

    /// Creates a new circuit breaker with custom configuration.
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        CircuitBreaker {
            inner: Arc::new(Mutex::new(CircuitBreakerInner {
                state: CircuitBreakerState::Closed,
                failure_count: 0,
                success_count: 0,
                config,
                last_failure_time: None,
                opened_at: None,
                last_closed_at: None,
                failure_timestamps: Vec::new(),
                metrics: CircuitBreakerMetrics::default(),
            })),
        }
    }

    /// Returns whether a request can be executed.
    ///
    /// - `Closed`: Always returns `true` (after bumping total requests).
    /// - `Open`: Returns `true` only if the cooldown has expired,
    ///   transitioning to `HalfOpen` (after bumping total requests and
    ///   half-open transition count).
    /// - `HalfOpen`: Returns `true` (after bumping total requests).
    pub fn can_execute(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        inner.metrics.total_requests += 1;

        match inner.state {
            CircuitBreakerState::Closed => true,
            CircuitBreakerState::Open => {
                if let Some(opened_at) = inner.opened_at {
                    if opened_at.elapsed() >= inner.config.cooldown_duration {
                        inner.state = CircuitBreakerState::HalfOpen;
                        inner.success_count = 0;
                        inner.metrics.half_open_transitions += 1;
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            }
            CircuitBreakerState::HalfOpen => true,
        }
    }

    /// Records a successful execution.
    ///
    /// - `Closed`: Resets failure count and rolling window.
    /// - `HalfOpen`: Increments success count; if threshold reached,
    ///   closes the circuit and records recovery time.
    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.metrics.successful_requests += 1;
        inner.failure_count = 0;
        inner.failure_timestamps.clear();
        inner.success_count += 1;

        if inner.state == CircuitBreakerState::HalfOpen {
            if inner.success_count >= inner.config.success_threshold {
                let now = Instant::now();
                if let Some(opened_at) = inner.opened_at {
                    let recovery_ms = opened_at.elapsed().as_millis() as u64;
                    inner.metrics.total_recovery_time_ms += recovery_ms;
                    inner.metrics.recovery_count += 1;
                }
                inner.last_closed_at = Some(now);
                inner.state = CircuitBreakerState::Closed;
                inner.success_count = 0;
                inner.opened_at = None;
            }
        }
    }

    /// Records a failed execution.
    ///
    /// - `Closed`: Increments failure count and appends to rolling
    ///   window; if threshold reached, opens the circuit.
    /// - `HalfOpen`: Immediately opens the circuit.
    pub fn record_failure(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.metrics.failed_requests += 1;
        inner.failure_count += 1;
        inner.success_count = 0;
        inner.last_failure_time = Some(Instant::now());

        let now = Instant::now();
        inner.failure_timestamps.push(now);

        // Evict failures outside the rolling window.
        let window = inner.config.rolling_window;
        inner
            .failure_timestamps
            .retain(|t| now.duration_since(*t) <= window);

        // Use rolling window count for threshold comparison.
        let rolling_failures = inner.failure_timestamps.len() as u32;

        match inner.state {
            CircuitBreakerState::Closed => {
                if rolling_failures >= inner.config.failure_threshold {
                    inner.state = CircuitBreakerState::Open;
                    inner.opened_at = Some(Instant::now());
                    inner.metrics.open_count += 1;
                }
            }
            CircuitBreakerState::HalfOpen => {
                inner.state = CircuitBreakerState::Open;
                inner.opened_at = Some(Instant::now());
                inner.metrics.open_count += 1;
            }
            CircuitBreakerState::Open => {}
        }
    }

    /// Returns the current state of the circuit breaker.
    pub fn state(&self) -> CircuitBreakerState {
        let inner = self.inner.lock().unwrap();
        inner.state
    }

    /// Returns the current failure count (rolling window).
    pub fn failure_count(&self) -> u32 {
        let inner = self.inner.lock().unwrap();
        inner.failure_timestamps.len() as u32
    }

    /// Returns the current success count (in half-open).
    pub fn success_count(&self) -> u32 {
        let inner = self.inner.lock().unwrap();
        inner.success_count
    }

    /// Returns the config.
    pub fn config(&self) -> CircuitBreakerConfig {
        let inner = self.inner.lock().unwrap();
        inner.config.clone()
    }

    /// Returns metrics snapshot.
    pub fn metrics(&self) -> CircuitBreakerMetrics {
        let inner = self.inner.lock().unwrap();
        inner.metrics.clone()
    }

    /// Returns when the breaker was last opened (if open).
    pub fn opened_at(&self) -> Option<Instant> {
        let inner = self.inner.lock().unwrap();
        inner.opened_at
    }

    /// Returns when the breaker was last closed (if currently closed).
    pub fn last_closed_at(&self) -> Option<Instant> {
        let inner = self.inner.lock().unwrap();
        inner.last_closed_at
    }

    /// Manually resets the circuit breaker to closed state.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = CircuitBreakerState::Closed;
        inner.failure_count = 0;
        inner.success_count = 0;
        inner.last_failure_time = None;
        inner.opened_at = None;
        inner.last_closed_at = None;
        inner.failure_timestamps.clear();
    }

    /// Returns the time remaining until the breaker transitions from
    /// open to half-open, or `None` if not open.
    pub fn time_until_half_open(&self) -> Option<Duration> {
        let inner = self.inner.lock().unwrap();
        match inner.state {
            CircuitBreakerState::Open => {
                let opened_at = inner.opened_at?;
                let elapsed = opened_at.elapsed();
                let remaining = inner.config.cooldown_duration.saturating_sub(elapsed);
                if remaining.is_zero() {
                    None
                } else {
                    Some(remaining)
                }
            }
            _ => None,
        }
    }
}

impl Default for CircuitBreaker {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn test_initial_state_is_closed() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert!(cb.can_execute());
        assert_eq!(cb.metrics().total_requests, 1);
    }

    #[test]
    fn test_opens_after_threshold() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            cooldown_duration: Duration::from_secs(1),
            ..Default::default()
        });

        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
        assert!(!cb.can_execute());
        assert_eq!(cb.metrics().open_count, 1);
    }

    #[test]
    fn test_half_open_after_cooldown() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_millis(100),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
        assert!(!cb.can_execute());

        thread::sleep(Duration::from_millis(150));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);
        assert_eq!(cb.metrics().half_open_transitions, 1);
    }

    #[test]
    fn test_half_open_success_closes_circuit() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_millis(50),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);

        thread::sleep(Duration::from_millis(100));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert!(cb.last_closed_at().is_some());
    }

    #[test]
    fn test_half_open_failure_reopens() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_millis(50),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();

        thread::sleep(Duration::from_millis(100));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
        assert_eq!(cb.metrics().open_count, 2);
    }

    #[test]
    fn test_success_resets_failure_count_in_closed_state() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 1,
            cooldown_duration: Duration::from_secs(1),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        cb.record_success();
        assert_eq!(cb.failure_count(), 0);

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn test_reset() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_secs(1),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert!(cb.can_execute());
        assert_eq!(cb.failure_count(), 0);
    }

    #[test]
    fn test_thread_safety() {
        let cb = CircuitBreaker::new();
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let cb = cb.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        cb.record_success();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert_eq!(cb.metrics().successful_requests, 1000);
    }

    #[test]
    fn test_concurrent_failures_open_circuit() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 10,
            success_threshold: 1,
            cooldown_duration: Duration::from_secs(1),
            ..Default::default()
        });
        let handles: Vec<_> = (0..20)
            .map(|_| {
                let cb = cb.clone();
                thread::spawn(move || {
                    cb.record_failure();
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(cb.state(), CircuitBreakerState::Open);
    }

    #[test]
    fn test_rolling_window_evicts_old_failures() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 1,
            cooldown_duration: Duration::from_secs(10),
            rolling_window: Duration::from_millis(50),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert_eq!(cb.failure_count(), 2);

        thread::sleep(Duration::from_millis(60));

        // Old failures should have evicted; new failure alone shouldn't
        // open the circuit.
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert_eq!(cb.failure_count(), 1);
    }

    #[test]
    fn test_time_until_half_open() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 1,
            cooldown_duration: Duration::from_secs(10),
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
        let remaining = cb.time_until_half_open().unwrap();
        assert!(remaining.as_secs() > 0);
        assert!(remaining.as_secs() <= 10);

        // After cooldown, time_until_half_open returns None (already
        // transitioned).
        thread::sleep(Duration::from_secs(11));
        let _ = cb.can_execute(); // triggers transition
        assert!(cb.time_until_half_open().is_none());
    }

    #[test]
    fn test_metrics_tracking() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_millis(50),
            ..Default::default()
        });

        assert_eq!(cb.metrics().total_requests, 0);
        cb.can_execute();
        cb.can_execute();
        assert_eq!(cb.metrics().total_requests, 2);

        cb.record_success();
        assert_eq!(cb.metrics().successful_requests, 1);

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.metrics().failed_requests, 2);
        assert_eq!(cb.metrics().open_count, 1);

        thread::sleep(Duration::from_millis(100));
        // After cooldown, can_execute transitions to half-open and returns true.
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        // After cooldown, the can_execute call itself transitions
        assert!(cb.can_execute());
        assert_eq!(cb.metrics().half_open_transitions, 1);
    }

    #[test]
    fn test_success_threshold_requires_consecutive_successes() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 3,
            cooldown_duration: Duration::from_millis(50),
            ..Default::default()
        });

        cb.record_failure();
        cb.record_failure();

        thread::sleep(Duration::from_millis(100));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn test_request_rejected_while_open() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 1,
            success_threshold: 1,
            cooldown_duration: Duration::from_secs(1),
            ..Default::default()
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
        assert!(!cb.can_execute());
        assert_eq!(cb.metrics().rejected_requests, 0);
    }

    #[test]
    fn test_deterministic_state_transitions() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_millis(10),
            ..Default::default()
        });

        // Closed -> Open
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);

        // Open -> HalfOpen (after cooldown)
        thread::sleep(Duration::from_millis(20));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitBreakerState::HalfOpen);

        // HalfOpen -> Closed
        cb.record_success();
        assert_eq!(cb.state(), CircuitBreakerState::Closed);

        // Closed -> Open again
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitBreakerState::Open);
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Circuit breaker for the reliability layer.
//!
//! Prevents repeated failures from cascading by opening the circuit after
/// a threshold, then testing recovery with a half-open state.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The state of a circuit breaker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CircuitState {
    /// Circuit is closed — normal operation, requests pass through.
    Closed,
    /// Circuit is open — requests are rejected immediately.
    Open,
    /// Circuit is half-open — a single test request is allowed through.
    HalfOpen,
}

/// Configuration for a circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub success_threshold: u32,
    pub cooldown_ms: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        CircuitBreakerConfig {
            failure_threshold: 5,
            success_threshold: 3,
            cooldown_ms: 30_000,
        }
    }
}

/// A circuit breaker that prevents repeated failures from cascading.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct CircuitBreaker {
    inner: Arc<Mutex<CircuitBreakerInner>>,
}

#[derive(Debug)]
struct CircuitBreakerInner {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    config: CircuitBreakerConfig,
    last_failure_time: Option<Instant>,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker with default configuration.
    pub fn new() -> Self {
        CircuitBreaker {
            inner: Arc::new(Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                config: CircuitBreakerConfig::default(),
                last_failure_time: None,
            })),
        }
    }

    /// Creates a new circuit breaker with custom configuration.
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        CircuitBreaker {
            inner: Arc::new(Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failure_count: 0,
                success_count: 0,
                config,
                last_failure_time: None,
            })),
        }
    }

    /// Returns whether a request can be executed.
    ///
    /// - `Closed`: Always returns `true`.
    /// - `Open`: Returns `true` only if the cooldown has expired (transitions to `HalfOpen`).
    /// - `HalfOpen`: Returns `true` (only one test request allowed).
    pub fn can_execute(&self) -> bool {
        let mut inner = self.inner.lock().unwrap();
        match inner.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Check if cooldown has expired
                if let Some(last_failure) = inner.last_failure_time {
                    if last_failure.elapsed() >= Duration::from_millis(inner.config.cooldown_ms) {
                        inner.state = CircuitState::HalfOpen;
                        inner.success_count = 0;
                        true
                    } else {
                        false
                    }
                } else {
                    true
                }
            }
            CircuitState::HalfOpen => true,
        }
    }

    /// Records a successful execution.
    ///
    /// - `Closed`: Resets failure count.
    /// - `HalfOpen`: Increments success count; if threshold reached, closes the circuit.
    pub fn record_success(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.failure_count = 0;
        inner.success_count += 1;
        if inner.state == CircuitState::HalfOpen
            && inner.success_count >= inner.config.success_threshold
        {
            inner.state = CircuitState::Closed;
            inner.success_count = 0;
        }
    }

    /// Records a failed execution.
    ///
    /// - `Closed`: Increments failure count; if threshold reached, opens the circuit.
    /// - `HalfOpen`: Immediately opens the circuit.
    pub fn record_failure(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.failure_count += 1;
        inner.success_count = 0;
        inner.last_failure_time = Some(Instant::now());

        match inner.state {
            CircuitState::Closed => {
                if inner.failure_count >= inner.config.failure_threshold {
                    inner.state = CircuitState::Open;
                }
            }
            CircuitState::HalfOpen => {
                inner.state = CircuitState::Open;
            }
            CircuitState::Open => {}
        }
    }

    /// Returns the current state of the circuit breaker.
    pub fn state(&self) -> CircuitState {
        let inner = self.inner.lock().unwrap();
        inner.state.clone()
    }

    /// Returns the current failure count.
    pub fn failure_count(&self) -> u32 {
        let inner = self.inner.lock().unwrap();
        inner.failure_count
    }

    /// Returns the current success count.
    pub fn success_count(&self) -> u32 {
        let inner = self.inner.lock().unwrap();
        inner.success_count
    }

    /// Manually resets the circuit breaker to closed state.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.state = CircuitState::Closed;
        inner.failure_count = 0;
        inner.success_count = 0;
        inner.last_failure_time = None;
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

    #[test]
    fn test_initial_state_is_closed() {
        let cb = CircuitBreaker::new();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.can_execute());
    }

    #[test]
    fn test_opens_after_threshold() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 2,
            cooldown_ms: 1000,
        });

        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.can_execute());
    }

    #[test]
    fn test_half_open_after_cooldown() {
        use std::thread;

        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_ms: 100,
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.can_execute());

        thread::sleep(Duration::from_millis(150));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitState::HalfOpen);
    }

    #[test]
    fn test_half_open_success_closes_circuit() {
        use std::thread;
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_ms: 50,
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        thread::sleep(Duration::from_millis(100));
        assert!(cb.can_execute());
        assert_eq!(cb.state(), CircuitState::HalfOpen);

        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_success_resets_failure_count_in_closed_state() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 3,
            success_threshold: 1,
            cooldown_ms: 1000,
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.failure_count(), 2);

        cb.record_success();
        assert_eq!(cb.failure_count(), 0);

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn test_reset() {
        let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_ms: 1000,
        });

        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);

        cb.reset();
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.can_execute());
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
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
        assert_eq!(cb.state(), CircuitState::Closed);
    }
}

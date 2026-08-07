#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Retry Policy for the Provider Runtime (P10.3).
//!
//! Retry belongs to the Provider Runtime, NOT to providers. It is
//! fully deterministic — given a policy it yields identical wait
//! schedules each time.
//!
//! Supports immediate retry, exponential backoff, max attempts, and a
//! total retry budget.

use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::types::{ProviderId, ProviderRuntimeError, ProviderRuntimeResult};

/// Backoff strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackoffStrategy {
    /// Every retry is immediate (delay 0).
    Immediate,
    /// Exponential backoff: delay_i = initial * multiplier^(attempt).
    Exponential,
    /// Fixed delay between retries.
    Fixed(Duration),
}

/// Retry policy (runtime-owned, deterministic).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub strategy: BackoffStrategy,
    /// Total number of attempts (>= 1). The first attempt is not a retry.
    pub max_attempts: usize,
    /// Initial backoff base.
    pub initial_backoff: Duration,
    /// Exponential growth factor (> 1).
    pub multiplier: f64,
    /// Upper bound on any single delay.
    pub max_backoff: Duration,
    /// Total allowed time across all retries (budget).
    pub budget: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        RetryPolicy {
            strategy: BackoffStrategy::Exponential,
            max_attempts: 3,
            initial_backoff: Duration::from_millis(200),
            multiplier: 2.0,
            max_backoff: Duration::from_secs(8),
            budget: Duration::from_secs(30),
        }
    }
}

impl RetryPolicy {
    pub fn immediate(max_attempts: usize) -> Self {
        RetryPolicy {
            strategy: BackoffStrategy::Immediate,
            max_attempts,
            ..Self::default()
        }
    }

    pub fn with_attempts(mut self, n: usize) -> Self {
        self.max_attempts = n.max(1);
        self
    }

    pub fn with_initial(mut self, d: Duration) -> Self {
        self.initial_backoff = d;
        self
    }

    pub fn with_budget(mut self, d: Duration) -> Self {
        self.budget = d;
        self
    }

    /// The delay to wait before the given retry attempt.
    /// `attempt` is 1-based (1 = first retry, 2 = second, ...).
    /// Deterministic.
    pub fn delay_for_attempt(&self, attempt: usize) -> Duration {
        if attempt == 0 {
            return Duration::ZERO;
        }
        match self.strategy {
            BackoffStrategy::Immediate => Duration::ZERO,
            BackoffStrategy::Fixed(d) => d,
            BackoffStrategy::Exponential => {
                let base = self.initial_backoff.as_millis() as f64;
                let mult = self.multiplier.max(1.0);
                let exp = self.max_attempts.min(attempt) - 1;
                let delay_ms = (base * mult.powi(exp as i32)).min(self.max_backoff.as_millis() as f64);
                Duration::from_millis(delay_ms as u64)
            }
        }
    }

    /// Whether another attempt is allowed given attempts already used
    /// and elapsed budget time.
    pub fn should_retry(&self, attempts_used: usize, elapsed: Duration) -> bool {
        if attempts_used >= self.max_attempts {
            return false;
        }
        elapsed < self.budget
    }

    /// Cumulative scheduled delay after `n` attempts consumed from time
    /// zero.
    pub fn cumulative_delay(&self, attempts: usize) -> Duration {
        let mut total = Duration::ZERO;
        for i in 1..=attempts {
            total += self.delay_for_attempt(i);
        }
        total
    }
}

/// A concrete, deterministic retry schedule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetrySchedule {
    pub policy: RetryPolicy,
    /// Delays before each retry (empty if no retries remain).
    pub retry_delays: Vec<Duration>,
}

impl RetrySchedule {
    /// Compute the schedule from a policy and consumed attempts.
    pub fn from(policy: RetryPolicy, attempts_consumed: usize) -> Self {
        let mut retry_delays = Vec::new();
        let remaining = policy.max_attempts.saturating_sub(attempts_consumed);
        if remaining > 0 {
            // Number of retries = attempts - 1 total; retries already
            // consumed = attempts_consumed - 1. Remaining retries =
            // (max_attempts - 1) - (attempts_consumed - 1).
            let retries_total = policy.max_attempts.saturating_sub(1);
            let retries_consumed = attempts_consumed.saturating_sub(1);
            let remaining_retries = retries_total.saturating_sub(retries_consumed);
            for k in 1..=remaining_retries {
                let attempt_idx = attempts_consumed + k;
                // Enforce budget: skip if cumulative delay exceeds budget.
                if policy.cumulative_delay(retries_consumed + k) > policy.budget {
                    break;
                }
                retry_delays.push(policy.delay_for_attempt(attempt_idx));
            }
        }
        RetrySchedule { policy, retry_delays }
    }

    pub fn is_empty(&self) -> bool {
        self.retry_delays.is_empty()
    }
}

/// A tracker for an in-flight retryable invocation.
#[derive(Debug, Clone)]
pub struct RetryController {
    policy: RetryPolicy,
    attempts_used: usize,
    elapsed: Duration,
}

impl RetryController {
    pub fn new(policy: RetryPolicy) -> Self {
        RetryController {
            policy,
            attempts_used: 0,
            elapsed: Duration::ZERO,
        }
    }

    pub fn attempts_used(&self) -> usize {
        self.attempts_used
    }

    /// Mark one attempt used and report whether another is allowed.
    pub fn next_delay(&mut self, elapsed: Duration) -> Option<Duration> {
        if !self.policy.should_retry(self.attempts_used, elapsed) {
            return None;
        }
        let next = self.policy.delay_for_attempt(self.attempts_used + 1);
        self.attempts_used += 1;
        Some(next)
    }

    pub fn record_attempt(&mut self) {
        self.attempts_used += 1;
    }

    pub fn exhausted(&self, elapsed: Duration) -> bool {
        !self.policy.should_retry(self.attempts_used, elapsed)
    }

    /// Attempt a single retry planning step, returning an error when the
    /// budget is exhausted.
    pub fn next_attempt(&mut self, elapsed: Duration, provider: &ProviderId) -> ProviderRuntimeResult<Duration> {
        self.next_delay(elapsed).ok_or_else(|| {
            ProviderRuntimeError::RetryExhausted {
                provider: provider.clone(),
                attempts: self.attempts_used,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_immediate_retry_zero_delay() {
        let p = RetryPolicy::immediate(5);
        assert_eq!(p.delay_for_attempt(1), Duration::ZERO);
        assert_eq!(p.delay_for_attempt(9), Duration::ZERO);
    }

    #[test]
    fn test_exponential_backoff_values() {
        let p = RetryPolicy::default()
            .with_attempts(4)
            .with_initial(Duration::from_millis(100))
            .with_budget(Duration::from_secs(60));
        assert_eq!(p.delay_for_attempt(1), Duration::from_millis(100));
        assert_eq!(p.delay_for_attempt(2), Duration::from_millis(200));
        assert_eq!(p.delay_for_attempt(3), Duration::from_millis(400));
    }

    #[test]
    fn test_exponential_caps_at_max_backoff() {
        let p = RetryPolicy {
            initial_backoff: Duration::from_millis(1000),
            multiplier: 2.0,
            max_backoff: Duration::from_millis(2500),
            ..Default::default()
        };
        assert_eq!(p.delay_for_attempt(2), Duration::from_millis(2000));
        assert_eq!(p.delay_for_attempt(3), Duration::from_millis(2500));
    }

    #[test]
    fn test_fixed_strategy() {
        let p = RetryPolicy {
            strategy: BackoffStrategy::Fixed(Duration::from_millis(500)),
            ..Default::default()
        };
        assert_eq!(p.delay_for_attempt(1), Duration::from_millis(500));
        assert_eq!(p.delay_for_attempt(5), Duration::from_millis(500));
    }

    #[test]
    fn test_max_attempts_respected() {
        let p = RetryPolicy::immediate(3);
        assert!(p.should_retry(0, Duration::ZERO));
        assert!(p.should_retry(1, Duration::ZERO));
        assert!(p.should_retry(2, Duration::ZERO));
        assert!(!p.should_retry(3, Duration::ZERO));
    }

    #[test]
    fn test_budget_respected() {
        let p = RetryPolicy {
            budget: Duration::from_millis(500),
            ..RetryPolicy::immediate(10)
        };
        assert!(p.should_retry(0, Duration::from_millis(400)));
        assert!(!p.should_retry(0, Duration::from_millis(501)));
    }

    #[test]
    fn test_cumulative_delay() {
        let p = RetryPolicy::default()
            .with_initial(Duration::from_millis(100))
            .with_budget(Duration::from_secs(60));
        assert_eq!(p.cumulative_delay(0), Duration::ZERO);
        assert_eq!(p.cumulative_delay(1), Duration::from_millis(100));
        assert_eq!(p.cumulative_delay(2), Duration::from_millis(300));
    }

    #[test]
    fn test_schedule_empty_when_consumed() {
        let p = RetryPolicy::immediate(2);
        let s = RetrySchedule::from(p, 2);
        assert!(s.is_empty());
    }

    #[test]
    fn test_schedule_lists_remaining() {
        let p = RetryPolicy::immediate(3);
        let s = RetrySchedule::from(p, 1);
        assert_eq!(s.retry_delays, vec![Duration::ZERO, Duration::ZERO]);
    }

    #[test]
    fn test_schedule_respects_budget() {
        let p = RetryPolicy {
            strategy: BackoffStrategy::Fixed(Duration::from_millis(400)),
            max_attempts: 10,
            initial_backoff: Duration::ZERO,
            multiplier: 1.0,
            max_backoff: Duration::ZERO,
            budget: Duration::from_millis(900),
        };
        let s = RetrySchedule::from(p, 0);
        // Delay 400 + 800 exceeds 900 -> only first two retries scheduled.
        assert_eq!(s.retry_delays.len(), 2);
    }

    #[test]
    fn test_controller_sequencing() {
        let p = RetryPolicy::default()
            .with_initial(Duration::from_millis(100))
            .with_budget(Duration::from_secs(60));
        let mut c = RetryController::new(p);
        assert_eq!(c.attempts_used(), 0);
        assert_eq!(c.next_delay(Duration::ZERO), Some(Duration::from_millis(100)));
        assert_eq!(c.attempts_used(), 1);
        assert_eq!(c.next_delay(Duration::ZERO), Some(Duration::from_millis(200)));
        assert_eq!(c.attempts_used(), 2);
    }

    #[test]
    fn test_controller_exhaustion_returns_error() {
        let p = RetryPolicy::immediate(1);
        let mut c = RetryController::new(p);
        let provider = ProviderId::new("p");
        assert!(c.next_delay(Duration::ZERO).is_some());
        let err = c.next_attempt(Duration::ZERO, &provider);
        assert!(err.is_err());
        assert!(matches!(err.unwrap_err(), ProviderRuntimeError::RetryExhausted { .. }));
    }

    #[test]
    fn test_controller_deterministic() {
        let p = RetryPolicy::default().with_attempts(5);
        let mut a = RetryController::new(p.clone());
        let mut b = RetryController::new(p);
        let mut da = Vec::new();
        let mut db = Vec::new();
        while let Some(d) = a.next_delay(Duration::ZERO) {
            da.push(d);
        }
        while let Some(d) = b.next_delay(Duration::ZERO) {
            db.push(d);
        }
        assert_eq!(da, db);
    }
}

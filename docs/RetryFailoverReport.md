# Retry & Failover Report

**Phase**: P10.3 — Provider Runtime Foundation

## Purpose

Retry and failover belong to the runtime, **NOT** to providers. They are
deterministic and preserve the request contract.

## Retry Policy

```rust
pub struct RetryPolicy {
    pub strategy: BackoffStrategy,  // Immediate | Exponential | Fixed
    pub max_attempts: usize,
    pub initial_backoff: Duration,
    pub multiplier: f64,            // > 1.0 for Exponential
    pub max_backoff: Duration,      // cap on any single delay
    pub budget: Duration,           // total retry time allowed
}

pub enum BackoffStrategy { Immediate, Exponential, Fixed }
```

- `delay_for_attempt(attempt)` — deterministic 1-based delay:
  - Immediate → `ZERO`
  - Fixed(d) → `d`
  - Exponential → `initial * multiplier^(attempt-1)`, capped at
    `max_backoff`.
- `should_retry(attempts_used, elapsed)` — respects `max_attempts` and
  `budget`.
- `cumulative_delay(attempts)` — sum of scheduled delays.

## Retry Schedule & Controller

- `RetrySchedule::from(policy, attempts_consumed)` — precomputes the
  remaining `retry_delays` (empty once consumed or when the budget is
  exceeded).
- `RetryController` — tracks `attempts_used`; yields the next delay via
  `next_delay(elapsed)`; returns `RetryExhausted` when the budget/attempt
  budget is spent.

Determinism: identical policy + state ⇒ identical schedules.

## Failover

```rust
pub struct FailoverPolicy {
    pub mode: FailoverMode,              // Deterministic | Ordered
    pub chain: Vec<ProviderId>,          // explicit ordered chain
    pub max_attempts: usize,
    pub failover_on_capability_mismatch: bool,
}
```

- `plan(&RouteRequest) -> Vec<ProviderId>` — builds the attempt order,
  either from the deterministic routing chain (best-first) or from an
  explicit Ordered chain, truncated to `max_attempts`.
- `execute(&RouteRequest, attempt: impl Fn(&ProviderId) -> Result<Outcome>)`
  — walks the chain; a non-Success outcome (or capability-mismatch error)
  fails over to the next provider; returns a `FailoverResult` with the
  attempted steps, the succeeded provider, and whether the failover was
  capability-driven.
- `plan_satisfies_capabilities` — verifies the whole plan preserves the
  request's required capabilities.

Failover is chain bookkeeping only; actual execution is supplied by the
caller's closure.

## Failover Modes Tested

Primary → Secondary → Fallback; health-based skip of unavailable
providers; capability-based failover; exhaustion and `max_attempts`
limits; deterministic ordering.

## Test Coverage

Retry (13): immediate zero, exponential values, max-backoff cap, fixed,
max attempts, budget, cumulative delay, empty/all/remaining schedules,
controller sequencing, exhaustion error, deterministic controller.
Failover (10): deterministic plan, ordered plan, truncation, first
success, fail-then-succeed, all-fail exhaust, capability-mismatch
trigger, empty chain error, max attempts, capability satisfaction.
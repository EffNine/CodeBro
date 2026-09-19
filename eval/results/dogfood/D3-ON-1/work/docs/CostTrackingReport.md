# Cost Tracking Report

**Phase**: P10.3 — Provider Runtime Foundation

## Purpose

Track estimated cost, actual cost, token usage, latency, success rate and
failure rate. Provider Runtime **reports** metrics; it does **NOT**
perform billing. Tracking is observational only — recording an outcome
never rejects or mutates a provider.

## Data Model

```rust
pub struct TokenUsage { pub input: usize, pub output: usize, pub total: usize }

pub struct CostObservation {
    pub provider: ProviderId,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub estimated_cost: f64,
    pub actual_cost: Option<f64>,
    pub latency_ms: u64,
    pub success: bool,
}
```

## ProviderCostSnapshot (`ProviderCostStats`)

Per-provider aggregates:

- `calls`, `successes`, `failures`, `timeouts`
- `total_input_tokens`, `total_output_tokens`
- `estimated_cost`, `actual_cost`
- `total_latency_ms`, `last_latency_ms`
- derived `success_rate()`, `failure_rate()`, `avg_latency_ms()`

## API

- `CostTracker::record(CostObservation)`
- `CostTracker::record_outcome(&ProviderId, Outcome)` — Success/Failure/Timeout
- `CostTracker::stats(&ProviderId) -> ProviderCostStats`
- `CostTracker::dashboard() -> CostDashboard`
- `CostTracker::summary() -> CostSummary`
- `CostTracker::timeouts(&ProviderId) -> usize`

`CostDashboard` aggregates calls, per-provider stats, bounded observation
history, total estimated cost, and overall success rate.

The descriptive `ProviderCost` model (input/output per-million cost) is
supplied by provider plugins and is only used to ESTIMATE; actual cost is
reported observationally.

## Tests (12)

Cover: token total, empty-stats defaults, success recording, failure
recording, success-rate math, timeout recording, mixed outcomes,
dashboard aggregation, cost summary, observation history, clear-resets,
and observational-only enforcement.
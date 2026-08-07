#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Cost Tracking for the Provider Runtime (P10.3).
//!
//! The runtime tracks Estimated Cost, Actual Cost, Token Usage,
//! Latency, Success Rate and Failure Rate. It REPORTS metrics; it does
//! NOT perform billing. Tracking is observational only.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::types::{CostObservation, Outcome, ProviderId};

/// Token usage for a single invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input: usize,
    pub output: usize,
    pub total: usize,
}

impl TokenUsage {
    pub fn new(input: usize, output: usize) -> Self {
        TokenUsage {
            input,
            output,
            total: input + output,
        }
    }
}

/// Aggregated per-provider cost statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderCostStats {
    pub provider: Option<ProviderId>,
    pub calls: usize,
    pub successes: usize,
    pub failures: usize,
    pub timeouts: usize,
    pub total_input_tokens: usize,
    pub total_output_tokens: usize,
    pub estimated_cost: f64,
    pub actual_cost: f64,
    pub total_latency_ms: u64,
    pub last_latency_ms: u64,
}

impl ProviderCostStats {
    pub fn success_rate(&self) -> f64 {
        if self.calls == 0 {
            1.0
        } else {
            self.successes as f64 / self.calls as f64
        }
    }

    pub fn failure_rate(&self) -> f64 {
        if self.calls == 0 {
            0.0
        } else {
            self.failures as f64 / self.calls as f64
        }
    }

    pub fn avg_latency_ms(&self) -> f64 {
        if self.calls == 0 {
            0.0
        } else {
            self.total_latency_ms as f64 / self.calls as f64
        }
    }
}

/// The full cost dashboard across providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostDashboard {
    pub providers: HashMap<String, ProviderCostStats>,
    pub observations: Vec<CostObservation>,
}

impl CostDashboard {
    pub fn total_calls(&self) -> usize {
        self.providers.values().map(|p| p.calls).sum()
    }

    pub fn total_estimated_cost(&self) -> f64 {
        self.providers
            .values()
            .map(|p| p.estimated_cost)
            .sum()
    }

    pub fn overall_success_rate(&self) -> f64 {
        let calls = self.total_calls();
        if calls == 0 {
            0.0
        } else {
            let s: usize = self.providers.values().map(|p| p.successes).sum();
            s as f64 / calls as f64
        }
    }
}

/// In-memory cost tracker. Observational, thread-safe.
#[derive(Clone, Default)]
pub struct CostTracker {
    inner: Arc<RwLock<CostDashboard>>,
    truncate_at: usize,
}

impl CostTracker {
    pub fn new() -> Self {
        CostTracker {
            inner: Arc::new(RwLock::new(CostDashboard::default())),
            truncate_at: 10_000,
        }
    }

    pub fn with_capacity(capacity: usize) -> Self {
        CostTracker {
            inner: Arc::new(RwLock::new(CostDashboard::default())),
            truncate_at: capacity,
        }
    }

    /// Record an invocation outcome. Observational only.
    pub fn record(&self, obs: CostObservation) {
        let mut inner = self.inner.write().unwrap();
        let key = obs.provider.to_string();
        let stats = inner.providers.entry(key.clone()).or_default();
        stats.provider = Some(obs.provider.clone());
        stats.calls += 1;
        stats.estimated_cost += obs.estimated_cost;
        stats.actual_cost += obs.actual_cost.unwrap_or(0.0);
        stats.total_latency_ms += obs.latency_ms;
        stats.last_latency_ms = obs.latency_ms;
        match obs.success {
            true => stats.successes += 1,
            false => {
                // Timeouts are recorded separately below when flagged.
                stats.failures += 1;
            }
        }
        let _ = key;
        if obs.input_tokens > 0 {
            stats.total_input_tokens += obs.input_tokens;
        }
        if obs.output_tokens > 0 {
            stats.total_output_tokens += obs.output_tokens;
        }
        inner.observations.push(obs);
        while inner.observations.len() > self.truncate_at {
            inner.observations.remove(0);
        }
    }

    /// Record with an explicit outcome (success / failure / timeout).
    pub fn record_outcome(&self, provider: &ProviderId, outcome: Outcome) {
        let mut inner = self.inner.write().unwrap();
        let key = provider.to_string();
        let stats = inner.providers.entry(key).or_default();
        stats.provider = Some(provider.clone());
        stats.calls += 1;
        match outcome {
            Outcome::Success => stats.successes += 1,
            Outcome::Failure => stats.failures += 1,
            Outcome::Timeout => stats.timeouts += 1,
        }
    }

    pub fn stats(&self, provider: &ProviderId) -> ProviderCostStats {
        let inner = self.inner.read().unwrap();
        inner
            .providers
            .get(&provider.to_string())
            .cloned()
            .unwrap_or_default()
    }

    pub fn dashboard(&self) -> CostDashboard {
        let inner = self.inner.read().unwrap();
        inner.clone()
    }

    pub fn clear(&self) {
        self.inner.write().unwrap().providers.clear();
    }

    /// Aggregate metrics snapshot.
    pub fn summary(&self) -> CostSummary {
        let d = self.dashboard();
        CostSummary {
            calls: d.total_calls(),
            estimated_cost: d.total_estimated_cost(),
            actual_cost: d.total_estimated_cost(),
            success_rate: d.total_success_rate(),
            provider_count: d.providers.len(),
        }
    }

    /// Timeout count for a provider.
    pub fn timeouts(&self, provider: &ProviderId) -> usize {
        self.stats(provider).timeouts
    }
}

impl CostDashboard {
    pub fn total_success_rate(&self) -> f64 {
        let calls = self.total_calls();
        if calls == 0 {
            0.0
        } else {
            let s: usize = self.providers.values().map(|p| p.successes).sum();
            s as f64 / calls as f64
        }
    }
}

/// Compact summary for reporting.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CostSummary {
    pub calls: usize,
    pub estimated_cost: f64,
    pub actual_cost: f64,
    pub success_rate: f64,
    pub provider_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(provider: &str, input: usize, output: usize, cost: f64, ok: bool, latency: u64) -> CostObservation {
        CostObservation {
            provider: ProviderId::new(provider),
            input_tokens: input,
            output_tokens: output,
            estimated_cost: cost,
            actual_cost: Some(cost),
            latency_ms: latency,
            success: ok,
        }
    }

    #[test]
    fn test_token_usage_total() {
        let t = TokenUsage::new(100, 50);
        assert_eq!(t.total, 150);
        assert_eq!(TokenUsage::default().total, 0);
    }

    #[test]
    fn test_empty_stats_defaults() {
        let c = CostTracker::new();
        let s = c.stats(&ProviderId::new("nobody"));
        assert_eq!(s.calls, 0);
        assert_eq!(s.success_rate(), 1.0);
        assert_eq!(s.failure_rate(), 0.0);
    }

    #[test]
    fn test_record_success_updates_stats() {
        let c = CostTracker::new();
        c.record(obs("p", 1000, 500, 0.012, true, 120));
        let s = c.stats(&ProviderId::new("p"));
        assert_eq!(s.calls, 1);
        assert_eq!(s.successes, 1);
        assert_eq!(s.total_input_tokens, 1000);
        assert_eq!(s.total_output_tokens, 500);
        assert!((s.estimated_cost - 0.012).abs() < 1e-9);
        assert_eq!(s.last_latency_ms, 120);
    }

    #[test]
    fn test_record_failure_updates_stats() {
        let c = CostTracker::new();
        c.record(obs("q", 10, 10, 0.0, false, 30));
        let s = c.stats(&ProviderId::new("q"));
        assert_eq!(s.failures, 1);
        assert_eq!(s.success_rate(), 0.0);
        assert_eq!(s.failure_rate(), 1.0);
    }

    #[test]
    fn test_success_rate_calculation() {
        let c = CostTracker::new();
        c.record(obs("r", 1, 1, 0.0, true, 10));
        c.record(obs("r", 1, 1, 0.0, false, 10));
        c.record(obs("r", 1, 1, 0.0, true, 10));
        assert!((c.stats(&ProviderId::new("r")).success_rate() - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn test_record_outcome_timeout() {
        let c = CostTracker::new();
        let id = ProviderId::new("t");
        c.record_outcome(&id, Outcome::Timeout);
        assert_eq!(c.timeouts(&id), 1);
        assert_eq!(c.stats(&id).calls, 1);
    }

    #[test]
    fn test_record_outcome_mixed() {
        let c = CostTracker::new();
        let id = ProviderId::new("m");
        c.record_outcome(&id, Outcome::Success);
        c.record_outcome(&id, Outcome::Failure);
        c.record_outcome(&id, Outcome::Timeout);
        let s = c.stats(&id);
        assert_eq!(s.successes, 1);
        assert_eq!(s.failures, 1);
        assert_eq!(s.timeouts, 1);
        assert_eq!(s.calls, 3);
    }

    #[test]
    fn test_dashboard_aggregation() {
        let c = CostTracker::new();
        c.record(obs("a", 100, 100, 0.1, true, 10));
        c.record(obs("a", 100, 100, 0.1, true, 10));
        c.record(obs("b", 100, 100, 0.2, false, 10));
        let d = c.dashboard();
        assert_eq!(d.total_calls(), 3);
        assert_eq!(d.providers.len(), 2);
        assert!((d.total_estimated_cost() - 0.4).abs() < 1e-9);
        assert!((d.total_success_rate() - (2.0 / 3.0)).abs() < 1e-9);
    }

    #[test]
    fn test_cost_summary() {
        let c = CostTracker::new();
        c.record(obs("a", 100, 100, 0.05, true, 5));
        let s = c.summary();
        assert_eq!(s.calls, 1);
        assert_eq!(s.provider_count, 1);
        assert!((s.success_rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn test_cost_observation_recorded_in_dashboard() {
        let c = CostTracker::new();
        c.record(obs("x", 5, 5, 0.001, true, 1));
        let d = c.dashboard();
        assert_eq!(d.observations.len(), 1);
    }

    #[test]
    fn test_clear_resets_stats() {
        let c = CostTracker::new();
        c.record(obs("p", 1, 1, 0.0, true, 1));
        c.clear();
        assert_eq!(c.stats(&ProviderId::new("p")).calls, 0);
    }

    #[test]
    fn test_tracking_is_observational_only() {
        // Tracking never rejects or mutates providers — it only records.
        let c = CostTracker::new();
        let before = c.dashboard().total_calls();
        c.record(obs("p", 1, 1, 0.0, false, 1));
        assert_eq!(c.dashboard().total_calls(), before + 1);
    }
}
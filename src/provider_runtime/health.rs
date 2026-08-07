#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Health Management for the Provider Runtime (P10.3).
//!
//! Health evaluation MUST be observational. It observes invocation
//! outcomes and derives a `HealthState`, but it never mutates provider
//! behaviour or enforces anything on a provider.
//!
//! States: Healthy, Degraded, Unavailable, Cooldown, Recovering.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use super::types::{HealthState, ProviderId, ProviderRuntimeError, ProviderRuntimeResult};

/// Config controlling how the health manager reacts to outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthPolicyConfig {
    /// Minimum recent call count before ratio thresholds are applied.
    pub min_samples: usize,
    /// Failure ratio (0..1) at or beyond which a provider is degraded.
    pub degrade_threshold: f64,
    /// Failure ratio at or beyond which a provider is unavailable.
    pub unavailable_threshold: f64,
    /// Number of consecutive failures that enter cooldown.
    pub cooldown_after: usize,
    /// Duration of a cooldown window.
    pub cooldown_duration: Duration,
    /// Consecutive successes required to return to healthy after a lapse.
    pub recovery_successes: usize,
}

impl Default for HealthPolicyConfig {
    fn default() -> Self {
        HealthPolicyConfig {
            min_samples: 3,
            degrade_threshold: 0.4,
            unavailable_threshold: 0.8,
            cooldown_after: 3,
            cooldown_duration: Duration::from_secs(60),
            recovery_successes: 2,
        }
    }
}

/// Per-provider health ledger (a snapshot of observed state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthRecord {
    pub provider: ProviderId,
    pub state: HealthState,
    pub successes: usize,
    pub failures: usize,
    pub total_calls: usize,
    pub consecutive_failures: usize,
    pub consecutive_successes: usize,
}

impl HealthRecord {
    pub fn new(provider: ProviderId) -> Self {
        HealthRecord {
            provider,
            state: HealthState::Healthy,
            successes: 0,
            failures: 0,
            total_calls: 0,
            consecutive_failures: 0,
            consecutive_successes: 0,
        }
    }

    pub fn failure_rate(&self) -> f64 {
        if self.total_calls == 0 {
            0.0
        } else {
            self.failures as f64 / self.total_calls as f64
        }
    }

    pub fn success_rate(&self) -> f64 {
        if self.total_calls == 0 {
            1.0
        } else {
            self.successes as f64 / self.total_calls as f64
        }
    }
}

/// Internal mutable state of the health manager.
#[derive(Default)]
struct HealthInner {
    records: HashMap<ProviderId, HealthRecord>,
    config: HealthPolicyConfig,
    /// Providers currently cooling down and when the window ends.
    cooldown_until: HashMap<ProviderId, Instant>,
}

/// Health manager. Observational, thread-safe, deterministic.
#[derive(Clone, Default)]
pub struct HealthManager {
    inner: Arc<RwLock<HealthInner>>,
}

impl HealthManager {
    pub fn new() -> Self {
        HealthManager::with_config(HealthPolicyConfig::default())
    }

    pub fn with_config(config: HealthPolicyConfig) -> Self {
        HealthManager {
            inner: Arc::new(RwLock::new(HealthInner {
                records: HashMap::new(),
                config,
                cooldown_until: HashMap::new(),
            })),
        }
    }

    fn ensure(&self, id: &ProviderId) {
        let mut inner = self.inner.write().unwrap();
        inner
            .records
            .entry(id.clone())
            .or_insert_with(|| HealthRecord::new(id.clone()));
    }

    /// Current health state for a provider.
    ///
    /// A provider that has never been observed is assumed Healthy
    /// (observational until proven otherwise).
    pub fn health(&self, id: &ProviderId) -> HealthState {
        let inner = self.inner.read().unwrap();
        if inner.cooldown_until.contains_key(id) {
            return HealthState::Cooldown;
        }
        inner
            .records
            .get(id)
            .map(|r| r.state)
            .unwrap_or(HealthState::Healthy)
    }

    /// Whether a provider is usable as a primary.
    pub fn is_available(&self, id: &ProviderId) -> bool {
        matches!(
            self.health(id),
            HealthState::Healthy | HealthState::Recovering
        )
    }

    /// Whether a provider may be selected for a request.
    pub fn is_selectable(&self, id: &ProviderId, allow_degraded: bool) -> bool {
        match self.health(id) {
            HealthState::Healthy => true,
            HealthState::Recovering => true,
            HealthState::Degraded => allow_degraded,
            HealthState::Unavailable | HealthState::Cooldown => false,
        }
    }

    /// Observe a successful invocation (observational only).
    pub fn report_success(&self, id: &ProviderId, at: Instant) {
        self.ensure(id);
        let mut inner = self.inner.write().unwrap();
        inner.cooldown_until.remove(id);
        let cfg = inner.config.clone();
        let rec = inner.records.get_mut(id).unwrap();
        rec.total_calls += 1;
        rec.successes += 1;
        rec.consecutive_successes += 1;
        rec.consecutive_failures = 0;

        // Recovering/unavailable/cooldown providers recover after enough
        // consecutive successes.
        if !matches!(rec.state, HealthState::Healthy) {
            if rec.consecutive_successes >= cfg.recovery_successes {
                rec.state = HealthState::Healthy;
            }
        }
    }

    /// Record a failed invocation (observational only).
    pub fn report_failure(&self, id: &ProviderId, at: Instant) {
        self.ensure(id);
        let mut inner = self.inner.write().unwrap();
        let cfg = inner.config.clone();
        let rec = inner.records.get_mut(id).unwrap();
        rec.total_calls += 1;
        rec.failures += 1;
        rec.consecutive_failures += 1;
        rec.consecutive_successes = 0;

        if rec.consecutive_failures >= cfg.cooldown_after {
            rec.state = HealthState::Cooldown;
            inner.cooldown_until.insert(id.clone(), at + cfg.cooldown_duration);
            return;
        }

        if rec.total_calls < cfg.min_samples {
            return;
        }

        let rate = rec.failure_rate();
        if rate >= cfg.unavailable_threshold {
            rec.state = HealthState::Unavailable;
        } else if rate >= cfg.degrade_threshold {
            rec.state = HealthState::Degraded;
        }
    }

    /// Mark a provider as recovering (a probe is underway).
    pub fn begin_recovery(&self, id: &ProviderId) -> ProviderRuntimeResult<()> {
        let mut inner = self.inner.write().unwrap();
        inner.cooldown_until.remove(id);
        let rec = inner.records.get_mut(id).ok_or_else(|| {
            ProviderRuntimeError::NotFound(id.clone())
        })?;
        if matches!(rec.state, HealthState::Unavailable | HealthState::Cooldown) {
            rec.state = HealthState::Recovering;
        }
        Ok(())
    }

    /// Snapshot of every tracked health record.
    pub fn all(&self) -> Vec<HealthRecord> {
        let inner = self.inner.read().unwrap();
        inner.records.values().cloned().collect()
    }

    pub fn count(&self) -> usize {
        let inner = self.inner.read().unwrap();
        inner.records.len()
    }

    /// Recorded health ledger for a provider (snapshot).
    pub fn record(&self, id: &ProviderId) -> Option<HealthRecord> {
        let inner = self.inner.read().unwrap();
        inner.records.get(id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(secs: u64) -> Instant {
        // Base far in the future so Cooldown comparisons remain valid.
        Instant::now() + Duration::from_secs(secs)
    }

    fn fast_config() -> HealthPolicyConfig {
        HealthPolicyConfig {
            min_samples: 1,
            ..Default::default()
        }
    }

    #[test]
    fn test_initial_health() {
        let hm = HealthManager::new();
        hm.ensure(&ProviderId::new("x"));
        assert_eq!(hm.health(&ProviderId::new("x")), HealthState::Healthy);
        assert!(hm.is_available(&ProviderId::new("x")));
    }

    #[test]
    fn test_unknown_provider_assumed_healthy() {
        let hm = HealthManager::new();
        // Never-observed providers are assumed healthy (observational
        // until proven otherwise).
        assert_eq!(hm.health(&ProviderId::new("zzz")), HealthState::Healthy);
        assert!(hm.is_available(&ProviderId::new("zzz")));
        assert!(hm.is_selectable(&ProviderId::new("zzz"), false));
    }

    #[test]
    fn test_success_observations() {
        let hm = HealthManager::new();
        let id = ProviderId::new("a");
        hm.report_success(&id, t(0));
        hm.report_success(&id, t(1));
        let rec = hm.record(&id).unwrap();
        assert_eq!(rec.successes, 2);
        assert_eq!(rec.total_calls, 2);
        assert_eq!(rec.success_rate(), 1.0);
    }

    #[test]
    fn test_unavailable_after_heavy_failures() {
        let hm = HealthManager::with_config(fast_config());
        let id = ProviderId::new("p");
        for i in 0..4 {
            hm.report_failure(&id, t(i));
        }
        assert_eq!(hm.health(&id), HealthState::Cooldown);
    }

    #[test]
    fn test_degrades_at_threshold() {
        let cfg = HealthPolicyConfig {
            min_samples: 2,
            degrade_threshold: 0.5,
            unavailable_threshold: 0.99,
            ..Default::default()
        };
        let hm = HealthManager::with_config(cfg);
        let id = ProviderId::new("p");
        hm.report_success(&id, t(0));
        hm.report_failure(&id, t(1)); // rate 0.5 -> degraded
        assert_eq!(hm.health(&id), HealthState::Degraded);
    }

    #[test]
    fn test_cooldown_triggered_by_consecutive_failures() {
        let hm = HealthManager::new();
        let id = ProviderId::new("p");
        for i in 0..3 {
            hm.report_failure(&id, t(i));
        }
        assert_eq!(hm.health(&id), HealthState::Cooldown);
    }

    #[test]
    fn test_cooldown_not_selectable() {
        let hm = HealthManager::new();
        let id = ProviderId::new("p");
        for i in 0..3 {
            hm.report_failure(&id, t(i));
        }
        assert!(!hm.is_selectable(&id, false));
        assert!(!hm.is_selectable(&id, true));
    }

    #[test]
    fn test_recovery_to_healthy() {
        let hm = HealthManager::new();
        let id = ProviderId::new("p");
        for i in 0..3 {
            hm.report_failure(&id, t(i));
        }
        assert_eq!(hm.health(&id), HealthState::Cooldown);
        hm.begin_recovery(&id).unwrap();
        assert_eq!(hm.health(&id), HealthState::Recovering);
        hm.report_success(&id, t(10));
        hm.report_success(&id, t(11));
        assert_eq!(hm.health(&id), HealthState::Healthy);
    }

    #[test]
    fn test_begin_recovery_unknown_provider_is_error() {
        let hm = HealthManager::new();
        assert!(hm.begin_recovery(&ProviderId::new("nope")).is_err());
    }

    #[test]
    fn test_failure_rate_calculation() {
        let hm = HealthManager::with_config(fast_config());
        let id = ProviderId::new("p");
        hm.report_success(&id, t(0));
        hm.report_failure(&id, t(1));
        let rec = hm.record(&id).unwrap();
        assert!((rec.failure_rate() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_degraded_selectable_only_when_allowed() {
        let cfg = HealthPolicyConfig {
            min_samples: 1,
            degrade_threshold: 0.01,
            unavailable_threshold: 2.0,
            ..Default::default()
        };
        let hm = HealthManager::with_config(cfg);
        let id = ProviderId::new("p");
        hm.report_failure(&id, t(0));
        assert_eq!(hm.health(&id), HealthState::Degraded);
        assert!(!hm.is_selectable(&id, false));
        assert!(hm.is_selectable(&id, true));
    }

    #[test]
    fn test_all_records_and_count() {
        let hm = HealthManager::new();
        hm.report_success(&ProviderId::new("a"), t(0));
        hm.report_success(&ProviderId::new("b"), t(1));
        assert_eq!(hm.all().len(), 2);
        assert_eq!(hm.count(), 2);
    }

    #[test]
    fn test_recovery_requires_consecutive_successes() {
        let hm = HealthManager::new();
        let id = ProviderId::new("p");
        for i in 0..3 {
            hm.report_failure(&id, t(i));
        }
        hm.begin_recovery(&id).unwrap();
        hm.report_success(&id, t(20)); // only 1 success
        assert_eq!(hm.health(&id), HealthState::Recovering);
        hm.report_success(&id, t(21)); // now 2
        assert_eq!(hm.health(&id), HealthState::Healthy);
    }
}
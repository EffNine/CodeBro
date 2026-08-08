#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider Discovery — descriptive queries over registered providers.
//!
//! Discovery is deterministic: it filters and orders by capability,
//! priority, and registration order. It performs no I/O and never talks
//! to a provider.

use super::capabilities::Capability;
use super::capabilities::CapabilitySet;
use super::health::HealthManager;
use super::registry::ProviderRegistry;
use super::types::{HealthState, ProviderId};

/// Result of a discovery query.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscoveryResult {
    /// Matching providers (descriptors only).
    pub providers: Vec<super::provider::RegisteredProvider>,
    /// The health state of each discovered provider, same order.
    pub health: Vec<HealthState>,
    /// Total providers considered.
    pub considered: usize,
}

/// Query constraints for discovery.
#[derive(Debug, Clone, Default)]
pub struct DiscoveryQuery {
    /// Required capabilities (all must be present).
    pub required: Vec<Capability>,
    /// Minimum priority (providers below are excluded).
    pub min_priority: Option<super::types::Priority>,
    /// Cap on returned providers.
    pub limit: Option<usize>,
    /// Only return providers that are selectable (respecting allow_degraded).
    pub healthy_only: bool,
    /// Allow degraded providers to be returned when healthy_only is true.
    pub allow_degraded: bool,
}

impl DiscoveryQuery {
    pub fn new() -> Self {
        DiscoveryQuery::default()
    }

    pub fn requiring(mut self, caps: Vec<Capability>) -> Self {
        self.required = caps;
        self
    }

    pub fn with_limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }

    pub fn only_healthy(mut self, allow_degraded: bool) -> Self {
        self.healthy_only = true;
        self.allow_degraded = allow_degraded;
        self
    }

    pub fn min_priority(mut self, min: Option<super::types::Priority>) -> Self {
        self.min_priority = min;
        self
    }
}

/// Discovery service over the registry + health manager.
#[derive(Clone)]
pub struct ProviderDiscovery {
    registry: ProviderRegistry,
    health: HealthManager,
}

impl ProviderDiscovery {
    pub fn new(registry: ProviderRegistry, health: HealthManager) -> Self {
        ProviderDiscovery { registry, health }
    }

    /// Run a query. Deterministic order: capability match, then priority
    /// (descending), then registration order.
    pub fn query(&self, q: &DiscoveryQuery) -> DiscoveryResult {
        let all = self.registry.all();
        let considered = all.len();

        let mut out = Vec::new();
        let mut health = Vec::new();

        for p in all {
            // Capability match (hard filter).
            if !q.required.is_empty() && !p.supports_all(&q.required) {
                continue;
            }
            if let Some(min) = q.min_priority {
                if p.priority.score() < min.score() {
                    continue;
                }
            }
            if q.healthy_only {
                let state = self.health.health(&p.id);
                let selectable = match state {
                    HealthState::Healthy | HealthState::Recovering => true,
                    HealthState::Degraded => q.allow_degraded,
                    _ => false,
                };
                if !selectable {
                    continue;
                }
            }
            let state = self.health.health(&p.id);
            health.push(state);
            out.push(p);
        }

        // Stable, deterministic ordering: priority desc, then registration.
        out.sort_by(|a, b| {
            b.priority
                .score()
                .cmp(&a.priority.score())
                .then(a.registration_seq.cmp(&b.registration_seq))
        });
        // Re-sort health to match.
        let mut ordered_health = Vec::with_capacity(out.len());
        for p in &out {
            ordered_health.push(self.health.health(&p.id));
        }

        if let Some(limit) = q.limit {
            out.truncate(limit);
            ordered_health.truncate(limit);
        }

        DiscoveryResult {
            providers: out,
            health: ordered_health,
            considered,
        }
    }

    /// All providers with the given capability (no health filter).
    pub fn with_capability(&self, cap: Capability) -> Vec<super::provider::RegisteredProvider> {
        let q = DiscoveryQuery::new().requiring(vec![cap]);
        self.query(&q).providers
    }

    /// Providers supporting every required capability, selectable only.
    pub fn find_usable(
        &self,
        required: &[Capability],
        allow_degraded: bool,
    ) -> Vec<super::provider::RegisteredProvider> {
        let q = DiscoveryQuery::new()
            .requiring(required.to_vec())
            .only_healthy(allow_degraded);
        self.query(&q).providers
    }

    /// Number of registered providers matching a capability set exactly.
    pub fn count_with(&self, caps: &CapabilitySet) -> usize {
        let mut n = 0;
        for p in self.registry.all() {
            if caps.iter().all(|c| p.supports_all(&[c])) {
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_runtime::provider::RegisteredProvider;
    use crate::provider_runtime::types::{Priority, ProviderCost};

    fn rec(id: &str, caps: &[Capability], priority: Priority, cost: f64) -> RegisteredProvider {
        RegisteredProvider::new(
            id,
            CapabilitySet::new(caps.iter().copied()),
            ProviderCost {
                input_per_million: cost,
                output_per_million: cost,
                cache_read_per_million: None,
            },
            priority,
        )
    }

    #[test]
    fn test_discovery_empty_registry() {
        let reg = ProviderRegistry::new();
        let hm = HealthManager::new();
        let d = ProviderDiscovery::new(reg, hm);
        let r = d.query(&DiscoveryQuery::default());
        assert_eq!(r.providers.len(), 0);
        assert_eq!(r.considered, 0);
    }

    #[test]
    fn test_discovery_returns_all_with_no_constraints() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec("b", &[], Priority::Normal, 1.0))
            .unwrap();
        let hm = HealthManager::new();
        let d = ProviderDiscovery::new(reg, hm);
        let r = d.query(&DiscoveryQuery::default());
        assert_eq!(r.providers.len(), 2);
        assert_eq!(r.considered, 2);
    }

    #[test]
    fn test_discovery_filters_by_capability() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::Streaming], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec("b", &[Capability::Vision], Priority::Normal, 1.0))
            .unwrap();
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        let r = d.query(&DiscoveryQuery::new().requiring(vec![Capability::Streaming]));
        assert_eq!(r.providers.len(), 1);
        assert_eq!(r.providers[0].id.as_str(), "a");
    }

    #[test]
    fn test_discovery_orders_by_priority_then_registration() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("low", &[], Priority::Low, 1.0))
            .unwrap();
        reg.register_value(rec("high", &[], Priority::High, 1.0))
            .unwrap();
        reg.register_value(rec("norm", &[], Priority::Normal, 1.0))
            .unwrap();
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        let r = d.query(&DiscoveryQuery::default());
        let ids: Vec<String> = r.providers.iter().map(|p| p.id.to_string()).collect();
        assert_eq!(ids, vec!["high", "norm", "low"]);
    }

    #[test]
    fn test_discovery_stable_for_equal_priority() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("first", &[], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec("second", &[], Priority::Normal, 1.0))
            .unwrap();
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        let r1 = d.query(&DiscoveryQuery::default());
        let r2 = d.query(&DiscoveryQuery::default());
        assert_eq!(r1.providers, r2.providers);
        assert_eq!(r1.providers[0].id.as_str(), "first");
    }

    #[test]
    fn test_discovery_healthy_only_skips_cooldown() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("good", &[], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec("bad", &[], Priority::Normal, 1.0))
            .unwrap();
        let hm = HealthManager::new();
        let t = std::time::Instant::now();
        for i in 0..3 {
            hm.report_failure(
                &crate::provider_runtime::types::ProviderId::new("bad"),
                t + std::time::Duration::from_secs(i),
            );
        }
        let d = ProviderDiscovery::new(reg, hm);
        let r = d.query(&DiscoveryQuery::new().only_healthy(false));
        assert_eq!(r.providers.len(), 1);
        assert_eq!(r.providers[0].id.as_str(), "good");
    }

    #[test]
    fn test_discovery_limit() {
        let reg = ProviderRegistry::new();
        for i in 0..5 {
            reg.register_value(rec(&format!("p{i}"), &[], Priority::Normal, 1.0))
                .unwrap();
        }
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        let r = d.query(&DiscoveryQuery::new().with_limit(2));
        assert_eq!(r.providers.len(), 2);
    }

    #[test]
    fn test_discovery_min_priority() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("low", &[], Priority::Low, 1.0))
            .unwrap();
        reg.register_value(rec("high", &[], Priority::High, 1.0))
            .unwrap();
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        let r = d.query(&DiscoveryQuery::new().min_priority(Some(Priority::High)));
        assert_eq!(r.providers.len(), 1);
        assert_eq!(r.providers[0].id.as_str(), "high");
    }

    #[test]
    fn test_discovery_health_vector_aligns() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[], Priority::Normal, 1.0))
            .unwrap();
        let hm = HealthManager::new();
        hm.report_success(&ProviderId::new("a"), std::time::Instant::now());
        let d = ProviderDiscovery::new(reg, hm);
        let r = d.query(&DiscoveryQuery::default());
        assert_eq!(r.health.len(), r.providers.len());
        assert_eq!(r.health[0], HealthState::Healthy);
    }

    #[test]
    fn test_with_capability_helper() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::ToolCalling], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec("b", &[], Priority::Normal, 1.0))
            .unwrap();
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        assert_eq!(d.with_capability(Capability::ToolCalling).len(), 1);
    }

    #[test]
    fn test_find_usable_respects_health() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::Streaming], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec("b", &[Capability::Streaming], Priority::Normal, 1.0))
            .unwrap();
        let hm = HealthManager::new();
        let t = std::time::Instant::now();
        for i in 0..3 {
            hm.report_failure(&ProviderId::new("b"), t + std::time::Duration::from_secs(i));
        }
        let d = ProviderDiscovery::new(reg, hm);
        let usable = d.find_usable(&[Capability::Streaming], false);
        assert_eq!(usable.len(), 1);
        assert_eq!(usable[0].id.as_str(), "a");
    }

    #[test]
    fn test_count_with() {
        let reg = ProviderRegistry::new();
        let mut caps = CapabilitySet::empty();
        caps.insert(Capability::JsonMode);
        reg.register_value(rec("a", &[Capability::JsonMode], Priority::Normal, 1.0))
            .unwrap();
        reg.register_value(rec(
            "b",
            &[Capability::JsonMode, Capability::Audio],
            Priority::Normal,
            1.0,
        ))
        .unwrap();
        let d = ProviderDiscovery::new(reg, HealthManager::new());
        assert_eq!(d.count_with(&caps), 2);
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider Router — deterministic provider selection.
//!
//! Selection order (MANDATORY contract):
//!
//! 1. Capability Match
//! 2. Policy
//! 3. Health
//! 4. Cost
//! 5. Priority
//! 6. Registration Order
//!
//! Provider name MUST NEVER influence routing. The provider id/name is
//! never read as part of the ordering key.

use serde::{Deserialize, Serialize};

use super::capabilities::{Capability, CapabilityMatch, CapabilitySet};
use super::health::HealthManager;
use super::provider::RegisteredProvider;
use super::registry::ProviderRegistry;
use super::types::{
    HealthState, ProviderId, ProviderRuntimeError, ProviderRuntimeResult, RouteRequest,
};

/// Routing policy — runtime configuration, not provider configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    /// Whether degraded providers may be selected when nothing healthy
    /// matches.
    pub allow_degraded_fallback: bool,
    /// Whether to skip providers in cooldown/unavailable entirely.
    pub skip_unhealthy: bool,
}

impl Default for RoutingPolicy {
    fn default() -> Self {
        RoutingPolicy {
            allow_degraded_fallback: true,
            skip_unhealthy: true,
        }
    }
}

/// Why a candidate was rejected during selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RejectionReason {
    CapabilityMismatch { missing: Vec<Capability> },
    ExcludedByRequest,
    Unhealthy(HealthState),
    CostCeiling,
}

/// A rejected candidate during selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rejection {
    pub provider: ProviderId,
    pub reason: RejectionReason,
}

/// The result of a deterministic routing decision.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouterDecision {
    pub provider: RegisteredProvider,
    /// Ordered list of filters that were applied and passed.
    pub applied: Vec<String>,
    /// Candidates rejected and why (diagnostics).
    pub rejected: Vec<Rejection>,
    /// Total candidates considered.
    pub considered: usize,
}

/// Deterministic router.
#[derive(Clone)]
pub struct ProviderRouter {
    registry: ProviderRegistry,
    health: HealthManager,
    policy: RoutingPolicy,
}

impl ProviderRouter {
    pub fn new(registry: ProviderRegistry, health: HealthManager) -> Self {
        ProviderRouter {
            registry,
            health,
            policy: RoutingPolicy::default(),
        }
    }

    pub fn with_policy(mut self, policy: RoutingPolicy) -> Self {
        self.policy = policy;
        self
    }

    pub fn policy(&self) -> &RoutingPolicy {
        &self.policy
    }

    /// Capabilities of a registered provider by id (for failover checks).
    pub fn provider_capabilities(&self, id: &ProviderId) -> Option<CapabilitySet> {
        self.registry.get(id).map(|p| p.capabilities)
    }

    /// Resolve the best provider for a request, deterministically.
    pub fn resolve(&self, request: &RouteRequest) -> ProviderRuntimeResult<RouterDecision> {
        let all = self.registry.all();
        let considered = all.len();

        if all.is_empty() {
            return Err(ProviderRuntimeError::NoSuitableProvider(
                "No providers registered".to_string(),
            ));
        }

        let mut rejected: Vec<Rejection> = Vec::new();
        let mut candidates: Vec<RegisteredProvider> = Vec::new();

        // Stage 1 — Capability match (hard filter).
        for p in all {
            if let Some(idx) = request.excluded.iter().position(|e| *e == p.id) {
                rejected.push(Rejection {
                    provider: p.id.clone(),
                    reason: RejectionReason::ExcludedByRequest,
                });
                continue;
            }

            let cm = CapabilityMatch::new(&request.required_capabilities, &p.capabilities);
            if !cm.compatible {
                rejected.push(Rejection {
                    provider: p.id.clone(),
                    reason: RejectionReason::CapabilityMismatch {
                        missing: cm.missing,
                    },
                });
                continue;
            }

            // Stage 2 — Policy: cost ceiling.
            if let Some(ceiling) = request.max_cost {
                if p.cost.routing_cost() > ceiling {
                    rejected.push(Rejection {
                        provider: p.id.clone(),
                        reason: RejectionReason::CostCeiling,
                    });
                    continue;
                }
            }

            // Stage 3 — Health.
            let state = self.health.health(&p.id);
            let selectable = match state {
                HealthState::Healthy | HealthState::Recovering => true,
                HealthState::Degraded => {
                    request.allow_degraded || self.policy.allow_degraded_fallback
                }
                HealthState::Unavailable | HealthState::Cooldown => !self.policy.skip_unhealthy,
            };
            if !selectable {
                rejected.push(Rejection {
                    provider: p.id.clone(),
                    reason: RejectionReason::Unhealthy(state),
                });
                continue;
            }

            candidates.push(p);
        }

        if candidates.is_empty() {
            return Err(ProviderRuntimeError::NoSuitableProvider(
                "No candidate passed the routing stages".to_string(),
            ));
        }

        // Stages 4-6 — deterministic ordering.
        // Key: (health_rank asc, cost asc, priority desc, registration asc).
        candidates.sort_by(|a, b| {
            let ha = health_rank(self.health.health(&a.id));
            let hb = health_rank(self.health.health(&b.id));
            ha.cmp(&hb)
                .then(
                    a.cost
                        .routing_cost()
                        .partial_cmp(&b.cost.routing_cost())
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(b.priority.score().cmp(&a.priority.score()))
                .then(a.registration_seq.cmp(&b.registration_seq))
        });

        let selected = candidates.into_iter().next().unwrap();

        Ok(RouterDecision {
            provider: selected,
            applied: vec![
                "capability_match".to_string(),
                "policy".to_string(),
                "health".to_string(),
                "cost".to_string(),
                "priority".to_string(),
                "registration_order".to_string(),
            ],
            rejected,
            considered,
        })
    }

    /// Deterministic fallback chain: all usable providers for a request,
    /// best-first. Used by the failover machinery.
    pub fn chain(&self, request: &RouteRequest) -> Vec<RouterDecision> {
        let all = self.registry.all();
        let considered = all.len();
        let mut out = Vec::new();
        for p in all {
            if !p.supports_all(&request.required_capabilities) {
                continue;
            }
            if !self.health.is_selectable(&p.id, request.allow_degraded) {
                continue;
            }
            out.push(RouterDecision {
                provider: p.clone(),
                applied: vec!["capability_match".to_string(), "health".to_string()],
                rejected: Vec::new(),
                considered,
            });
        }
        out.sort_by(|a, b| {
            let ha = health_rank(self.health.health(&a.provider.id));
            let hb = health_rank(self.health.health(&b.provider.id));
            ha.cmp(&hb)
                .then(
                    a.provider
                        .cost
                        .routing_cost()
                        .partial_cmp(&b.provider.cost.routing_cost())
                        .unwrap_or(std::cmp::Ordering::Equal),
                )
                .then(
                    b.provider
                        .priority
                        .score()
                        .cmp(&a.provider.priority.score()),
                )
                .then(
                    a.provider
                        .registration_seq
                        .cmp(&b.provider.registration_seq),
                )
        });
        out
    }
}

/// Health rank used purely for ordering — never exposed as behavior.
fn health_rank(state: HealthState) -> u8 {
    match state {
        HealthState::Healthy => 0,
        HealthState::Recovering => 1,
        HealthState::Degraded => 2,
        HealthState::Unavailable => 3,
        HealthState::Cooldown => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_runtime::types::{Priority, ProviderCost};

    fn rec(id: &str, caps: &[Capability], cost: f64, priority: Priority) -> RegisteredProvider {
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

    fn healthy_router() -> ProviderRouter {
        ProviderRouter::new(ProviderRegistry::new(), HealthManager::new())
    }

    #[test]
    fn test_resolve_no_providers() {
        let r = healthy_router();
        let err = r.resolve(&RouteRequest::new()).unwrap_err();
        assert!(matches!(err, ProviderRuntimeError::NoSuitableProvider(_)));
    }

    #[test]
    fn test_resolve_selects_capability_match() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "novision",
            &[Capability::Streaming],
            1.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec(
            "vision",
            &[Capability::Streaming, Capability::Vision],
            5.0,
            Priority::Normal,
        ))
        .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let decision = r
            .resolve(&RouteRequest::new().with_capabilities(vec![Capability::Vision]))
            .unwrap();
        assert_eq!(decision.provider.id.as_str(), "vision");
    }

    #[test]
    fn test_resolve_rejects_capability_mismatch() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("only", &[Capability::Streaming], 1.0, Priority::Normal))
            .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let err = r
            .resolve(&RouteRequest::new().with_capabilities(vec![Capability::Audio]))
            .unwrap_err();
        assert!(matches!(err, ProviderRuntimeError::NoSuitableProvider(_)));
    }

    #[test]
    fn test_resolve_lower_cost_wins() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "cheap",
            &[Capability::Streaming],
            1.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec(
            "pricey",
            &[Capability::Streaming],
            50.0,
            Priority::Normal,
        ))
        .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r.resolve(&RouteRequest::new()).unwrap();
        assert_eq!(d.provider.id.as_str(), "cheap");
    }

    #[test]
    fn test_resolve_higher_priority_wins_at_equal_cost() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "normal",
            &[Capability::Streaming],
            10.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec(
            "critical",
            &[Capability::Streaming],
            10.0,
            Priority::Critical,
        ))
        .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r.resolve(&RouteRequest::new()).unwrap();
        assert_eq!(d.provider.id.as_str(), "critical");
    }

    #[test]
    fn test_resolve_registration_order_is_tiebreaker() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "later",
            &[Capability::Streaming],
            5.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec(
            "earlier",
            &[Capability::Streaming],
            5.0,
            Priority::Normal,
        ))
        .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r.resolve(&RouteRequest::new()).unwrap();
        // Earlier registration wins on the final tiebreak.
        assert_eq!(d.provider.id.as_str(), "later");
    }

    #[test]
    fn test_resolve_skips_cooldown() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "down",
            &[Capability::Streaming],
            1.0,
            Priority::Critical,
        ))
        .unwrap();
        reg.register_value(rec("up", &[Capability::Streaming], 2.0, Priority::Normal))
            .unwrap();
        let hm = HealthManager::new();
        let t = std::time::Instant::now();
        for i in 0..3 {
            hm.report_failure(
                &ProviderId::new("down"),
                t + std::time::Duration::from_secs(i),
            );
        }
        let r = ProviderRouter::new(reg, hm);
        let d = r.resolve(&RouteRequest::new()).unwrap();
        assert_eq!(d.provider.id.as_str(), "up");
    }

    #[test]
    fn test_resolve_prefers_healthy_over_degraded() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "deg",
            &[Capability::Streaming],
            0.1,
            Priority::Critical,
        ))
        .unwrap();
        reg.register_value(rec("ok", &[Capability::Streaming], 5.0, Priority::Low))
            .unwrap();
        let hm = HealthManager::new();
        let cfg = crate::provider_runtime::health::HealthPolicyConfig {
            min_samples: 1,
            degrade_threshold: 0.01,
            unavailable_threshold: 1.0,
            ..Default::default()
        };
        let hm = HealthManager::with_config(cfg);
        hm.report_failure(&ProviderId::new("deg"), std::time::Instant::now());
        let r = ProviderRouter::new(reg, hm);
        let d = r
            .resolve(&RouteRequest::new().allow_degraded(true))
            .unwrap();
        assert_eq!(d.provider.id.as_str(), "ok");
    }

    #[test]
    fn test_resolve_cost_ceiling() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "cheap",
            &[Capability::Streaming],
            1.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec(
            "rich",
            &[Capability::Streaming],
            100.0,
            Priority::Critical,
        ))
        .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r
            .resolve(&RouteRequest::new().with_cost_ceiling(10.0))
            .unwrap();
        assert_eq!(d.provider.id.as_str(), "cheap");
    }

    #[test]
    fn test_resolve_all_above_ceiling_fails() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("p1", &[Capability::Streaming], 20.0, Priority::Normal))
            .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let err = r
            .resolve(&RouteRequest::new().with_cost_ceiling(1.0))
            .unwrap_err();
        assert!(matches!(err, ProviderRuntimeError::NoSuitableProvider(_)));
    }

    #[test]
    fn test_resolve_excluded_provider() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::Streaming], 1.0, Priority::Critical))
            .unwrap();
        reg.register_value(rec("b", &[Capability::Streaming], 2.0, Priority::Normal))
            .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r
            .resolve(&RouteRequest::new().excluding(vec![ProviderId::new("a")]))
            .unwrap();
        assert_eq!(d.provider.id.as_str(), "b");
    }

    #[test]
    fn test_resolve_records_rejections() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "noaud",
            &[Capability::Streaming],
            1.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec("audio", &[Capability::Audio], 1.0, Priority::Normal))
            .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r
            .resolve(&RouteRequest::new().with_capabilities(vec![Capability::Audio]))
            .unwrap();
        assert_eq!(d.provider.id.as_str(), "audio");
        assert!(d
            .rejected
            .iter()
            .any(|r| matches!(r.reason, RejectionReason::CapabilityMismatch { .. })));
    }

    #[test]
    fn test_resolve_deterministic_across_calls() {
        let reg = ProviderRegistry::new();
        for i in 0..20 {
            let cost = (i as f64) * 0.7 + 0.1;
            reg.register_value(rec(
                &format!("p{i}"),
                &[Capability::Streaming],
                cost,
                Priority::Normal,
            ))
            .unwrap();
        }
        let r = ProviderRouter::new(reg, HealthManager::new());
        let a = r.resolve(&RouteRequest::new()).unwrap();
        let b = r.resolve(&RouteRequest::new()).unwrap();
        assert_eq!(a.provider.id, b.provider.id);
        assert_eq!(a, b);
    }

    #[test]
    fn test_resolve_considered_counts_all() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::Streaming], 1.0, Priority::Normal))
            .unwrap();
        reg.register_value(rec("b", &[Capability::Audio], 1.0, Priority::Normal))
            .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r
            .resolve(&RouteRequest::new().with_capabilities(vec![Capability::Streaming]))
            .unwrap();
        assert_eq!(d.considered, 2);
        assert_eq!(d.applied.len(), 6);
    }

    #[test]
    fn test_name_never_influences_routing() {
        // Two providers identical in every routing key but with wildly
        // different ids; selection must be registration-order based.
        let reg = ProviderRegistry::new();
        reg.register_value(rec(
            "zzz-alpha",
            &[Capability::Streaming],
            3.0,
            Priority::Normal,
        ))
        .unwrap();
        reg.register_value(rec(
            "aaa-beta",
            &[Capability::Streaming],
            3.0,
            Priority::Normal,
        ))
        .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let d = r.resolve(&RouteRequest::new()).unwrap();
        assert_eq!(d.provider.id.as_str(), "zzz-alpha");
    }

    #[test]
    fn test_chain_orders_deterministically() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::Streaming], 9.0, Priority::Normal))
            .unwrap();
        reg.register_value(rec("b", &[Capability::Streaming], 1.0, Priority::Normal))
            .unwrap();
        reg.register_value(rec("c", &[Capability::Streaming], 5.0, Priority::High))
            .unwrap();
        let r = ProviderRouter::new(reg, HealthManager::new());
        let chain = r.chain(&RouteRequest::new().with_capabilities(vec![Capability::Streaming]));
        let ids: Vec<String> = chain.iter().map(|d| d.provider.id.to_string()).collect();
        assert_eq!(ids, vec!["b", "c", "a"]);
    }

    #[test]
    fn test_chain_filters_unhealthy() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("good", &[Capability::Streaming], 1.0, Priority::Normal))
            .unwrap();
        reg.register_value(rec("bad", &[Capability::Streaming], 1.0, Priority::Normal))
            .unwrap();
        let hm = HealthManager::new();
        let t = std::time::Instant::now();
        for i in 0..3 {
            hm.report_failure(
                &ProviderId::new("bad"),
                t + std::time::Duration::from_secs(i),
            );
        }
        let r = ProviderRouter::new(reg, hm);
        let chain = r.chain(&RouteRequest::new().with_capabilities(vec![Capability::Streaming]));
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].provider.id.as_str(), "good");
    }
}

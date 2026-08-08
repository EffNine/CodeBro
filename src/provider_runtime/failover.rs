#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Failover Policy for the Provider Runtime (P10.3).
//!
//! Failover preserves the request contract: the same request may be
//! routed through a chain of providers. Primary → Secondary → Fallback
//! chain. Failover may be health-based or capability-based.

use serde::{Deserialize, Serialize};

use super::capabilities::Capability;
use super::router::ProviderRouter;
use super::types::{
    Outcome, ProviderId, ProviderRuntimeError, ProviderRuntimeResult, RouteRequest,
};

/// How the failover chain is derived for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailoverMode {
    /// Use the deterministic routing chain (best first).
    Deterministic,
    /// Respect an explicit ordered list of primary/secondary ids.
    Ordered,
}

/// Failover policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailoverPolicy {
    pub mode: FailoverMode,
    /// Explicit ordered chain (used when mode == Ordered).
    pub chain: Vec<ProviderId>,
    /// Maximum providers to attempt before giving up.
    pub max_attempts: usize,
    /// Whether to consider capability mismatch as a failover trigger.
    pub failover_on_capability_mismatch: bool,
}

impl Default for FailoverPolicy {
    fn default() -> Self {
        FailoverPolicy {
            mode: FailoverMode::Deterministic,
            chain: Vec::new(),
            max_attempts: 3,
            failover_on_capability_mismatch: true,
        }
    }
}

impl FailoverPolicy {
    pub fn ordered(chain: Vec<ProviderId>) -> Self {
        FailoverPolicy {
            mode: FailoverMode::Ordered,
            chain,
            ..Self::default()
        }
    }

    pub fn with_max_attempts(mut self, n: usize) -> Self {
        self.max_attempts = n.max(1);
        self
    }
}

/// One step in a failover execution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailoverStep {
    pub provider: ProviderId,
    pub index: usize,
    pub outcome: Outcome,
}

/// Result of a failover attempt across a chain.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FailoverResult {
    pub attempted: Vec<FailoverStep>,
    pub succeeded: Option<ProviderId>,
    /// True when a capability mismatch triggered the failover.
    pub capability_driven: bool,
    /// Reason the chain ended without success.
    pub exhausted_reason: Option<String>,
}

impl FailoverResult {
    pub fn new() -> Self {
        FailoverResult {
            attempted: Vec::new(),
            succeeded: None,
            capability_driven: false,
            exhausted_reason: None,
        }
    }
}

/// Failover executor. It plans and walks a chain, preserving the
/// request contract across providers.
#[derive(Clone)]
pub struct Failover {
    router: ProviderRouter,
    policy: FailoverPolicy,
}

impl Failover {
    pub fn new(router: ProviderRouter, policy: FailoverPolicy) -> Self {
        Failover { router, policy }
    }

    pub fn policy(&self) -> &FailoverPolicy {
        &self.policy
    }

    /// The deterministic candidate chain for a request.
    pub fn plan(&self, request: &RouteRequest) -> Vec<ProviderId> {
        match self.policy.mode {
            FailoverMode::Ordered => {
                let mut chain = self.policy.chain.clone();
                chain.truncate(self.policy.max_attempts);
                chain
            }
            FailoverMode::Deterministic => {
                let decs = self.router.chain(&request);
                let mut ids: Vec<ProviderId> = decs.into_iter().map(|d| d.provider.id).collect();
                ids.truncate(self.policy.max_attempts);
                ids
            }
        }
    }

    /// Walk the plan, recording steps. The caller supplies the actual
    /// execution closure; failover is runtime bookkeeping only.
    pub fn execute<F>(
        &self,
        request: &RouteRequest,
        mut attempt: F,
    ) -> ProviderRuntimeResult<FailoverResult>
    where
        F: FnMut(&ProviderId) -> ProviderRuntimeResult<Outcome>,
    {
        let plan = self.plan(request);
        if plan.is_empty() {
            return Err(ProviderRuntimeError::FailoverExhausted {
                total: self.policy.max_attempts,
                attempted: 0,
            });
        }
        let mut result = FailoverResult::new();
        let mut capability_driven = false;

        for (index, id) in plan.iter().enumerate() {
            if result.attempted.len() >= self.policy.max_attempts {
                break;
            }
            let outcome = match attempt(id) {
                Ok(o) => o,
                Err(e) => {
                    // A capability mismatch is a valid failover trigger.
                    if self.policy.failover_on_capability_mismatch
                        && matches!(e, ProviderRuntimeError::CapabilityMismatch { .. })
                    {
                        capability_driven = true;
                    }
                    // The provider refused to run — fail over.
                    result.attempted.push(FailoverStep {
                        provider: id.clone(),
                        index,
                        outcome: Outcome::Failure,
                    });
                    continue;
                }
            };
            result.attempted.push(FailoverStep {
                provider: id.clone(),
                index,
                outcome,
            });
            if outcome == Outcome::Success {
                result.succeeded = Some(id.clone());
                result.capability_driven = capability_driven;
                return Ok(result);
            }
        }

        result.capability_driven = capability_driven;
        result.exhausted_reason = Some("No provider in the chain succeeded".to_string());
        Ok(result)
    }

    /// Verify that the entire plan satisfies the required capabilities
    /// (capability-based failover keeps the request contract).
    pub fn plan_satisfies_capabilities(
        &self,
        request: &RouteRequest,
        required: &[Capability],
    ) -> bool {
        self.plan(request).iter().all(|id| {
            self.router
                .provider_capabilities(id)
                .map(|c| c.has_all(required))
                .unwrap_or(false)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::Outcome;
    use super::*;
    use crate::provider_runtime::{
        capabilities::CapabilitySet,
        health::HealthManager,
        provider::RegisteredProvider,
        registry::ProviderRegistry,
        router::ProviderRouter,
        types::{Priority, ProviderCost},
    };

    fn rec(id: &str, caps: &[Capability], cost: f64) -> RegisteredProvider {
        RegisteredProvider::new(
            id,
            CapabilitySet::new(caps.iter().copied()),
            ProviderCost {
                input_per_million: cost,
                output_per_million: cost,
                cache_read_per_million: None,
            },
            Priority::Normal,
        )
    }

    fn mk(ids: &[(&str, f64)]) -> (ProviderRegistry, ProviderRouter) {
        let reg = ProviderRegistry::new();
        for (id, cost) in ids {
            reg.register_value(rec(id, &[Capability::Streaming], *cost))
                .unwrap();
        }
        (reg.clone(), ProviderRouter::new(reg, HealthManager::new()))
    }

    fn success(_: &ProviderId) -> ProviderRuntimeResult<Outcome> {
        Ok(Outcome::Success)
    }

    #[test]
    fn test_plan_deterministic_mode() {
        let (_, router) = mk(&[("a", 3.0), ("b", 1.0), ("c", 2.0)]);
        let f = Failover::new(router, FailoverPolicy::default());
        let plan = f.plan(&RouteRequest::new().with_capabilities(vec![Capability::Streaming]));
        assert_eq!(
            plan,
            vec![
                ProviderId::new("b"),
                ProviderId::new("c"),
                ProviderId::new("a")
            ]
        );
    }

    #[test]
    fn test_plan_ordered_mode() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0)]);
        let f = Failover::new(
            router,
            FailoverPolicy::ordered(vec![ProviderId::new("b"), ProviderId::new("a")]),
        );
        let plan = f.plan(&RouteRequest::new());
        assert_eq!(plan, vec![ProviderId::new("b"), ProviderId::new("a")]);
    }

    #[test]
    fn test_plan_ordered_truncates_to_max_attempts() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0), ("c", 1.0), ("d", 1.0)]);
        let f = Failover::new(
            router,
            FailoverPolicy::ordered(vec![
                ProviderId::new("a"),
                ProviderId::new("b"),
                ProviderId::new("c"),
                ProviderId::new("d"),
            ])
            .with_max_attempts(2),
        );
        let plan = f.plan(&RouteRequest::new());
        assert_eq!(plan.len(), 2);
    }

    #[test]
    fn test_execute_first_success() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0)]);
        let f = Failover::new(router, FailoverPolicy::default());
        let result = f.execute(&RouteRequest::new(), success).unwrap();
        assert_eq!(result.succeeded, Some(ProviderId::new("a")));
        assert_eq!(result.attempted.len(), 1);
        assert_eq!(result.attempted[0].outcome, Outcome::Success);
    }

    #[test]
    fn test_execute_fail_then_succeed() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0)]);
        let f = Failover::new(router, FailoverPolicy::default());
        let mut calls = 0;
        let result = f
            .execute(&RouteRequest::new(), |id| {
                calls += 1;
                if id.as_str() == "a" {
                    Ok(Outcome::Failure)
                } else {
                    Ok(Outcome::Success)
                }
            })
            .unwrap();
        assert_eq!(result.succeeded, Some(ProviderId::new("b")));
        assert_eq!(result.attempted.len(), 2);
        assert_eq!(calls, 2);
    }

    #[test]
    fn test_execute_all_fail_exhausted() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0)]);
        let f = Failover::new(router, FailoverPolicy::default());
        let result = f
            .execute(&RouteRequest::new(), |_| Ok(Outcome::Failure))
            .unwrap();
        assert!(result.succeeded.is_none());
        assert_eq!(result.attempted.len(), 2);
        assert!(result.exhausted_reason.is_some());
    }

    #[test]
    fn test_execute_capability_mismatch_triggers() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0)]);
        let f = Failover::new(router, FailoverPolicy::default());
        let result = f
            .execute(&RouteRequest::new(), |id| {
                if id.as_str() == "a" {
                    Err(ProviderRuntimeError::CapabilityMismatch {
                        requested: vec![],
                        available: vec![],
                    })
                } else {
                    Ok(Outcome::Success)
                }
            })
            .unwrap();
        assert!(result.capability_driven);
        assert_eq!(result.succeeded, Some(ProviderId::new("b")));
    }

    #[test]
    fn test_execute_empty_plan_errors() {
        let (reg, _) = mk(&[]);
        let router = ProviderRouter::new(reg, HealthManager::new());
        let f = Failover::new(router, FailoverPolicy::default());
        let err = f.execute(&RouteRequest::new(), success).unwrap_err();
        assert!(matches!(
            err,
            ProviderRuntimeError::FailoverExhausted { .. }
        ));
    }

    #[test]
    fn test_execute_respects_max_attempts() {
        let (_, router) = mk(&[("a", 1.0), ("b", 1.0), ("c", 1.0)]);
        let f = Failover::new(router, FailoverPolicy::default().with_max_attempts(2));
        let result = f
            .execute(&RouteRequest::new(), |_| Ok(Outcome::Failure))
            .unwrap();
        assert_eq!(result.attempted.len(), 2);
    }

    #[test]
    fn test_plan_satisfies_capabilities() {
        let reg = ProviderRegistry::new();
        reg.register_value(rec("a", &[Capability::Streaming, Capability::Vision], 1.0))
            .unwrap();
        reg.register_value(rec("b", &[Capability::Streaming], 1.0))
            .unwrap();
        let router = ProviderRouter::new(reg, HealthManager::new());
        let f = Failover::new(router, FailoverPolicy::default());
        assert!(!f.plan_satisfies_capabilities(&RouteRequest::new(), &[Capability::Vision],));
    }
}

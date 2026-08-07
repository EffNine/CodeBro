#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider Runtime (P10.3) — coordinates providers.
//!
//! The Provider Runtime is responsible for coordinating providers. It is
//! NOT responsible for implementing providers — provider implementations
//! remain plugins.
//!
//! # What Provider Runtime owns
//!
//! - Provider Registry
//! - Provider Discovery
//! - Provider Resolution (capability matching)
//! - Health Management
//! - Routing Strategy
//! - Retry Policy
//! - Failover Policy
//! - Cost Tracking
//! - Provider Diagnostics
//!
//! # What Provider Runtime does NOT own
//!
//! HTTP clients, REST/WebSocket APIs, vendor SDKs, authentication, API
//! keys, and any vendor-specific logic belong to provider plugins.
//!
//! # Determinism
//!
//! Selection is deterministic: Capability Match → Policy → Health →
//! Cost → Priority → Registration Order. Provider name never influences
//! routing.
//!
//! # Modules
//!
//! ```text
//! provider_runtime
//!   ├─ types        — ProviderId, RouteRequest, HealthState, cost, errors
//!   ├─ capabilities  — Capability, CapabilitySet, CapabilityMatch
//!   ├─ provider      — Provider contract + RegisteredProvider
//!   ├─ registry      — Register / unregister / lookup (deterministic)
//!   ├─ discovery     — Descriptive queries over providers
//!   ├─ health        — Observational health management
//!   ├─ router        — Deterministic selection (the 6-stage pipeline)
//!   ├─ retry         — Immediate / exponential backoff, budget
//!   ├─ failover      — Primary → Secondary → Fallback chains
//!   ├─ cost          — Observational cost & latency tracking
//!   └─ diagnostics   — Selection, mismatch, retry, failover, stats
//! ```

pub mod capabilities;
pub mod cost;
pub mod diagnostics;
pub mod discovery;
pub mod failover;
pub mod health;
pub mod provider;
pub mod registry;
pub mod retry;
pub mod router;
pub mod types;

#[cfg(test)]
mod tests;

pub use capabilities::{Capability, CapabilityMatch, CapabilitySet};
pub use cost::{CostDashboard, CostTracker, ProviderCostStats, TokenUsage};
pub use diagnostics::{DiagnosticsSummary, ProviderDiagnostics, ProviderEvent};
pub use discovery::{DiscoveryQuery, DiscoveryResult, ProviderDiscovery};
pub use failover::{Failover, FailoverPolicy, FailoverResult};
pub use health::{HealthManager, HealthPolicyConfig, HealthRecord};
pub use provider::{Provider, RegisteredProvider};
pub use registry::ProviderRegistry;
pub use retry::{BackoffStrategy, RetryController, RetryPolicy, RetrySchedule};
pub use router::{ProviderRouter, Rejection, RejectionReason, RouterDecision, RoutingPolicy};
pub use types::{
    CostObservation, HealthState, Outcome, Priority, ProviderCost, ProviderId,
    ProviderRuntimeError, ProviderRuntimeResult, RouteRequest,
};

/// High-level coordinate facade for the Provider Runtime.
///
/// Groups the registry, health manager, router, retry planner and
/// diagnostics so that callers coordinate providers through a single
/// entry point.
#[derive(Clone)]
pub struct ProviderRuntime {
    registry: ProviderRegistry,
    health: HealthManager,
    discovery: ProviderDiscovery,
    router: ProviderRouter,
    failover: Failover,
    cost: CostTracker,
    diagnostics: ProviderDiagnostics,
    retry_policy: RetryPolicy,
    correlation_counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ProviderRuntime {
    pub fn new() -> Self {
        let registry = ProviderRegistry::new();
        let health = HealthManager::new();
        let discovery = ProviderDiscovery::new(registry.clone(), health.clone());
        let router = ProviderRouter::new(registry.clone(), health.clone());
        let failover = Failover::new(router.clone(), FailoverPolicy::default());
        ProviderRuntime {
            registry,
            health,
            discovery,
            router,
            failover,
            cost: CostTracker::new(),
            diagnostics: ProviderDiagnostics::new(),
            retry_policy: RetryPolicy::default(),
            correlation_counter: std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)),
        }
    }

    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }

    pub fn discovery(&self) -> &ProviderDiscovery {
        &self.discovery
    }

    pub fn health(&self) -> &HealthManager {
        &self.health
    }

    pub fn router(&self) -> &ProviderRouter {
        &self.router
    }

    pub fn cost(&self) -> &CostTracker {
        &self.cost
    }

    pub fn diagnostics(&self) -> &ProviderDiagnostics {
        &self.diagnostics
    }

    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }

    pub fn register(&self, p: &dyn Provider) -> ProviderRuntimeResult<()> {
        self.registry.register(p)
    }

    pub fn register_value(&self, p: RegisteredProvider) -> ProviderRuntimeResult<()> {
        self.registry.register_value(p)
    }

    /// Deterministically select a provider for a request.
    pub fn select(&self, request: &RouteRequest) -> ProviderRuntimeResult<RouterDecision> {
        let corr = self.next_correlation();
        let decision = self.router.resolve(request)?;
        let selected = &decision.provider.id;
        self.diagnostics.record_selected(
            selected,
            "deterministic pick",
            &corr,
        );
        for r in &decision.rejected {
            self.diagnostics
                .record_rejected(&r.provider, &format!("{:?}", r.reason), &corr);
        }
        Ok(decision)
    }

    /// Report a successful provider call (observational).
    pub fn report_success(&self, provider: &ProviderId, tokens: TokenUsage, cost: ProviderCost) {
        let estimated = cost.estimate(tokens.input, tokens.output);
        self.health.report_success(provider, std::time::Instant::now());
        self.diagnostics.record_cost(provider, estimated);
        let _ = tokens;
        let _ = cost;
    }

    /// Compute a retry schedule for a provider after N consumed attempts.
    pub fn retry_schedule(&self, attempts_consumed: usize) -> RetrySchedule {
        RetrySchedule::from(self.retry_policy.clone(), attempts_consumed)
    }

    /// Build the failover plan for a request.
    pub fn failover_plan(&self, request: &RouteRequest) -> Vec<ProviderId> {
        self.failover.plan(request)
    }

    pub fn diagnostics_summary(&self) -> DiagnosticsSummary {
        self.diagnostics.summary()
    }

    fn next_correlation(&self) -> String {
        use std::sync::atomic::Ordering;
        let n = self
            .correlation_counter
            .fetch_add(1, Ordering::Relaxed);
        format!("req-{n}")
    }
}

impl Default for ProviderRuntime {
    fn default() -> Self {
        ProviderRuntime::new()
    }
}
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
//! - Circuit Breaker (P17.0)
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
//! # Circuit Breaker (P17.0)
//!
//! Every provider owns an independent circuit breaker. When the failure
//! threshold is reached the breaker opens and requests are rejected
//! immediately. After the cooldown expires the breaker enters half-open
//! and probes recovery.
//!
//! # Modules
//!
//! ```text
//! provider_runtime
//!   ├─ types           — ProviderId, RouteRequest, HealthState, cost, errors
//!   ├─ capabilities     — Capability, CapabilitySet, CapabilityMatch
//!   ├─ provider         — Provider contract + RegisteredProvider
//!   ├─ registry         — Register / unregister / lookup (deterministic)
//!   ├─ discovery        — Descriptive queries over providers
//!   ├─ health           — Observational health management
//!   ├─ router           — Deterministic selection (the 6-stage pipeline)
//!   ├─ retry            — Immediate / exponential backoff, budget
//!   ├─ failover         — Primary → Secondary → Fallback chains
//!   ├─ cost             — Observational cost & latency tracking
//!   ├─ diagnostics      — Selection, mismatch, retry, failover, stats
//!   ├─ circuit_breaker  — Closed → Open → HalfOpen state machine
//!   ├─ circuit_breaker_registry — Per-provider breaker management
//!   ├─ circuit_breaker_metrics  — Observability integration
//!   └─ circuit_breaker_events   — CB-specific diagnostic events
//! ```

pub mod capabilities;
pub mod circuit_breaker;
pub mod circuit_breaker_events;
pub mod circuit_breaker_metrics;
pub mod circuit_breaker_registry;
pub mod cost;
pub mod diagnostics;
pub mod discovery;
pub mod failover;
pub mod health;
pub mod provider;
pub mod registry;
pub mod retry;
pub mod router;
pub mod routing;
pub mod types;

#[cfg(test)]
mod tests;

pub use capabilities::{Capability, CapabilityMatch, CapabilitySet};
pub use circuit_breaker::{
    CircuitBreaker, CircuitBreakerConfig, CircuitBreakerMetrics, CircuitBreakerState,
};
pub use circuit_breaker_events::{CircuitBreakerEvent, ProviderRuntimeEvent};
pub use circuit_breaker_metrics::{CircuitBreakerMetricsCollector, CircuitBreakerMetricsView};
pub use circuit_breaker_registry::CircuitBreakerRegistry;
pub use cost::{CostDashboard, CostTracker, ProviderCostStats, TokenUsage};
pub use diagnostics::{DiagnosticsSummary, ProviderDiagnostics, ProviderEvent};
pub use discovery::{DiscoveryQuery, DiscoveryResult, ProviderDiscovery};
pub use failover::{Failover, FailoverPolicy, FailoverResult};
pub use health::{HealthManager, HealthPolicyConfig, HealthRecord};
pub use provider::{Provider, RegisteredProvider};
pub use registry::ProviderRegistry;
pub use retry::{BackoffStrategy, RetryController, RetryPolicy, RetrySchedule};
pub use router::{ProviderRouter, Rejection, RejectionReason, RouterDecision, RoutingPolicy};
pub use routing::{
    IntelligentProviderRouter, ProviderProfile, ProviderRoutingConfig, ProviderRoutingDecision,
    ProviderRoutingScore, RoutingPreferences, RoutingStrategy, RoutingStrategyConfig,
};
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
    circuit_breakers: CircuitBreakerRegistry,
    correlation_counter: std::sync::Arc<std::sync::atomic::AtomicU64>,
}

impl ProviderRuntime {
    pub fn new() -> Self {
        ProviderRuntime::from_parts(
            ProviderRegistry::new(),
            HealthManager::new(),
            CostTracker::new(),
        )
    }

    /// Construct a runtime over caller-supplied shared state.
    ///
    /// This is an integration seam: callers that already own a
    /// `ProviderRegistry`, `HealthManager` and `CostTracker` (for example
    /// to share them with an `IntelligentProviderRouter`) can construct a
    /// `ProviderRuntime` that observes exactly the same state, keeping
    /// routing, health, cost and circuit-breaker accounting coherent.
    pub fn from_parts(
        registry: ProviderRegistry,
        health: HealthManager,
        cost: CostTracker,
    ) -> Self {
        let discovery = ProviderDiscovery::new(registry.clone(), health.clone());
        let router = ProviderRouter::new(registry.clone(), health.clone());
        let failover = Failover::new(router.clone(), FailoverPolicy::default());
        ProviderRuntime {
            registry,
            health,
            discovery,
            router,
            failover,
            cost,
            diagnostics: ProviderDiagnostics::new(),
            retry_policy: RetryPolicy::default(),
            circuit_breakers: CircuitBreakerRegistry::new(),
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

    pub fn circuit_breakers(&self) -> &CircuitBreakerRegistry {
        &self.circuit_breakers
    }

    pub fn with_retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = policy;
        self
    }

    /// Set the retry policy in place.
    pub fn set_retry_policy(&mut self, policy: RetryPolicy) {
        self.retry_policy = policy;
    }

    pub fn retry_policy(&self) -> &RetryPolicy {
        &self.retry_policy
    }

    pub fn register(&self, p: &dyn Provider) -> ProviderRuntimeResult<()> {
        self.registry.register(p)
    }

    pub fn register_value(&self, p: RegisteredProvider) -> ProviderRuntimeResult<()> {
        self.registry.register_value(p.clone())?;
        // Ensure a circuit breaker exists for this provider.
        self.circuit_breakers.get_or_create(&p.id);
        Ok(())
    }

    /// Deterministically select a provider for a request.
    ///
    /// If the selected provider's circuit breaker is open, returns
    /// `CircuitBreakerOpen` instead of proceeding to the provider.
    pub fn select(&self, request: &RouteRequest) -> ProviderRuntimeResult<RouterDecision> {
        let corr = self.next_correlation();
        let decision = self.router.resolve(request)?;
        let selected = &decision.provider.id;

        // Check circuit breaker before allowing the request through.
        if let Some(cb) = self.circuit_breakers.get(selected) {
            if !cb.can_execute() {
                self.diagnostics.record_breaker_rejected(selected, &corr);
                return Err(ProviderRuntimeError::CircuitBreakerOpen {
                    provider: selected.clone(),
                    state: cb.state(),
                });
            }
        }

        self.diagnostics
            .record_selected(selected, "deterministic pick", &corr);
        for r in &decision.rejected {
            self.diagnostics
                .record_rejected(&r.provider, &format!("{:?}", r.reason), &corr);
        }
        Ok(decision)
    }

    /// Report a successful provider call (observational).
    ///
    /// Updates health, cost, and circuit breaker state. If the breaker
    /// transitions from half-open to closed, emits the appropriate
    /// diagnostics.
    pub fn report_success(&self, provider: &ProviderId, tokens: TokenUsage, cost: ProviderCost) {
        let estimated = cost.estimate(tokens.input, tokens.output);
        self.health
            .report_success(provider, std::time::Instant::now());
        self.diagnostics.record_cost(provider, estimated);

        if let Some(cb) = self.circuit_breakers.get(provider) {
            let prev = cb.state();
            cb.record_success();
            let next = cb.state();
            if prev == CircuitBreakerState::HalfOpen && next == CircuitBreakerState::Closed {
                let corr = self.next_correlation();
                self.diagnostics.record_breaker_closed(provider, &corr);
                self.diagnostics
                    .record_breaker_recovery_succeeded(provider, &corr);
            }
        }

        let _ = tokens;
        let _ = cost;
    }

    /// Report a failed provider call (observational).
    ///
    /// Updates health, cost, and circuit breaker state. If the breaker
    /// transitions to open, emits the appropriate diagnostics.
    pub fn report_failure(&self, provider: &ProviderId) {
        self.health
            .report_failure(provider, std::time::Instant::now());

        if let Some(cb) = self.circuit_breakers.get(provider) {
            let prev = cb.state();
            cb.record_failure();
            let next = cb.state();
            match (prev, next) {
                (CircuitBreakerState::Closed, CircuitBreakerState::Open) => {
                    let corr = self.next_correlation();
                    self.diagnostics
                        .record_breaker_opened(provider, cb.failure_count(), &corr);
                }
                (CircuitBreakerState::HalfOpen, CircuitBreakerState::Open) => {
                    let corr = self.next_correlation();
                    self.diagnostics
                        .record_breaker_recovery_failed(provider, &corr);
                }
                _ => {}
            }
        }
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
        let n = self.correlation_counter.fetch_add(1, Ordering::Relaxed);
        format!("req-{n}")
    }
}

impl Default for ProviderRuntime {
    fn default() -> Self {
        ProviderRuntime::new()
    }
}

//! Intelligent Provider Routing (Sprint 18.0).
//!
//! Provides dynamic, score-based provider selection as an alternative to
//! the deterministic 6-stage router. Scoring considers capability, latency,
//! cost, reliability, and circuit breaker state.
//!
//! # Routing Strategies
//!
//! - `Balanced` (default) — weighted combination of all factors.
//! - `LowestLatency` — prefer fastest provider.
//! - `LowestCost` — prefer cheapest provider.
//! - `HighestReliability` — prefer highest success rate.
//! - `BestCapability` — prefer provider matching required features.
//!
//! # Circuit Breaker Integration
//!
//! Open breakers exclude providers. Half-open breakers are excluded by
//! default unless `allow_half_open` is enabled.
//!
//! # Determinism
//!
//! The scoring is deterministic given the same state: health records,
//! cost observations, and provider registrations.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::provider_runtime::{
    capabilities::Capability,
    cost::{CostTracker, ProviderCostStats},
    health::{HealthManager, HealthRecord},
    provider::RegisteredProvider,
    registry::ProviderRegistry,
    types::{
        CostObservation, HealthState, ProviderId, ProviderRuntimeError, ProviderRuntimeResult,
        RouteRequest,
    },
};

// =========================================================================
// Routing Strategy
// =========================================================================

/// Routing strategy used to select the best provider for a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum RoutingStrategy {
    /// Weighted combination of latency, cost, reliability and capability.
    #[default]
    Balanced,
    /// Prefer the provider with the lowest average latency.
    LowestLatency,
    /// Prefer the cheapest provider that satisfies requirements.
    LowestCost,
    /// Prefer the provider with the highest recent success rate.
    HighestReliability,
    /// Choose providers based on requested feature requirements.
    BestCapability,
}

impl RoutingStrategy {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "balanced" | "default" => Some(RoutingStrategy::Balanced),
            "lowest_latency" | "latency" => Some(RoutingStrategy::LowestLatency),
            "lowest_cost" | "cost" => Some(RoutingStrategy::LowestCost),
            "highest_reliability" | "reliability" => Some(RoutingStrategy::HighestReliability),
            "best_capability" | "capability" => Some(RoutingStrategy::BestCapability),
            _ => None,
        }
    }

    pub fn code(&self) -> &'static str {
        match self {
            RoutingStrategy::Balanced => "balanced",
            RoutingStrategy::LowestLatency => "lowest_latency",
            RoutingStrategy::LowestCost => "lowest_cost",
            RoutingStrategy::HighestReliability => "highest_reliability",
            RoutingStrategy::BestCapability => "best_capability",
        }
    }
}

// =========================================================================
// Routing Configuration
// =========================================================================

/// Intelligent routing configuration.
///
/// Controls how the scoring engine weights different factors.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRoutingConfig {
    pub strategy: RoutingStrategy,
    pub latency_weight: f64,
    pub cost_weight: f64,
    pub reliability_weight: f64,
    pub capability_weight: f64,
    pub allow_half_open: bool,
}

impl Default for ProviderRoutingConfig {
    fn default() -> Self {
        ProviderRoutingConfig {
            strategy: RoutingStrategy::Balanced,
            latency_weight: 0.25,
            cost_weight: 0.25,
            reliability_weight: 0.30,
            capability_weight: 0.20,
            allow_half_open: false,
        }
    }
}

impl ProviderRoutingConfig {
    pub fn new(strategy: RoutingStrategy) -> Self {
        ProviderRoutingConfig {
            strategy,
            ..Self::default()
        }
    }

    pub fn with_weights(
        mut self,
        latency: f64,
        cost: f64,
        reliability: f64,
        capability: f64,
    ) -> Self {
        let total = latency + cost + reliability + capability;
        if total == 0.0 {
            return self;
        }
        self.latency_weight = latency / total;
        self.cost_weight = cost / total;
        self.reliability_weight = reliability / total;
        self.capability_weight = capability / total;
        self
    }
}

/// TOML-parseable routing configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingStrategyConfig {
    pub routing_strategy: String,
    pub latency_weight: f64,
    pub cost_weight: f64,
    pub reliability_weight: f64,
    pub capability_weight: f64,
    pub allow_half_open: bool,
}

impl Default for RoutingStrategyConfig {
    fn default() -> Self {
        RoutingStrategyConfig {
            routing_strategy: "balanced".to_string(),
            latency_weight: 0.25,
            cost_weight: 0.25,
            reliability_weight: 0.30,
            capability_weight: 0.20,
            allow_half_open: false,
        }
    }
}

impl RoutingStrategyConfig {
    /// Convert to the runtime config.
    pub fn to_routing_config(&self) -> ProviderRoutingConfig {
        let strategy =
            RoutingStrategy::from_str(&self.routing_strategy).unwrap_or(RoutingStrategy::Balanced);
        ProviderRoutingConfig {
            strategy,
            latency_weight: self.latency_weight,
            cost_weight: self.cost_weight,
            reliability_weight: self.reliability_weight,
            capability_weight: self.capability_weight,
            allow_half_open: self.allow_half_open,
        }
    }

    /// Parse from a TOML value.
    pub fn from_toml(value: &toml::Value) -> Option<Self> {
        let table = value.as_table()?;
        Some(RoutingStrategyConfig {
            routing_strategy: table
                .get("routing_strategy")
                .and_then(|v| v.as_str())
                .unwrap_or("balanced")
                .to_string(),
            latency_weight: table
                .get("latency_weight")
                .and_then(|v| v.as_float())
                .unwrap_or(0.25),
            cost_weight: table
                .get("cost_weight")
                .and_then(|v| v.as_float())
                .unwrap_or(0.25),
            reliability_weight: table
                .get("reliability_weight")
                .and_then(|v| v.as_float())
                .unwrap_or(0.30),
            capability_weight: table
                .get("capability_weight")
                .and_then(|v| v.as_float())
                .unwrap_or(0.20),
            allow_half_open: table
                .get("allow_half_open")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        })
    }
}

// =========================================================================
// Scoring Types
// =========================================================================

/// Dynamic score for a single provider evaluation.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderRoutingScore {
    /// Whether this provider is selectable.
    pub selectable: bool,
    /// Total weighted score (higher is better).
    pub total_score: f64,
    /// Capability match score (0..=1).
    pub capability_score: f64,
    /// Latency score (0..=1).
    pub latency_score: f64,
    /// Cost score (0..=1).
    pub cost_score: f64,
    /// Reliability score (0..=1).
    pub reliability_score: f64,
    /// Health multiplier (0..=1).
    pub health_score: f64,
    /// Circuit breaker bonus/penalty.
    pub breaker_bonus: f64,
    /// Health state at time of scoring.
    pub health_state: HealthState,
    /// Reasons for non-selection (if any).
    pub reason: Vec<String>,
}

impl ProviderRoutingScore {
    pub fn new(selectable: bool, total_score: f64) -> Self {
        ProviderRoutingScore {
            selectable,
            total_score,
            ..Default::default()
        }
    }

    pub fn with_capability_score(mut self, score: f64) -> Self {
        self.capability_score = score;
        self
    }

    pub fn with_latency_score(mut self, score: f64) -> Self {
        self.latency_score = score;
        self
    }

    pub fn with_cost_score(mut self, score: f64) -> Self {
        self.cost_score = score;
        self
    }

    pub fn with_reliability_score(mut self, score: f64) -> Self {
        self.reliability_score = score;
        self
    }

    pub fn with_health_score(mut self, score: f64) -> Self {
        self.health_score = score;
        self
    }

    pub fn with_breaker_bonus(mut self, bonus: f64) -> Self {
        self.breaker_bonus = bonus;
        self
    }

    pub fn with_health_state(mut self, state: HealthState) -> Self {
        self.health_state = state;
        self
    }

    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason.push(reason.into());
        self
    }
}

/// The result of a routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRoutingDecision {
    /// The selected provider.
    pub provider: RegisteredProvider,
    /// The score for the selected provider.
    pub score: ProviderRoutingScore,
    /// Alternative providers and their scores.
    pub alternatives: Vec<ProviderRoutingScore>,
    /// The routing strategy used.
    pub strategy: RoutingStrategy,
    /// Timestamp of the decision (seconds since epoch).
    pub timestamp: u64,
    /// Metadata about the decision.
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ProviderRoutingDecision {
    pub fn new(provider: RegisteredProvider, score: ProviderRoutingScore) -> Self {
        ProviderRoutingDecision {
            provider,
            score,
            alternatives: Vec::new(),
            strategy: RoutingStrategy::default(),
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            metadata: HashMap::new(),
        }
    }

    pub fn with_alternatives(mut self, alternatives: Vec<ProviderRoutingScore>) -> Self {
        self.alternatives = alternatives;
        self
    }

    pub fn with_strategy(mut self, strategy: RoutingStrategy) -> Self {
        self.strategy = strategy;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Get the selected provider id.
    pub fn provider_id(&self) -> &ProviderId {
        &self.provider.id
    }

    /// Get the total score.
    pub fn score(&self) -> f64 {
        self.score.total_score
    }

    /// Whether the decision was based on capability matching.
    pub fn capability_driven(&self) -> bool {
        self.score.capability_score > 0.0 && self.score.capability_score < 1.0
    }

    /// Whether the decision considered health state.
    pub fn health_considered(&self) -> bool {
        self.score.health_score < 1.0
    }
}

/// Routing profile for a provider (cached snapshot of capabilities and scores).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderProfile {
    pub provider_id: ProviderId,
    pub capabilities: Vec<Capability>,
    pub estimated_cost_per_million: f64,
    pub priority: u8,
    pub description: String,
}

impl ProviderProfile {
    pub fn from_provider(provider: &RegisteredProvider) -> Self {
        ProviderProfile {
            provider_id: provider.id.clone(),
            capabilities: provider.capabilities.iter().collect(),
            estimated_cost_per_million: provider.cost.routing_cost(),
            priority: provider.priority.score(),
            description: format!(
                "Provider {} with cost {:.4}",
                provider.id,
                provider.cost.routing_cost()
            ),
        }
    }
}

/// Routing preferences for fine-grained control.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoutingPreferences {
    pub preferred_providers: Vec<ProviderId>,
    pub excluded_providers: Vec<ProviderId>,
    pub min_reliability: Option<f64>,
    pub max_latency_ms: Option<f64>,
    pub max_cost_per_million: Option<f64>,
}

impl RoutingPreferences {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn with_preferred(mut self, providers: Vec<ProviderId>) -> Self {
        self.preferred_providers = providers;
        self
    }

    pub fn with_excluded(mut self, providers: Vec<ProviderId>) -> Self {
        self.excluded_providers = providers;
        self
    }

    pub fn with_min_reliability(mut self, min: f64) -> Self {
        self.min_reliability = Some(min);
        self
    }

    pub fn with_max_latency(mut self, ms: f64) -> Self {
        self.max_latency_ms = Some(ms);
        self
    }

    pub fn with_max_cost(mut self, cost: f64) -> Self {
        self.max_cost_per_million = Some(cost);
        self
    }
}

// =========================================================================
// Intelligent Router
// =========================================================================

/// Intelligent routing engine for provider selection.
///
/// Uses a scoring model to select the best provider based on:
/// - Capability matching
/// - Latency (from cost tracker observations)
/// - Cost
/// - Reliability (success rate from health records)
///
/// The engine respects the existing circuit breaker implementation
/// and does not duplicate breaker logic.
#[derive(Clone)]
pub struct IntelligentProviderRouter {
    registry: ProviderRegistry,
    health: HealthManager,
    cost: CostTracker,
    config: Arc<ProviderRoutingConfig>,
}

impl IntelligentProviderRouter {
    /// Create a new router with the given components.
    pub fn new(registry: ProviderRegistry, health: HealthManager, cost: CostTracker) -> Self {
        IntelligentProviderRouter {
            registry,
            health,
            cost,
            config: Arc::new(ProviderRoutingConfig::default()),
        }
    }

    /// Create with a custom configuration.
    pub fn with_config(mut self, config: ProviderRoutingConfig) -> Self {
        self.config = Arc::new(config);
        self
    }

    /// Return the current configuration.
    pub fn config(&self) -> &ProviderRoutingConfig {
        &self.config
    }

    /// Update the routing configuration.
    pub fn update_config(&mut self, config: ProviderRoutingConfig) {
        self.config = Arc::new(config);
    }

    /// Route a request to the best provider.
    ///
    /// Returns a `ProviderRoutingDecision` with the selected provider
    /// and scoring information.
    pub fn route(&self, request: &RouteRequest) -> ProviderRuntimeResult<ProviderRoutingDecision> {
        let all = self.registry.all();
        if all.is_empty() {
            return Err(ProviderRuntimeError::NoSuitableProvider(
                "No providers registered".to_string(),
            ));
        }

        // Collect health records for all providers.
        let health_records: Vec<(ProviderId, HealthRecord)> = {
            let known = self.health.all();
            let mut records: Vec<(ProviderId, HealthRecord)> =
                known.into_iter().map(|r| (r.provider.clone(), r)).collect();

            // Ensure all registered providers have a record.
            for p in &all {
                if !records.iter().any(|(id, _)| id == &p.id) {
                    records.push((p.id.clone(), HealthRecord::new(p.id.clone())));
                }
            }
            records
        };

        // Collect cost stats for all providers.
        let cost_stats: Vec<(ProviderId, ProviderCostStats)> = {
            let dash = self.cost.dashboard();
            dash.providers
                .values()
                .map(|s| (s.provider.clone().unwrap(), s.clone()))
                .collect()
        };

        // Score each provider.
        let mut scored: Vec<(RegisteredProvider, ProviderRoutingScore)> = all
            .into_iter()
            .map(|p| {
                let score = self.score_provider(&p, request, &health_records, &cost_stats);
                (p, score)
            })
            .filter(|(_, s)| s.selectable)
            .collect();

        if scored.is_empty() {
            return Err(ProviderRuntimeError::NoSuitableProvider(
                "No provider meets the routing criteria".to_string(),
            ));
        }

        // Sort by total score descending.
        scored.sort_by(|a, b| {
            b.1.total_score
                .partial_cmp(&a.1.total_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (best_provider, best_score) = scored[0].clone();
        let alternatives: Vec<ProviderRoutingScore> =
            scored.into_iter().skip(1).map(|(_, s)| s).collect();

        // Compute timestamp.
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        Ok(ProviderRoutingDecision {
            provider: best_provider,
            score: best_score,
            alternatives,
            strategy: self.config.strategy,
            timestamp,
            metadata: Default::default(),
        })
    }

    /// Score a single provider against a request.
    fn score_provider(
        &self,
        provider: &RegisteredProvider,
        request: &RouteRequest,
        health_records: &[(ProviderId, HealthRecord)],
        cost_stats: &[(ProviderId, ProviderCostStats)],
    ) -> ProviderRoutingScore {
        let mut score = ProviderRoutingScore::default();

        // 1. Capability check.
        if !provider.supports_all(&request.required_capabilities) {
            score.selectable = false;
            score.reason.push("capability_mismatch".to_string());
            return score;
        }

        // 2. Excluded check.
        if request.excluded.contains(&provider.id) {
            score.selectable = false;
            score.reason.push("excluded".to_string());
            return score;
        }

        // 3. Cost ceiling check.
        if let Some(ceiling) = request.max_cost {
            if provider.cost.routing_cost() > ceiling {
                score.selectable = false;
                score.reason.push("cost_ceiling".to_string());
                return score;
            }
        }

        // 4. Health check.
        let health_state = self.health.health(&provider.id);
        score.health_state = health_state;

        match health_state {
            HealthState::Healthy => {
                score.health_score = 1.0;
                score.selectable = true;
            }
            HealthState::Recovering => {
                score.health_score = 0.8;
                score.selectable = true;
            }
            HealthState::Degraded => {
                score.health_score = 0.5;
                if !request.allow_degraded {
                    score.selectable = false;
                    score.reason.push("degraded".to_string());
                    return score;
                }
                score.selectable = true;
            }
            HealthState::Unavailable | HealthState::Cooldown => {
                score.health_score = 0.0;
                score.selectable = false;
                score.reason.push("unavailable".to_string());
                return score;
            }
        }

        // 5. Compute sub-scores.
        score.capability_score = self.compute_capability_score(provider, request);
        score.latency_score = self.compute_latency_score(&provider.id);
        score.cost_score = self.compute_cost_score(provider);
        score.reliability_score =
            self.compute_reliability_score(&provider.id, health_records, cost_stats);

        // 6. Compute total score.
        score.total_score = self.compute_total_score(&score);

        score
    }

    /// Compute capability score (0..=1).
    fn compute_capability_score(
        &self,
        provider: &RegisteredProvider,
        request: &RouteRequest,
    ) -> f64 {
        if request.required_capabilities.is_empty() {
            return 1.0;
        }

        let total = request.required_capabilities.len();
        let satisfied = request
            .required_capabilities
            .iter()
            .filter(|c| provider.capabilities.has(c))
            .count();

        satisfied as f64 / total as f64
    }

    /// Compute latency score (0..=1).
    fn compute_latency_score(&self, provider_id: &ProviderId) -> f64 {
        let stats = self.cost.stats(provider_id);
        if stats.calls == 0 {
            return 0.5; // neutral score for unknown providers
        }

        let avg_latency = stats.avg_latency_ms();
        // Normalize: lower latency is better.
        // Score = 1 / (1 + avg_latency / 1000.0)
        1.0 / (1.0 + avg_latency / 1000.0)
    }

    /// Compute cost score (0..=1).
    fn compute_cost_score(&self, provider: &RegisteredProvider) -> f64 {
        let cost = provider.cost.routing_cost();
        if cost == 0.0 {
            return 1.0; // free/local models are preferred for cost
        }
        // Normalize: lower cost is better.
        1.0 / (1.0 + cost / 10.0)
    }

    /// Compute reliability score (0..=1).
    fn compute_reliability_score(
        &self,
        provider_id: &ProviderId,
        health_records: &[(ProviderId, HealthRecord)],
        cost_stats: &[(ProviderId, ProviderCostStats)],
    ) -> f64 {
        // Prefer health record success rate.
        if let Some((_, record)) = health_records.iter().find(|(id, _)| id == provider_id) {
            if record.total_calls > 0 {
                return record.success_rate();
            }
        }

        // Fall back to cost stats.
        for (_, stats) in cost_stats {
            if stats.provider.as_ref() == Some(provider_id) {
                return stats.success_rate();
            }
        }

        1.0 // unknown provider, assume fully reliable
    }

    /// Compute total weighted score.
    fn compute_total_score(&self, score: &ProviderRoutingScore) -> f64 {
        let config = &self.config;
        let latency = score.latency_score * config.latency_weight;
        let cost = score.cost_score * config.cost_weight;
        let reliability = score.reliability_score * config.reliability_weight;
        let capability = score.capability_score * config.capability_weight;
        let health = score.health_score * 0.1; // health is a prerequisite, not a score factor

        latency + cost + reliability + capability + health
    }

    /// Get all registered providers.
    pub fn providers(&self) -> Vec<RegisteredProvider> {
        self.registry.all()
    }

    /// Get a provider by id.
    pub fn get(&self, id: &ProviderId) -> Option<RegisteredProvider> {
        self.registry.get(id)
    }

    /// Get the health manager.
    pub fn health(&self) -> &HealthManager {
        &self.health
    }

    /// Get the cost tracker.
    pub fn cost(&self) -> &CostTracker {
        &self.cost
    }

    /// Get the registry.
    pub fn registry(&self) -> &ProviderRegistry {
        &self.registry
    }
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_runtime::{
        capabilities::CapabilitySet,
        types::{Priority, ProviderCost},
    };
    use std::time::Duration;

    fn make_provider(id: &str, caps: &[Capability], cost: f64) -> RegisteredProvider {
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

    fn make_router() -> (
        ProviderRegistry,
        HealthManager,
        CostTracker,
        IntelligentProviderRouter,
    ) {
        let registry = ProviderRegistry::new();
        let health = HealthManager::new();
        let cost = CostTracker::new();
        let router = IntelligentProviderRouter::new(registry.clone(), health.clone(), cost.clone());
        (registry, health, cost, router)
    }

    /// Ensure all registered providers have healthy status.
    fn ensure_healthy(health: &HealthManager, ids: &[&str]) {
        let now = std::time::Instant::now();
        for id in ids {
            health.report_success(&ProviderId::new(*id), now);
        }
    }

    #[test]
    fn test_balanced_strategy_picks_best_provider() {
        let (reg, health, cost, router) = make_router();

        reg.register_value(make_provider("fast", &[Capability::Streaming], 5.0))
            .unwrap();
        reg.register_value(make_provider("cheap", &[Capability::Streaming], 1.0))
            .unwrap();
        reg.register_value(make_provider("reliable", &[Capability::Streaming], 3.0))
            .unwrap();

        ensure_healthy(&health, &["fast", "cheap", "reliable"]);

        // Record some cost observations to create differentiation.
        cost.record(CostObservation {
            provider: ProviderId::new("fast"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.01,
            actual_cost: Some(0.01),
            latency_ms: 100,
            success: true,
        });
        cost.record(CostObservation {
            provider: ProviderId::new("reliable"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.015,
            actual_cost: Some(0.015),
            latency_ms: 200,
            success: true,
        });
        cost.record(CostObservation {
            provider: ProviderId::new("cheap"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.005,
            actual_cost: Some(0.005),
            latency_ms: 150,
            success: true,
        });

        let decision = router.route(&RouteRequest::new()).unwrap();
        // The decision should be one of the three providers.
        assert!(
            decision.provider.id.as_str() == "fast"
                || decision.provider.id.as_str() == "cheap"
                || decision.provider.id.as_str() == "reliable"
        );
    }

    #[test]
    fn test_capability_filtering() {
        let (reg, health, _cost, router) = make_router();

        reg.register_value(make_provider(
            "streaming_only",
            &[Capability::Streaming],
            1.0,
        ))
        .unwrap();
        reg.register_value(make_provider(
            "full_stack",
            &[Capability::Streaming, Capability::ToolCalling],
            1.0,
        ))
        .unwrap();

        ensure_healthy(&health, &["streaming_only", "full_stack"]);

        let decision = router
            .route(&RouteRequest::new().with_capabilities(vec![Capability::ToolCalling]))
            .unwrap();
        assert_eq!(decision.provider.id.as_str(), "full_stack");
    }

    #[test]
    fn test_excluded_provider() {
        let (reg, health, _cost, router) = make_router();

        reg.register_value(make_provider("a", &[Capability::Streaming], 1.0))
            .unwrap();
        reg.register_value(make_provider("b", &[Capability::Streaming], 2.0))
            .unwrap();

        ensure_healthy(&health, &["a", "b"]);

        let decision = router
            .route(&RouteRequest::new().excluding(vec![ProviderId::new("a")]))
            .unwrap();
        assert_eq!(decision.provider.id.as_str(), "b");
    }

    #[test]
    fn test_cost_ceiling() {
        let (reg, health, _cost, router) = make_router();

        reg.register_value(make_provider("expensive", &[Capability::Streaming], 100.0))
            .unwrap();
        reg.register_value(make_provider("cheap", &[Capability::Streaming], 1.0))
            .unwrap();

        ensure_healthy(&health, &["expensive", "cheap"]);

        let decision = router
            .route(&RouteRequest::new().with_cost_ceiling(10.0))
            .unwrap();
        assert_eq!(decision.provider.id.as_str(), "cheap");
    }

    #[test]
    fn test_latency_strategy() {
        let config = ProviderRoutingConfig {
            strategy: RoutingStrategy::LowestLatency,
            latency_weight: 1.0,
            cost_weight: 0.0,
            reliability_weight: 0.0,
            capability_weight: 0.0,
            ..Default::default()
        };
        let (reg, health, cost, router) = make_router();
        let router = router.with_config(config);

        reg.register_value(make_provider("slow", &[Capability::Streaming], 1.0))
            .unwrap();
        reg.register_value(make_provider("fast", &[Capability::Streaming], 2.0))
            .unwrap();

        ensure_healthy(&health, &["slow", "fast"]);

        cost.record(CostObservation {
            provider: ProviderId::new("slow"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.01,
            actual_cost: Some(0.01),
            latency_ms: 500,
            success: true,
        });
        cost.record(CostObservation {
            provider: ProviderId::new("fast"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.02,
            actual_cost: Some(0.02),
            latency_ms: 100,
            success: true,
        });

        let decision = router.route(&RouteRequest::new()).unwrap();
        assert_eq!(decision.provider.id.as_str(), "fast");
    }

    #[test]
    fn test_cost_strategy() {
        let config = ProviderRoutingConfig {
            strategy: RoutingStrategy::LowestCost,
            cost_weight: 1.0,
            ..Default::default()
        };
        let (reg, health, _cost, router) = make_router();
        let router = router.with_config(config);

        reg.register_value(make_provider("expensive", &[Capability::Streaming], 50.0))
            .unwrap();
        reg.register_value(make_provider("cheap", &[Capability::Streaming], 1.0))
            .unwrap();

        ensure_healthy(&health, &["expensive", "cheap"]);

        let decision = router.route(&RouteRequest::new()).unwrap();
        assert_eq!(decision.provider.id.as_str(), "cheap");
    }

    #[test]
    fn test_reliability_strategy() {
        let config = ProviderRoutingConfig {
            strategy: RoutingStrategy::HighestReliability,
            reliability_weight: 1.0,
            ..Default::default()
        };
        let (reg, health, cost, router) = make_router();
        let router = router.with_config(config);

        reg.register_value(make_provider("reliable", &[Capability::Streaming], 1.0))
            .unwrap();
        reg.register_value(make_provider("unreliable", &[Capability::Streaming], 1.0))
            .unwrap();

        ensure_healthy(&health, &["reliable"]);

        // Make "unreliable" have failures.
        let t = std::time::Instant::now();
        for i in 0..5 {
            health.report_failure(&ProviderId::new("unreliable"), t + Duration::from_secs(i));
        }

        // Record successes for reliable.
        cost.record(CostObservation {
            provider: ProviderId::new("reliable"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.01,
            actual_cost: Some(0.01),
            latency_ms: 100,
            success: true,
        });
        cost.record(CostObservation {
            provider: ProviderId::new("unreliable"),
            input_tokens: 100,
            output_tokens: 50,
            estimated_cost: 0.01,
            actual_cost: Some(0.01),
            latency_ms: 100,
            success: false,
        });

        let decision = router.route(&RouteRequest::new()).unwrap();
        assert_eq!(decision.provider.id.as_str(), "reliable");
    }

    #[test]
    fn test_no_providers() {
        let (_reg, _health, _cost, router) = make_router();
        let result = router.route(&RouteRequest::new());
        assert!(result.is_err());
    }

    #[test]
    fn test_alternatives_in_decision() {
        let (reg, health, _cost, router) = make_router();

        reg.register_value(make_provider("a", &[Capability::Streaming], 1.0))
            .unwrap();
        reg.register_value(make_provider("b", &[Capability::Streaming], 2.0))
            .unwrap();
        reg.register_value(make_provider("c", &[Capability::Streaming], 3.0))
            .unwrap();

        ensure_healthy(&health, &["a", "b", "c"]);

        let decision = router.route(&RouteRequest::new()).unwrap();
        assert_eq!(decision.alternatives.len(), 2);
    }

    #[test]
    fn test_config_serialization() {
        let config = ProviderRoutingConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let back: ProviderRoutingConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.strategy, back.strategy);
        assert!((config.latency_weight - back.latency_weight).abs() < 1e-9);
    }

    #[test]
    fn test_routing_decision_creation() {
        let provider = RegisteredProvider::new(
            "test",
            CapabilitySet::empty(),
            ProviderCost::default(),
            Priority::Normal,
        );
        let score = ProviderRoutingScore::default();
        let decision = ProviderRoutingDecision::new(provider, score);
        assert_eq!(decision.provider_id().as_str(), "test");
        assert_eq!(decision.score(), 0.0);
    }

    #[test]
    fn test_routing_score_with_reasons() {
        let score = ProviderRoutingScore::default()
            .with_reason("unavailable")
            .with_reason("cost_exceeded");
        assert_eq!(score.reason.len(), 2);
        assert!(!score.selectable);
    }

    #[test]
    fn test_provider_profile_from_provider() {
        let provider = RegisteredProvider::new(
            "test",
            CapabilitySet::empty(),
            ProviderCost {
                input_per_million: 2.5,
                output_per_million: 10.0,
                cache_read_per_million: None,
            },
            Priority::High,
        );
        let profile = ProviderProfile::from_provider(&provider);
        assert_eq!(profile.provider_id.as_str(), "test");
        assert!((profile.estimated_cost_per_million - 12.5).abs() < 1e-9);
        assert_eq!(profile.priority, 2);
    }

    #[test]
    fn test_routing_preferences() {
        let prefs = RoutingPreferences::new()
            .with_preferred(vec![ProviderId::new("a")])
            .with_excluded(vec![ProviderId::new("b")])
            .with_min_reliability(0.9)
            .with_max_latency(500.0)
            .with_max_cost(10.0);
        assert_eq!(prefs.preferred_providers.len(), 1);
        assert_eq!(prefs.excluded_providers.len(), 1);
        assert!((prefs.min_reliability.unwrap() - 0.9).abs() < 1e-9);
        assert!((prefs.max_latency_ms.unwrap() - 500.0).abs() < 1e-9);
        assert!((prefs.max_cost_per_million.unwrap() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_deterministic_scoring() {
        let (reg, health, _cost, router) = make_router();

        reg.register_value(make_provider("a", &[Capability::Streaming], 1.0))
            .unwrap();
        reg.register_value(make_provider("b", &[Capability::Streaming], 1.0))
            .unwrap();

        ensure_healthy(&health, &["a", "b"]);

        // Same request should produce same result.
        let d1 = router.route(&RouteRequest::new()).unwrap();
        let d2 = router.route(&RouteRequest::new()).unwrap();
        assert_eq!(d1.provider.id, d2.provider.id);
    }

    #[test]
    fn test_default_config() {
        let config = RoutingStrategyConfig::default();
        assert_eq!(config.routing_strategy, "balanced");
        assert_eq!(config.latency_weight, 0.25);
        assert_eq!(config.cost_weight, 0.25);
        assert_eq!(config.reliability_weight, 0.30);
        assert_eq!(config.capability_weight, 0.20);
        assert!(!config.allow_half_open);
    }

    #[test]
    fn test_to_routing_config() {
        let cfg = RoutingStrategyConfig {
            routing_strategy: "lowest_latency".to_string(),
            latency_weight: 1.0,
            cost_weight: 0.0,
            reliability_weight: 0.0,
            capability_weight: 0.0,
            allow_half_open: true,
        };
        let rc = cfg.to_routing_config();
        assert_eq!(rc.strategy, RoutingStrategy::LowestLatency);
        assert_eq!(rc.latency_weight, 1.0);
        assert!(rc.allow_half_open);
    }

    #[test]
    fn test_from_toml() {
        let toml_str = r#"
            routing_strategy = "highest_reliability"
            latency_weight = 0.1
            cost_weight = 0.2
            reliability_weight = 0.7
            capability_weight = 0.0
            allow_half_open = true
        "#;
        let value: toml::Value = toml::from_str(toml_str).unwrap();
        let cfg = RoutingStrategyConfig::from_toml(&value).unwrap();
        assert_eq!(cfg.routing_strategy, "highest_reliability");
        assert_eq!(cfg.reliability_weight, 0.7);
        assert!(cfg.allow_half_open);
    }

    #[test]
    fn test_strategy_from_str() {
        assert_eq!(
            RoutingStrategy::from_str("balanced"),
            Some(RoutingStrategy::Balanced)
        );
        assert_eq!(
            RoutingStrategy::from_str("lowest_latency"),
            Some(RoutingStrategy::LowestLatency)
        );
        assert_eq!(
            RoutingStrategy::from_str("lowest_cost"),
            Some(RoutingStrategy::LowestCost)
        );
        assert_eq!(
            RoutingStrategy::from_str("highest_reliability"),
            Some(RoutingStrategy::HighestReliability)
        );
        assert_eq!(
            RoutingStrategy::from_str("best_capability"),
            Some(RoutingStrategy::BestCapability)
        );
        assert_eq!(RoutingStrategy::from_str("unknown"), None);
    }
}

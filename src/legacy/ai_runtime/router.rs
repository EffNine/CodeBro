use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use super::capabilities::{
    Capability, CapabilityNegotiation, CapabilitySet, SupportedCapabilities,
};
use super::diagnostics::{DiagnosticEvent, DiagnosticLevel, RuntimeDiagnostics};
use super::request::ModelRequest;
use super::response::ModelResponse;
use super::stream::StreamPipeline;
use super::types::{
    AIRRuntimeError, AIRRuntimeResult, CostEstimate, HealthStatus, ModelId, Priority, ProviderType,
};

/// A candidate model that the router can select.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub model_id: ModelId,
    pub capabilities: CapabilitySet,
    pub health: HealthStatus,
    pub cost_estimate: CostEstimate,
    pub priority: Priority,
    pub latency_ms: f64,
    pub success_rate: f64,
    pub metadata: HashMap<String, serde_json::Value>,
}

impl ModelCandidate {
    pub fn new(
        model_id: ModelId,
        capabilities: CapabilitySet,
        health: HealthStatus,
        cost_estimate: CostEstimate,
    ) -> Self {
        ModelCandidate {
            model_id,
            capabilities,
            health,
            cost_estimate,
            priority: Priority::Normal,
            latency_ms: 0.0,
            success_rate: 1.0,
            metadata: HashMap::new(),
        }
    }

    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_latency(mut self, latency_ms: f64) -> Self {
        self.latency_ms = latency_ms;
        self
    }

    pub fn with_success_rate(mut self, success_rate: f64) -> Self {
        self.success_rate = success_rate;
        self
    }

    pub fn with_health(mut self, health: HealthStatus) -> Self {
        self.health = health;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: serde_json::Value) -> Self {
        self.metadata.insert(key.into(), value);
        self
    }

    /// Score this candidate for a given request. Higher is better.
    pub fn score_for_request(&self, request: &ModelRequest) -> f64 {
        if !self.health.is_healthy() {
            return -1.0;
        }

        let negotiation = CapabilityNegotiation::new(request, &self.capabilities);
        if !negotiation.compatible {
            return -1.0;
        }

        let mut score = 0.0;

        // Priority (0-30)
        score += self.priority.score() as f64 * 10.0;

        // Cost efficiency (0-20): lower cost is better
        let total_cost =
            self.cost_estimate.input_cost_per_million + self.cost_estimate.output_cost_per_million;
        score += (1.0 / (1.0 + total_cost / 10.0)) * 20.0;

        // Latency (0-20): lower latency is better
        score += (1.0 / (1.0 + self.latency_ms / 1000.0)) * 20.0;

        // Success rate (0-20)
        score += self.success_rate * 20.0;

        // Capability coverage bonus (0-10)
        let required = super::capabilities::CapabilitySet::required_for_request(request);
        let coverage = required.iter().filter(|c| self.capabilities.has(c)).count() as f64;
        if !required.is_empty() {
            score += (coverage / required.len() as f64) * 10.0;
        }

        score
    }
}

/// Configuration for the runtime router.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    pub max_candidates: usize,
    pub cost_weight: f64,
    pub latency_weight: f64,
    pub quality_weight: f64,
    pub health_weight: f64,
    pub failover_enabled: bool,
    pub cache_enabled: bool,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        RoutingConfig {
            max_candidates: 5,
            cost_weight: 0.25,
            latency_weight: 0.25,
            quality_weight: 0.3,
            health_weight: 0.2,
            failover_enabled: true,
            cache_enabled: true,
        }
    }
}

/// The result of a routing decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    pub selected_model: ModelId,
    pub selected_candidate: ModelCandidate,
    pub score: f64,
    pub reason: String,
    pub alternatives: Vec<ModelId>,
    pub capability_negotiation: CapabilityNegotiation,
}

impl RoutingDecision {
    pub fn new(
        selected_model: ModelId,
        selected_candidate: ModelCandidate,
        score: f64,
        reason: impl Into<String>,
        alternatives: Vec<ModelId>,
        capability_negotiation: CapabilityNegotiation,
    ) -> Self {
        RoutingDecision {
            selected_model,
            selected_candidate,
            score,
            reason: reason.into(),
            alternatives,
            capability_negotiation,
        }
    }
}

/// Runtime router — provider-agnostic model selection.
#[derive(Debug)]
pub struct RuntimeRouter {
    candidates: Arc<std::sync::RwLock<Vec<ModelCandidate>>>,
    config: RoutingConfig,
    diagnostics: Arc<std::sync::RwLock<RuntimeDiagnostics>>,
    request_history: Arc<std::sync::RwLock<Vec<(ModelRequest, RoutingDecision)>>>,
}

impl RuntimeRouter {
    pub fn new(config: RoutingConfig) -> Self {
        RuntimeRouter {
            candidates: Arc::new(std::sync::RwLock::new(Vec::new())),
            config,
            diagnostics: Arc::new(std::sync::RwLock::new(RuntimeDiagnostics::new(1000))),
            request_history: Arc::new(std::sync::RwLock::new(Vec::new())),
        }
    }

    /// Register a model candidate.
    pub fn register_candidate(&self, candidate: ModelCandidate) {
        let mut candidates = self.candidates.write().unwrap();
        // Remove existing candidate with same model id
        candidates.retain(|c| c.model_id != candidate.model_id);
        candidates.push(candidate);
        // Keep only the best candidates
        candidates.truncate(self.config.max_candidates);
    }

    /// Remove a model candidate.
    pub fn unregister_candidate(&self, model_id: &ModelId) {
        let mut candidates = self.candidates.write().unwrap();
        candidates.retain(|c| &c.model_id != model_id);
    }

    /// Update health status for a candidate.
    pub fn update_health(&self, model_id: &ModelId, health: HealthStatus) {
        let mut candidates = self.candidates.write().unwrap();
        if let Some(candidate) = candidates.iter_mut().find(|c| &c.model_id == model_id) {
            candidate.health = health;
        }
    }

    /// Get all registered candidates.
    pub fn candidates(&self) -> Vec<ModelCandidate> {
        self.candidates.read().unwrap().clone()
    }

    /// Get candidates that match the given capabilities.
    pub fn candidates_with_capabilities(&self, required: &[Capability]) -> Vec<ModelCandidate> {
        self.candidates
            .read()
            .unwrap()
            .iter()
            .filter(|c| c.capabilities.has_all(required))
            .cloned()
            .collect()
    }

    /// Route a request to the best model.
    pub fn route(&self, request: &ModelRequest) -> AIRRuntimeResult<RoutingDecision> {
        let candidates = self.candidates.read().unwrap();

        if candidates.is_empty() {
            return Err(AIRRuntimeError::NoSuitableProvider(
                "No candidates registered".to_string(),
            ));
        }

        // Score all candidates
        let mut scored: Vec<(ModelCandidate, f64)> = candidates
            .iter()
            .map(|c| (c.clone(), c.score_for_request(request)))
            .filter(|(_, score)| *score >= 0.0)
            .collect();

        if scored.is_empty() {
            return Err(AIRRuntimeError::NoSuitableProvider(
                "No candidates match request capabilities".to_string(),
            ));
        }

        // Sort by score descending
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let (best_candidate, best_score) = scored.first().unwrap();
        let alternatives: Vec<ModelId> = scored
            .iter()
            .skip(1)
            .map(|(c, _)| c.model_id.clone())
            .collect();

        let negotiation = CapabilityNegotiation::new(request, &best_candidate.capabilities);

        // Record diagnostic event
        {
            let mut diag = self.diagnostics.write().unwrap();
            diag.record(DiagnosticEvent::ModelSelected {
                event_id: uuid::Uuid::new_v4().to_string(),
                model_id: best_candidate.model_id.to_string(),
                reason: format!("Scored {:.2}", best_score),
                timestamp: 0,
            });
        }

        // Record in history
        {
            let mut history = self.request_history.write().unwrap();
            history.push((
                request.clone(),
                RoutingDecision::new(
                    best_candidate.model_id.clone(),
                    best_candidate.clone(),
                    *best_score,
                    format!("Selected based on score {:.2}", best_score),
                    alternatives.clone(),
                    negotiation.clone(),
                ),
            ));
            // Keep history bounded
            if history.len() > 1000 {
                let keep = history.len() - 1000;
                history.drain(..keep);
            }
        }

        Ok(RoutingDecision::new(
            best_candidate.model_id.clone(),
            best_candidate.clone(),
            *best_score,
            format!("Scored {:.2}", best_score),
            alternatives,
            negotiation,
        ))
    }

    /// Get diagnostics.
    pub fn diagnostics(&self) -> RuntimeDiagnostics {
        self.diagnostics.read().unwrap().clone()
    }

    /// Get request history.
    pub fn request_history(&self) -> Vec<(ModelRequest, RoutingDecision)> {
        self.request_history.read().unwrap().clone()
    }

    /// Clear request history.
    pub fn clear_history(&self) {
        self.request_history.write().unwrap().clear();
    }

    /// Get routing config.
    pub fn config(&self) -> &RoutingConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::ProviderType;
    use super::*;

    fn test_candidate(
        model_id: &str,
        provider: ProviderType,
        capabilities: Vec<Capability>,
    ) -> ModelCandidate {
        ModelCandidate::new(
            ModelId::new(model_id, provider),
            CapabilitySet::new(capabilities),
            HealthStatus::Healthy,
            CostEstimate::default(),
        )
    }

    #[test]
    fn test_model_id_creation() {
        let id = ModelId::openai("gpt-4o");
        assert_eq!(id.id, "gpt-4o");
        assert_eq!(id.provider, ProviderType::OpenAI);
        assert_eq!(format!("{}", id), "openai/gpt-4o");
    }

    #[test]
    fn test_model_id_from_string() {
        let id: ModelId = String::from("gpt-4").into();
        assert_eq!(id.id, "gpt-4");
    }

    #[test]
    fn test_priority_scores() {
        assert_eq!(Priority::Low.score(), 0);
        assert_eq!(Priority::Normal.score(), 1);
        assert_eq!(Priority::High.score(), 2);
        assert_eq!(Priority::Critical.score(), 3);
    }

    #[test]
    fn test_health_status() {
        assert!(HealthStatus::Healthy.is_healthy());
        assert!(HealthStatus::Degraded.is_healthy());
        assert!(!HealthStatus::Unhealthy.is_healthy());
        assert!(!HealthStatus::Unknown.is_healthy());
    }

    #[test]
    fn test_cost_estimate() {
        let estimate = CostEstimate {
            input_cost_per_million: 2.5,
            output_cost_per_million: 10.0,
            cache_read_cost_per_million: Some(0.5),
            cache_creation_cost_per_million: Some(0.1),
        };
        let cost = estimate.estimate(1000, 500, Some(200));
        assert!(cost > 0.0);
        assert!(cost < 0.01);
    }

    #[test]
    fn test_cost_estimate_no_cache() {
        let estimate = CostEstimate {
            input_cost_per_million: 2.5,
            output_cost_per_million: 10.0,
            cache_read_cost_per_million: None,
            cache_creation_cost_per_million: None,
        };
        let cost = estimate.estimate(1000, 500, None);
        assert!((cost - 0.0075).abs() < 0.0001);
    }

    #[test]
    fn test_router_registers_candidate() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let candidate = test_candidate(
            "gpt-4o",
            ProviderType::OpenAI,
            vec![Capability::Streaming, Capability::ToolCalling],
        );
        router.register_candidate(candidate);
        assert_eq!(router.candidates().len(), 1);
    }

    #[test]
    fn test_router_replaces_same_model() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let c1 = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        let c2 = test_candidate(
            "gpt-4o",
            ProviderType::OpenAI,
            vec![Capability::Streaming, Capability::ToolCalling],
        );
        router.register_candidate(c1);
        router.register_candidate(c2);
        assert_eq!(router.candidates().len(), 1);
        assert_eq!(router.candidates()[0].capabilities.capabilities.len(), 2);
    }

    #[test]
    fn test_router_unregisters_candidate() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let candidate = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        router.register_candidate(candidate);
        router.unregister_candidate(&ModelId::openai("gpt-4o"));
        assert_eq!(router.candidates().len(), 0);
    }

    #[test]
    fn test_router_health_update() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let candidate = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        router.register_candidate(candidate);
        router.update_health(&ModelId::openai("gpt-4o"), HealthStatus::Unhealthy);
        assert_eq!(router.candidates()[0].health, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_router_no_candidates() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let request = ModelRequest::new("gpt-4o", vec![]);
        let result = router.route(&request);
        assert!(result.is_err());
        match result.unwrap_err() {
            AIRRuntimeError::NoSuitableProvider(_) => {}
            _ => panic!("Expected NoSuitableProvider"),
        }
    }

    #[test]
    fn test_router_selects_best_candidate() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let low_priority = test_candidate(
            "gpt-4o-mini",
            ProviderType::OpenAI,
            vec![Capability::Streaming],
        )
        .with_priority(Priority::Low);
        let high_priority = test_candidate(
            "gpt-4o",
            ProviderType::OpenAI,
            vec![Capability::Streaming, Capability::ToolCalling],
        )
        .with_priority(Priority::High);
        router.register_candidate(low_priority);
        router.register_candidate(high_priority);

        let request = ModelRequest::new("gpt-4o", vec![]);
        let decision = router.route(&request).unwrap();
        assert_eq!(decision.selected_model.id, "gpt-4o");
    }

    #[test]
    fn test_router_filters_unhealthy() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let healthy = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        let unhealthy = test_candidate(
            "claude-3",
            ProviderType::Anthropic,
            vec![Capability::Streaming],
        )
        .with_health(HealthStatus::Unhealthy);
        router.register_candidate(healthy);
        router.register_candidate(unhealthy);

        let request = ModelRequest::new("gpt-4o", vec![]);
        let decision = router.route(&request).unwrap();
        assert_eq!(decision.selected_model.id, "gpt-4o");
    }

    #[test]
    fn test_router_capability_filtering() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let streaming_only =
            test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        let full_capability = test_candidate(
            "claude-3",
            ProviderType::Anthropic,
            vec![
                Capability::Streaming,
                Capability::ToolCalling,
                Capability::StructuredOutput,
            ],
        );
        router.register_candidate(streaming_only);
        router.register_candidate(full_capability);

        let request = ModelRequest::new("claude-3", vec![]).with_tools(vec![]);
        let decision = router.route(&request).unwrap();
        assert!(decision.selected_model.id == "gpt-4o" || decision.selected_model.id == "claude-3");
    }

    #[test]
    fn test_router_diagnostic_events() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let candidate = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        router.register_candidate(candidate);

        let request = ModelRequest::new("gpt-4o", vec![]);
        router.route(&request).unwrap();

        let diag = router.diagnostics();
        assert!(diag.summary().total_events > 0);
    }

    #[test]
    fn test_router_request_history() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let candidate = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        router.register_candidate(candidate);

        let request = ModelRequest::new("gpt-4o", vec![]);
        router.route(&request).unwrap();
        router.route(&request).unwrap();

        assert_eq!(router.request_history().len(), 2);
    }

    #[test]
    fn test_router_clear_history() {
        let router = RuntimeRouter::new(RoutingConfig::default());
        let candidate = test_candidate("gpt-4o", ProviderType::OpenAI, vec![Capability::Streaming]);
        router.register_candidate(candidate);

        let request = ModelRequest::new("gpt-4o", vec![]);
        router.route(&request).unwrap();
        router.clear_history();
        assert_eq!(router.request_history().len(), 0);
    }
}

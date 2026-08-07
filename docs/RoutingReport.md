# Routing Report

## Overview

The RuntimeRouter implements provider-agnostic model selection based on capabilities, cost, health, and priority—never on provider name.

## Routing Algorithm

### Scoring Function

Each ModelCandidate is scored using a weighted formula:

```
score = (priority × 10) + cost_efficiency + latency_score + success_rate + capability_coverage
```

### Weights

| Factor | Weight | Range |
|--------|--------|-------|
| Priority | 0-30 | Priority::score() × 10 |
| Cost Efficiency | 0-20 | 1/(1 + total_cost/10) × 20 |
| Latency | 0-20 | 1/(1 + latency_ms/1000) × 20 |
| Success Rate | 0-20 | success_rate × 20 |
| Capability Coverage | 0-10 | (matched/required) × 10 |

### Filtering

Candidates are filtered before scoring:
1. **Health Check**: Unhealthy candidates score -1.0 (excluded)
2. **Capability Check**: Missing required capabilities score -1.0 (excluded)

## RoutingConfig

```rust
pub struct RoutingConfig {
    pub max_candidates: usize,      // Maximum candidates to consider
    pub cost_weight: f64,           // Cost importance (default: 0.25)
    pub latency_weight: f64,        // Latency importance (default: 0.25)
    pub quality_weight: f64,        // Quality importance (default: 0.3)
    pub health_weight: f64,         // Health importance (default: 0.2)
    pub failover_enabled: bool,     // Enable automatic failover
    pub cache_enabled: bool,        // Enable response caching
}
```

## ModelCandidate

```rust
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
```

## RuntimeRouter API

### Registration

```rust
let router = RuntimeRouter::new(RoutingConfig::default());
router.register_candidate(candidate);
router.unregister_candidate(&model_id);
router.update_health(&model_id, HealthStatus::Unhealthy);
```

### Routing

```rust
let decision = router.route(&request)?;
```

Returns `RoutingDecision` with:
- `selected_model`: The chosen ModelId
- `selected_candidate`: The full ModelCandidate
- `score`: The calculated score
- `reason`: Human-readable explanation
- `alternatives`: fallback models
- `capability_negotiation`: Matched/missing capabilities

### Diagnostics

```rust
let diag = router.diagnostics();
let summary = diag.summary();
let history = router.request_history();
```

## Routing Examples

### Example 1: Basic Routing

```rust
let router = RuntimeRouter::new(RoutingConfig::default());
router.register_candidate(
    ModelCandidate::new(
        ModelId::openai("gpt-4o"),
        CapabilitySet::new(vec![Capability::Streaming, Capability::ToolCalling]),
        HealthStatus::Healthy,
        CostEstimate::default(),
    ).with_priority(Priority::High)
);

let request = ModelRequest::new("gpt-4o", vec![])
    .with_stream(true);
let decision = router.route(&request).unwrap();
assert_eq!(decision.selected_model.id, "gpt-4o");
```

### Example 2: Cost-Based Selection

When two models have identical capabilities and priority, the router selects the lower-cost option:

```rust
let cheap = ModelCandidate::new(
    ModelId::openai("gpt-4o-mini"),
    CapabilitySet::new(vec![Capability::Streaming]),
    HealthStatus::Healthy,
    CostEstimate { input_cost_per_million: 0.15, ..default() },
).with_priority(Priority::Normal);

let expensive = ModelCandidate::new(
    ModelId::openai("gpt-4o"),
    CapabilitySet::new(vec![Capability::Streaming]),
    HealthStatus::Healthy,
    CostEstimate { input_cost_per_million: 2.50, ..default() },
).with_priority(Priority::Normal);
```

Result: `gpt-4o-mini` is selected due to lower cost.

### Example 3: Health-Based Filtering

Unhealthy candidates are automatically excluded:

```rust
let healthy = ModelCandidate::new(..., HealthStatus::Healthy, ...);
let unhealthy = ModelCandidate::new(..., HealthStatus::Unhealthy, ...);
```

Result: Only `healthy` candidate is considered.

## Test Coverage

16 router tests covering:
- Candidate registration and removal
- Health status updates
- Capability-based filtering
- Cost-based selection
- Priority-based selection
- Diagnostic event recording
- Request history tracking
- History cleanup

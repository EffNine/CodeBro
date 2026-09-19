# Model Routing Policy — P6 Design Specification

**Document:** `docs/design/MODEL_ROUTING_POLICY.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Model Routing Policy defines how CodeBro selects which LLM model to use for different task types. It provides a deterministic routing framework that can be customized through preferences, profiles, and approved recommendations.

---

## 2. Routing Strategies

### 2.1 Strategy: Simple (Default)

All tasks use the primary model. No routing decisions are made.

```
Task → Primary Model
```

### 2.2 Strategy: Role-Based

Different agent roles use different models, as configured in preferences or profiles.

```
Simple Task      → Primary Model
Complex Review   → Reviewer Model
Research         → Research Model
Planning         → Planner Model
Implementation   → Implementer Model
```

### 2.3 Strategy: Cost-Optimized

Tasks are routed to the cheapest model that meets quality thresholds.

```
Quality Threshold Check:
  If task complexity ≤ simple_threshold → Use cheapest capable model
  If task complexity ≤ moderate_threshold → Use mid-tier model
  If task complexity ≥ complex_threshold → Use highest-quality model
```

### 2.4 Strategy: Hybrid

Combines role-based and cost-optimized routing. Role overrides take precedence; unconfigured roles fall back to cost-optimized routing.

```
For each role:
  If role has override → Use override model
  Else → Use cost-optimized model for task complexity
```

---

## 3. Task Complexity Classification

The routing policy classifies tasks into complexity tiers before routing:

| Tier | Criteria | Example Tasks |
|------|----------|---------------|
| **Simple** | Single tool call, straightforward response | "What is this function?", "Show me the git status" |
| **Moderate** | 2-3 tools, some analysis required | "Add a test for X", "Explain this error" |
| **Complex** | Multi-step, requires planning + implementation | "Refactor the auth module", "Migrate to TypeScript" |

### 3.1 Classification Algorithm

```rust
fn classify_complexity(task: &str) -> TaskComplexity {
    let lower = task.to_lowercase();

    // Simple indicators
    if lower.contains("what is")
        || lower.contains("explain")
        || lower.contains("show me")
        || lower.contains("where is")
        || lower.contains("does this")
        || lower.contains("how does")
    {
        return TaskComplexity::Simple;
    }

    // Complex indicators
    if lower.contains("refactor")
        || lower.contains("redesign")
        || lower.contains("migrate")
        || lower.contains("implement a")
        || lower.contains("build a complete")
        || lower.contains("rewrite")
    {
        return TaskComplexity::Complex;
    }

    // Moderate indicators
    if lower.contains("add")
        || lower.contains("create")
        || lower.contains("fix")
        || lower.contains("update")
        || lower.contains("modify")
    {
        return TaskComplexity::Moderate;
    }

    // Default to moderate
    TaskComplexity::Moderate
}
```

---

## 4. Routing Policy Configuration

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingPolicy {
    /// Selected strategy: simple, role_based, cost_optimized, hybrid
    pub strategy: RoutingStrategy,

    /// Thresholds for cost-optimized strategy
    pub complexity_thresholds: ComplexityThresholds,

    /// Minimum quality score for each complexity tier
    pub min_quality: HashMap<String, f32>,

    /// Allowed models for each complexity tier
    pub allowed_models: HashMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplexityThresholds {
    /// Tasks with estimated tokens below this are simple
    pub simple_token_limit: usize,

    /// Tasks with estimated tokens below this are moderate
    pub moderate_token_limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RoutingStrategy {
    Simple,
    RoleBased,
    CostOptimized,
    Hybrid,
}
```

### 4.1 Default Routing Policy

```rust
RoutingPolicy {
    strategy: RoutingStrategy::RoleBased,
    complexity_thresholds: ComplexityThresholds {
        simple_token_limit: 500,
        moderate_token_limit: 2000,
    },
    min_quality: {
        "simple".to_string() => 0.6,
        "moderate".to_string() => 0.7,
        "complex".to_string() => 0.8,
    },
    allowed_models: {
        "simple".to_string() => vec!["gpt-4o-mini".to_string(), "gpt-4o".to_string()],
        "moderate".to_string() => vec!["gpt-4o".to_string(), "claude-haiku".to_string()],
        "complex".to_string() => vec!["gpt-4o".to_string(), "claude-sonnet-4".to_string(), "claude-opus-4".to_string()],
    },
}
```

---

## 5. Routing Decision Flow

```
Task received
     │
     ▼
Classify complexity (simple/moderate/complex)
     │
     ▼
Get routing strategy from preferences
     │
     ▼
┌──────────────────────────────────────────────┐
│  Strategy: Simple                            │
│  → Use primary model                         │
└──────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────┐
│  Strategy: Role-Based                        │
│  → Check role overrides                      │
│  → Check profile overrides                   │
│  → Fall back to primary model                │
└──────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────┐
│  Strategy: Cost-Optimized                    │
│  → Select cheapest model in allowed list     │
│  → That meets min_quality threshold          │
└──────────────────────────────────────────────┘
     │
     ▼
┌──────────────────────────────────────────────┐
│  Strategy: Hybrid                            │
│  → Role overrides first                      │
│  → Cost-optimized fallback                   │
└──────────────────────────────────────────────┘
     │
     ▼
Cost Policy check (is proposed model within budget?)
     │
     ▼
If cost exceeds budget → Recommend downgrade
If cost is within budget → Use resolved model
```

---

## 6. Trait Contract

```rust
pub trait ModelRoutingTrait: Send + Sync {
    /// Get the current routing strategy
    fn get_strategy(&self) -> &RoutingStrategy;

    /// Set the routing strategy (requires approval)
    fn set_strategy(&mut self, strategy: RoutingStrategy) -> Result<AdaptiveEvent>;

    /// Resolve the model for a task and role
    fn resolve_model(
        &self,
        task: &str,
        role: &str,
        primary_model: &str,
    ) -> ResolvedModel;

    /// Get the cost estimate for routing a task
    fn estimate_routing_cost(
        &self,
        task: &str,
        role: &str,
        primary_model: &str,
    ) -> CostEstimate;

    /// Get routing history
    fn get_history(&self) -> &[RoutingDecision];

    /// Check if a model switch would exceed cost limits
    fn check_cost_compliance(&self, proposed_model: &str, task_cost: f64) -> CostCompliance;
}

pub struct RoutingDecision {
    pub timestamp: String,
    pub task: String,
    pub role: String,
    pub complexity: TaskComplexity,
    pub strategy: RoutingStrategy,
    pub primary_model: String,
    pub resolved_model: String,
    pub estimated_cost: f64,
    pub cost_delta: f64,
}

pub enum CostCompliance {
    Compliant,
    Warning { proposed_cost: f64, limit: f64 },
    Blocked { reason: String },
}
```

---

## 7. TUI Integration

### 7.1 View: `/routing`

```
┌─────────────────────────────────────────────┐
│  MODEL ROUTING                              │
├─────────────────────────────────────────────┤
│  Strategy: Role-Based                       │
│                                             │
│  Complexity Thresholds:                     │
│  Simple:    < 500 tokens                    │
│  Moderate:  < 2000 tokens                   │
│  Complex:   ≥ 2000 tokens                   │
│                                             │
│  Allowed Models by Tier:                    │
│  ─────────────────────────────────          │
│  Simple:    gpt-4o-mini, gpt-4o             │
│  Moderate:  gpt-4o, claude-haiku            │
│  Complex:   gpt-4o, claude-sonnet-4,        │
│             claude-opus-4                   │
│                                             │
│  [Change Strategy]  [Edit Thresholds]       │
│  [Close]                                      │
└─────────────────────────────────────────────┘
```

---

## 8. Anti-Patterns

```rust
// NEVER: Route to a model not in the allowed_models list
// ALWAYS: Validate against allowed models before routing

// NEVER: Allow a routing decision that exceeds the daily cost limit
// ALWAYS: Check CostCompliance before applying

// NEVER: Change routing strategy without user approval
// ALWAYS: Emit a Recommendation for strategy changes
```

---

## 9. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [SUBAGENT_ORCHESTRATION_SPEC.md](./SUBAGENT_ORCHESTRATION_SPEC.md)
- [COST_POLICY.md](./COST_POLICY.md)

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

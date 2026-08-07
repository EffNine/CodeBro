# Cost Policy — P6 Design Specification

**Document:** `docs/design/COST_POLICY.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Cost Policy provides transparent cost management for all LLM operations. It tracks spending, warns before exceeding limits, and prevents silent cost increases. No cost change occurs without explicit user awareness.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                       Cost Policy                             │
│                                                               │
│  ┌───────────────────┐    ┌───────────────────┐              │
│  │  Cost Tracker     │    │  Limit Enforcer   │              │
│  │  (accumulates     │    │  (checks daily/   │              │
│  │   costs per       │    │   session limits) │              │
│  │   operation)      │    │                   │              │
│  └────────┬──────────┘    └────────┬──────────┘              │
│           │                        │                          │
│           └─────────┬──────────────┘                          │
│                     ▼                                         │
│           ┌─────────────────────┐                             │
│           │  Cost Comparison    │                             │
│           │  (model A vs. model │                             │
│           │   B for same task)  │                             │
│           └──────────┬──────────┘                             │
│                      ▼                                        │
│           ┌─────────────────────┐                             │
│           │  Warning / Block    │                             │
│           │  Generator          │                             │
│           └─────────────────────┘                             │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                     Approval Gate
```

---

## 3. Cost Tracking

### 3.1 Cost Model

Costs are tracked per operation using a pricing lookup table:

```rust
pub struct PricingTable {
    /// Maps model names to per-token costs (input and output)
    pub models: HashMap<String, ModelPricing>,
}

pub struct ModelPricing {
    pub input_cost_per_million: f64,
    pub output_cost_per_million: f64,
    pub caching_cost_per_million: Option<f64>,
}

impl PricingTable {
    pub fn estimate_cost(&self, model: &str, input_tokens: usize, output_tokens: usize) -> f64 {
        match self.models.get(model) {
            Some(pricing) => {
                (input_tokens as f64 * pricing.input_cost_per_million / 1_000_000.0)
                    + (output_tokens as f64 * pricing.output_cost_per_million / 1_000_000.0)
            }
            None => 0.0, // Unknown models are treated as free (conservative)
        }
    }
}
```

### 3.2 Default Pricing (Approximate)

```rust
const DEFAULT_PRICING: &[(&str, f64, f64)] = &[
    ("gpt-4o-mini", 0.15, 0.60),
    ("gpt-4o", 2.50, 10.00),
    ("claude-haiku", 0.25, 1.25),
    ("claude-sonnet-4", 3.00, 15.00),
    ("claude-opus-4", 15.00, 75.00),
    ("deepseek-v3", 0.14, 0.28),
];
```

### 3.3 Cost Accumulation

```rust
pub struct CostTracker {
    daily_costs: HashMap<String, f64>,     // date → cost
    session_costs: HashMap<String, f64>,   // session_id → cost
    total_cost: f64,
    operation_log: Vec<CostRecord>,
}

pub struct CostRecord {
    pub timestamp: String,
    pub session_id: String,
    pub model: String,
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub estimated_cost: f64,
    pub task_description: String,
}
```

---

## 4. Limit Enforcement

### 4.1 Limit Types

| Limit Type | Scope | Default | Configurable |
|------------|-------|---------|--------------|
| Daily limit | All sessions in one day | $5.00 | Yes |
| Session limit | Single session | $2.00 | Yes |
| Per-task limit | Individual task | $1.00 | Yes |

### 4.2 Enforcement Behavior

```rust
pub enum LimitCheckResult {
    Compliant,                    // Within all limits
    Warning { limit_type: String, current: f64, limit: f64 },
    Blocked { limit_type: String, current: f64, limit: f64, reason: String },
}
```

| Result | Action |
|--------|--------|
| `Compliant` | Proceed normally |
| `Warning` (80-99% of limit) | Emit `CostWarning` event to TUI |
| `Blocked` (≥100% of limit) | Prevent task execution; require user override |

### 4.3 Override Behavior

When a limit is blocked, the user may override with explicit confirmation:

```
[Cost Warning] Your daily limit ($5.00) has been reached.
  Current spending: $5.00
  This task will cost approximately: $0.75
  Proposed new total: $5.75

  [Continue Anyway]  [Cancel Task]
```

The override is recorded in the audit log and does not change the limit.

---

## 5. Model Comparison

When a routing decision could use multiple models, the Cost Policy provides a comparison:

```rust
pub struct ModelComparison {
    pub task_description: String,
    pub complexity: TaskComplexity,
    pub options: Vec<ModelCostOption>,
    pub recommended: usize,
}

pub struct ModelCostOption {
    pub model: String,
    pub estimated_input_tokens: usize,
    pub estimated_output_tokens: usize,
    pub estimated_cost: f64,
    pub quality_estimate: f32,
    pub is_recommended: bool,
}
```

### 5.1 Comparison Display

```
┌─────────────────────────────────────────────┐
│  MODEL COMPARISON                           │
├─────────────────────────────────────────────┤
│  Task: Refactor auth module                 │
│  Complexity: Complex                        │
│                                             │
│  ┌──────────────┬──────────┬──────────┐   │
│  │ Model        │ Cost     │ Quality  │   │
│  ├──────────────┼──────────┼──────────┤   │
│  │ gpt-4o-mini  │ $0.12    │ 0.6      │   │
│  │ gpt-4o       │ $1.50    │ 0.8      │ ✓ │
│  │ claude-sonnet│ $2.25    │ 0.9      │   │
│  └──────────────┴──────────┴──────────┘   │
│                                             │
│  Recommended: gpt-4o (best quality/cost)    │
│                                             │
│  [Use gpt-4o]  [Choose Different]  [Close]  │
└─────────────────────────────────────────────┘
```

---

## 6. Trait Contract

```rust
pub trait CostPolicyTrait: Send + Sync {
    /// Get current spending for a time period
    fn get_spending(&self, period: CostPeriod) -> f64;

    /// Get the configured limits
    fn get_limits(&self) -> &CostLimits;

    /// Set spending limits (requires approval)
    fn set_limits(&mut self, limits: CostLimits) -> Result<AdaptiveEvent>;

    /// Check if a proposed operation is within limits
    fn check_limit(&self, estimated_cost: f64, period: CostPeriod) -> LimitCheckResult;

    /// Estimate the cost of a task with a specific model
    fn estimate_task_cost(&self, model: &str, input_tokens: usize, output_tokens: usize) -> f64;

    /// Get model cost comparisons for a task
    fn compare_models(&self, task: &str, complexity: TaskComplexity) -> ModelComparison;

    /// Record a completed operation's cost
    fn record_cost(&mut self, record: CostRecord);

    /// Get cost history
    fn get_history(&self) -> &[CostRecord];

    /// Reset daily counters (called at midnight)
    fn reset_daily_counters(&mut self);
}

pub enum CostPeriod {
    Today,
    Session,
    AllTime,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostLimits {
    pub daily_limit_usd: Option<f64>,
    pub session_limit_usd: Option<f64>,
    pub per_task_limit_usd: Option<f64>,
    pub warning_threshold: f32, // 0.0-1.0, default 0.8
}
```

---

## 7. Integration Points

### 7.1 Before Task Execution

```rust
// In AdaptiveOrchestrator::run_task()
let estimated_cost = cost_policy.estimate_task_cost(model, input_tokens, output_tokens);
match cost_policy.check_limit(estimated_cost, CostPeriod::Today) {
    LimitCheckResult::Compliant => { /* proceed */ }
    LimitCheckResult::Warning { .. } => {
        adaptive_bus.publish(AdaptiveEvent::CostWarning { /* ... */ });
        // Continue but warn
    }
    LimitCheckResult::Blocked { .. } => {
        // Show approval dialog to user
        return Ok(TaskResult::BlockedByCost);
    }
}
```

### 7.2 Before Model Switch

```rust
// In ModelResolver::resolve_model()
let new_cost = cost_policy.estimate_task_cost(new_model, ...);
let old_cost = cost_policy.estimate_task_cost(old_model, ...);
let delta = new_cost - old_cost;

if delta > 0.0 {
    // Cost increase — requires approval
    recommendation.cost_impact = Some(CostImpact {
        delta_usd: delta,
        delta_percentage: (delta / old_cost) as f32,
        category: CostCategory::ModelUpgrade,
        ...
    });
    recommendation.required_approval = true;
}
```

---

## 8. TUI Integration

### 8.1 View: `/cost`

```
┌─────────────────────────────────────────────┐
│  COST DASHBOARD                             │
├─────────────────────────────────────────────┤
│  Today: $2.35 / $5.00 (47%) ████████░░      │
│  Session: $0.80 / $2.00 (40%) █████░░░░░    │
│  All-time: $47.20                           │
│                                             │
│  Limits:                                    │
│  Daily: $5.00  Session: $2.00               │
│                                             │
│  Recent Activity:                           │
│  ─────────────────────────────────          │
│  14:32  gpt-4o       $0.15  "Explain X"    │
│  14:28  gpt-4o-mini  $0.02  "List files"   │
│  14:15  claude-sonnet $0.45  "Review PR"   │
│                                             │
│  [Set Limits]  [History]  [Close]           │
└─────────────────────────────────────────────┘
```

### 8.2 Title Bar Integration

The title bar shows real-time cost status:

```
CODEBRO | WS: myproject | Model: gpt-4o | Cost: $2.35/$5.00 | Tools: ✓
```

When spending exceeds 80%:
```
CODEBRO | WS: myproject | Model: gpt-4o | Cost: $4.10/$5.00 ⚠ | Tools: ✓
```

---

## 9. Cost Reporting

### 9.1 Daily Summary

At the end of each day (or on request), the Cost Policy generates a summary:

```
Daily Cost Report — 2026-08-06
━━━━━━━━━━━━━━━━━━━━━━━━━━━━
Total: $3.45
Operations: 24
Average per operation: $0.14

By model:
  gpt-4o:       $2.10 (61%)
  gpt-4o-mini:  $0.55 (16%)
  claude-sonnet: $0.80 (23%)

By category:
  Coding:   $1.50
  Review:   $1.20
  Research: $0.75
```

### 9.2 Export

Cost data can be exported as CSV for external analysis:

```rust
fn export_csv(&self) -> Result<String> {
    // Generate CSV with columns: date, session, model, tokens_in, tokens_out, cost, task
}
```

---

## 10. Anti-Patterns

```rust
// NEVER: Allow a task to proceed if it would exceed the hard limit
// WITHOUT user approval

// NEVER: Hide cost information from the user
// ALWAYS: Show cost in title bar and /cost panel

// NEVER: Estimate cost without being conservative
// ALWAYS: Over-estimate rather than under-estimate
```

---

## 11. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [MODEL_ROUTING_POLICY.md](./MODEL_ROUTING_POLICY.md)
- [USER_PREFERENCE_MODEL.md](./USER_PREFERENCE_MODEL.md)

---

## 12. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

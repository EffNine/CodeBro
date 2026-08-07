# Recommendation Engine — P6 Design Specification

**Document:** `docs/design/RECOMMENDATION_ENGINE_SPEC.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Recommendation Engine generates actionable suggestions based on observed developer behavior, preferences, and context. It is the primary output mechanism of the Adaptive Platform — every adaptive subsystem feeds recommendations through this engine.

**Key principle:** The Recommendation Engine NEVER performs actions. It ONLY generates recommendations that require explicit user approval.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                    Recommendation Engine                      │
│                                                               │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐   │
│  │ Preference   │  │ Intent       │  │ Workflow         │   │
│  │ Engine       │  │ Engine       │  │ Engine           │   │
│  │ (current     │  │ (recent      │  │ (detected        │   │
│  │  prefs)      │  │  intents)    │  │  patterns)       │   │
│  └──────┬───────┘  └──────┬───────┘  └────────┬─────────┘   │
│         │                 │                    │             │
│         └─────────────────┼────────────────────┘             │
│                           ▼                                  │
│              ┌────────────────────────┐                      │
│              │   Recommendation      │                      │
│              │   Generator           │                      │
│              │                       │                      │
│              │  - Aggregate inputs   │                      │
│              │  - Score candidates   │                      │
│              │  - Deduplicate        │                      │
│              │  - Rank by relevance  │                      │
│              └────────────┬──────────┘                      │
│                           ▼                                  │
│              ┌────────────────────────┐                      │
│              │   Cost Policy         │                      │
│              │   (evaluate cost      │                      │
│              │    impact)            │                      │
│              └────────────┬──────────┘                      │
│                           ▼                                  │
│              ┌────────────────────────┐                      │
│              │   Trust Model         │                      │
│              │   (score confidence   │                      │
│              │    & risk)            │                      │
│              └────────────┬──────────┘                      │
│                           ▼                                  │
│              ┌────────────────────────┐                      │
│              │   Recommendation     │                      │
│              │   (final output)      │                      │
│              └────────────────────────┘                      │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                        Approval Gate
```

---

## 3. Recommendation Structure

Every recommendation carries the following mandatory fields:

```rust
pub struct Recommendation {
    /// Unique identifier
    pub id: String,

    /// Short title for display
    pub title: String,

    /// Detailed description
    pub body: String,

    /// Confidence score (0.0–1.0)
    pub confidence: f32,

    /// Natural language explanation of why this recommendation was made
    pub reasoning: String,

    /// Supporting evidence (data points, past experiences, metrics)
    pub evidence: Vec<String>,

    /// Estimated cost impact (None if no cost change)
    pub cost_impact: Option<CostImpact>,

    /// Expected benefit of accepting this recommendation
    pub expected_benefit: String,

    /// How reversible is this change?
    pub reversibility: Reversibility,

    /// Does this require explicit approval?
    pub required_approval: bool,

    /// Source subsystem that generated this recommendation
    pub source: RecommendationSource,

    /// Timestamp
    pub created_at: String,
}

pub struct CostImpact {
    pub current_estimate_usd: f64,
    pub proposed_estimate_usd: f64,
    pub delta_usd: f64,
    pub delta_percentage: f32,
    pub category: CostCategory,
}

pub enum CostCategory {
    ModelUpgrade,
    ModelDowngrade,
    NewIntegration,
    WorkflowAutomation,
    Other,
}

pub enum Reversibility {
    FullyReversible,    // Can be undone with one click
    PartiallyReversible, // Requires manual effort to undo
    Irreversible,       // Cannot be undone
}

pub enum RecommendationSource {
    PreferenceEngine,
    IntentEngine,
    WorkflowEngine,
    ProfileEngine,
    ModelRouting,
    CostPolicy,
    McpLifecycle,
    SkillLifecycle,
    TrustModel,
}
```

---

## 4. Generation Rules

### 4.1 When to Generate a Recommendation

| Trigger | Condition |
|---------|-----------|
| Preference change | Any change to `Preferences` requires a confirmation recommendation |
| Intent detected | Confidence >= 0.5 produces a recommendation |
| Workflow detected | A pattern observed 3+ times produces a recommendation |
| Profile mismatch | Current workspace language differs from primary language preference |
| Cost threshold | Current spending exceeds 80% of daily limit |
| Model routing | Suggested model change has cost delta > 0 |
| MCP discovery | New MCP server found with matching tools |
| Skill match | A skill matches current task with confidence >= 0.6 |

### 4.2 When NOT to Generate a Recommendation

| Condition | Reason |
|-----------|--------|
| Confidence < 0.5 | Insufficient evidence |
| Already recommended within 5 minutes | Avoid notification fatigue |
| User previously rejected same recommendation | Do not repeat without new evidence |
| Reversibility is Irreversible and confidence < 0.9 | Too risky without strong evidence |
| Cost delta is negative (saves money) and confidence < 0.7 | Low-value optimization |

### 4.3 Deduplication

Recommendations are deduplicated by:
1. Same `source` + same `title` within 10 minutes → suppressed
2. Same `source` + same intent key within 1 hour → suppressed unless confidence increased by > 0.2

---

## 5. Ranking and Prioritization

When multiple recommendations are generated simultaneously, they are ranked by:

```rust
fn rank_score(rec: &Recommendation) -> f32 {
    let confidence_score = rec.confidence * 0.4;
    let benefit_score = benefit_weight(&rec.expected_benefit) * 0.3;
    let urgency_score = urgency_weight(&rec) * 0.2;
    let cost_efficiency = cost_efficiency_score(&rec) * 0.1;

    confidence_score + benefit_score + urgency_score + cost_efficiency
}

fn benefit_weight(benefit: &str) -> f32 {
    match benefit.to_lowercase().as_str() {
        b if b.contains("cost") || b.contains("save") => 1.0,
        b if b.contains("speed") || b.contains("faster") => 0.8,
        b if b.contains("quality") || b.contains("better") => 0.7,
        b if b.contains("convenience") || b.contains("easier") => 0.5,
        _ => 0.3,
    }
}

fn urgency_weight(rec: &Recommendation) -> f32 {
    match &rec.reversibility {
        Reversibility::FullyReversible => 0.3,
        Reversibility::PartiallyReversible => 0.5,
        Reversibility::Irreversible => 0.8,
    }
}
```

---

## 6. Trait Contract

```rust
pub trait RecommendationEngineTrait: Send + Sync {
    /// Generate recommendations based on current context
    fn generate(&self, context: &RecommendationContext) -> Vec<Recommendation>;

    /// Get the most recent recommendations (for TUI display)
    fn get_recent(&self, limit: usize) -> Vec<&Recommendation>;

    /// Get recommendation history
    fn get_history(&self) -> &[TrackedRecommendation];

    /// Record that a recommendation was approved
    fn record_approval(&mut self, recommendation_id: &str);

    /// Record that a recommendation was rejected
    fn record_rejection(&mut self, recommendation_id: &str, reason: Option<String>);

    /// Get recommendation statistics
    fn get_statistics(&self) -> RecommendationStatistics;

    /// Clear stale recommendations (older than threshold)
    fn clear_stale(&mut self, max_age_hours: u64) -> usize;
}

pub struct RecommendationContext {
    pub current_preferences: HashMap<String, String>,
    pub recent_intents: Vec<String>,
    pub active_profile: Option<String>,
    pub current_task: Option<String>,
    pub workspace_info: WorkspaceInfo,
    pub recent_cost: f64,
    pub daily_cost_limit: Option<f64>,
}

pub struct TrackedRecommendation {
    pub recommendation: Recommendation,
    pub approved: bool,
    pub rejected: bool,
    pub rejection_reason: Option<String>,
    pub approved_at: Option<String>,
}

pub struct RecommendationStatistics {
    pub total_generated: usize,
    pub total_approved: usize,
    pub total_rejected: usize,
    pub approval_rate: f32,
    pub average_confidence: f32,
}
```

---

## 7. TUI Integration

### 7.1 Recommendation Display

Recommendations appear in a dedicated panel when triggered:

```
┌─────────────────────────────────────────────┐
│  RECOMMENDATION                             │
├─────────────────────────────────────────────┤
│  Switch language preference to Rust         │
│  ─────────────────────────────────          │
│  Confidence: ████████░░ 85%                 │
│  Reasoning: You mentioned "mostly Rust"     │
│  Evidence: 3 recent Rust tasks              │
│  Benefit: Better code intelligence          │
│  Reversible: Yes                            │
│                                             │
│  [Approve]  [Reject]  [Dismiss]             │
└─────────────────────────────────────────────┘
```

### 7.2 Batch Display

When multiple recommendations arrive simultaneously, they are displayed as a list:

```
┌─────────────────────────────────────────────┐
│  RECOMMENDATIONS (3)                        │
├─────────────────────────────────────────────┤
│  1. Switch language to Rust      [85%]     │
│  2. Set cost tier to minimal   [70%]       │
│  3. Enable auto-save on edits    [60%]     │
│                                             │
│  [Approve All]  [Review]  [Dismiss All]     │
└─────────────────────────────────────────────┘
```

### 7.3 Silent Mode

Users can configure a silent mode where recommendations are logged but not displayed:

```
/workflows suggest_workflows=false  →  recommendations are logged but not shown
```

---

## 8. Anti-Patterns

```rust
// NEVER: Generate a recommendation without evidence
// ALWAYS: Include at least one evidence string

// NEVER: Recommend irreversible changes with low confidence
// ALWAYS: Set required_approval=true when reversibility=Irreversible and confidence<0.9

// NEVER: Generate duplicate recommendations within the cooldown window
// ALWAYS: Check deduplication rules before generating
```

---

## 9. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [USER_PREFERENCE_MODEL.md](./USER_PREFERENCE_MODEL.md)
- [INTENT_ENGINE_SPEC.md](./INTENT_ENGINE_SPEC.md)
- [TRUST_MODEL.md](./TRUST_MODEL.md)
- [COST_POLICY.md](./COST_POLICY.md)

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

# Explainability Policy

**Document:** `docs/policies/EXPLAINABILITY_POLICY.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.0 Implementation Readiness

---

## 1. Purpose

This policy mandates that **every recommendation** produced by CodeBro's adaptive subsystems must be fully explainable. No black-box recommendations are permitted. Every recommendation must expose its reasoning chain so users can understand, trust, and intervene.

---

## 2. Core Principle

**Transparency by Default.** Users must always know:
- Why a recommendation was made
- Why it was made now
- What evidence supports it
- How confident the system is
- What the estimated cost impact is
- What the expected benefit is
- Whether the recommendation is reversible

---

## 3. Required Explanation Fields

Every recommendation MUST include all of the following fields:

### 3.1 Why

The reason this recommendation exists.

```rust
pub struct Recommendation {
    pub why: String,          // "User has repeatedly used 'fmt' before changes"
    pub why_now: String,      // "A file modification is detected"
    pub evidence: Vec<Evidence>,
    pub confidence: f32,      // 0.0–1.0
    pub estimated_cost_impact: CostImpact,
    pub expected_benefit: Benefit,
    pub reversible: bool,
    // ... other fields
}
```

### 3.2 Why Now

The trigger that caused this recommendation to be made at this moment.

| Trigger | Example |
|---------|---------|
| File change detected | "File `src/main.rs` was modified" |
| Tool execution completed | "Test suite passed after edit" |
| User preference matched | "User preference for 'rustfmt' matched" |
| Pattern recognized | "Project uses Cargo workspace pattern" |
| Time-based | "Daily recommendation refresh" |

### 3.3 Evidence

All data that supports the recommendation. Evidence must be:
- **Citable**: Each piece of evidence references a source.
- **Verifiable**: User can inspect the source.
- **Complete**: No selective omission of contrary evidence.

```rust
pub struct Evidence {
    pub source: EvidenceSource,
    pub description: String,
    pub value: String,
    pub timestamp: DateTime<Utc>,
}

pub enum EvidenceSource {
    FileChange(PathBuf),
    ToolOutput(String),
    UserPreference(String),
    ProjectMemory(String),
    SessionHistory(Uuid),
    Configuration(String),
}
```

### 3.4 Confidence

A float between 0.0 and 1.0 indicating how confident the system is in this recommendation.

| Confidence | Meaning | User Display |
|------------|---------|-------------|
| 0.0–0.3 | Low | "Unlikely to be relevant" |
| 0.3–0.6 | Medium | "Possible match" |
| 0.6–0.8 | High | "Likely relevant" |
| 0.8–1.0 | Very High | "Strong match" |

**Rules:**
- Confidence must never be 1.0 (absolute certainty is impossible).
- Confidence must decrease when evidence is weak or contradictory.
- Confidence must be recalculated when new evidence arrives.

### 3.5 Estimated Cost Impact

The estimated resource cost of acting on this recommendation.

```rust
pub struct CostImpact {
    pub tokens_estimated: Option<usize>,
    pub time_estimated: Option<Duration>,
    pub file_changes_estimated: Option<usize>,
    pub command_runs_estimated: Option<usize>,
    pub cost_usd_estimated: Option<f64>,
}
```

### 3.6 Expected Benefit

The expected benefit of acting on this recommendation.

```rust
pub struct Benefit {
    pub description: String,
    pub magnitude: BenefitMagnitude,
    pub type_: BenefitType,
}

pub enum BenefitMagnitude {
    Negligible,
    Low,
    Medium,
    High,
    Critical,
}

pub enum BenefitType {
    TimeSaved,
    QualityImproved,
    RiskReduced,
    ConsistencyImproved,
    KnowledgePreserved,
}
```

### 3.7 Reversible

Whether the recommended action can be undone without permanent consequences.

| Value | Meaning |
|-------|---------|
| `true` | Action can be undone (e.g., creating a file, running a command) |
| `false` | Action is irreversible (e.g., deleting a file, changing a credential) |

**Rule:** Irreversible actions require double confirmation and explicit user acknowledgment.

---

## 4. Explanation by Subsystem

### 4.1 Preference Engine

| Field | Example |
|-------|---------|
| Why | "User preference for 'test-before-commit' detected from 5 previous sessions" |
| Why now | "Commit detected in git-tracked project" |
| Evidence | ["Session 1: user ran cargo test before commit", "Session 3: user approved test suggestion", "Session 5: user preference saved"] |
| Confidence | 0.85 |
| Cost Impact | "~2 tokens, ~30s execution time" |
| Expected Benefit | "Prevents broken commits; magnitude: High; type: RiskReduced" |
| Reversible | true |

### 4.2 Intent Engine

| Field | Example |
|-------|---------|
| Why | "User intent to 'fix lint errors' inferred from recent tool usage pattern" |
| Why now | "Lint tool reported 12 errors in current file" |
| Evidence | ["Tool: cargo clippy → 12 errors", "User preference: auto-fix lints = true", "History: user fixed lints in 80% of sessions"] |
| Confidence | 0.72 |
| Cost Impact | "~50 tokens, ~5s execution time" |
| Expected Benefit | "Resolves all lint errors; magnitude: Medium; type: QualityImproved" |
| Reversible | true |

### 4.3 Workflow Engine

| Field | Example |
|-------|---------|
| Why | "Workflow rule 'run tests on save' triggered by file modification" |
| Why now | "File `src/lib.rs` saved" |
| Evidence | ["Workflow rule: run_tests_on_save = true", "File change detected at line 42", "Previous test runs: 95% pass rate"] |
| Confidence | 0.95 |
| Cost Impact | "~100 tokens, ~10s execution time" |
| Expected Benefit | "Catches regressions early; magnitude: High; type: RiskReduced" |
| Reversible | true |

### 4.4 Recommendation Engine

| Field | Example |
|-------|---------|
| Why | "Recommendation to use 'tokio::spawn' based on project patterns" |
| Why now | "Async function detected in `src/main.rs`" |
| Evidence | ["Project uses Tokio (Cargo.toml)", "12 existing tokio::spawn calls in project", "0 manual thread::spawn calls"] |
| Confidence | 0.88 |
| Cost Impact | "~20 tokens, ~0s execution time" |
| Expected Benefit | "Consistent async pattern; magnitude: Low; type: ConsistencyImproved" |
| Reversible | true |

### 4.5 Profile Engine

| Field | Example |
|-------|---------|
| Why | "User profile suggests preference for 'minimal output'" |
| Why now | "Tool output exceeds 1000 characters" |
| Evidence | ["Profile setting: max_output_length = 1000", "Current output: 2500 characters", "User previously truncated output in 3 sessions"] |
| Confidence | 0.78 |
| Cost Impact | "~0 tokens, ~0s execution time" |
| Expected Benefit | "Reduced noise in TUI; magnitude: Low; type: QualityImproved" |
| Reversible | true |

---

## 5. TUI Display

### 5.1 Recommendation Card

```
┌─────────────────────────────────────────────────────┐
│  💡 RECOMMENDATION                                   │
├─────────────────────────────────────────────────────┤
│  Why: User preference for 'test-before-commit'       │
│  Why Now: Commit detected in git-tracked project    │
│                                                     │
│  Evidence:                                          │
│    • Session 1: user ran cargo test before commit   │
│    • Session 3: user approved test suggestion       │
│    • Session 5: user preference saved               │
│                                                     │
│  Confidence: 85%  │  Reversible: Yes                │
│  Cost: ~2 tokens, ~30s  │  Benefit: RiskReduced     │
│                                                     │
│  [ Accept ]  [ Reject ]  [ Explain More ]           │
└─────────────────────────────────────────────────────┘
```

### 5.2 Explain More (Drill-Down)

When the user clicks "Explain More":
1. Show full evidence chain with source citations.
2. Show alternative recommendations that were considered but rejected.
3. Show confidence breakdown (what increased/decreased confidence).
4. Show historical accuracy of similar recommendations.

---

## 6. Anti-Patterns (What Not to Do)

| Anti-Pattern | Why It Violates Policy |
|-------------|----------------------|
| "I think this is a good idea" | No evidence cited |
| "Based on your history" | Vague — which history? |
| Confidence = 1.0 | Absolute certainty is impossible |
| Hidden recommendations | All recommendations must be visible |
| Unreversible actions without warning | User must know if action is irreversible |
| Cost not disclosed | User has right to know resource impact |
| benefit not disclosed | User has right to know expected value |

---

## 7. Validation

### 7.1 Pre-Recommendation Validation

Before any recommendation is shown, the system validates:

| Check | Rule |
|-------|------|
| Why present? | `why` field is non-empty |
| Why now present? | `why_now` field is non-empty |
| Evidence complete? | At least 1 evidence item |
| Confidence valid? | 0.0 < `confidence` < 1.0 |
| Cost disclosed? | `estimated_cost_impact` is populated |
| Benefit disclosed? | `expected_benefit` is populated |
| Reversibility stated? | `reversible` field is set |

If any check fails, the recommendation is **not shown** and an error is logged.

### 7.2 Post-Recommendation Validation

After user action (accept/reject), the system validates:

| Check | Rule |
|-------|------|
| User decision recorded? | Yes |
| Explanation satisfied user? | Feedback collected |
| Recommendation accurate? | Outcome compared to expected benefit |

---

## 8. Logging

### 8.1 Recommendation Log

Every recommendation is logged:

```json
{
  "timestamp": "2026-08-06T10:00:00Z",
  "subsystem": "preference_engine",
  "recommendation_id": "550e8400-...",
  "why": "User preference for 'test-before-commit' detected",
  "why_now": "Commit detected in git-tracked project",
  "evidence": [...],
  "confidence": 0.85,
  "cost_impact": {...},
  "benefit": {...},
  "reversible": true,
  "user_action": "accepted",
  "user_feedback": null
}
```

### 8.2 Log Retention

- Recommendation logs are retained for **90 days**.
- Logs are stored in `~/.codebro/recommendation_log.jsonl`.
- Logs are included in data export.

---

## 9. References

- [ADR-009: Configuration Versioning](../ADR/adr-009-configuration-versioning.md)
- [ADAPTIVE_MEMORY_POLICY.md](./ADAPTIVE_MEMORY_POLICY.md)
- [DX Principles](../vision/DX_PRINCIPLES.md)

---

## 10. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

# Trust Model — P6 Design Specification

**Document:** `docs/design/TRUST_MODEL.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Trust Model provides a scoring system that assesses the trustworthiness of every recommendation. It ensures that high-risk actions receive appropriate scrutiny and that users can understand why a recommendation is (or isn't) trustworthy.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                       Trust Model                             │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              Trust Scorer                               │ │
│  │                                                         │ │
│  │  Input: Recommendation                                  │ │
│  │                                                         │ │
│  │  Factors:                                               │ │
│  │  ├─ Confidence (0.0-1.0)                               │ │
│  │  ├─ Evidence Strength (0.0-1.0)                        │ │
│  │  ├─ Cost Risk (0.0-1.0)                                │ │
│  │  ├─ Reversibility Factor (0.0-1.0)                     │ │
│  │  └─ Historical Accuracy (0.0-1.0)                      │ │
│  │                                                         │ │
│  │  Output: TrustScore { composite, breakdown }           │ │
│  └─────────────────────────────────────────────────────────┘ │
│                             │                                │
│              ┌──────────────┼──────────────┐                │
│              ▼              ▼              ▼                 │
│     ┌─────────────┐  ┌─────────────┐  ┌─────────────┐      │
│     │  Explanation│  │  Risk       │  │  History    │      │
│     │  Generator  │  │  Classifier │  │  Tracker    │      │
│     └─────────────┘  └─────────────┘  └─────────────┘      │
└───────────────────────────────────────────────────────────────┘
```

---

## 3. Trust Score Calculation

### 3.1 Formula

```
composite = (confidence × 0.30) + (evidence × 0.25) + ((1 - cost_risk) × 0.20) + (reversibility × 0.15) + (historical_accuracy × 0.10)
```

### 3.2 Factor Definitions

| Factor | Range | How It's Calculated |
|--------|-------|---------------------|
| **Confidence** | 0.0–1.0 | From the recommendation's confidence score |
| **Evidence Strength** | 0.0–1.0 | Number and quality of evidence strings |
| **Cost Risk** | 0.0–1.0 | 0.0 = no cost change, 1.0 = very high cost increase |
| **Reversibility** | 0.0–1.0 | 1.0 = fully reversible, 0.0 = irreversible |
| **Historical Accuracy** | 0.0–1.0 | Ratio of past similar recommendations that were correct |

### 3.3 Composite Score Interpretation

| Composite Range | Trust Level | Action Required |
|-----------------|-------------|-----------------|
| 0.0–0.3 | **Low** | Must require explicit approval; show full explanation |
| 0.3–0.6 | **Medium** | Show recommendation with warning; approval required |
| 0.6–0.8 | **High** | Show recommendation; approval recommended but not forced |
| 0.8–1.0 | **Very High** | Show recommendation; auto-apply only if explicitly configured |

---

## 4. Trust Score Structure

```rust
pub struct TrustScore {
    /// Overall composite score (0.0–1.0)
    pub composite: f32,

    /// Individual factor scores
    pub confidence: f32,
    pub evidence_strength: f32,
    pub cost_risk: f32,
    pub reversibility_factor: f32,
    pub historical_accuracy: f32,

    /// Trust level classification
    pub level: TrustLevel,

    /// Human-readable explanation
    pub explanation: String,

    /// Risk classification
    pub risk_level: RiskLevel,

    /// Recommended action
    pub recommended_action: RecommendedAction,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TrustLevel {
    Low,
    Medium,
    High,
    VeryHigh,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RiskLevel {
    Safe,
    Moderate,
    Risky,
    Dangerous,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RecommendedAction {
    AutoApprove,      // Trust >= 0.9 and reversibility = FullyReversible
    ApproveRecommended, // Trust >= 0.7
    ApproveRequired,   // Trust >= 0.4
    RejectSuggested,   // Trust < 0.4
}
```

---

## 5. Explanation Generator

The Trust Model generates a human-readable explanation for every score:

```rust
impl TrustModel {
    pub fn explain(&self, score: &TrustScore) -> String {
        let mut parts = Vec::new();

        parts.push(format!(
            "Trust score: {:.0}% ({})",
            score.composite * 100.0,
            score.level
        ));

        if score.confidence < 0.7 {
            parts.push(format!(
                "  · Confidence is low ({:.0}%) — insufficient data",
                score.confidence * 100.0
            ));
        }

        if score.evidence_strength < 0.5 {
            parts.push("  · Weak evidence — few supporting data points".to_string());
        }

        if score.cost_risk > 0.5 {
            parts.push(format!(
                "  · High cost risk ({:.0}%) — this may increase spending",
                score.cost_risk * 100.0
            ));
        }

        if score.reversibility_factor < 0.5 {
            parts.push("  · Low reversibility — changes may be hard to undo".to_string());
        }

        if score.historical_accuracy < 0.6 {
            parts.push(format!(
                "  · Poor historical accuracy ({:.0}%) — similar recommendations often wrong",
                score.historical_accuracy * 100.0
            ));
        }

        parts.join("\n")
    }
}
```

---

## 6. Historical Accuracy Tracking

The Trust Model tracks the accuracy of past recommendations to inform future scores:

```rust
pub struct TrustHistory {
    pub records: Vec<TrustRecord>,
    pub max_records: usize,
}

pub struct TrustRecord {
    pub timestamp: String,
    pub recommendation_id: String,
    pub trust_score: f32,
    pub was_correct: bool,
    pub outcome: String,
}

impl TrustModel {
    pub fn record_outcome(&mut self, recommendation_id: &str, was_correct: bool) {
        self.history.record(TrustRecord {
            timestamp: chrono::Local::now().to_rfc3339(),
            recommendation_id: recommendation_id.to_string(),
            trust_score: self.last_score_for(recommendation_id),
            was_correct,
            outcome: if was_correct { "approved" }.to_string(),
        });
    }

    pub fn get_historical_accuracy(&self, recommendation_type: &str) -> f32 {
        let relevant: Vec<&TrustRecord> = self.history.records.iter()
            .filter(|r| r.recommendation_type == recommendation_type)
            .collect();

        if relevant.is_empty() {
            return 0.5; // Default to neutral
        }

        relevant.iter().filter(|r| r.was_correct).count() as f32 / relevant.len() as f32
    }
}
```

---

## 7. Trait Contract

```rust
pub trait TrustModelTrait: Send + Sync {
    /// Score a recommendation
    fn score(&self, recommendation: &Recommendation) -> TrustScore;

    /// Generate a human-readable explanation
    fn explain(&self, score: &TrustScore) -> String;

    /// Record the outcome of a recommendation
    fn record_outcome(&mut self, recommendation_id: &str, was_correct: bool);

    /// Get historical accuracy for a recommendation type
    fn get_historical_accuracy(&self, recommendation_type: &str) -> f32;

    /// Get trust history
    fn get_history(&self) -> &[TrustRecord];

    /// Get the recommended action for a score
    fn get_recommended_action(&self, score: &TrustScore) -> RecommendedAction;

    /// Check if a recommendation requires explicit approval
    fn requires_approval(&self, score: &TrustScore) -> bool;
}
```

---

## 8. TUI Integration

### 8.1 Trust Display on Recommendations

Every recommendation in the TUI shows its trust score:

```
┌─────────────────────────────────────────────┐
│  RECOMMENDATION                             │
├─────────────────────────────────────────────┤
│  Switch language preference to Rust         │
│  ─────────────────────────────────          │
│  Trust: ████████████ 87% (High)             │
│  Confidence: 85%  Evidence: Strong          │
│  Cost Impact: None  Reversible: Yes         │
│                                             │
│  Reasoning: You mentioned "mostly Rust"     │
│  Evidence: 3 recent Rust tasks              │
│                                             │
│  [Approve]  [Reject]  [Details]             │
└─────────────────────────────────────────────┘
```

### 8.2 Low-Trust Warning

When trust is below 0.4, a warning is prominently displayed:

```
⚠ LOW TRUST (32%)
  This recommendation has weak evidence and high risk.
  Review carefully before approving.
```

---

## 9. Anti-Patterns

```rust
// NEVER: Auto-approve a recommendation with trust < 0.6
// ALWAYS: Require explicit approval for low-trust recommendations

// NEVER: Hide trust scores from the user
// ALWAYS: Display trust score on every recommendation

// NEVER: Manipulate trust scores to force approval
// ALWAYS: Calculate trust scores deterministically
```

---

## 10. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [RECOMMENDATION_ENGINE_SPEC.md](./RECOMMENDATION_ENGINE_SPEC.md)
- [COST_POLICY.md](./COST_POLICY.md)

---

## 11. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

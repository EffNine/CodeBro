# Intent Engine — P6 Design Specification

**Document:** `docs/design/INTENT_ENGINE_SPEC.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Intent Engine converts natural language statements into structured preference updates. It is the bridge between how developers speak and how CodeBro stores preferences.

**Key principle:** The Intent Engine does NOT execute changes. It generates `IntentUpdate` structs that flow through the Approval Gate.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                      Intent Engine                            │
│                                                               │
│  ┌─────────────────┐    ┌─────────────────┐                  │
│  │  Input Parser   │ →  │  Intent         │                  │
│  │  (tokenizer)    │    │  Classifier     │                  │
│  └─────────────────┘    └────────┬────────┘                  │
│                                  │                            │
│                          ┌───────▼────────┐                  │
│                          │  Intent        │                  │
│                          │  Disambiguator │                  │
│                          └───────┬────────┘                  │
│                                  │                            │
│                          ┌───────▼────────┐                  │
│                          │  Confidence    │                  │
│                          │  Scorer        │                  │
│                          └───────┬────────┘                  │
│                                  │                            │
│                    ┌─────────────▼──────────────┐            │
│                    │  Output: Vec<IntentUpdate> │            │
│                    │  + confidence score        │            │
│                    └────────────────────────────┘            │
│                                                               │
│  ┌─────────────────────────────────────────────────────┐     │
│  │              Historical Context                     │     │
│  │  (previous intents, approved changes,               │     │
│  │   rejected intents — used for disambiguation)       │     │
│  └─────────────────────────────────────────────────────┘     │
└───────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    Approval Gate
                    (requires user confirmation)
```

---

## 3. Intent Classification

The Intent Engine uses a deterministic rule-based classifier. No LLM is used for intent parsing.

### 3.1 Intent Types

| Intent Type | Trigger Patterns | Resulting Action |
|-------------|-----------------|------------------|
| **SetCodingPreference** | "I prefer idiomatic code", "always add comments", "I like minimal style" | Update `coding.*` |
| **SetCostPreference** | "I want to spend less", "limit my daily cost to $X", "use cheaper models" | Update `cost.*` |
| **SetProviderPreference** | "use Claude for reviews", "prefer local models", "always use DeepSeek" | Update `provider.*` |
| **SetLanguagePreference** | "I mostly write Rust", "I work in Python", "my language is Go" | Update `language.*` |
| **SetWorkflowPreference** | "always ask before installing", "show me diffs", "run tests before commit" | Update `workflow.*` |
| **SwitchProfile** | "switch to review mode", "I'm in research mode", "use low-cost profile" | Change active profile |
| **SetModelOverride** | "use Claude for planning", "use GPT for coding" | Add to `provider.role_overrides` |
| **SetCostLimit** | "my daily budget is $10", "don't spend more than $5/day" | Update `cost.daily_limit_usd` |

### 3.2 Pattern Matching Rules

```rust
// Example: Cost preference detection
fn detect_cost_intent(text: &str) -> Option<IntentUpdate> {
    let lower = text.to_lowercase();

    // Pattern: "spend less" / "reduce cost" / "cheaper models"
    if lower.contains("spend less") || lower.contains("reduce cost")
        || lower.contains("cheaper models") || lower.contains("lower cost") {
        return Some(IntentUpdate {
            action: IntentAction::SetPreference {
                key: "cost.preferred_tier".to_string(),
                value: "minimal".to_string(),
            },
            confidence: 0.7,
        });
    }

    // Pattern: "limit to $X" / "daily budget is $X"
    if let Some(amount) = extract_dollar_amount(&lower) {
        return Some(IntentUpdate {
            action: IntentAction::SetCostLimit { daily_limit: amount },
            confidence: 0.9,
        });
    }

    None
}

// Example: Language preference detection
fn detect_language_intent(text: &str) -> Option<IntentUpdate> {
    let lower = text.to_lowercase();

    // Pattern: "I mostly write X" / "I work in X" / "my language is X"
    for lang in ["rust", "python", "javascript", "typescript", "go", "java", "c++", "c"] {
        if lower.contains(&format!("write {}", lang))
            || lower.contains(&format!("work in {}", lang))
            || lower.contains(&format!("language is {}", lang))
            || lower.contains(&format!("mostly {}", lang)) {
            return Some(IntentUpdate {
                action: IntentAction::SetPreference {
                    key: "language.primary_language".to_string(),
                    value: lang.to_string(),
                },
                confidence: 0.85,
            });
        }
    }
    None
}
```

---

## 4. Confidence Scoring

Each intent update carries a confidence score (0.0–1.0) based on:

| Factor | Weight | Description |
|--------|--------|-------------|
| Pattern match clarity | 0.4 | Exact phrase match = 1.0, partial = 0.5 |
| Context consistency | 0.3 | Matches previous intents = +0.2 |
| Historical approval rate | 0.2 | Similar intents previously approved = +0.1 |
| Ambiguity penalty | -0.3 | Multiple possible interpretations |

**Rule:** If confidence < 0.5, the intent is dropped silently (no recommendation generated).
**Rule:** If confidence >= 0.5 and < 0.8, the intent is presented as a low-confidence suggestion.
**Rule:** If confidence >= 0.8, the intent is presented as a high-confidence recommendation.

---

## 5. Disambiguation

When multiple intents match the same input, the engine resolves conflicts:

```rust
fn resolve_conflicts(intents: Vec<IntentUpdate>) -> Vec<IntentUpdate> {
    // Group by action type
    // If same key has different values, keep highest confidence
    // If different keys, keep all
    // If same key, same value, deduplicate
}
```

### 5.1 Disambiguation Examples

| Input | Conflict | Resolution |
|-------|----------|------------|
| "I write Rust but sometimes Python" | Two language preferences | Set primary=Rust, add Python to additional |
| "Use Claude for reviews, but also for coding" | Two model overrides | Both are kept (different roles) |
| "Spend less money" | Vague cost preference | Set preferred_tier=minimal (most conservative) |
| "Always ask before doing anything" | Broad workflow preference | Set ask_before_install=true, ask_before_command=true |

---

## 6. Trait Contract

```rust
pub trait IntentEngineTrait: Send + Sync {
    /// Parse natural language into one or more IntentUpdates
    fn parse_intent(&self, natural_language: &str) -> Vec<IntentUpdate>;

    /// Suggest preferences based on current context and conversation history
    fn suggest_preferences(&self, context: &IntentContext) -> Vec<PreferenceSuggestion>;

    /// Apply an intent update (returns event for TUI, does not mutate state)
    fn apply_intent(&mut self, intent: &IntentUpdate) -> Result<AdaptiveEvent>;

    /// Get the confidence threshold for auto-approving intents
    fn get_confidence_threshold(&self) -> f32;

    /// Set the confidence threshold
    fn set_confidence_threshold(&mut self, threshold: f32);

    /// Get intent history
    fn get_history(&self) -> &[ParsedIntent];
}

pub struct IntentContext {
    pub current_message: String,
    pub recent_messages: Vec<String>,
    pub current_preferences: HashMap<String, String>,
    pub active_profile: Option<String>,
}

pub struct PreferenceSuggestion {
    pub key: String,
    pub suggested_value: String,
    pub reasoning: String,
    pub confidence: f32,
}

pub struct ParsedIntent {
    pub timestamp: String,
    pub input: String,
    pub updates: Vec<IntentUpdate>,
    pub confidence: f32,
    pub approved: bool,
}
```

---

## 7. Processing Flow

```
User types: "I mostly write Rust and I want to reduce costs"
        ↓
Input Parser (tokenize, lowercase, strip punctuation)
        ↓
Intent Classifier (match against rules)
        ↓
  ├─ detect_language_intent → SetPreference(language.primary_language, "rust"), confidence: 0.85
  ├─ detect_cost_intent → SetPreference(cost.preferred_tier, "minimal"), confidence: 0.7
  └─ detect_cost_limit → (no dollar amount found)
        ↓
Confidence Scorer
        ↓
  ├─ Update 1: confidence 0.85 → HIGH
  └─ Update 2: confidence 0.70 → MEDIUM
        ↓
Disambiguator (no conflicts)
        ↓
Output: [IntentUpdate(language.primary_language, "rust", 0.85),
        IntentUpdate(cost.preferred_tier, "minimal", 0.70)]
        ↓
Recommendation Engine (packages into Recommendation)
        ↓
Approval Gate (user approves both)
        ↓
Preference Engine (writes both updates)
        ↓
Learning Policy (records the approved intent)
```

---

## 8. TUI Integration

### 8.1 View: `/intent`

The intent view shows:
- Recent natural language inputs
- Parsed intent updates
- Confidence scores
- Approval status

### 8.2 Real-Time Feedback

When the user types in the input area, the Intent Engine can optionally show a preview:

```
> "I mostly write Rust"
  ↳ Detected: Set language.primary_language = "rust" (confidence: 0.85)
     [Preview — will apply when you submit]
```

This preview is purely informational; no state change occurs until the user submits.

---

## 9. Anti-Patterns

```rust
// NEVER: Use an LLM to parse intent
// The Intent Engine is deterministic and requires no external calls

// NEVER: Auto-apply intents without approval
// All intents flow through the Approval Gate

// NEVER: Store raw natural language in preferences
// Only structured IntentUpdates are persisted
```

---

## 10. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [USER_PREFERENCE_MODEL.md](./USER_PREFERENCE_MODEL.md)
- [RECOMMENDATION_ENGINE_SPEC.md](./RECOMMENDATION_ENGINE_SPEC.md)

---

## 11. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

# Adaptive Developer Platform — P6 Design Specification

**Document:** `docs/design/ADAPTIVE_PLATFORM_SPEC.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Executive Summary

This document defines the architecture of the Adaptive Developer Platform that underpins P6 through P9. It describes twelve subsystems that together enable CodeBro to adapt to the developer — not the reverse.

**Core thesis:** Every adaptive action must remain transparent and require explicit user approval whenever it changes behavior, configuration, or cost.

The platform is **not autonomous**. It observes, recommends, and waits. The developer is always in the loop.

---

## 2. Design Principles (P6)

Every subsystem defined herein satisfies these principles:

| Principle | Statement |
|-----------|-----------|
| Zero Configuration | Defaults are sensible; explicit overrides are optional |
| Developer First | The developer's intent drives all adaptive behavior |
| Human in Control | No behavioral change without explicit approval |
| Cost Transparency | Every model switch shows cost impact before execution |
| Progressive Discovery | Adaptive features are discoverable, not intrusive |
| Observable AI | Every adaptive decision is logged and visible |
| Adaptive, Not Autonomous | The platform suggests; the developer decides |
| Platform before Features | Foundation subsystems precede capability additions |
| Deterministic before AI | Rule-based behavior takes precedence over probabilistic |
| Everything Manageable from TUI | No config file editing required for any adaptive setting |

---

## 3. Platform Architecture Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                         Adaptive Developer Platform                     │
│                                                                         │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐  ┌────────────┐ │
│  │ Preference    │  │ Intent       │  │ Recommendation│  │ Workflow   │ │
│  │ Engine       │  │ Engine       │  │ Engine       │  │ Engine     │ │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └─────┬──────┘ │
│         │                 │                 │                │         │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌──────▼───────┐  ┌─────▼──────┐ │
│  │ Profile      │  │ Subagent     │  │ Model        │  │ Cost       │ │
│  │ Engine       │  │ Orchestrator │  │ Routing      │  │ Policy     │ │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘  └─────┬──────┘ │
│         │                 │                 │                │         │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌──────▼───────┐  ┌─────▼──────┐ │
│  │ MCP          │  │ Skill        │  │ Learning     │  │ Trust      │ │
│  │ Lifecycle    │  │ Lifecycle    │  │ Policy       │  │ Model      │ │
│  └──────────────┘  └──────────────┘  └──────────────┘  └────────────┘ │
│                                                                         │
│                    All subsystems expose TUI panels                     │
│                    All actions require explicit approval                │
│                    All state is persisted and observable                │
└─────────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────────┐
│                        Existing Platform (P0–P5)                        │
│                                                                         │
│   TUI  ←→  Agent Coordinator  ←→  Tools/Executor  ←→  Providers        │
│                                                                         │
│   Memory  ←→  Skill Manager  ←→  Experience Replay  ←→  Decision Engine │
└─────────────────────────────────────────────────────────────────────────┘
```

---

## 4. Module Placement

The Adaptive Developer Platform introduces a new top-level module: `src/adaptive/`

```
src/
├── adaptive/                    # NEW — P6 Adaptive Platform
│   ├── mod.rs
│   ├── preference.rs            # Preference Engine
│   ├── intent.rs                # Intent Engine
│   ├── recommendation.rs        # Recommendation Engine
│   ├── workflow.rs              # Workflow Engine
│   ├── profile.rs               # Profile Engine
│   ├── orchestrator.rs          # Subagent Orchestrator
│   ├── routing.rs               # Model Routing Policy
│   ├── cost.rs                  # Cost Policy
│   ├── mcp_lifecycle.rs         # MCP Lifecycle
│   ├── skill_lifecycle.rs       # Skill Lifecycle
│   ├── learning.rs              # Learning Policy
│   ├── trust.rs                 # Trust Model
│   └── types.rs                 # Shared types and traits
├── agent/                       # Existing — frozen
├── tools/                       # Existing — frozen
├── providers/                   # Existing — frozen
├── tui/                         # Existing — extended with adaptive panels
├── config/                      # Existing — extended with adaptive fields
└── settings/                    # Existing — extended with adaptive settings
```

### 4.1 Hard Boundaries (Adapted from Architecture Manifest)

| Boundary | Rule | Rationale |
|----------|------|-----------|
| `adaptive/` → `agent/` | Adaptive subsystems may emit `AdaptiveEvent` but may not call agent logic directly | Separation of concerns; agent logic is testable without adaptive |
| `agent/` → `adaptive/` | Agent may read preferences from `PreferenceEngine` via trait but may not depend on adaptive internals | Agent must remain functional without adaptive |
| `adaptive/` → `tools/` | Adaptive subsystems may not execute tools directly | Tools are synchronous; adaptive is observational |
| `adaptive/` → `providers/` | Adaptive subsystems may not call providers directly | Provider abstraction is sole interface to LLM |
| `tui/` → `adaptive/` | TUI may read adaptive state via traits but may not mutate directly | TUI is display-only; mutations go through approval gates |
| `config/` → `adaptive/` | Config may not depend on adaptive | Config is loaded before adaptive initialization |

### 4.2 Permitted Data Flow

```
User Input / Natural Language
        ↓
   Preference Engine (read current preferences)
        ↓
   Intent Engine (parse intent → update preferences)
        ↓
   Profile Engine (select matching profile)
        ↓
   Recommendation Engine (generate recommendations)
        ↓
   Cost Policy (evaluate cost impact)
        ↓
   Trust Model (assess confidence & risk)
        ↓
   Approval Gate (TUI presents to user)
        ↓
   User Approval
        ↓
   Subagent Orchestrator (route with selected models)
        ↓
   Existing Pipeline (tools → providers → response)
```

---

## 5. Event System Extension

A new event type crosses the adaptive/TUI boundary:

```rust
pub enum AdaptiveEvent {
    PreferenceChanged { key: String, old: String, new: String },
    IntentDetected { intent: String, confidence: f32 },
    RecommendationReady { recommendation: Recommendation },
    WorkflowSuggestion { workflow: WorkflowSuggestion },
    ProfileSwitched { from: String, to: String },
    ModelRoutingSuggested { task: String, from_model: String, to_model: String, cost_delta: f64 },
    CostWarning { message: String, current_cost: f64, proposed_cost: f64 },
    McpRecommendation { mcp: McpRecommendation },
    SkillRecommendation { skill: SkillRecommendation },
    TrustAssessment { item: String, trust_score: f32, reasoning: String },
    LearningRecorded { type_: String, source: String },
}

pub struct AdaptiveEventBus {
    tx: mpsc::Sender<AdaptiveEvent>,
    rx: mpsc::Receiver<AdaptiveEvent>,
}
```

`AdaptiveEvent` is the only event type that crosses the `adaptive/` → `tui/` boundary. The TUI listens to `AppEvent::AdaptiveEvent(AdaptiveEvent)`.

---

## 6. Persistence

All adaptive state is persisted to `~/.codebro/adaptive/`:

```
~/.codebro/
├── config.toml              # Existing config
├── memory.json              # Existing memory
├── experiences.json         # Existing experiences
├── skills/                  # Existing skills
└── adaptive/                # NEW — P6 adaptive state
    ├── preferences.json     # Developer preferences
    ├── intents.json         # Parsed intent history
    ├── recommendations.json # Recommendation history
    ├── workflows.json       # Detected workflow patterns
    ├── profiles.json        # User profiles
    ├── routing.json         # Model routing policies
    ├── cost_log.json        # Cost tracking log
    ├── mcp_registry.json    # MCP discovery & approval state
    ├── skill_registry.json  # Skill lifecycle state
    ├── learning_log.json    # Learning policy audit trail
    └── trust_log.json       # Trust scoring history
```

Each file is JSON-serialized, pretty-printed, and human-readable. No binary formats.

---

## 7. Approval Gate Architecture

Every adaptive action that changes behavior, configuration, or cost must pass through the **Approval Gate**:

```
┌─────────────────────────────────────────────────────┐
│                   Approval Gate                     │
│                                                     │
│  1. Adaptive subsystem generates recommendation     │
│  2. Trust Model scores the recommendation           │
│  3. Cost Policy evaluates cost impact               │
│  4. Recommendation is packaged with:                │
│     - confidence (0.0–1.0)                          │
│     - reasoning (natural language)                  │
│     - evidence (supporting data)                    │
│     - cost_delta (estimated cost change)            │
│     - benefit (expected outcome)                    │
│     - reversibility (can this be undone?)           │
│  5. TUI presents recommendation to user             │
│  6. User approves or rejects                        │
│  7. If approved: preference/intent is updated       │
│  8. Learning Policy records the decision            │
└─────────────────────────────────────────────────────┘
```

**Rule:** If `reversibility == false` and `confidence < 0.9`, the recommendation MUST require explicit confirmation — no silent defaults.

---

## 8. Trait Abstractions

Each subsystem exposes a formal trait, following ADR-008 patterns:

| Trait | Module | Purpose |
|-------|--------|---------|
| `PreferenceEngineTrait` | `preference` | Read/write developer preferences |
| `IntentEngineTrait` | `intent` | Parse natural language into preference updates |
| `RecommendationEngineTrait` | `recommendation` | Generate actionable recommendations |
| `WorkflowEngineTrait` | `workflow` | Observe and suggest workflow patterns |
| `ProfileEngineTrait` | `profile` | Manage developer profiles |
| `OrchestratorTrait` | `orchestrator` | Route subagents with model overrides |
| `ModelRoutingTrait` | `routing` | Determine model selection policy |
| `CostPolicyTrait` | `cost` | Track and warn about cost changes |
| `McPLifecycleTrait` | `mcp_lifecycle` | Manage MCP server discovery and installation |
| `SkillLifecycleTrait` | `skill_lifecycle` | Manage skill discovery and adoption |
| `LearningPolicyTrait` | `learning` | Control what CodeBro may learn |
| `TrustModelTrait` | `trust` | Score and explain recommendation trustworthiness |

### 8.1 PreferenceEngineTrait

```rust
pub trait PreferenceEngineTrait: Send + Sync {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&mut self, key: &str, value: &str) -> Result<()>;
    fn get_all(&self) -> HashMap<String, String>;
    fn has_changed(&self, key: &str, new_value: &str) -> bool;
    fn save(&self) -> Result<()>;
    fn load(&mut self) -> Result<()>;
}
```

### 8.2 IntentEngineTrait

```rust
pub trait IntentEngineTrait: Send + Sync {
    fn parse_intent(&self, natural_language: &str) -> Vec<IntentUpdate>;
    fn suggest_preferences(&self, context: &IntentContext) -> Vec<PreferenceSuggestion>;
    fn apply_intent(&mut self, intent: &IntentUpdate) -> Result<AdaptiveEvent>;
}

pub struct IntentUpdate {
    pub action: IntentAction,
    pub target: String,
    pub value: String,
    pub confidence: f32,
}

pub enum IntentAction {
    SetPreference { key: String, value: String },
    AddProfile { profile: Profile },
    SwitchProfile { from: String, to: String },
    SetModelOverride { role: String, model: String },
    SetCostLimit { daily_limit: f64 },
}
```

### 8.3 RecommendationEngineTrait

```rust
pub trait RecommendationEngineTrait: Send + Sync {
    fn generate(&self, context: &RecommendationContext) -> Vec<Recommendation>;
    fn get_history(&self) -> Vec<TrackedRecommendation>;
}

pub struct Recommendation {
    pub id: String,
    pub title: String,
    pub body: String,
    pub confidence: f32,
    pub reasoning: String,
    pub evidence: Vec<String>,
    pub cost_impact: Option<CostImpact>,
    pub expected_benefit: String,
    pub reversibility: Reversibility,
    pub required_approval: bool,
}

pub enum Reversibility {
    FullyReversible,
    PartiallyReversible,
    Irreversible,
}
```

### 8.4 TrustModelTrait

```rust
pub trait TrustModelTrait: Send + Sync {
    fn score(&self, recommendation: &Recommendation) -> TrustScore;
    fn explain(&self, score: &TrustScore) -> String;
    fn record_outcome(&mut self, recommendation_id: &str, was_correct: bool);
}

pub struct TrustScore {
    pub confidence: f32,
    pub evidence_strength: f32,
    pub cost_risk: f32,
    pub reversibility_factor: f32,
    pub composite: f32,
}
```

---

## 9. TUI Integration

The TUI extends its existing event loop and panel system:

### 9.1 New Event Variant

```rust
pub enum AppEvent {
    // ... existing variants ...
    AdaptiveEvent(AdaptiveEvent),
}
```

### 9.2 New Panels

| Panel | Trigger | Content |
|-------|---------|---------|
| Recommendations | Incoming `RecommendationReady` | Title, confidence bar, reasoning, approve/reject buttons |
| Cost Warnings | Incoming `CostWarning` | Current vs. proposed cost, approval required |
| Workflow Suggestions | Incoming `WorkflowSuggestion` | Detected pattern, proposed automation |
| Profile Switch | Incoming `ProfileSwitched` | Current profile, available profiles |
| Trust Alerts | Incoming `TrustAssessment` | Low-trust warning with explanation |
| MCP/Skill Recommendations | Incoming `McpRecommendation` / `SkillRecommendation` | Discovery results, install button |

### 9.3 Command Palette Integration

New slash commands accessible from the command palette:

| Command | Description |
|---------|-------------|
| `/profile` | View and switch profiles |
| `/preferences` | View and edit preferences |
| `/routing` | View model routing policy |
| `/cost` | View cost history and limits |
| `/workflows` | View detected workflow patterns |
| `/trust` | View trust scores for recent recommendations |
| `/mcp` | Manage MCP servers |
| `/intent` | Express an intent in natural language |

---

## 10. Learning Policy

### 10.1 Allowed Learning

| Category | What is Learned | Storage |
|----------|----------------|---------|
| Preferences | Explicitly set or approved preference changes | `preferences.json` |
| Accepted Recommendations | Which recommendations were accepted | `learning_log.json` |
| Rejected Recommendations | Which were rejected (to avoid repetition) | `learning_log.json` |
| Workflow Patterns | Repeated action sequences | `workflows.json` |
| Intent Updates | Natural language → preference mappings | `intents.json` |

### 10.2 Forbidden Learning

| Category | Reason |
|----------|--------|
| Silently changing behavior | Violates "Human in Control" |
| Silently changing models | Violates "Cost Transparency" |
| Silently installing integrations | Violates "Human in Control" |
| Silently editing configuration | Violates "Explicit over Implicit" |
| Learning from failed recommendations | Failed recommendations are noise, not signal |
| Learning from rejected intents | User rejection is a signal to stop, not continue |

### 10.3 Learning Audit Trail

Every learning event is recorded:

```rust
pub struct LearningRecord {
    pub timestamp: String,
    pub category: LearningCategory,
    pub source: String,
    pub action_taken: String,
    pub user_approved: bool,
    pub reversibility: Reversibility,
}
```

The audit trail is exposed in the TUI under `/trust`.

---

## 11. Risk Assessment

### 11.1 High-Risk Subsystems

| Subsystem | Risk | Mitigation |
|-----------|------|------------|
| Intent Engine | May misparse natural language, leading to unwanted preference changes | All intent-driven changes require approval; low-confidence intents are ignored |
| Model Routing | May silently switch to a more expensive model | Cost Policy intercepts all model changes; approval required before switch |
| MCP Lifecycle | May install untrusted MCP servers | Discovery requires approval; installation requires approval; validation before enablement |
| Cost Policy | May fail to track cost accurately | Conservative defaults; daily budget alerts; hard cap enforcement |

### 11.2 Anti-Patterns (Forbidden)

```rust
// NEVER: Auto-apply intent without approval
intent_engine.apply_intent(intent, ApprovalMode::Auto);

// ALWAYS: Require explicit approval
if intent_engine.requires_approval(intent) {
    tui.show_approval_dialog(intent);
}
```

---

## 12. Phase Boundaries

| Phase | Adaptive Platform Scope |
|-------|------------------------|
| P6 | Design this specification; implement Preference, Intent, Recommendation, Trust, Cost Policy, and Learning Policy |
| P7 | Implement Workflow, Profile, Subagent Orchestrator, and Model Routing |
| P8 | Implement MCP and Skill Lifecycle management |
| P9 | Integration hardening, UI polish, performance optimization |

---

## 13. References

- [Architecture Manifest v1.0](../architecture/architecture_manifest_v1.md)
- [ADR-008: Intelligence Platform Architecture](../ADR/adr-008-intelligence-platform-architecture.md)
- [Design Principles](../principles/design_principles.md)
- [Configuration Model](../vision/CONFIGURATION_MODEL.md)
- [SOP v1.0](../SOP/codebro_sop_v1.md)

---

## 14. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

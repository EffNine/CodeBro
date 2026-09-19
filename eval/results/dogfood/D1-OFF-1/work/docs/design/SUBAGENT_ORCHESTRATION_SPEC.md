# Subagent Orchestrator — P6 Design Specification

**Document:** `docs/design/SUBAGENT_ORCHESTRATION_SPEC.md`
**Version:** 1.0.0
**Phase:** P6 — Adaptive Intelligence
**Status:** Proposed — Design Summit
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Purpose

The Subagent Orchestrator extends the existing `AgentCoordinator` with adaptive model routing capabilities. It determines which model each subagent role uses, while respecting user preferences and cost policies.

**Key principle:** All subagents inherit the primary model by default. Users may optionally override each role — but only through explicit TUI interaction or approved recommendations.

---

## 2. Architecture

```
┌───────────────────────────────────────────────────────────────┐
│                   Subagent Orchestrator                       │
│                                                               │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │              Existing AgentCoordinator                  │ │
│  │  (task graph, parallel execution, event bus)            │ │
│  └──────────────────────┬──────────────────────────────────┘ │
│                         │                                     │
│  ┌──────────────────────▼──────────────────────────────────┐ │
│  │              Role Router                                │ │
│  │                                                         │ │
│  │  ┌──────────┐  ┌──────────┐  ┌──────────┐             │ │
│  │  │ Planner  │  │ Reviewer │  │ Research │             │ │
│  │  │ Role     │  │ Role     │  │ Role     │             │ │
│  │  └────┬─────┘  └────┬─────┘  └────┬─────┘             │ │
│  │       │             │             │                    │ │
│  │  ┌────▼─────┐  ┌────▼─────┐  ┌───▼─────┐             │ │
│  │  │Coder Role│  │ Debugger │  │  ...     │             │ │
│  │  └──────────┘  └──────────┘  └──────────┘             │ │
│  └──────────────────────┬──────────────────────────────────┘ │
│                         │                                     │
│  ┌──────────────────────▼──────────────────────────────────┐ │
│  │              Model Resolver                             │ │
│  │  1. Check role-specific override                        │ │
│  │  2. Check profile overrides                             │ │
│  │  3. Fall back to primary model                          │ │
│  └──────────────────────┬──────────────────────────────────┘ │
│                         │                                     │
│              ┌──────────┴──────────┐                         │
│              ▼                     ▼                          │
│     ┌──────────────┐      ┌──────────────┐                  │
│     │ Cost Policy  │      │ Trust Model  │                  │
│     │ (check cost  │      │ (check trust  │                  │
│     │  impact)     │      │  of switch)  │                  │
│     └──────────────┘      └──────────────┘                  │
└───────────────────────────────────────────────────────────────┘
```

---

## 3. Role Definitions

The orchestrator manages the following agent roles:

| Role | Purpose | Default Behavior |
|------|---------|-----------------|
| `planner` | Task decomposition and planning | Inherits primary model |
| `researcher` | Code analysis and research | Inherits primary model |
| `architect` | Architecture decisions and design | Inherits primary model |
| `implementer` | Code generation and editing | Inherits primary model |
| `reviewer` | Code review and quality assessment | Inherits primary model |
| `debugger` | Bug analysis and fix proposals | Inherits primary model |
| `tester` | Test generation and execution | Inherits primary model |

### 3.1 Role-to-Subagent Mapping

| Role | Existing Subagent |
|------|-------------------|
| `researcher` | `ResearchAgent` |
| `planner` | `PlanningAgent` |
| `implementer` | `CodingAgent` |
| `reviewer` | `ReviewAgent` |
| `tester` | `TestingAgent` |

`architect` and `debugger` are new roles that may be implemented as subagents or handled by the main coordinator.

---

## 4. Model Resolution Algorithm

```rust
fn resolve_model_for_role(
    &self,
    role: &str,
    primary_model: &str,
    preferences: &Preferences,
    active_profile: Option<&Profile>,
) -> ResolvedModel {
    // 1. Check role-specific override in preferences
    if let Some(override_model) = preferences.provider.role_overrides.get(role) {
        return ResolvedModel {
            model: override_model.clone(),
            source: ModelSource::PreferenceOverride,
            requires_approval: false,
        };
    }

    // 2. Check profile model overrides
    if let Some(profile) = active_profile {
        if let Some(override_model) = profile.model_overrides.get(role) {
            return ResolvedModel {
                model: override_model.clone(),
                source: ModelSource::ProfileOverride,
                requires_approval: false,
            };
        }
    }

    // 3. Fall back to primary model
    ResolvedModel {
        model: primary_model.to_string(),
        source: ModelSource::Default,
        requires_approval: false,
    }
}
```

---

## 5. Model Override Flow

### 5.1 Setting a Role Override

When a user sets a model override for a role (via TUI, intent, or approved recommendation):

```
User: "Use Claude for reviews"
        ↓
Intent Engine → IntentUpdate { action: SetModelOverride { role: "reviewer", model: "claude-sonnet-4" } }
        ↓
Recommendation Engine packages as Recommendation
        ↓
Approval Gate (user approves)
        ↓
Preference Engine writes to provider.role_overrides["reviewer"] = "claude-sonnet-4"
        ↓
Cost Policy evaluates cost impact of switching reviewer to a different model
        ↓
AdaptiveEvent::ModelRoutingSuggested emitted to TUI
```

### 5.2 Dynamic Override During Task

If a task requires a different model than the current role override:

```
Task: "Review this PR"
        ↓
Orchestrator resolves reviewer model
        ↓
Cost Policy checks: proposed cost vs. current cost
        ↓
If cost delta > threshold → Recommendation: "Switch reviewer to Claude Sonnet 4 (est. +$0.50)"
        ↓
User approval required
```

---

## 6. Trait Contract

```rust
pub trait OrchestratorTrait: Send + Sync {
    /// Get the configured model for a role
    fn resolve_model(&self, role: &str, primary_model: &str) -> ResolvedModel;

    /// Set a model override for a role (requires approval)
    fn set_role_override(&mut self, role: &str, model: &str) -> Result<AdaptiveEvent>;

    /// Clear a model override for a role
    fn clear_role_override(&mut self, role: &str) -> Result<AdaptiveEvent>;

    /// Get all role overrides
    fn get_role_overrides(&self) -> &HashMap<String, String>;

    /// Run a task with the resolved model for each role
    fn run_with_resolved_models(
        &self,
        task: &str,
        primary_model: &str,
    ) -> Result<TaskResult>;

    /// Estimate the cost of using resolved models for a task
    fn estimate_task_cost(&self, task: &str, primary_model: &str) -> CostEstimate;
}

pub struct ResolvedModel {
    pub model: String,
    pub source: ModelSource,
    pub requires_approval: bool,
}

pub enum ModelSource {
    Default,              // Primary model
    PreferenceOverride,   // Set in preferences
    ProfileOverride,      // Set in active profile
    Recommendation,       // Applied from approved recommendation
}

pub struct CostEstimate {
    pub primary_model_cost: f64,
    pub resolved_model_cost: f64,
    pub delta: f64,
    pub per_role: Vec<RoleCost>,
}

pub struct RoleCost {
    pub role: String,
    pub model: String,
    pub estimated_cost: f64,
}
```

---

## 7. Integration with Existing AgentCoordinator

The Subagent Orchestrator wraps the existing `AgentCoordinator`:

```rust
pub struct AdaptiveOrchestrator {
    coordinator: AgentCoordinator,
    preference_engine: Box<dyn PreferenceEngineTrait>,
    cost_policy: Box<dyn CostPolicyTrait>,
    trust_model: Box<dyn TrustModelTrait>,
    profile_engine: Box<dyn ProfileEngineTrait>,
}

impl AdaptiveOrchestrator {
    pub fn new(...) -> Self {
        // Initialize with existing coordinator
        // Add adaptive layer on top
    }

    pub async fn run_task(&self, task: &str) -> Result<TaskResult> {
        // 1. Resolve models for all roles
        // 2. Check cost impact
        // 3. If cost change > threshold, emit recommendation
        // 4. Run through existing coordinator
        // 5. Record outcomes
    }
}
```

---

## 8. TUI Integration

### 8.1 View: `/routing`

```
┌─────────────────────────────────────────────┐
│  MODEL ROUTING                              │
├─────────────────────────────────────────────┤
│  Primary Model: gpt-4o                      │
│                                             │
│  Role Overrides:                            │
│  ─────────────────────────────────          │
│  planner    → gpt-4o         (default)      │
│  researcher → gpt-4o         (default)      │
│  architect  → gpt-4o         (default)      │
│  implementer→ gpt-4o         (default)      │
│  reviewer   → claude-sonnet-4  (override)   │
│  debugger   → gpt-4o         (default)      │
│  tester     → gpt-4o         (default)      │
│                                             │
│  Active Profile: Coding                     │
│  Profile overrides: reviewer → claude-opus-4│
│                                             │
│  [Set Override]  [Clear]  [Close]           │
└─────────────────────────────────────────────┘
```

### 8.2 Title Bar Integration

The title bar shows the active model routing summary:

```
CODEBRO | WS: myproject | Model: gpt-4o | Reviewer: claude-sonnet-4 | Tools: ✓
```

---

## 9. Anti-Patterns

```rust
// NEVER: Allow a role override that doesn't exist in the provider
// ALWAYS: Validate model names against available models

// NEVER: Silently switch models during a task
// ALWAYS: Resolve models before task execution begins

// NEVER: Override a role with a model that exceeds cost limits
// ALWAYS: Run through Cost Policy before applying overrides
```

---

## 10. References

- [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md)
- [MODEL_ROUTING_POLICY.md](./MODEL_ROUTING_POLICY.md)
- [COST_POLICY.md](./COST_POLICY.md)
- [PROFILE_ENGINE_SPEC.md](./PROFILE_ENGINE_SPEC.md)

---

## 11. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

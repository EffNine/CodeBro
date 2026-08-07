# Design Documents — Adaptive Developer Platform

**Phase:** P6–P9
**Status:** Design Summit Complete
**Date:** 2026-08-06

---

## Overview

This directory contains the complete architectural blueprint for the Adaptive Developer Platform. These documents define the architecture for P6 through P9 and are **design-only** — no implementation code is included.

---

## Core Specifications (P6)

| Document | Subsystem | Summary |
|----------|-----------|---------|
| [ADAPTIVE_PLATFORM_SPEC.md](./ADAPTIVE_PLATFORM_SPEC.md) | Platform Overview | Architecture, module placement, event system, approval gate, persistence |
| [USER_PREFERENCE_MODEL.md](./USER_PREFERENCE_MODEL.md) | Preference Engine | Schema, validation, merge semantics, audit trail |
| [INTENT_ENGINE_SPEC.md](./INTENT_ENGINE_SPEC.md) | Intent Engine | Rule-based natural language classification, confidence scoring |
| [RECOMMENDATION_ENGINE_SPEC.md](./RECOMMENDATION_ENGINE_SPEC.md) | Recommendation Engine | Unified recommendation structure, ranking, deduplication |
| [WORKFLOW_ENGINE_SPEC.md](./WORKFLOW_ENGINE_SPEC.md) | Workflow Engine | Pattern detection, sliding window, suggestion generation |
| [PROFILE_ENGINE_SPEC.md](./PROFILE_ENGINE_SPEC.md) | Profile Engine | Built-in profiles, merge semantics, TUI management |
| [SUBAGENT_ORCHESTRATION_SPEC.md](./SUBAGENT_ORCHESTRATION_SPEC.md) | Subagent Orchestrator | Role-based model resolution, wrapper around AgentCoordinator |
| [MODEL_ROUTING_POLICY.md](./MODEL_ROUTING_POLICY.md) | Model Routing | 4 strategies, complexity classification, cost compliance |
| [COST_POLICY.md](./COST_POLICY.md) | Cost Policy | Tracking, limits, model comparison, enforcement |
| [TRUST_MODEL.md](./TRUST_MODEL.md) | Trust Model | Multi-factor scoring, explanations, historical accuracy |

---

## Extension Specifications (P7–P8)

| Document | Subsystem | Phase |
|----------|-----------|-------|
| [MCP_LIFECYCLE.md](./MCP_LIFECYCLE.md) | MCP Lifecycle | P8 |
| [SKILL_LIFECYCLE.md](./SKILL_LIFECYCLE.md) | Skill Lifecycle | P8 |

---

## Deliverables

| Document | Description |
|----------|-------------|
| [ARCHITECTURE_REVIEW.md](./ARCHITECTURE_REVIEW.md) | Subsystem-by-subsystem review with compliance assessment |
| [RISK_ASSESSMENT.md](./RISK_ASSESSMENT.md) | Risk register with severity, likelihood, and mitigation |
| [VISION_COMPLIANCE_REPORT.md](./VISION_COMPLIANCE_REPORT.md) | Verification against CodeBro vision statement (100% compliant) |
| [IMPLEMENTATION_ROADMAP.md](./IMPLEMENTATION_ROADMAP.md) | Phased implementation plan for P6–P9 |
| [GO_HOLD_RECOMMENDATION.md](./GO_HOLD_RECOMMENDATION.md) | GO with conditions recommendation |

---

## Key Design Decisions

1. **Approval Gate is mandatory** — Every adaptive action that changes behavior, configuration, or cost requires explicit user approval
2. **Deterministic before AI** — Intent Engine uses rule-based classification; no LLM calls in the adaptive path
3. **Trait-abstracted subsystems** — Each subsystem exposes a formal trait for testability and future swap-in
4. **JSON persistence** — All adaptive state is stored as human-readable JSON in `~/.codebro/adaptive/`
5. **Additive TUI changes** — New panels are added without modifying existing rendering logic
6. **Merge semantics for profiles** — Profile switches overlay preferences without replacing unrelated values

---

## Architecture Diagrams

### Adaptive Platform

```
┌─────────────────────────────────────────────────────────────────┐
│                      Adaptive Platform                          │
│                                                                 │
│  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌────────────────┐ │
│  │Preference│  │  Intent  │  │Recommended│ │  Workflow      │ │
│  │ Engine   │  │  Engine  │  │  Engine   │ │  Engine        │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └───────┬────────┘ │
│       │             │             │                │           │
│  ┌────▼─────┐  ┌────▼─────┐  ┌────▼─────┐  ┌───────▼────────┐ │
│  │ Profile  │  │Subagent  │  │  Model   │  │    Cost        │ │
│  │ Engine   │  │Orchestr. │  │ Routing  │  │   Policy       │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────┘  └───────┬────────┘ │
│       │             │             │                │           │
│  ┌────▼─────────────────────────────────────────────────────┐ │
│  │              Approval Gate (all paths)                    │ │
│  │    ┌─────────┐  ┌─────────┐  ┌─────────┐  ┌─────────┐   │ │
│  │    │  MCP    │  │  Skill  │  │ Learning │  │  Trust  │   │ │
│  │    │ Lifecycle│  │Lifecycle│  │  Policy  │  │  Model  │   │ │
│  │    └─────────┘  └─────────┘  └─────────┘  └─────────┘   │ │
│  └───────────────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                              │
                              ▼
                    ┌─────────────────┐
                    │ Existing P0-P5  │
                    │ Platform        │
                    └─────────────────┘
```

### Preference Flow

```
User: "I mostly write Rust"
    │
    ▼
Intent Engine (parse)
    │
    ▼
IntentUpdate { action: SetPreference, key: "language.primary_language", value: "rust" }
    │
    ▼
Recommendation Engine (package)
    │
    ▼
Trust Model (score)
    │
    ▼
Approval Gate (user confirms)
    │
    ▼
Preference Engine (write + audit)
    │
    ▼
AdaptiveEvent::PreferenceChanged → TUI notification
```

### Intent Flow

```
Natural Language Input
    │
    ▼
Input Parser (tokenize, lowercase)
    │
    ▼
Intent Classifier (rule matching)
    │
    ├──→ detect_language_intent()
    ├──→ detect_cost_intent()
    ├──→ detect_provider_intent()
    ├──→ detect_workflow_intent()
    └──→ detect_profile_intent()
    │
    ▼
Confidence Scorer
    │
    ├──→ confidence < 0.5: Drop silently
    ├──→ confidence 0.5-0.8: Low-confidence suggestion
    └──→ confidence >= 0.8: High-confidence recommendation
    │
    ▼
Disambiguator (resolve conflicts)
    │
    ▼
Output: Vec<IntentUpdate>
```

### Recommendation Flow

```
Subsystem Event (e.g., IntentDetected)
    │
    ▼
Recommendation Engine (generate)
    │
    ├──→ Check deduplication
    ├──→ Check cooldown
    ├──→ Score by relevance
    └──→ Package with evidence, reasoning, cost impact
    │
    ▼
Trust Model (score)
    │
    ▼
Cost Policy (evaluate)
    │
    ▼
Output: Recommendation (with confidence, reasoning, evidence, cost_delta, reversibility)
    │
    ▼
TUI Display → User Approval
```

### Model Routing

```
Task Received
    │
    ▼
Classify Complexity (simple/moderate/complex)
    │
    ▼
Get Strategy (from preferences)
    │
    ├──→ Simple: Use primary model
    ├──→ Role-Based: Check role overrides → profile overrides → primary
    ├──→ Cost-Optimized: Cheapest model meeting quality threshold
    └──→ Hybrid: Role overrides first, cost-optimized fallback
    │
    ▼
Cost Policy Check
    │
    ├──→ Compliant: Use resolved model
    ├──→ Warning: Proceed with warning
    └──→ Blocked: Require override approval
```

### Learning Flow

```
User Action (approve/reject recommendation)
    │
    ▼
Learning Policy (decide what to learn)
    │
    ├──→ Allowed: Record in learning_log.json
    │   ├── Preference changes
    │   ├── Accepted recommendations
    │   ├── Rejected recommendations
    │   └── Workflow patterns
    │
    └──→ Forbidden: Skip
        ├── Silent behavior changes
        ├── Silent model changes
        ├── Silent integration installs
        └── Silent config edits
    │
    ▼
Audit Trail Updated
```

### Approval Flow

```
Adaptive Subsystem generates change
    │
    ▼
Trust Model scores the change
    │
    ├──→ Trust < 0.4: Must reject or require explicit confirmation
    ├──→ Trust 0.4-0.7: Require approval
    ├──→ Trust 0.7-0.9: Recommend approval
    └──→ Trust >= 0.9: Can auto-apply if configured
    │
    ▼
Cost Policy checks impact
    │
    ├──→ Cost increase: Show delta, require approval
    ├──→ Cost decrease: Show savings, approval recommended
    └──→ No cost change: Standard approval flow
    │
    ▼
TUI presents to user with:
    - Title
    - Confidence
    - Reasoning
    - Evidence
    - Cost impact
    - Reversibility
    - [Approve] [Reject] buttons
    │
    ▼
User decides
    │
    ├──→ Approve: Apply change, record in audit, notify TUI
    └──→ Reject: Log rejection, record in learning policy
```

---

## Quick Reference

### TUI Commands

| Command | Subsystem |
|---------|-----------|
| `/preferences` | Preference Engine |
| `/intent` | Intent Engine |
| `/recommendations` | Recommendation Engine |
| `/workflows` | Workflow Engine |
| `/profile` | Profile Engine |
| `/routing` | Subagent Orchestrator + Model Routing |
| `/cost` | Cost Policy |
| `/mcp` | MCP Lifecycle |
| `/skills` | Skill Lifecycle |
| `/trust` | Trust Model + Learning Policy |

### Persistence Files

| File | Subsystem |
|------|-----------|
| `~/.codebro/adaptive/preferences.json` | Preference Engine |
| `~/.codebro/adaptive/intents.json` | Intent Engine |
| `~/.codebro/adaptive/recommendations.json` | Recommendation Engine |
| `~/.codebro/adaptive/workflows.json` | Workflow Engine |
| `~/.codebro/adaptive/profiles.json` | Profile Engine |
| `~/.codebro/adaptive/routing.json` | Model Routing |
| `~/.codebro/adaptive/cost_log.json` | Cost Policy |
| `~/.codebro/adaptive/mcp_registry.json` | MCP Lifecycle |
| `~/.codebro/adaptive/skill_registry.json` | Skill Lifecycle |
| `~/.codebro/adaptive/learning_log.json` | Learning Policy |
| `~/.codebro/adaptive/trust_log.json` | Trust Model |

---

## Next Steps

1. Review this design with the engineering team
2. Create ADR for config format versioning
3. Begin P6 implementation after architecture review approval
4. Follow the Implementation Roadmap for phased delivery

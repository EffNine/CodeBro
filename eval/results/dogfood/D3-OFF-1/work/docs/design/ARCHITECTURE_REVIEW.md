# Architecture Review — Adaptive Developer Platform

**Document:** `docs/design/ARCHITECTURE_REVIEW.md`
**Version:** 1.0.0
**Phase:** P6 — Design Summit
**Status:** Proposed
**Date:** 2026-08-06
**Reviewer:** CodeBro Engineering

---

## 1. Executive Summary

This review assesses the architectural design of the Adaptive Developer Platform for P6. The platform introduces twelve subsystems that collectively enable CodeBro to adapt to developer preferences while maintaining explicit human control over every change.

**Overall Assessment:** The architecture is sound, follows established CodeBro patterns, and respects the frozen core. The trust-based approval gate provides a robust safety mechanism. The design is ready for implementation pending the items noted in Section 4.

---

## 2. Architecture Compliance

### 2.1 Architecture Manifest v1.0 Compliance

| Requirement | Status | Notes |
|-------------|--------|-------|
| Module boundaries respected | ✓ | `adaptive/` is a new top-level module; no cross-boundary violations |
| Provider trait is sole LLM interface | ✓ | Adaptive subsystems read from Provider config but don't call providers directly |
| Tool trait respected | ✓ | Adaptive subsystems never execute tools |
| Event system extended properly | ✓ | New `AdaptiveEvent` follows same pattern as `AgentEvent` |
| Memory persistence as JSON | ✓ | All adaptive state is JSON-serialized |
| TUI remains display-only | ✓ | TUI listens to events; mutations go through approval gate |

### 2.2 Design Principle Compliance

| Principle | Compliance | Evidence |
|-----------|------------|----------|
| Zero Configuration | ✓ | All subsystems have sensible defaults; nothing required to start |
| Developer First | ✓ | Intent Engine parses natural language; profiles adapt to context |
| Human in Control | ✓ | Every behavioral change requires approval |
| Cost Transparency | ✓ | Cost Policy tracks and displays all costs; no silent changes |
| Progressive Discovery | ✓ | Adaptive features appear in TUI panels only when relevant |
| Observable AI | ✓ | All adaptive actions logged; audit trail exposed in TUI |
| Adaptive, Not Autonomous | ✓ | No subsystem auto-executes without approval |
| Platform before Features | ✓ | Foundation subsystems (Preference, Intent, Trust) precede capabilities |
| Deterministic before AI | ✓ | Intent Engine uses rule-based classification, not LLM |
| Everything from TUI | ✓ | All subsystems have TUI panels and slash commands |

---

## 3. Subsystem Review

### 3.1 Preference Engine

**Strengths:**
- Deterministic, no LLM dependency
- Clear schema with validation
- Audit trail for every change
- Merge semantics support per-project and per-session overrides

**Risks:**
- Schema migration path not fully detailed
- Default values for new fields need careful consideration

**Recommendation:** Proceed as designed. Add a migration test suite.

### 3.2 Intent Engine

**Strengths:**
- Rule-based classification is fast and predictable
- Confidence scoring prevents low-quality intents from noise
- Disambiguation handles common conflicts

**Risks:**
- Pattern coverage may be incomplete at launch
- Edge cases in natural language may produce unexpected parses

**Recommendation:** Proceed as designed. Add extensive pattern test cases. Consider a "learned patterns" extension for P7.

### 3.3 Recommendation Engine

**Strengths:**
- Unified output format for all adaptive subsystems
- Deduplication prevents notification fatigue
- Ranking considers multiple factors

**Risks:**
- Ranking formula may need tuning after real-world use
- Silent mode could hide important recommendations

**Recommendation:** Proceed as designed. Add A/B testing capability for ranking weights.

### 3.4 Workflow Engine

**Strengths:**
- Sliding window approach is memory-efficient
- Pattern detection is deterministic
- Integration with SkillManager bridges P6 and P3

**Risks:**
- Minimum occurrence threshold (3) may be too high for complex workflows
- Pattern matching is sequence-based only; timing is ignored

**Recommendation:** Proceed as designed. Allow threshold tuning via preferences.

### 3.5 Profile Engine

**Strengths:**
- Six well-designed built-in profiles
- Merge semantics prevent preference loss on switch
- User-created profiles supported

**Risks:**
- Profile switching could inadvertently override useful preferences
- No built-in profile comparison tool

**Recommendation:** Proceed as designed. Add a "diff profiles" feature to the TUI.

### 3.6 Subagent Orchestrator

**Strengths:**
- Wraps existing AgentCoordinator without modification
- Role-based model resolution is clean
- Cost Policy integration prevents unauthorized model upgrades

**Risks:**
- New roles (architect, debugger) have no subagent implementations yet

**Recommendation:** Proceed as designed. Architect and Debugger roles can be stub implementations for P6.

### 3.7 Model Routing Policy

**Strengths:**
- Four strategies provide flexibility
- Complexity classification is deterministic
- Cost compliance check prevents budget violations

**Risks:**
- Token estimation for routing decisions is approximate

**Recommendation:** Proceed as designed. Use conservative token estimates (1.5× actual).

### 3.8 Cost Policy

**Strengths:**
- Multi-level limits (daily, session, per-task)
- Conservative cost estimation (unknown models = free)
- Override mechanism with audit logging

**Risks:**
- Pricing table is approximate and may drift from actual prices
- No support for usage-based pricing models

**Recommendation:** Proceed as designed. Add a periodic pricing table refresh mechanism for P7.

### 3.9 MCP Lifecycle

**Strengths:**
- Six-stage lifecycle is comprehensive
- Sandbox validation protects against malicious servers
- Registry provides persistence across sessions

**Risks:**
- Network-based MCP servers (SSE) require careful security review
- Registry format is not versioned

**Recommendation:** Proceed as designed. Add SSE server validation as a P7 item.

### 3.10 Skill Lifecycle

**Strengths:**
- Extends existing SkillManager cleanly
- Discovery from multiple sources
- Integration with Workflow Engine for auto-generation

**Risks:**
- Skill package format is not defined (assumes existing format)
- Community source requires external dependency

**Recommendation:** Proceed as designed. Define skill package format in an ADR before implementation.

### 3.11 Learning Policy

**Strengths:**
- Clear allowed/forbidden boundaries
- Audit trail for every learning event
- Reversibility tracking

**Risks:**
- Learning from rejected intents is forbidden, which may slow adaptation

**Recommendation:** Proceed as designed. Consider allowing negative learning (avoiding previously rejected patterns) for P7.

### 3.12 Trust Model

**Strengths:**
- Multi-factor scoring provides nuanced assessment
- Human-readable explanations build trust
- Historical accuracy improves over time

**Risks:**
- Weights are arbitrary and may need tuning
- Composite score may mask individual factor concerns

**Recommendation:** Proceed as designed. Add weight tuning via preferences for P7.

---

## 4. Architecture Gaps

The following gaps were identified and should be addressed before P6 implementation begins:

| Gap | Severity | Recommendation |
|-----|----------|----------------|
| Config format versioning for adaptive state | Medium | Define in an ADR; include version field in preferences.json |
| Migration path from P5 settings to P6 preferences | Medium | Write migration scripts in P6.5 |
| Default pricing table freshness mechanism | Low | Add a periodic check in P7 |
| SSE MCP server security model | Medium | Defer to P7; implement stdio-only for P6 |
| Skill package format specification | Low | Define in an ADR before P8 implementation |

---

## 5. Integration Points

### 5.1 With Existing AgentCoordinator

The Adaptive Orchestrator wraps `AgentCoordinator` without modifying it. All integration happens through the trait abstraction layer.

### 5.2 With TUI

The TUI extension is additive — new panels and event variants are added without modifying existing rendering logic.

### 5.3 With Intelligence Layer

The adaptive platform reads from the intelligence layer (project language, framework) but does not write to it. Read-only access is maintained.

### 5.4 With Tool Executor

No direct integration. The adaptive platform observes tool execution through `AgentEvent` clones.

---

## 6. Non-Goals Verification

The following non-goals are explicitly NOT implemented:

| Non-Goal | How It's Excluded |
|----------|-------------------|
| Adaptive behavior (automatic) | Every action requires approval |
| Automatic learning | Learning is logged but not acted upon without approval |
| Automatic installation | MCP and Skill installation require approval |
| Autonomous agents | Subagent orchestrator routes but doesn't auto-decide |
| Automatic model switching | Model routing suggestions require approval |
| Self-modifying behavior | All changes go through Preference Engine with audit trail |

---

## 7. Cross-Reference Validation

| Document | Cross-References | Consistent |
|----------|-----------------|------------|
| ADAPTIVE_PLATFORM_SPEC.md | All 12 subsystem specs | ✓ |
| USER_PREFERENCE_MODEL.md | INTENT_ENGINE_SPEC, PROFILE_ENGINE_SPEC | ✓ |
| INTENT_ENGINE_SPEC.md | USER_PREFERENCE_MODEL, RECOMMENDATION_ENGINE | ✓ |
| RECOMMENDATION_ENGINE_SPEC.md | TRUST_MODEL, COST_POLICY | ✓ |
| WORKFLOW_ENGINE_SPEC.md | SKILL_LIFECYCLE, RECOMMENDATION_ENGINE | ✓ |
| PROFILE_ENGINE_SPEC.md | SUBAGENT_ORCHESTRATION, USER_PREFERENCE_MODEL | ✓ |
| SUBAGENT_ORCHESTRATION_SPEC.md | MODEL_ROUTING_POLICY, COST_POLICY | ✓ |
| MODEL_ROUTING_POLICY.md | COST_POLICY, SUBAGENT_ORCHESTRATION | ✓ |
| COST_POLICY.md | All cost-impacting subsystems | ✓ |
| MCP_LIFECYCLE.md | TRUST_MODEL, COST_POLICY | ✓ |
| SKILL_LIFECYCLE.md | WORKFLOW_ENGINE, COST_POLICY | ✓ |
| TRUST_MODEL.md | RECOMMENDATION_ENGINE, COST_POLICY | ✓ |

---

## 8. Reviewer Recommendations

1. **Approve** the architecture for P6 implementation
2. **Require** ADR for config format versioning before P6 implementation
3. **Defer** SSE MCP server support to P7
4. **Define** skill package format in a separate ADR before P8
5. **Add** integration tests for the approval gate before P6 acceptance

---

## 9. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

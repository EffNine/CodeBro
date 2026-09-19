# Architecture Readiness Report — P6.0

**Document:** `docs/reports/p6.0/ArchitectureReadinessReport.md`
**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-06
**Owner:** CodeBro Engineering

---

## 1. Executive Summary

The CodeBro architecture has been audited for readiness to support P6 adaptive intelligence. The audit confirms that the existing architecture provides a solid foundation for adaptive behavior, with clear module boundaries, valid dependency graphs, and comprehensive existing contracts.

**Architecture Readiness: READY with conditions.**

---

## 2. Architecture Foundation

### 2.1 Module Boundaries (Verified)

| Boundary | Rule | Status |
|----------|------|--------|
| `tui/` → `agent/` | TUI may emit `AgentEvent` but may not call agent logic directly | ✅ Valid |
| `agent/` → `tools/` | Agents may not call tools directly; all tool execution goes through `tools::executor` | ✅ Valid |
| `tools/` → `providers/` | Tools may not call LLM providers | ✅ Valid |
| `providers/` → `tools/` | Providers may not call tools | ✅ Valid |
| `intelligence/` → `tools/` | Intelligence layer may not execute tools | ✅ Valid |
| `config/` → `agent/` | Config may not depend on agent | ✅ Valid |
| `session/` → `agent/` | Session tracker does not depend on agent events directly | ✅ Valid |

### 2.2 Data Flow (Verified)

```
User Input → cli/ → tui/ → agent/coordinator/ → tools/executor/ → providers/ → tui/ → session/
```

**Reverse flow is prohibited.** All adaptive subsystems must respect this单向 data flow.

### 2.3 Provider Abstraction (Verified)

- `Provider` trait is the sole interface to LLM communication.
- API keys never leave the provider module.
- One provider per session enforced.

### 2.4 Tool Abstraction (Verified)

- `Tool` trait is the sole interface for tool execution.
- All tool execution goes through `tools::executor::run_tool_pipeline()`.
- Tool arguments are strings; structured parsing is the tool's responsibility.

---

## 3. Dependency Analysis

### 3.1 No Cyclic Dependencies

The dependency graph is a Directed Acyclic Graph (DAG):

```
main → cli → onboarding → provider_manager → providers
                → workspace_discovery
                → capability_discovery
         → settings
         → tui → agent → tools → reliability
                    → providers
         → session → config
         → intelligence
```

**No cycles detected.**

### 3.2 P5 Isolation

P5 modules (`settings/`, `provider_manager/`, `workspace_discovery/`, `capability_discovery/`, `onboarding/`) have **no dependencies on existing agent/runtime code**. This ensures:
- Independent testability
- Clear layer boundaries
- P6 can add adaptive intelligence without P5 changes

### 3.3 P6 Extension Points

| Extension Point | Location | ADR Required |
|----------------|----------|-------------|
| New provider | `src/providers/` | Yes |
| New tool | `src/tools/` | Yes |
| New AgentEvent variant | `src/agent/events.rs` | Yes |
| New memory tier | `src/agent/memory.rs` | Yes |
| New intelligence component | `src/intelligence/` | Yes |
| New config field | `src/config/mod.rs` | Yes (ADR-009) |
| New top-level module | `src/` | Yes |

---

## 4. Contract Completeness

### 4.1 Existing Contracts (Complete)

| Contract | Module | Status |
|----------|--------|--------|
| Tool Contract | `tools/` | ✅ Accepted |
| Intelligence Contract | `intelligence/` | ✅ Accepted |
| Memory Contract | `agent/memory.rs` | ✅ Accepted |
| Context Contract | `intelligence/context/` | ✅ Accepted |
| Symbol Contract | `intelligence/index/` | ✅ Accepted |
| Reasoning Contract | `intelligence/reasoning/` | ✅ Accepted |
| Provider Capabilities | `providers/` | ✅ Accepted |
| Runtime Sequence | `agent/` | ✅ Accepted |

### 4.2 P6 Contracts (Pending)

| Contract | Module | Required For |
|----------|--------|-------------|
| Preference Contract | `src/preference_engine/` | Preference Engine |
| Recommendation Contract | `src/recommendation_engine/` | Recommendation Engine |
| MCP Contract | `src/mcp_manager/` | MCP Manager |
| Learning Contract | `src/learning_engine/` | Learning Engine |
| Intent Contract | `src/intent_engine/` | Intent Engine |
| Workflow Contract | `src/workflow_engine/` | Workflow Engine |

---

## 5. ADR Completeness

### 5.1 Existing ADRs (Complete)

| ADR | Title | Status |
|-----|-------|--------|
| ADR-001 | Provider Runtime Architecture | ✅ Accepted |
| ADR-002 | Tool Runtime Architecture | ✅ Accepted |
| ADR-003 | Runtime State Machine | ✅ Accepted |
| ADR-004 | Reliability Layer | ✅ Accepted |
| ADR-005 | Tool Capability Model | ✅ Accepted |
| ADR-006 | Tool Lifecycle Management | ✅ Accepted |
| ADR-007 | Tool Hook System | ✅ Accepted |
| ADR-008 | Intelligence Platform Architecture | ✅ Accepted |
| ADR-009 | Configuration Versioning | 📝 Proposed |

### 5.2 P6 ADRs (Pending)

| ADR | Title | Required For |
|-----|-------|-------------|
| ADR-010 | Preference Engine Architecture | Preference Engine |
| ADR-011 | Recommendation Engine Architecture | Recommendation Engine |
| ADR-012 | MCP Manager Architecture | MCP Manager |
| ADR-013 | Learning Engine Architecture | Learning Engine |
| ADR-014 | Approval Gate Architecture | All adaptive features |
| ADR-015 | Plugin Sandbox Architecture | Plugin execution |

---

## 6. RFC Completeness

### 6.1 Existing RFCs (Complete)

| RFC | Title | Status |
|-----|-------|--------|
| RFC-001 | React Runtime Loop | ✅ Accepted |
| RFC-002 | Tool Plugin Architecture | ✅ Accepted |

### 6.2 P6 RFCs (Pending)

| RFC | Title | Required For |
|-----|-------|-------------|
| RFC-003 | Adaptive Intelligence Architecture | P6 overall |
| RFC-004 | Preference Engine Design | Preference Engine |
| RFC-005 | MCP Sandbox Design | MCP Manager |

---

## 7. Security Posture

### 7.1 Reviewed Areas

| Area | Status | Findings |
|------|--------|----------|
| API key storage | ✅ Reviewed | Keychain stub not implemented |
| Provider credentials | ✅ Reviewed | Secure by design |
| Permission boundaries | ✅ Reviewed | Approval gate needed |
| Shell execution | ✅ Reviewed | Sandbox needed |
| Filesystem access | ✅ Reviewed | Path validation needed |
| MCP execution | ✅ Reviewed | Sandbox specified |
| Plugin execution | ✅ Reviewed | Sandbox not yet specified |
| Prompt injection | ✅ Reviewed | Sanitization needed |
| Tool injection | ✅ Reviewed | Validation needed |
| Privilege escalation | ✅ Reviewed | Sandboxing needed |

### 7.2 Critical Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Approval gate bypass | Possible | Critical | Mandatory approval for all Ask/Dangerous actions |
| Command injection | Possible | Critical | Input validation + command allowlist |
| Malicious MCP server | Possible | Critical | Sandbox isolation + approval |
| Malicious plugin | Possible | Critical | Plugin sandbox + approval |
| Prompt injection via tool output | Likely | High | Output sanitization layer |
| Workflow privilege escalation | Unlikely | Critical | Each step through approval gate |

---

## 8. Configuration Architecture

### 8.1 Current State

| Aspect | Status |
|--------|--------|
| Config struct | ⚠️ Missing `format_version`, `last_modified`, `codebro_version` |
| Config loading | ✅ Implemented |
| Config persistence | ✅ Implemented |
| Config validation | ⚠️ Minimal — no schema validation |
| Config migration | ❌ Not implemented |
| Config backup | ❌ Not implemented |

### 8.2 Required Changes (Per ADR-009)

1. Add `format_version: u32` to `ConfigMetadata`
2. Add `last_modified: DateTime<Utc>` to `ConfigMetadata`
3. Add `codebro_version: String` to `ConfigMetadata`
4. Implement `load_with_migration()` method
5. Implement `validate()` method
6. Implement backup/restore system
7. Implement migration registry

---

## 9. Event System

### 9.1 Current AgentEvent Variants

| Variant | Module | P6 Relevance |
|---------|--------|-------------|
| `TaskStarted` | `agent/events.rs` | Used by all engines |
| `TaskCompleted` | `agent/events.rs` | Used by all engines |
| `ToolExecuted` | `agent/events.rs` | Used by all engines |
| `MemoryUpdated` | `agent/events.rs` | Used by Preference Engine |
| `PreferenceChanged` | (pending) | **New — needed for P6** |
| `RecommendationShown` | (pending) | **New — needed for P6** |
| `ApprovalRequested` | (pending) | **New — needed for P6** |
| `ApprovalDecision` | (pending) | **New — needed for P6** |

### 9.2 Required Event Additions

| Event | Purpose | Phase |
|-------|---------|-------|
| `PreferenceChanged` | User preference updated | P6.1 |
| `RecommendationShown` | Recommendation displayed to user | P6.2 |
| `ApprovalRequested` | Approval gate triggered | P6.1 |
| `ApprovalDecision` | User responded to approval | P6.1 |
| `McpServerActivated` | MCP server activated | P6.1 |
| `McpServerRemoved` | MCP server removed | P6.1 |

---

## 10. Architecture Consistency Check

| Check | Status | Notes |
|-------|--------|-------|
| Manifest matches code | ✅ | All modules present |
| Contracts match implementation | ✅ | Traits implemented |
| ADRs match architecture | ✅ | All ADRs reflected in manifest |
| P5 modules isolated from P0-P4 | ✅ | Verified in Architecture Report-P5 |
| P6 extension points documented | ✅ | Extension points in architecture snapshot |
| No prohibited imports | ✅ | Verified in audit |
| No cyclic dependencies | ✅ | Verified in audit |

---

## 11. Findings

### 11.1 Critical Findings

| ID | Finding | Impact | Resolution |
|----|---------|--------|------------|
| F-001 | Config struct missing `format_version` | P6 config migration will fail | Update Config struct per ADR-009 |
| F-002 | No approval gate implementation | P6 adaptive actions cannot be controlled | Implement per APPROVAL_GATE_SPEC |
| F-003 | No MCP sandbox implementation | MCP servers cannot be safely activated | Implement per MCP_SANDBOX_SPEC |

### 11.2 Warning Findings

| ID | Finding | Impact | Resolution |
|----|---------|--------|------------|
| F-004 | P6 ADRs not yet created | Architectural decisions not documented | Create ADR-010 through ADR-015 |
| F-005 | P6 contracts not yet created | Interface boundaries not defined | Create P6 contracts |
| F-006 | P6 AgentEvent variants not defined | New adaptive events not supported | Add variants to agent/events.rs |

### 11.3 Informational Findings

| ID | Finding | Impact | Resolution |
|----|---------|--------|------------|
| F-007 | Security risk matrix complete | All risks documented | Review during P6 |
| F-008 | Privacy policy complete | Privacy boundaries defined | Enforce during P6 |
| F-009 | Explainability policy complete | Recommendation transparency defined | Enforce during P6 |

---

## 12. Recommendations

1. **Resolve F-001 before P6.1**: Add `format_version`, `last_modified`, and `codebro_version` to Config struct.
2. **Resolve F-002 before P6.1**: Implement approval gate as a prerequisite for all adaptive behavior.
3. **Resolve F-003 before P6.1**: Implement MCP sandbox as a prerequisite for MCP activation.
4. **Create P6 ADRs during P6.0 follow-up**: ADR-010 through ADR-015 should be created before P6.1 implementation.
5. **Create P6 contracts during P6.0 follow-up**: Contracts should be defined before implementation begins.
6. **Add P6 AgentEvent variants**: Define new event variants for adaptive behavior before P6.1.

---

## 13. Architecture Readiness Decision

| Option | Decision | Rationale |
|--------|----------|-----------|
| READY | ✅ **ARCHITECTURE READY WITH CONDITIONS** | Foundation is solid; critical conditions (approval gate, MCP sandbox, config versioning) must be resolved before P6.1 |
| NOT READY | — | — |
| BLOCKED | — | — |

**Architecture is ready for P6 implementation with the conditions listed above.**

---

## 14. Sign-Off

| Role | Name | Date | Status |
|------|------|------|--------|
| Lead Architect | — | — | Pending |
| Security Reviewer | — | — | Pending |
| QA Lead | — | — | Pending |

---

**This report is submitted for architecture review before proceeding to P6.1.**

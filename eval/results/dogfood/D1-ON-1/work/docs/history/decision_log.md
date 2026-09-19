# CodeBro Decision Log

**Document:** `docs/history/decision_log.md`
**Version:** 1.0.0
**Part of:** CodeBro Engineering Baseline

---

## 1. Purpose

This log records every significant engineering decision made in CodeBro. It provides historical context for future decisions and prevents repeated debates on settled topics.

Entries are added chronologically. Old entries are never deleted — only superseded entries are marked.

---

## 2. Decision Log

### DEC-001: Architecture-First Development Process

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | All future development must follow the SOP-governed phased lifecycle with human approval gates. |
| **Rationale** | The architecture audit revealed that the codebase had significant technical debt, dead code, and unclear module boundaries. Unstructured feature development would compound these issues. A formal process ensures that every change is understood, validated, and reversible. |
| **Related ADR** | ADR-001 (to be created during P0.5) |
| **Related RFC** | None |
| **Status** | Active |

---

### DEC-002: Single Production Execution Path

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | The production execution path is `tui/ui.rs::run_chat_pipeline()` → `tools::executor::run_tool_pipeline()` → `agent::coordinator::run_task()` → `providers` (via `call_ai_streaming`). The deprecated `Agent` struct in `src/agent/agent.rs` is not part of this path. |
| **Rationale** | The audit revealed two execution paths: the deprecated monolithic `Agent::run()` and the production `run_chat_pipeline()`. Maintaining two paths creates confusion, duplicate bugs, and maintenance burden. All future work must use the production path. |
| **Related ADR** | ADR-002 (to be created during P1) |
| **Related RFC** | None |
| **Status** | Active |

---

### DEC-003: Subagents Are Analysis-Only (Currently)

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | Subagents (`ResearchAgent`, `PlanningAgent`, `CodingAgent`, `TestingAgent`, `ReviewAgent`) currently produce text output only. They do NOT execute real tools. Real tool execution happens exclusively in `tools::executor::run_tool_pipeline()` and `execute_tool_call()`. |
| **Rationale** | The current subagent implementations are lightweight analysis helpers. Giving them tool execution capability is a P6 feature that requires a separate ADR. For now, the separation is intentional and documented. |
| **Related ADR** | ADR-003 (to be created during P6) |
| **Related RFC** | RFC-001 (to be created for P6 multi-agent tool execution) |
| **Status** | Active — under review in P6 |

---

### DEC-004: Intelligence Layer Not Wired to Production

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | The intelligence layer (`intelligence/`) is built and tested but is NOT wired into the production tool pipeline. The production pipeline uses `executor.rs` which has its own basic search (`grep_files`, `search_files`). The intelligence layer will be integrated in P4. |
| **Rationale** | The intelligence layer was developed as a separate module. Integrating it requires changes to `run_tool_pipeline()` and `SmartToolRouter`. This is a significant architectural change that requires its own ADR and phase. |
| **Related ADR** | ADR-004 (to be created during P4) |
| **Related RFC** | RFC-002 (to be created for P4 intelligence integration) |
| **Status** | Active — scheduled for P4 |

---

### DEC-005: Provider Trait Not Used in Production Streaming

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | The `Provider` trait defines `stream_response()` but the production path (`call_ai_streaming()` in `tui/ui.rs`) makes a raw `reqwest` call, bypassing the trait entirely. The trait will be wired into production in P1. |
| **Rationale** | The trait was defined during early development but the streaming implementation was written directly in the TUI. Wiring the trait requires refactoring `call_ai_streaming()` to use `Provider::stream_response()`. This is a P1 task. |
| **Related ADR** | ADR-005 (to be created during P1) |
| **Related RFC** | None (trivial enough to not need RFC) |
| **Status** | Active — scheduled for P1 |

---

### DEC-006: ChangePlan/Approval Workflow Is Disconnected

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | The `/apply` + `/approve` workflow exists in the TUI but is NOT connected to the main execution pipeline. File writes in the pipeline happen without approval. This will be fixed in P2. |
| **Rationale** | The approval workflow was built as a safety feature but was never wired into `run_tool_pipeline()`. The pipeline currently reads files for context but does not write them. When write capability is added, it must go through `ChangePlan`. |
| **Related ADR** | ADR-006 (to be created during P2) |
| **Related RFC** | RFC-003 (to be created for P2 safety features) |
| **Status** | Active — scheduled for P2 |

---

### DEC-007: Session Save on Every Event

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | Sessions are saved to disk on every `AgentEvent` recording. This is intentional for crash recovery but creates high I/O frequency. P2 will evaluate whether batching saves is acceptable. |
| **Rationale** | Immediate save ensures that no session data is lost on crash. The performance impact is minimal (JSON write of a few KB), but it is worth monitoring. |
| **Related ADR** | None |
| **Related RFC** | None |
| **Status** | Active — under observation |

---

### DEC-008: Dead Code in Agent Module

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | The following modules contain dead or near-dead code and will be cleaned up in P0: `dispatcher/`, `prompt/`, legacy `indexer/`, unused subagent implementations. |
| **Rationale** | These modules were created during early development but are not used in the production path. They add maintenance burden and confusion. Removing them simplifies the codebase without affecting functionality. |
| **Related ADR** | None (cleanup, not architectural change) |
| **Related RFC** | None |
| **Status** | Active — scheduled for P0 cleanup |

---

### DEC-009: Two Session Types Exist

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | There are two `Session` types: `agent::memory::Session` (in memory.json) and `session::Session` (in session files). They have overlapping fields but different structures. They will be unified in P1 or P2. |
| **Rationale** | The duplication was created during independent development of memory and session systems. Unifying them reduces maintenance burden and confusion. |
| **Related ADR** | ADR-007 (to be created during P1 or P2) |
| **Related RFC** | RFC-004 (to be created if unification is complex) |
| **Status** | Active — technical debt, scheduled for cleanup |

---

### DEC-010: Intelligence Layer Uses SQLite, Not Vector Embeddings

| Field | Value |
|-------|-------|
| **Date** | 2026-01-01 |
| **Decision** | The intelligence layer uses SQLite for symbol storage and keyword-based semantic search (name, prefix, partial match). Embedding-based search is NOT implemented. Adding embeddings requires a new dependency and a new ADR. |
| **Rationale** | Embedding-based search would improve relevance but adds a significant dependency (embedding model + inference). The current keyword-based approach is fast, deterministic, and sufficient for the P1-P3 phases. |
| **Related ADR** | ADR-008 (to be created if embeddings are added in P4+) |
| **Related RFC** | RFC-005 (to be created if embeddings are proposed) |
| **Status** | Active — embeddings are P3+ consideration |

---

## 3. Decision Log Rules

1. **New decisions** are appended at the bottom with the next sequential number.
2. **Superseded decisions** are marked with `Status: Superseded` and a reference to the superseding decision.
3. **Deprecated decisions** are marked with `Status: Deprecated` and a reason.
4. **Decisions are never deleted.** Even rejected decisions are recorded for historical context.
5. **Each decision references** the ADR and RFC that implement or propose it.

---

## 4. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [ADR Template](../ADR/template.md)
- [RFC Template](../RFC/template.md)

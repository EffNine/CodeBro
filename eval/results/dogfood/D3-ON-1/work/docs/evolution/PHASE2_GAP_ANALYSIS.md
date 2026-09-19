# Phase 2 — Gap Analysis

Date: 2026-09-06. Legend: R = reuse existing CodeBro, B = build, I = ignore/out-of-scope, O = OpenCode already owns it.

| # | Capability | Current CodeBro | OpenCode | Hermes | Desired CodeBro | Verdict |
|---|---|---|---|---|---|---|
| 1 | User modelling / fingerprint | None | None | Profiles: isolated `~/.hermes/profiles/*` with USER.md/MEMORY.md prose | Structured fingerprint: semantic preference records, hierarchy global→project→task | B (reuse core provenance) |
| 2 | Persistent cross-session knowledge | engineering_memory (JSON) + verified facts | None (static AGENTS.md only) | MEMORY.md/USER.md flat files | Keep existing stores; add durable user-context records | R + B |
| 3 | Session history | None | Native session DB — agent cannot cross-session search | state.db sessions (40+ cols), parent/child | CodeBro-observed engineering activity stream + session clustering | B (message capture = out of scope) |
| 4 | Historical search | Lexical fact search (project) | None agent-side | FTS5 + trigram dual index over messages | FTS5 over CodeBro durable records/events/sessions | B (rusqlite bundled) |
| 5 | Context assembly | `engineering_context.rs` compose() — unwired | Per-session only | 3-tier prompt, frozen snapshot | Wire compose() as `context` capability incl. fingerprint/intents; bounded | R (wire existing) |
| 6 | Memory write gates | Secret redaction; resolver bounds | — | Threat scan, char caps, drift detection, flock, backoff | Provenance/authority gates + redaction + lifecycle | R + B |
| 7 | Provenance / authority | Strong on facts; light on memory | None | None (no confidence/expiry) | USER_CONFIRMED/AI_INFERRED/OBSERVED/PROJECT_DERIVED/IMPORTED/SYSTEM_DERIVED on all durable records | B (extend core enum) |
| 8 | Knowledge lifecycle | memory: active→superseded/expired + adjustments | — | No decay | OBSERVED→INFERRED→CONFIRMED; ACTIVE→SUPERSEDED/EXPIRED/REJECTED; confidence decays w/o evidence | R + B |
| 9 | Skills — format/execution | None | Native SKILL.md execution (agentskills.io) | skill_manager + skills_guard + trust levels + read-before-write | CodeBro authoring/versioning/validation; **publish into OpenCode skill dirs**; execution stays O | B (P4) |
| 10 | Learning loop | None | None | /learn + background-review forks | Candidates → validate → persist/reject/defer; evidence-bound; reversible | B (P3) |
| 11 | Experience extraction | RCA hypotheses (never persisted, deliberate) | None | Trajectory saving | Persist typed SUCCESS/FAILURE/REJECTED from CodeBro-observed events only | B (P3) |
| 12 | Intent records | Roadmap/decisions in identity | None | None | INTENT records (goal/rationale/priority/status/scope) | B (P1) |
| 13 | Project identity | Strong (identity-runtime) | AGENTS.md static | Per-profile SOUL.md | Keep | R |
| 14 | Durable task state | None | Session tree only | kanban.db + circuit breaker + claims | State-machine task records driven by OpenCode; no executor | B (P5) |
| 15 | Scheduling / automation | CLI only | None | cron/ + workflows | Scheduled reindex/health/dependency reports via OS cron + CLI | B (P6, deferred) |
| 16 | Checkpoints / reversibility | write_atomic + rollback | Undo/compaction | Git shadow store (GIT_DIR trick) | Git-shadow checkpoints for skills + context versions | B (P4) |
| 17 | Approval / safety | mutation_lock, confirm flags, fail-closed | Native per-tool permission model incl. `codebro_*` patterns | approval.py + contextvars | Reuse OpenCode permissions; CodeBro adds domain gates | R + O |
| 18 | Delegation | consult (one-shot) | Native subagents | Async delegation w/ SQLite tracking | Ignore — OpenCode owns | I |
| 19 | Config hierarchy | config.toml + env | Deep merge, 8 sources | Deep-merge + last-known-good | Extend config additively | R |
| 20 | Portability / import | None | — | hermes_state_portability.py | Provenance-ready, exportable, no LLM lock-in | B (P0 design) |
| 21 | Language adaptation | None | Model-level | USER.md language/style notes | Semantic preferences + language tags + verbatim evidence | B (P1) |
| 22 | Engineering intelligence | Differentiator (facts/impact/change/sandbox/RCA) | None | None | Preserve untouched | R |
| 23 | Embeddings/vector | None | None | memory_store.db vectorized (empty in practice) | Not needed — FTS5 + lexical scorer; trait seam for future | I |

## Boundary conclusion

OpenCode owns: agent loop, subagents, skill execution, permissions, transcript DB, TUI.
The durable, queryable, cross-session engineering self — user fingerprint, decision/intent knowledge, experiences, versioned skills, task state — is CodeBro's lane. All v1 non-goals (no TUI/agent loop/embeddings, memory never promoted to facts, no second engineering source of truth, sandbox fails closed) remain respected.

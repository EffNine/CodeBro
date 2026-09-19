# Engineering Runtime Design Summit — Report

**Phase**: P10.5 — Engineering Runtime Design Summit
**Status**: APPROVED TO DESIGN — NO IMPLEMENTATION
**Version**: 1.0.0
**Date**: 2026-08-08
**Owner**: CodeBro Engineering
**Theme**: Engineering Runtime — the deterministic knowledge layer

---

## 1. Summit Charter

### 1.1 Purpose

Design the **Engineering Runtime**: the intelligence layer between the
Workspace Runtime and the AI Runtime that answers engineering questions
**without an LLM whenever possible**.

The runtime is **not** a language server, **not** a compiler, **not** a git
client. It is an **engineering knowledge runtime** — deterministic graph
derivations over parsed symbol facts and workspace facts.

### 1.2 Scope

**In scope**
- Symbol Registry, Dependency/Module/Call/Test-Impact/Architecture graphs
- Relationship Resolution
- Impact Analysis (rename/delete/API/dependency/test/module/arch/circular/
  unused/dead-code)
- Context Compiler (token-efficient fragments)
- Engineering Diagnostics
- Graph strategy, performance budget, implementation roadmap

**Out of scope (explicitly)**
- Any code, parser, AST, or graph construction
- Filesystem, git, provider, memory, AI, workspace discovery
- LSP implementation, compiler, execution
- Modifying any existing runtime

### 1.3 Constraints

- **No implementation.** This summit produces architecture only.
- **Platform Foundation frozen.** Existing v1.0–P10.4 APIs are immutable.
- **Deterministic before AI.** Rule-based answers win over probabilistic.
- **Lazy by default.** No eager graph construction.
- **Incremental only.** Change-event-driven updates.
- **Observable.** Every query and graph is instrumented.

---

## 2. Deliverables

| # | Deliverable | File | Status |
|---|-------------|------|--------|
| 1 | Engineering Runtime Architecture | `EngineeringArchitecture.md` | ✅ DESIGNED |
| 2 | Graph Strategy | `GraphStrategy.md` | ✅ DESIGNED |
| 3 | Context Compiler | `ContextCompiler.md` | ✅ DESIGNED |
| 4 | Impact Analysis | `ImpactAnalysis.md` | ✅ DESIGNED |
| 5 | Performance Budget | `PerformanceBudget.md` | ✅ DESIGNED |
| 6 | Implementation Roadmap | `ImplementationRoadmap.md` | ✅ DESIGNED |
| 7 | Design Summit Report | `DesignSummitReport.md` | ✅ THIS DOCUMENT |

All deliverables are **architecture-only**. No source file outside
`docs/engineering_runtime/` was created or modified.

---

## 3. Key Design Decisions

| Decision | Rationale |
|----------|-----------|
| **Answer without tokens** | deterministic graph queries are the default path; the AI Runtime only sees questions that need probabilistic reasoning |
| **Consume, don't parse** | ingestion adapters read parsed facts from `intelligence/` and workspace facts — the runtime never re-parses or walks the FS |
| **Graph classification taxonomy** | Always Available / Lazy / Optional / Never Cached — every graph declares build trigger, invalidation, cache policy, budget, complexity |
| **Symbol Registry is the only Always Available graph** | every other graph and query depends on it; it is the cheapest structure |
| **Call Graph is lazy + query-scoped** | the most expensive graph is never resident |
| **Architecture Graph is Optional** | only when the project declares architecture rules |
| **All 10 questions are deterministic** | pure graph algorithms (BFS/DFS/SCC/fan-in) — zero-token answers |
| **Impact gates destructive ops** | rename/delete/API changes block on `Breaking`/`Critical` severity via existing `ChangePlan` workflow |
| **Context Compiler budgets without an LLM tokenizer** | deterministic token estimate; fragments ≤ 30% of prompt |
| **Budgets enforced from step one** | diagnostics map 1:1 to the budget table |

---

## 4. Performance Budget (design targets)

| Metric | Target |
|--------|--------|
| Cold startup | < 100 ms |
| Idle memory | < 128 MB |
| Graph resident total | ≤ 92 MB |
| Hot query latency | < 5 ms |
| First lazy build | < 250 ms |
| Context fragment generation | < 10 ms |
| Fragment token share | ≤ 30% of prompt |

---

## 5. Ownership Summary

### Engineering Runtime owns
Symbol Registry · Dependency Graph · Module Graph · Call Graph (lazy) ·
Test Impact Graph · Architecture Graph · Engineering Diagnostics ·
Context Compiler · Impact Analysis · Relationship Resolution

### Engineering Runtime does NOT own
Filesystem · Git · Provider · Memory · AI · Workspace discovery ·
LSP implementation · Compiler · Execution · Parsing

---

## 6. Engineering Questions — Coverage

| Question | Deterministic Answer |
|----------|----------------------|
| What depends on this file/symbol? | ✅ Dependency Graph |
| Which modules are affected? | ✅ Module Graph |
| Which tests may fail? | ✅ Test Impact Graph |
| Which APIs may break? | ✅ Symbol Registry + Dependency |
| Which services use this component? | ✅ Architecture Graph |
| Rename impact | ✅ Impact Analyzer |
| Delete impact | ✅ Impact Analyzer |
| Architecture violations | ✅ Architecture Graph |
| Circular dependency | ✅ SCC |
| Unused module | ✅ fan-in = 0 |
| Dead code candidates | ✅ Call Graph |

**100% of question types have a deterministic, no-LLM path.**

---

## 7. Acceptance Criteria Compliance

| Criterion | Status |
|-----------|--------|
| NO code | ✅ only markdown under `docs/engineering_runtime/` |
| NO runtime implementation | ✅ |
| NO parser | ✅ consumed as facts from intelligence layer |
| NO AST | ✅ |
| NO graph construction | ✅ design specifies contracts only |
| Architecture delivered | ✅ EngineeringArchitecture.md |
| Graph strategy delivered | ✅ GraphStrategy.md |
| Ownership rules delivered | ✅ EngineeringArchitecture.md §3 |
| Performance budget delivered | ✅ PerformanceBudget.md |
| Roadmap delivered | ✅ ImplementationRoadmap.md |
| Context compiler delivered | ✅ ContextCompiler.md |
| Impact analysis delivered | ✅ ImpactAnalysis.md |

---

## 8. Open Questions for Chief Architect Review

1. **Public API marking precision** — exact heuristics for "public" at
   module boundaries (language-specific), or rely on visibility + import
   reachability only?
2. **Test mapping source** — parse-time analysis only, or also consume
   runtime coverage traces from test runs when available?
3. **Architecture rule declaration format** — reuse `docs/architecture/`
   manifests, a new `.codebro/architecture.toml`, or conventions only?
4. **Architecture Graph default** — Optional (requires rules) vs.
   Always-declared defaults per language? Design chose **Optional**.
5. **Dead-code entry seeding** — which registration/macro patterns seed the
   "reachable" set per language (proc-macro, DI containers, plugin
   registration)?
6. **Stale-graph policy** — accept `StaleGraph` answers for non-breaking
   queries by default, or always fresh-build before any answer?

---

## 9. Risks

| Risk | Mitigation |
|------|------------|
| Symbol ingestion schema mismatch | versioned fact contract + validation tests |
| Lazy builds slow on huge repos | node caps, partial answers, debounced batch invalidation |
| False dead-code positives | entry-point seeding + registration markers |
| Graph memory bloat | LRU eviction + per-graph budgets + diagnostics |
| Circular graph construction | explicit build order (no implicit full builds) |

---

## 10. Stop Condition

This summit is complete. Per the charter:

- **STOP.**
- Submit: Architecture · Graph Strategy · Ownership Rules · Performance
  Budget · Roadmap.
- **Await Chief Architect Review.**
- **DO NOT IMPLEMENT.**

---

## 11. Sign-off

| Role | Name | Date | Status |
|------|------|------|--------|
| Chief Architect | CodeBro Engineering | 2026-08-08 | Pending Review |
| Runtime Owner | CodeBro Engineering | 2026-08-08 | Pending Review |

---

*End of Engineering Runtime Design Summit — P10.5 — APPROVED TO DESIGN*

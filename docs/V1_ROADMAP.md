# CodeBro v1.0 Roadmap

Status: ACTIVE — baseline locked on branch `cleanup/runtime-consolidation`.
Target: **CodeBro v1.0** — *"The engineering intelligence runtime layer for AI coding agents."*

---

## 1. Current architecture (baseline lock)

Baseline verified at roadmap creation time:

| Check | Result |
|-------|--------|
| Branch | `cleanup/runtime-consolidation` |
| Latest tag | `v0.7.0-mcp-rc2` |
| Working tree | clean |
| `cargo check` | PASS |
| `cargo test` | 3213 passed / 0 failed / 11 ignored |
| `cargo clippy -- -D warnings` | PASS |

### Shape of the codebase

Single binary crate (`codebro`), edition 2021, two distinct regions under `src/`:

**Live runtime (~48k lines, production path):**

| Module | Role |
|--------|------|
| `mcp/` | MCP server over stdio (rmcp): 16 tools, thin handlers |
| `fact_store/` | Immutable validated fact store (frozen after build) |
| `engineering_facts/` | Canonical facts model (symbols, modules, packages, tests, deps, relationships) |
| `init/` | Indexing pipeline (`codebro init`) — Rust + Go today |
| `intelligence/` | Tree-sitter parser platform (Rust, Go, Python, JS, TS grammars available) |
| `engineering_memory/` + `memory_runtime/` | Persistent agent-recorded memory (low trust, bounded) |
| `project_identity/` | Declared-intent identity store (medium-high trust) |
| `sandbox/` | Execution abstraction: local + OpenSandbox backends, evidence envelope |
| `coding/` | ChangeEngine: guarded single-file mutation seam |
| `impact/` | Structural impact analysis (relationship edges) |
| `consultant/` | Conductor-gateway AI consultation backend |
| `providers/`, `credentials/`, `config/` | Model provider registry + secrets + config |
| `cli/`, `doctor/` | CLI surface + diagnostics |
| `provenance.rs`, `persistence.rs`, `error.rs`, `cancellation.rs` | Cross-cutting primitives |

**Legacy region (`src/legacy/`, ~110k lines, TEST-ONLY):**
Pre-MCP TUI agent + Adaptive Developer Platform experiments.
Compiled only under `#[cfg(test)]` so its regression suite survives until retirement.
No entry point from `main`; unreachable from any MCP tool. See `docs/LEGACY_RETIREMENT.md`.

### Product identity (unchanged)

CodeBro IS an MCP-first engineering intelligence runtime providing:
repository understanding, verified engineering facts, project identity,
engineering memory, impact analysis, safe code modification, reproducible
execution evidence.

CodeBro is NOT a coding agent, chat UI, model provider, or orchestration framework.

### Trust model (invariant, never violated)

| Class | Source | Trust |
|-------|--------|-------|
| Verified facts | `codebro init` scan of real source | High |
| Project identity | Human-authored declarations | Medium-high |
| Engineering memory | Agents via `record_memory` | Low — never promoted to facts |

---

## 2. V1 goals

Phased delivery; each phase ends with tests green + a phase report in its commit message.

| Phase | Goal | Key deliverables |
|-------|------|------------------|
| 0 | Baseline lock | This document; verified green baseline |
| 1 | Runtime modularization | Cargo workspace: `core`, `mcp-server`, `fact-store`, `identity-runtime`, `memory-runtime`, `sandbox-runtime`, `impact-engine`, `change-engine`; enforced dependency direction |
| 2 | Legacy retirement | `docs/LEGACY_RETIREMENT.md`; delete `src/legacy/` from the shipped product |
| 3 | Universal repository intelligence | Init/indexing for Rust, Go, Python, JavaScript, TypeScript (+ practical extras); Repository Intelligence Graph |
| 4 | Incremental fact index | File-hash/git-diff driven re-index; atomic crash-safe writes; `facts diff` command |
| 5 | Engineering memory V2 | Lifecycle, expiration, confidence adjustment, provenance, conflict detection; facts/memory separation preserved |
| 6 | Impact analysis V2 | Engineering dependency graph: calls, modules, APIs, configs, tests, docs; evidence-carrying answers to "what breaks?" |
| 7 | Transactional change engine | Multi-file prepare → validate → preview → apply → verify; rollback; conflict detection |
| 8 | Execution evidence system | Full evidence envelope on every execution; cargo/npm/pnpm/yarn/pytest/go-test support |
| 9 | MCP API stability | Versioned frozen contract; `docs/MCP_API_V1.md`; schema compatibility guarantees |
| 10 | Production hardening | CI pipeline, release workflow, security/dependency audits, benchmark suite (small/medium/large repos) |

## 3. Non-goals (permanent)

Never implement:

- chat UI / TUI
- autonomous agent loop
- prompt engineering framework
- model orchestration / workflow automation
- marketplace or plugin ecosystem

External agents own reasoning, planning, and user interaction.
CodeBro owns understanding, memory, safety, and verification.

Also out of scope: resurrecting Adaptive Platform concepts, porting anything
from `tui-legacy` into main, adding a second source of truth, weakening
sandbox fail-closed behaviour, or promoting memory into the fact store.

## 4. Compatibility guarantees (v1 contract)

1. **MCP tool names and JSON argument shapes are frozen at v1.0.** Additions are
   additive (new optional args, new tools); removals or renames require a major
   version bump and a documented migration path.
2. **`.codebro/` state files remain backward-compatible.** `facts.json`,
   `engineering_memory.json`, `project_identity.json` readers must accept files
   written by v0.7.x; writers bump `schema_version` deliberately and document it.
3. **Verified facts are never silently rewritten.** Incremental indexing (Phase 4)
   must produce byte-identical results to a full rebuild for identical input.
4. **Memory never mutates facts.** The structural separation between
   `.codebro/facts.json` and `.codebro/engineering_memory.json` is permanent.
5. **Sandbox execution fails closed.** When the configured backend is
   unavailable there is no silent local fallback.
6. **The CLI verbs** (`serve --root`, `init --root`, `doctor --root`,
   `list-models`) keep their meaning; new subcommands are additive.

## 5. Delivery discipline

- Commit per logical change: `feat(runtime):`, `feat(index):`, `feat(memory):`,
  `feat(impact):`, `feat(change):`, `docs(v1):`, `fix(...)`, `test(...)`.
- Every phase closes with: changes, architecture impact, tests added,
  benchmarks (where measurable), remaining risks.
- Tests stay green at every commit; clippy `-D warnings` stays clean.
- No phase begins before the previous one's verification has passed.

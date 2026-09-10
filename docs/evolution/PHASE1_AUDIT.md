# Phase 1 — CodeBro Audit (v1.2 baseline)

Status: FROZEN / SHIPPED / VERIFIED baseline. This audit informs the "brother in coding" evolution.
Date: 2026-09-06. Sources: repo inspection + parallel deep-dives of every crate, live state files, Hermes installation, OpenCode.

## Architecture at a glance

- Rust 2021 workspace, toolchain 1.97, **10 crates** (`core, parsers, fact-store, identity-runtime, memory-runtime, sandbox-runtime, impact-engine, indexer, change-engine, mcp-server`), 135 source files, ~56.5k LOC. Dependency direction enforced by `scripts/check_workspace_deps.sh` (mcp-server → services → parsers/core).
- Single binary `codebro`: `serve --root` (stdio MCP via `rmcp`), `init`, `doctor`, `facts diff`, `list-models`, `consult`.
- Config `~/.codebro/config.toml` + env (`CODEBRO_API_KEY/BASE_URL/MODEL`, `CONDUCTOR_*`, `OPEN_SANDBOX_URL`).
- **17 frozen v1.0 MCP tools** (`docs/MCP_API_V1.md`): workspace_context, engineering_facts, engineering_memory, record_memory, delete_memory, memory_stats, update_identity, apply_change, apply_changes, sandbox_exec, sandbox_test, sandbox_build, sandbox_status, impact_analyze, reindex, repository_health, consult. Contract is **additive-only** (new tools / optional args = minor).
- Multi-workspace via `WorkspaceRegistry` (canonicalized root → `Arc<WorkspaceState>`: facts cache by mtime, `mutation_lock`, recent-edits ring, last-RCA, journal lock). Mutating tools serialize per workspace. Cross-process = single-writer + atomic rename.
- `CodeBroMcpServer { registry, tool_router, sandbox_runtime }`.

## Key reuse assets for the evolution

| Asset | Location | Value |
|---|---|---|
| Provenance / ClaimEnvelope / trust | `crates/core/src/provenance.rs:218-247` | SourceKind, confidence, trust = base × freshness × confidence. Direct mapping onto authority model. |
| Engineering memory lifecycle | `crates/memory-runtime/src/engineering_memory/` | status active/expired/superseded, expires_at, supersedes, confidence adjustments (≤16), provenance, redaction, atomic writes + quarantine, bounded resolver (500 chars, min conf 0.3, explicit truncation), schema 1.1.0 ← 1.0.0. |
| Context composer (unwired) | `crates/mcp-server/src/engineering_context.rs` | compose() → provenance-tagged sections (facts ≤10, decisions ≤5, memory ≤5, evidence ≤5, impact targets ≤5). Seed of the always-available context packet. |
| Project identity | `crates/identity-runtime/src/project_identity/` | decisions/roadmap/constraints with status, migration precedent (0.9.0→1.0.0), conservative inference, 8 projection files. |
| Persistence primitives | `crates/core/src/persistence.rs` | `write_atomic` (temp+rename+fsync), `quarantine_file`; `repo_state.rs` RepoState/RepoIdentity (evidence binding). |
| Deterministic retrieval | `crates/mcp-server/src/mcp/facts.rs:627-657` | lexical 100/80/60/30/15, trust = provenance × freshness; hard caps (default 10 / max 50). |
| RCA engine | `crates/mcp-server/src/debugging/` | deterministic, evidence-scored, never persisted — the template for learning-loop discipline. |
| Execution evidence | `crates/sandbox-runtime/src/sandbox/evidence_journal.rs` | journal bounded 200 records / 30 days / 256 KiB, quarantine. |

## Storage summary

- Pure JSON everywhere: `.codebro/facts.json` (FactsModel, no top-level schema_version, immutable once built, mtime-cached), `.codebro/engineering_memory.json` (schema 1.1.0), `.codebro/project_identity.json` (schema 1.0.0) + 7 projection files, `.codebro/metadata.json`, `.codebro/execution_evidence.json`.
- `rusqlite = { version = "0.31", features = ["bundled"] }` declared at workspace root `Cargo.toml:34` but **unused in any crate source** — bundled SQLite is pre-approved in the dependency graph.
- No user model, no session concept, no append/history stream (memory is key-upsert), no embeddings/vector (deliberate).

## Test posture

- memory-runtime 120, identity-runtime 89, fact-store 75, core 103, sandbox-runtime 104, mcp-server ~350+ (unit + integration).
- Trust separation enforced mechanically: `trust_separation.rs`, `debugging_isolation.rs`, `legacy_isolation.rs` (source-tree scans).
- Patterns: `tempfile::tempdir()`, `call_tool` direct-invocation helper, schema-version fixtures, atomic-write round-trips.

## Debt relevant to the evolution

1. `bounded_response` (256 KiB) defined but not applied uniformly across handlers.
2. `#![allow(dead_code)]` atop every module (deliberate product surface).
3. `engineering_context.rs` compose() never wired as an MCP tool.
4. docs/vision + docs/philosophy are TUI-era stale; authoritative evolution doc: `docs/V1_ROADMAP.md` (DELIVERED) + `docs/codebro_v1.2_agent_runtime_report.md`.
5. Multi-workspace is per-call, not per-connection; no user/session distinction.
6. Parse cache SCHEMA=2 content-addressed; facts diff projects impact only (no incremental patch).

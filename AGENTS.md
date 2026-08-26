# CodeBro — Agent Working Notes

## Repo at a glance

- **Language / toolchain:** Rust 2021 workspace. Ten crates under `crates/` (core, parsers, fact-store, identity-runtime, memory-runtime, sandbox-runtime, impact-engine, indexer, change-engine, mcp-server). No `src/` leaf code — all sources live in `crates/<crate>/src/`. Dependency direction is enforced by `scripts/check_workspace_deps.sh` (mcp-server → services → parsers/core).
- **Binary entry point:** `crates/mcp-server/src/main.rs` → `codebro_mcp_server::run()`.
- **Positioning:** Engineering context & memory layer for AI coding agents, exposed as an MCP server.
- **Tests:** `cargo test`. Focused: `cargo test <module_name>`. Trust current output — test counts evolve.
- **Build:** `cargo build --release && cargo install --path crates/mcp-server`
- **CLI:** `codebro serve --root <path>`, `codebro init --root <path>`, `codebro doctor --root <path>`, `codebro list-models`, `codebro facts diff --root <path>`.
- **Config:** Optional `~/.codebro/config.toml`. Env vars honoured: `CODEBRO_API_KEY`, `CODEBRO_BASE_URL`, `CODEBRO_MODEL`.
- **Sandbox config:** `OPEN_SANDBOX_URL` activates the OpenSandbox backend. When configured but unavailable, execution fails closed — no silent Local fallback.
- **Project state:** `.codebro/` inside the workspace root (facts.json, engineering_memory.json, project_identity.json, metadata.json). Git-ignored; not part of the source tree.

## Core architecture

MCP-first. The old TUI was removed (ADR-012); preserved only on `tui-legacy` branch.

| Crate / path | Role |
|---|---|
| `crates/mcp-server/src/mcp/mod.rs` | MCP server: 17 tools over stdio (`rmcp` transport) |
| `crates/mcp-server/src/mcp/facts.rs` | Relevance-ranked fact retrieval engine |
| `crates/sandbox-runtime/src/sandbox/` | Sandbox execution abstraction (trait + local + OpenSandbox backends) |
| `crates/indexer/src/init/` | Fact-store population pipeline (`codebro init`) |
| `crates/mcp-server/src/doctor/` | Diagnostics (`codebro doctor`) |
| `crates/fact-store/src/engineering_facts/` | Canonical facts model (symbols, modules, packages, tests, build targets, dependencies, relationships, references, diagnostics, architecture rules) |
| `crates/fact-store/src/fact_store/` | Immutable indexed fact store + validation, lookup, query, snapshot |
| `crates/memory-runtime/src/engineering_memory/` | Persistent engineering memory runtime (load, record, update, delete, snapshot, resolve) |
| `crates/identity-runtime/src/project_identity/` | Project identity runtime |
| `crates/change-engine/src/coding/` | ChangeEngine: guarded mutation seam |
| `crates/parsers/src/intelligence/` | Tree-sitter parser platform (Rust, Go, Python, JS, TS) |
| `crates/core/src/` | Shared primitives (provenance, error, config, persistence, tools) |

## Things that are easy to get wrong

1. **No `src/` leaf code.** All Rust sources live under `crates/<crate>/src/`. Don't look for `src/main.rs` or `src/coding/` at the workspace root.
2. **`#![allow(...)]` at the top of every module.** This is intentional — the crates expose deliberate product surface beyond current callers. Do not remove these.
3. **Async throughout.** `tokio` with `#[tokio::main]` and `#[tokio::test]`. Never block on futures at top level.
4. **Tool arguments are typed structs.** Handlers take `Parameters<T>` where `T` derives `Deserialize + Serialize + schemars::JsonSchema`; rmcp generates each tool's input schema from it. The test helper `call_tool` deserializes JSON into these types — keep both in sync when adding tools.
5. **Mock providers in tests.** Implement the `Provider` trait directly (see `crates/mcp-server/src/consultant/providers/mock.rs`).
6. **Temp directories via `tempfile::tempdir()` for all filesystem tests.** Never write to `/tmp` directly.
7. **Imports edge direction.** `RelationshipKind::Imports` facts store source=importer → target=imported, matching `Calls` (caller→callee) and `dep::<a>-><b>`. Do not flip orientation in producers or consumers.

## MCP contract

17 tools over stdio (`rmcp`). Handlers are thin adapters — construct runtimes and delegate, do not duplicate logic. Full design: `docs/design/MCP_SERVER.md`.

| Tool | R/W | Description |
|---|---|---|
| `workspace_context` | read | Project orientation: identity, root, fact counts |
| `engineering_facts` | read | Relevance-ranked fact retrieval (lexical matching, not embeddings). Filters: `query`, `kind`, `path`, `limit` |
| `engineering_memory` | read | Resolve persistent memory by task keywords. Entries carry confidence, source, tags |
| `memory_stats` | read | Store stats: entry count, token budget, tag distribution, confidence, recency |
| `record_memory` | write | Upsert a memory entry (secret-redacted). Updating a key replaces the full logical entry |
| `delete_memory` | write | Delete by exact key. **Requires `confirm=true`** — omitting is a no-op |
| `update_identity` | write | Update project identity (`.codebro/project_identity.json`). Requires existing identity |
| `apply_change` | write | Guarded single-file mutation via ChangeEngine. For new files pass `old=""` |
| `apply_changes` | write | Transactional multi-file mutation: validate → conflict check → apply with rollback |
| `sandbox_exec` | write | Execute command in isolated sandbox. Read-only build/test/lint only |
| `sandbox_test` | write | Run tests with structured verification. Auto-detects project type |
| `sandbox_build` | write | Build/check with structured verification |
| `sandbox_status` | read | Sandbox runtime status and capabilities |
| `impact_analyze` | read | Structural impact: directed edges, related tests, provenance |
| `reindex` | write | Full fact reindex via `codebro init` pipeline |
| `repository_health` | read | Workspace health report (delegates to `codebro doctor`) |
| `consult` | write | Ask Conductor gateway for opinions (provider/mode shaped) |

## Engineering facts

- `codebro init` scans with tree-sitter and persists to `.codebro/facts.json`. Auto-detects Rust (`Cargo.toml`), Go (`go.mod`), Node (`package.json`) and Python (`pyproject.toml` / `setup.py` / `setup.cfg` / `requirements.txt`) manifests.
- Verified edges come from AST (`call_expression`, `use`/`import`). Heuristic edges from symbol co-occurrence. Verified wins over heuristic for the same `(source, target, kind)` tuple.
- Test facts: Rust `#[test]`-style attributes and Go `Test*` prefixes mark tests; `TestFact.tested` lists symbols exercised by verified calls from the test's own function.
- Parse cache is content-addressed (SHA-256) and schema-versioned — bump `SCHEMA` in `crates/indexer/src/init/cache.rs` when parse output shape changes.
- `engineering_facts` uses deterministic lexical matching (exact 100, prefix 80, substring 60, path 30, summary 15), NOT embeddings. Limit defaults to 10, hard cap 50.
- An empty query without `kind` or `path` is rejected.
- The fact store is immutable once loaded (cached by mtime). `.codebro/facts.json` is the source of truth.

## Engineering memory

- Resolution bounded: max 20 entries, 500-token budget, min confidence 0.3.
- Oversized entries are returned as explicit excerpts with `…[truncated for memory budget]` marker — never silently truncated.
- **Critical invariant:** agent-recorded memory is never promoted to the verified fact store. The two stores are structurally separate.
- Persistence schema version: `1.1.0` (adds lifecycle/provenance) in `.codebro/engineering_memory.json`; `1.0.0` stores load unchanged.

## Mutation safety

The ChangeEngine (`crates/change-engine/src/coding/change_engine.rs`) is the **only** mutation seam for `apply_change` and the Coding subagent. It enforces:

- Workspace boundary: paths outside root denied.
- Path traversal denial: literal `..` rejected outright.
- Symlink escape prevention: canonicalized paths checked against canonical root — at prepare time AND again at apply time (a path swapped to a symlink between prepare and apply is refused, not written through).
- No blind overwrite: existing files require non-empty `old`.
- Ambiguous replacement rejection: duplicate `old` text denied.
- Stale-content protection: prepare snapshots file; apply refuses if content changed.
- Controlled file creation: dedicated path for new files (PatchEngine cannot reconstruct).

Within one server process, all mutating MCP tools (`apply_change`, `apply_changes`, `record_memory`, `delete_memory`, `update_identity`, `reindex`) serialize on a per-workspace lock (`CodeBroMcpServer::mutation_lock`); new mutating tools must acquire it as their first statement. Cross-process writers still rely on the single-writer assumption.

## Root-cause hypotheses

`crates/mcp-server/src/debugging/` turns failure evidence (parsed diagnostics, failing-test linkage, recent-edit correlation, bounded impact context) into deterministic ranked hypotheses embedded additively in `sandbox_test`/`sandbox_build` responses as `root_cause`. Invariants: hypotheses are derived runtime evidence — never persisted, never written to the fact store or engineering memory; ranking is transparent heuristic weights with dedup by (kind, source); correlated representations of one event cannot stack (weaker location channels are subsumed by stronger ones, and a diagnostic without line precision never earns exact-location weight); serialized evidence is exactly what was scored; freshness/ambiguity reduce confidence; "changed recently" is never claimed as "caused". Consult `mode=debugging` injects the latest analysis as `codebro://root-cause-hypotheses` file context.

For normal coding edits use your native editing tools. `apply_change` is for controlled/autonomous workflows.

## Development workflow

```bash
cargo build --release          # build
cargo install --path crates/mcp-server  # install
cargo test                     # all tests
cargo test <module_name>       # focused
cargo test mcp::tests          # MCP tool lifecycle
cargo test coding::tests       # ChangeEngine
cargo test init                # init pipeline
cargo test doctor              # diagnostics
scripts/check_workspace_deps.sh # verify dependency direction
```

Conventions:
- Add regression tests for behavioural changes.
- Run targeted tests first, then the full suite before declaring completion.
- Targeted test selection: `apply_change` responses carry `recommended_tests` (tests whose own body was edited or which exercise an edited symbol via `TestFact.tested`, determined from the edited line range, sorted, capped at 32); pass them to `sandbox_test` as `test_filter` to run exactly those. Filters are honored for cargo/go/pytest runners and ignored where no standard selection exists; go supports single-name targeting only (multi-filter falls back to the full suite); an explicit `command` overrides filtering entirely.
- `sandbox_test` / `sandbox_build` responses include structured `diagnostics`, a coarse `classification` (`success` | `compile_error` | `test_failure` | `timeout` | `denied` | `unknown_failure`), and `affected_modules` attribution from the fact store. Diagnostics are heuristic interpretations of raw output — never treat them as a replacement for the raw evidence, and never write them into the fact store (facts are repository structure; diagnostics are execution evidence).
- Never fabricate test results or claims about security properties.
- Keep MCP handlers thin — delegate to the canonical runtime.
- `tempfile::tempdir()` for all filesystem tests.

## Repository safety

- `crates/` — canonical source. `Cargo.toml` — package metadata and version. `docs/design/` and `docs/ADR/` — design docs.
- `.codebro/` — runtime state. Use CodeBro commands (`codebro init`, `codebro doctor`, MCP tools) for intentional state changes.
- `CHANGELOG.md` — authoritative change history. `Cargo.lock` — committed lockfile.
- Do not commit: `.codebro/*.json`, `target/`.
- Destructive ops to avoid without approval: `git clean`, `git gc`, destructive rewrites, deleting `.codebro/` contents, modifying release commits or tags.

Before important operations:
```bash
git status
git log --oneline -5
```

## Legacy architecture

Pre-MCP architecture (~110k lines: TUI agent loop, Adaptive Developer Platform) was deleted. See `docs/LEGACY_RETIREMENT.md`. History on tags `v0.7.0-mcp-rc1/rc2` and branch `tui-legacy`. Guards in `crates/mcp-server/tests/legacy_isolation.rs` prevent reintroduction. Do not recreate legacy concepts (agent loops, prompt assembly, plugin SDKs, tool registries).

## When to stop and ask

- A change would alter the core MCP/runtime boundary.
- A change would introduce a second source of truth.
- A change would require weakening security guarantees.
- A destructive repository operation is required.
- A new MCP tool is proposed without a clear product requirement.
- An architectural decision conflicts with existing documented constraints.
- A release/tag would need to be rewritten.
- Requirements are ambiguous enough that choosing incorrectly could change product direction.

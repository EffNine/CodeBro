# CodeBro — Agent Working Notes

## Repo at a glance

- **Language / toolchain:** Rust, edition 2021, single `[[bin]]` crate (`codebro`). No library target.
- **Positioning:** Engineering context & memory layer for AI coding agents, exposed as an MCP server.
- **Entry point:** `src/main.rs` → `cli::run()` → dispatches to `serve`, `init`, `doctor`, or `list-models`.
- **Tests:** `cargo test`. Run focused: `cargo test <module_name>`. Test counts evolve — trust the current output rather than hardcoding numbers.
- **Build:** `cargo build --release && cargo install --path .`
- **CLI commands:** `codebro serve --root <path>`, `codebro init --root <path>`, `codebro doctor --root <path>`, `codebro list-models`.
- **Config:** Optional `~/.codebro/config.toml` (provider, base_url, model, api_key). Env vars honoured: `CODEBRO_API_KEY`, `CODEBRO_BASE_URL`, `CODEBRO_MODEL`.
- **Project state:** `.codebro/` directory inside the workspace root (facts.json, engineering_memory.json, project_identity.json, metadata.json). Not part of the source tree; ignored by git.

## Product identity

CodeBro is:
- Persistent engineering context and memory for AI coding agents.
- An MCP server that answers "where am I, what are this project's verified facts, and what do we know from prior sessions."

CodeBro is NOT:
- A replacement coding agent.
- A model provider.
- A chat UI or TUI.
- A generic filesystem / shell / Git MCP.
- An autonomous agent loop.
- A vector database or embedding service.

The host agent owns: model, conversation, planning, tool selection, native Read/Grep/Edit tools, execution strategy, UX.
CodeBro owns: project/workspace identity, structured engineering facts, persistent engineering memory, optional guarded mutation.

## Canonical architecture

The MCP-first architecture is the production path. The old TUI was removed from `main` (ADR-012) and is preserved only on the `tui-legacy` branch.

Core subsystem directories (all under `src/`):

| Directory | Role |
|-----------|------|
| `mcp/` | MCP server: 7 tools over stdio (the public interface) |
| `mcp/facts.rs` | Deterministic relevance-ranked fact retrieval engine |
| `init/` | Fact-store population pipeline (`codebro init`) |
| `doctor/` | Diagnostics (`codebro doctor`) |
| `engineering_facts/` | Canonical facts model (symbols, modules, packages, tests, build targets, dependencies, relationships, references, diagnostics, architecture rules) |
| `fact_store/` | Immutable indexed fact store + validation, lookup, query, snapshot, statistics |
| `engineering_memory/` | Persistent engineering memory runtime (load, record, update, delete, snapshot, resolve) |
| `project_identity/` | Project identity runtime (workspace language, frameworks, constraints) |
| `coding/` | ChangeEngine: guarded mutation seam (permissions, limits, contract, runtime) |
| `cli/` | CLI parsing (serve, init, doctor, list-models) |
| `intelligence/` | Tree-sitter parser platform (Rust, Python, JavaScript, TypeScript, Go) |
| `memory_runtime/` | Generic memory runtime (used by engineering_memory) |
| `provider_runtime/` | Provider registry, health, circuit breaker, cost tracking |
| `config/` | Configuration loading / persistence |
| `workspace_runtime/` | Workspace discovery, metadata, environment detection |

## Things that are easy to get wrong

1. **Legacy modules were removed in ADR-012.** Do not reference or recreate:
   - `src/context/` — superseded by `engineering_context` + `assembly`
   - `src/prompt/` — superseded by `prompt_builder`
   - `src/indexer/` (`RepositoryIndex`) — dead after context removal
   - `intelligence/memory/` (`IntelligenceMemory`) — superseded by `project_identity` / `engineering_facts`
   - `reliability/health.rs` and `reliability/circuit_breaker.rs` — moved to `provider_runtime`

2. **The `#[allow(...)]` blanket at the top of every module.** Every source file starts with `#![allow(dead_code, unused_imports, unused_variables, clippy::all)]`. This is intentional and not a signal to remove them.

3. **The pre-existing clippy warning is known.** `src/intelligence/parser/tree_sitter.rs:813` has an unreachable pattern (`method_declaration` appears twice). It generates a warning but does not affect correctness. Do not paper over it with a structural change unless the duplication is genuinely a bug.

4. **Async throughout.** `tokio` with `#[tokio::main]` and `#[tokio::test]`. Never block on futures at the top level; use `std::thread::spawn` + dedicated runtime when a sync context is required (see `cli::run` model discovery).

5. **Tool arguments are passed as JSON strings, not typed structs.** The MCP handler deserializes at runtime.

6. **Mock providers are the standard for integration tests.** Implement the `Provider` trait directly (see `src/planning/tests.rs` for the `PlanningMockProvider` pattern).

7. **Temp directories via `tempfile::tempdir()` for all filesystem tests.** Do not write to `/tmp` directly.

## MCP contract

The server exposes exactly 7 tools over stdio (`rmcp` transport). MCP handlers are thin adapters — they do not duplicate business logic; they construct runtime instances and delegate.

### Tool inventory

| Tool | Read/Write | Description |
|------|------------|-------------|
| `workspace_context` | read | Compact workspace orientation: project identity, workspace root, fact-store counts. Call first to understand the project. |
| `engineering_facts` | read | Relevance-ranked fact retrieval over verified facts (symbols, modules, tests, packages, build targets, dependencies). Supports `query`, `kind`, `path`, `limit` filters. Returns compact fact records with locations and provenance (not raw ids). Zero-result responses include deterministic recovery hints. |
| `engineering_memory` | read | Resolve persistent engineering memory (decisions, constraints, prior context) by task keywords. Entries carry confidence, source, and tags so the agent can judge trustworthiness. Memory is bounded by the resolver's entry/token budget. |
| `memory_stats` | read | Read-only statistics about the engineering memory store: entry count, configured token budget, tag distribution, average confidence, oldest/newest timestamps. |
| `record_memory` | write | Upsert a persistent engineering memory entry. Values are secret-redacted before storage. Updating an existing key updates the full logical entry (value AND confidence, importance, tags, source). Keys are capped at 256 chars; values at 64 KB; tags at 32 entries, 64 chars each. |
| `delete_memory` | write | Delete an engineering memory entry by exact key. **Requires `confirm=true`** — omitting it is a no-op. Prevents accidental or speculative deletion. Deleting a missing key errors. |
| `apply_change` | write *(optional)* | Guarded single-file mutation through the ChangeEngine. Enforces workspace boundary, refuses stale or ambiguous edits. For new files pass `old=""`. Agents should use their native editing tools for normal coding edits; this is available for controlled/autonomous workflows. |

Full design: [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md).

### Server instructions

The `ServerInfo.instructions` field (embedded in `#[tool_handler]`) tells connected agents when and how to use each tool. These instructions are part of the public contract and must stay accurate.

## Engineering facts

### How facts are generated

`codebro init` scans the workspace with tree-sitter parsers and freezes results into `.codebro/facts.json`. Supported languages (verified in `src/init/mod.rs`):
- Rust (via `Cargo.toml` manifest + tree-sitter-rust)
- Go (via `go.mod` manifest + tree-sitter-go)

Other tree-sitter parsers are available (Python, JavaScript, TypeScript) but `init` currently only auto-detects Rust and Go manifests.

### Storage

Facts are stored in `.codebro/facts.json` as a `FactsModel`. The MCP server loads this into an immutable `FactStore` once per server process (cached by mtime). After `build()`, the store is frozen — no mutation path.

### Retrieval

`engineering_facts` uses deterministic lexical string matching (exact name 100, prefix 80, substring 60, path 30, summary 15), NOT embeddings or vector search. Results are sorted deterministically: score desc, kind asc, name asc, path asc. Limit defaults to 10, hard cap is 50.

An empty query without `kind` or `path` filter is rejected (would enumerate the whole store ambiguously).

### Source of truth

The authoritative source for verified facts is `.codebro/facts.json`, produced by `codebro init`. The fact store validates for duplicates, broken indexes, missing ids, and orphan records. `codebro doctor` surfaces validation issues.

## Engineering memory

### Operational model

Agents should:
- Use `workspace_context` for initial project orientation.
- Use `engineering_facts` for structured, project-wide questions about code.
- Use `engineering_memory` when prior engineering decisions or context matter.
- Use `record_memory` for durable decisions worth preserving across sessions.
- Use `memory_stats` to judge whether the store holds meaningful state before relying on it.

Agents should NOT:
- Treat every memory entry as verified truth.
- Overwrite memory casually (use explicit keys; upsert is intended for updates).
- Delete memory casually (`confirm=true` gate prevents accidents).
- Bypass the memory runtime (no direct file writes to `.codebro/engineering_memory.json`).
- Create a parallel memory store.
- Assume memory is unlimited.

### Resolution behaviour

Memory resolution is bounded:
- Max entries: 20 (`DEFAULT_MAX_ENTRIES`)
- Token budget: 500 (`DEFAULT_TOKEN_BUDGET`)
- Min confidence: 0.3 (`DEFAULT_MIN_CONFIDENCE`)

Entries are ranked by importance desc, confidence desc, key asc, id asc. Oversized entries that exceed the remaining budget are returned as explicit excerpts (truncated at a sentence/paragraph boundary when possible, otherwise hard-cut) with the marker `…[truncated for memory budget]`. Truncation is always explicit — never silent.

### Persistence

Memory is persisted to `.codebro/engineering_memory.json` with schema version `1.0.0`. The file includes `workspace_root`, `schema_version`, `entries`, and `updated_at`. Updates (via `record_memory` on an existing key) replace the full logical entry — value, confidence, importance, tags, and source — while preserving id, key, and created_at.

## Trust model

Three distinct classes of information are never blurred:

| Class | Source | Trust level | Surface |
|-------|--------|-------------|---------|
| **Verified facts** | `codebro init` (tree-sitter scan of real source) | High — deterministic, provenance-carrying, validated (0-issue store) | `engineering_facts`, `workspace_context` |
| **Engineering decisions / identity** | Human-authored project identity and constraints | Medium-high — declared intent | `workspace_context` (identity), `record_memory` with explicit source |
| **Agent-recorded memory** | Agents calling `record_memory` | **Low — unverified, self-declared confidence** | `engineering_memory`, `memory_stats` |

Critical invariant: agent-recorded memory is **never** promoted to the verified fact store. There is no promotion path. The two stores are structurally separate.

## Mutation safety

The ChangeEngine (`src/coding/permissions.rs`) is the **only** mutation seam for the MCP `apply_change` tool and for the Coding subagent. It enforces:

- **Workspace boundary:** Paths resolving outside the workspace root are denied.
- **Path traversal denial:** Literal `..` components are rejected outright.
- **Symlink escape prevention:** Canonicalized paths are checked against the canonical workspace root. A symlink inside the root pointing outside is denied; a symlinked parent escaping the root is also denied.
- **No blind overwrite:** Existing files require a non-empty `old` text. Passing `old=""` for an existing file is rejected.
- **Ambiguous replacement rejection:** If `old` occurs more than once in the file, the change is denied — the caller must supply more context for a unique match.
- **Stale-content protection:** The prepare phase reads current content; apply refuses if the file changed between prepare and apply.
- **Plan adherence (strict mode):** The ChangeEngine supports strict plan adherence — when `strict=true`, changes to files not in the plan are denied. The current MCP `apply_change` handler uses the plan-less/non-strict path, so plan adherence is not enforced by the MCP tool itself.
- **Controlled file creation:** New files use a dedicated creation path (PatchEngine cannot reconstruct from a non-existent base). Creation still goes through the prepare/apply seam and is subject to staleness checks.

For normal coding edits, agents should use their native editing tools. `apply_change` is optional and intended for controlled/autonomous workflows.

## Development workflow

### Commands

```bash
# Build release
cargo build --release

# Install locally
cargo install --path .

# Run all tests
cargo test

# Run focused tests
cargo test <module_name>

# MCP tool lifecycle tests
cargo test mcp::tests

# Coding subagent tests
cargo test coding::tests

# Init / doctor tests
cargo test init
cargo test doctor
```

### Known issues

- `cargo clippy -- -D warnings` fails on a pre-existing unreachable pattern at `src/intelligence/parser/tree_sitter.rs:813`. This is a known cosmetic warning, not a correctness issue.
- Some modules have ignored tests. Run `cargo test` and trust the output.

### Conventions

- Add regression tests for behavioural changes.
- Run targeted tests first, then the full suite before declaring completion.
- Never fabricate test results or claims about security properties.
- Keep MCP handlers thin — delegate to the canonical runtime, do not duplicate logic.
- Use `tempfile::tempdir()` for all filesystem tests.

## Repository safety

### Source of truth

- `src/` — canonical source code.
- `Cargo.toml` — package metadata, dependencies, version.
- `docs/design/` — current design documentation.
- `docs/ADR/` — Architecture Decision Records.
- `.codebro/` — runtime state (facts, memory, identity). Do not delete or rewrite casually; use CodeBro runtime commands (`codebro init`, `codebro doctor`, MCP tools) for intentional state changes.
- `CHANGELOG.md` — authoritative change history.
- `Cargo.lock` — committed dependency lockfile.

### Generated / runtime state (do not commit)

- `.codebro/facts.json` — produced by `codebro init`.
- `.codebro/engineering_memory.json` — produced by `record_memory`.
- `.codebro/project_identity.json` — produced by project identity runtime.
- `target/` — build artifacts.

### Destructive operations to avoid

Do **not** run without explicit approval and understanding of consequences:
- `git clean`
- `git gc`
- destructive repository rewrites
- deletion of `.codebro/` contents (destroys project state)
- modification of release commits or tags

Before important operations, verify Git integrity:
```bash
git status
git log --oneline -5
```

## Testing & verification policy

"Test passed" means the command actually ran successfully. Never fabricate:
- test counts
- build status
- benchmark results
- security verification
- MCP availability

To verify MCP startup after changes:
```bash
cargo build --release
# Then test with a real agent or use:
cargo test mcp::tests
```

## Documentation

- `docs/design/MCP_SERVER.md` — MCP architecture, tool contracts, verified tests.
- `docs/ADR/` — Architecture Decision Records (historical context on structural changes).
- `CHANGELOG.md` — recent changes and sprint notes.
- `README.md` — user-facing documentation.

AGENTS.md is an operational guide, not a duplicate of ADRs or design docs. When detailed documentation already exists, reference it.

## Release discipline

- **Current version:** `v0.7.0-mcp-rc1` (see `Cargo.toml` and tag `v0.7.0-mcp-rc1`).
- **Release commit:** `ef8b9a3` — this is the v0.7.0-mcp-rc1 release commit. `origin/main` may contain post-RC1 commits.
- The RC1 tag must not be modified or moved.
- Future changes after RC1 use new commits and new tags.
- Release build: `cargo build --release` produces `target/release/codebro`.

## Legacy TUI

The old TUI is preserved on the `tui-legacy` branch. It is historical/legacy code and is **NOT** the canonical architecture of `main`.

Do **not**:
- Port TUI assumptions back into `main`.
- Add TUI dependencies to the MCP-first runtime.
- Make runtime code depend on TUI.
- Treat TUI code as required for MCP functionality.
- Modify `tui-legacy` unless explicitly requested.

The TUI was removed from `main` in commit `dec9227` (refactor!: remove TUI from main — MCP-first engineering runtime).

## When to stop and ask

Agents should stop and ask for clarification when:
- A change would alter the core MCP/runtime boundary.
- A change would introduce a second source of truth.
- A change would require weakening security guarantees.
- A destructive repository operation is required.
- A new MCP tool is proposed without a clear product requirement.
- An architectural decision conflicts with existing documented constraints.
- A release/tag would need to be rewritten.
- Requirements are ambiguous enough that choosing incorrectly could change product direction.

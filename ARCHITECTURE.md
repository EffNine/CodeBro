# CodeBro Architecture

**CodeBro is an engineering intelligence + execution runtime for AI coding agents, exposed as an MCP server.**

It is not a chat UI, not an agent loop, and not a model provider. The host
agent (OpenCode, Claude Code, Codex, Cursor, ...) owns planning, tool
selection and conversation; CodeBro owns engineering truth and safe execution,
exposed over MCP stdio (`codebro serve`).

```
AI Coding Agent (OpenCode / Claude Code / Cursor)
        │  MCP over stdio (rmcp)
        ▼
CodeBroMcpServer ──── crates/mcp-server/src/mcp/  (17 tools, frozen v1 contract)
        │              mutating tools serialize on a per-workspace lock
        ▼
Engineering Runtime (crates/)
        ├─ identity-runtime/   declared intent (.codebro/project_identity.json + projections)
        ├─ fact-store/         canonical fact model (14 kinds) + immutable indexed store
        ├─ impact-engine/      AST-verified relationship graph + bounded BFS traversal
        ├─ memory-runtime/     persistent agent memory (bounded resolution, lifecycle, trust)
        └─ indexer/            tree-sitter indexing pipeline (Rust, Go, Python, JS/TS)
                               content-addressed parse cache, facts diff
        │
        ▼
Core Services
        ├─ change-engine/      ChangeEngine — the single guarded mutation seam
        │                      (prepare guards → apply re-validation → rollback transactions)
        ├─ sandbox-runtime/    policy-gated execution (Local PTY | OpenSandbox, fail-closed)
        │                      VerificationResult with parsed diagnostics + classification
        ├─ mcp-server: consultant/, doctor/, cli/
        └─ core/               error, provenance/trust, repo_state freshness,
                               atomic persistence, shell/pty/patch primitives, config
```

## Dependency direction

The rule is strict and one-way, enforced by `scripts/check_workspace_deps.sh`
in CI:

```
mcp-server  →  services (fact-store, impact-engine, memory-runtime,
               identity-runtime, indexer, sandbox-runtime, change-engine)  →  core
parsers sits beside core; both have no upward edges.
```

The legacy TUI architecture was deleted from `main` (ADR-012); it is preserved
only on the `tui-legacy` branch and tags, with mechanical guards in
`crates/mcp-server/tests/legacy_isolation.rs`.

## The 17 MCP tools

| # | Tool | Kind |
|---|------|------|
| 1 | `workspace_context` | read |
| 2 | `engineering_facts` | read |
| 3 | `engineering_memory` | read |
| 4 | `memory_stats` | read |
| 5 | `apply_change` | guarded write |
| 6 | `apply_changes` | transactional write |
| 7 | `record_memory` | write |
| 8 | `delete_memory` (confirm-gated) | write |
| 9 | `update_identity` | declared-intent write |
| 10 | `sandbox_exec` | policy-gated exec |
| 11 | `sandbox_test` | verified exec |
| 12 | `sandbox_build` | verified exec |
| 13 | `sandbox_status` | read |
| 14 | `impact_analyze` | read |
| 15 | `reindex` | rebuild |
| 16 | `repository_health` | read |
| 17 | `consult` | external call |

Frozen contract: [`docs/MCP_API_V1.md`](docs/MCP_API_V1.md).
Full design: [`docs/design/MCP_SERVER.md`](docs/design/MCP_SERVER.md).

## Trust model

Three information classes never blur:

| Class | Source | Trust |
|---|---|---|
| Verified facts | tree-sitter scan by `codebro init` | high — provenance-carrying, validated |
| Identity / decisions | human-authored + deterministic inference | medium-high |
| Agent-recorded memory | `record_memory` calls | low — self-declared confidence |

There is no promotion path from agent memory into the fact store.

Execution evidence (diagnostics parsed from build/test output) lives in tool
responses only — never in the fact store.

## The edit→verify loop

```
apply_change ──► preview + guards + InvalidationAdvisory
                    │                        └─ recommended_tests (via TestFact.tested)
                    ▼
sandbox_test ──► filtered run (test_filter) ──► VerificationResult
                    ├─ diagnostics (rustc/go/pytest parsing)
                    ├─ classification (compile_error | test_failure | …)
                    ├─ affected_modules (fact-store attribution)
                    └─ related_recent_changes (this session's edits)
```

## CLI

```
codebro serve --root <path>    # MCP server over stdio
codebro init --root <path>     # scan workspace → .codebro/facts.json
codebro doctor --root <path>   # diagnose runtime state
codebro list-models            # provider model discovery
codebro facts diff --root <p>  # file-level digest diff vs current store
codebro consult ...            # ask the consultant from the terminal
codebro auth status            # consultant auth state
```

## Historical note

Versions ≤0.6 were a TUI coding assistant; a later design phase added an
"Adaptive Developer Platform" (intent/preference/recommendation engines).
Both directions are retired and their code deleted from `main`; history is
preserved on the `tui-legacy` branch and release tags.

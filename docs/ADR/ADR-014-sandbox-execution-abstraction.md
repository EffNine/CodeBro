# ADR-014: Sandbox Execution Abstraction

**Status:** Accepted
**Date:** 2026-08-16
**Phase:** P7+ (MCP + Execution)
**Updated:** 2026-08-16 (P1.2 — Evidence Provenance + Sandbox Capabilities)

## Context

CodeBro's primary direction is now **engineering infrastructure/runtime + MCP server for AI coding agents**. External agents need to run build, test, lint, and verification commands in controlled environments. The existing Testing subagent has a robust command execution pipeline, but it is not exposed to external agents through the MCP layer.

Previously, the MCP server had zero execution tools. Agents could inspect facts and memory but could not verify changes or run validation commands through CodeBro.

## Decision

Introduce a **sandbox execution abstraction** with:

1. A `SandboxBackend` trait that abstracts over execution backends.
2. A **Local backend** that wraps the existing `RunCommand` + PTY infrastructure, gated by `LocalCommandPolicy`.
3. An **OpenSandbox backend** that forwards requests to a remote HTTP API (configured via `OPEN_SANDBOX_URL`).
4. Four MCP tools: `sandbox_exec`, `sandbox_test`, `sandbox_build`, `sandbox_status`.
5. Structured `ExecutionResult` as the return type — exit code is authoritative, never reinterpreted.
6. `VerificationResult` contract for expected-result checking on semantic tools.
7. Separate stdout/stderr capture (already present in `RunCommand`; validated end-to-end).
8. **Evidence provenance envelope** (P1.2): `execution_id`, `repo_identity`, `repo_state`, `timestamp`, `resolved_command`, `reproducibility`, `sandbox_capabilities`, `freshness`, `artifacts`.
9. **Formal capability descriptors** per backend (P1.2): isolation level, filesystem scope, network access, resource limits, timeout enforcement, output limits, environment control.
10. **Fail-closed backend selection** (P1.2): OpenSandbox explicitly configured but unavailable fails closed — no silent fallback to Local.
11. **Repository-state binding** (P1.2): evidence is bound to a concrete `RepoState` (commit SHA, dirty flag, working-tree hash) captured before execution.
12. **Freshness model** (P1.2): evidence can be classified as `fresh`, `stale`, or `unknown` by comparing stored `repo_state` against current state.

The MCP layer never references OpenSandbox-specific types. Backend selection is environment-driven, not code-driven.

## Consequences

### Positive

- Agents can now run bounded validation commands through CodeBro with structured evidence.
- Command policy is enforced before any process spawns (defense in depth).
- Output is secret-redacted and capped automatically.
- Backend swap is a single environment variable — no code changes.
- The trait abstraction leaves room for future backends (docker, k8s, etc.).
- Semantic tools (`sandbox_test`, `sandbox_build`) auto-detect project type and return verification results.
- Expected-result contracts let callers express assertions (exit code, success) and get structured violation reports.
- Full end-to-end integration tests exercise the MCP boundary with real fixture projects.
- **Evidence is provenance-bound**: every result carries an `execution_id`, timestamp, repo identity, repo state, capabilities, and reproducibility classification.
- **Agents can inspect guarantees before executing**: `sandbox_status` returns a formal `SandboxCapabilities` descriptor.
- **Fail-closed security**: explicitly configured OpenSandbox that is unavailable returns a structured denial rather than silently degrading to Local.
- **Freshness detection**: agents can determine whether evidence is stale by comparing repo state.

### Negative

- New module adds ~1200 lines of Rust code (up from ~900).
- Local backend duplicates some policy logic from `TestingCommandPolicy` (intentional: sandbox policy is stricter — no `cargo run`, no `npm install`, etc.).
- OpenSandbox backend is a thin HTTP client; production use requires a running OpenSandbox service.
- `run_async` helper in OpenSandbox backend adds indirection to work around nested tokio runtime constraints in tests.
- `git` subprocess calls for repo-state capture add a small overhead to every execution.

### Trade-offs

- **Local-first, OpenSandbox-optional:** The local backend is always available. OpenSandbox is opt-in via environment variable. This avoids a hard dependency on an external service.
- **Policy reuse vs. duplication:** The sandbox policy is a stricter subset of the Testing subagent policy. Duplication is intentional — the sandbox must work independently of the subagent runtime.
- **Structured JSON vs. text:** Tools return pretty-printed JSON. This is more tokens than a plain text result, but necessary for agents to parse exit codes, denial reasons, and metadata deterministically.
- **Semantic vs. generic tools:** `sandbox_test` and `sandbox_build` are convenience wrappers around `sandbox_exec` with project-aware defaults. They are not independent execution paths — they delegate to the same `SandboxRuntime::execute`.
- **Evidence vs. verified facts:** Execution evidence is **never** promoted into the verified fact store. It remains a separate, low-trust class (see Trust Model below).

## Architecture

```
External Agent
    ↓
CodeBro MCP
    ↓
Semantic operation (sandbox_exec | sandbox_test | sandbox_build)
    ↓
SandboxRuntime::execute()
    ↓
SandboxBackend trait
    ├── LocalSandboxBackend (PTY + RunCommand + LocalCommandPolicy)
     └── OpenSandboxBackend (lifecycle API: create sandbox → SSE command → delete)
    ↓
ExecutionResult (structured JSON + provenance envelope)
    ├─ execution_id, timestamp, resolved_command
    ├─ repo_identity, repo_state
    ├─ sandbox_capabilities, reproducibility
    ├─ artifacts, freshness
    └─ (original fields: exit_code, stdout, stderr, etc.)
    ↓
VerificationResult (for semantic tools: execution + pass/fail + violations)
```

## Implementation

- `src/sandbox/mod.rs` — trait, types (`ExecutionResult`, `VerificationResult`, `RepoIdentity`, `RepoState`, `Reproducibility`, `Freshness`, `Artifact`, `SandboxCapabilities`, `SandboxStatusResponse`), `SandboxRuntime` with fail-closed semantics and provenance enrichment
- `src/sandbox/local.rs` — local backend with policy + `capabilities()`
- `src/sandbox/opensandbox.rs` — HTTP backend with `run_async` helper, TCP-based `is_available()`, `capabilities()`
- `src/sandbox/mcp.rs` — standalone sandbox tool handlers (tests)
- `src/mcp/mod.rs` — main server tools wired in, argument schemas, resolution helpers

## Tests

- 81 sandbox tests (local backend, policy, opensandbox serialization, runtime, verification results, provenance, freshness, capabilities, fail-closed)
- 35 MCP integration tests (sandbox_exec, sandbox_test, sandbox_build, status with capabilities, provenance fields, repo identity/state, fixtures, timeout, metadata, expectations)
- 2 fixture projects: `tests/fixtures/cargo-project` (passing), `tests/fixtures/cargo-project-failing` (intentional failure)
- OpenSandbox live integration tests gated by `OPEN_SANDBOX_INTEGRATION=1` env var + `opensandbox-integration` cargo feature
- All 2979 tests pass; 0 regressions; 11 ignored (5 live integration tests skip when env not configured)

## OpenSandbox API Contract (P1.3 verified)

The actual OpenSandbox server API is a **lifecycle-based sandbox management** API,
not a simple `POST /exec` endpoint. The verified contract:

| Step | Method | Path | Purpose |
|------|--------|------|---------|
| 1 | POST | `/sandboxes` | Create sandbox from container image |
| 2 | GET | `/sandboxes/{id}` | Poll status until Running |
| 3 | GET | `/sandboxes/{id}/endpoints/44772` | Get execd host-mapped port |
| 4 | POST | `http://127.0.0.1:PORT/command` | Execute shell command (SSE stream) |
| 5 | DELETE | `/sandboxes/{id}` | Terminate sandbox |

**Command execution response (SSE):**
- Events: `init`, `ping`, `stdout`, `stderr`, `execution_complete`, `error`
- Exit code derived from event type: `execution_complete` → 0, `error.evalue` → parsed int
- Timeout detected when error contains "killed" or "timeout"

**Configuration:** `OPEN_SANDBOX_URL`, `OPEN_SANDBOX_API_KEY`, `OPEN_SANDBOX_TIMEOUT_SECS`, `OPEN_SANDBOX_MAX_OUTPUT_BYTES`, `OPEN_SANDBOX_IMAGE`, `OPEN_SANDBOX_RESOURCE_CPU`, `OPEN_SANDBOX_RESOURCE_MEMORY`

**Live integration test:** Run with `OPEN_SANDBOX_URL=http://localhost:8080 OPEN_SANDBOX_INTEGRATION=1 cargo test --features opensandbox-integration sandbox::opensandbox`

**Limitations:**
- Workspace files are not automatically available inside the sandbox; commands must use tools present in the container image.
- `sandbox_test`/`sandbox_build` with Cargo fixtures work via the local backend; OpenSandbox requires a Rust-capable image or pre-copied files.
- Minimum sandbox timeout is 60 seconds (OpenSandbox server constraint).

Execution evidence occupies a distinct trust tier:

| Class | Source | Trust | Surface |
|---|---|---|---|
| **Verified facts** | `codebro init` (tree-sitter scan) | High | `engineering_facts` |
| **Execution evidence** | Sandbox backend + provenance envelope | Medium — machine-collected but not tree-sitter verified | `sandbox_exec`, `sandbox_test`, `sandbox_build` |
| **Agent-recorded memory** | `record_memory` | Low | `engineering_memory` |

Critical invariants:
- Execution evidence is **never** promoted into the verified fact store.
- Agent memory is **never** promoted into execution evidence or verified facts.
- Local execution is **never** represented as equivalent to isolated sandbox execution (capabilities differ).
- An explicitly requested stronger backend **must fail closed** when unavailable unless the user explicitly configured a weaker fallback.

## Security Properties (updated)

| Property | Mechanism | Verified |
|---|---|---|
| Fail-closed backend selection | OpenSandbox configured but unavailable → denied result, no local fallback | ✓ |
| Capability transparency | `sandbox_status` exposes formal `SandboxCapabilities` before execution | ✓ |
| Repo-state binding | `RepoState::capture()` runs git commands before execution | ✓ deterministic |
| Freshness detection | Compare stored `working_tree_hash` against current capture | ✓ |
| Reproducibility classification | Enum with four tiers; default is `unknown` | ✓ |
| No silent degradation | `enrich_with_provenance()` always populates capabilities | ✓ |


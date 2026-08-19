# MCP Server — Engineering Runtime Interface

**Document:** `docs/design/MCP_SERVER.md`
**Version:** 2.0.0
**Phase:** P7+ — MCP Server + Sandbox Execution
**Status:** Implemented
**Date:** 2026-08-16
**Owner:** CodeBro Engineering

---

## 1. Purpose

CodeBro exposes its engineering context layer as a **Model Context Protocol
(MCP) server** so that battle-tested agent frontends — Claude Code, OpenCode,
Codex, Cursor, Goose — can act as the interface while CodeBro owns project
truth and persistent engineering context (plus an optional guarded mutation
API for controlled workflows).

The legacy chat TUI was moved to the `tui-legacy` branch and is out of scope
for this design.

---

## 2. Positioning

CodeBro is an **engineering context & memory layer with isolated execution**.
It exposes two families of capabilities over MCP:

1. **Engineering context** — project identity, verified facts, engineering
   memory, guarded mutations. These are the differentiated surfaces that no
   host agent can replicate without re-implementing CodeBro's fact store and
   memory runtime.
2. **Isolated execution** — policy-gated command execution through a sandbox
   abstraction (local or OpenSandbox backend). The sandbox returns structured
   evidence (exit code, stdout, stderr, duration, denial reason) so agents
   receive machine facts, not model prose.

The host agent owns: model, conversation, agent loop, planning, tool
selection, UX, execution strategy.
CodeBro owns: project truth, engineering state, persistent context, guarded
mutations (optional), sandbox execution, policy, verification boundaries.

### 2.1 Trust model: verified facts vs agent-recorded memory

CodeBro deliberately keeps **three distinct classes of information** and the
MCP surfaces never blur them:

| Class | Source | Trust | Surface |
|---|---|---|---|
| **Verified facts** | `codebro init` (tree-sitter scan of real source) | High — deterministic, provenance-carrying, validated (0-issue store) | `engineering_facts`, `workspace_context` |
| **Engineering decisions** | Human-authored identity/constraints | Medium-high — declared intent | `workspace_context` (identity), `record_memory` with explicit source |
| **Agent-recorded context** | Agents calling `record_memory` | **Low — unverified beliefs** with self-declared confidence | `engineering_memory` (confidence score shown), `memory_stats` |

Consequences:

- `engineering_memory` content is **agent-recorded context, not verified
  engineering truth**. Responses carry the self-declared `confidence`, plus
  `source`/`tags` provenance so the host agent can judge trustworthiness.
- `record_memory` accepts arbitrary agent text; it is **never** promoted to
  the verified fact store. There is no promotion path in this phase.
- Server instructions tell agents to treat memory as contextual, and the
  fact store as verified.
- This phase does **not** introduce a governance system; the distinction is
  preserved structurally (separate stores, separate tools, provenance on
  memory) and documented here.

---

## 3. Architecture

```
              ┌─────────────────┐
              │  Claude Code    │
              │  OpenCode       │
              │  Codex          │
              │  Cursor         │
              └────────┬────────┘
                       │
                       │ MCP (stdio)
                       ▼
              ┌─────────────────┐
              │     codebro     │
              │  `codebro serve`│
              ├─────────────────┤
              │ Engineering     │
              │ Runtime         │
              ├─────────────────┤
              │ project_identity│
              │ fact_store      │
              │ engineering_    │
              │   memory        │
              │ change engine   │
              │ permissions     │
              │ sandbox (NEW)   │
              └────────┬────────┘
                       │
              ┌────────┴────────┐
              │  Sandbox Backend │
              ├──────────────────┤
              │ Local (PTY)      │
              │ OpenSandbox (HTTP)│
              └──────────────────┘
```

Transport: **stdio** (one server process per host agent session). Each tool
call constructs a fresh view of the runtime from the workspace root. The
immutable fact store is cached per process (with an mtime check so a
concurrent `codebro init` is picked up), avoiding a 20+ MB JSON re-parse on
every call (~8× faster steady-state).

---

## 4. Tools

### 4.1 `workspace_context`

Returns project identity (name, languages, frameworks, build system,
constraints, architecture summary), the workspace root and current fact-store
counts.

**Use:** orient the agent at session start.

### 4.2 `engineering_facts`

**Relevance-ranked fact retrieval** over the verified fact store
(deterministic lexical matching — not embedding/vector search). Arguments:

- `query` (required): symbol/module/test name, name fragment, or path
  fragment, matched case-insensitively.
- `kind` (optional): filter by fact kind.
- `path` (optional): path substring filter.
- `limit` (optional): defaults to 10, capped at 50.

Returns **actual fact records** (kind, name, path, line, summary,
provenance) ranked deterministically — not raw ids. Matching: exact name >
name prefix > name substring > path substring > signature substring.

**Use:** "where is X defined", "what functions/structs exist", "which
module owns Y". This replaces the earlier id/count-only behavior which
caused agents to fall back to their own Glob/Grep/Read tools.

### 4.3 `engineering_memory`

Resolves engineering memory entries for a task query
(`task_keywords`, `active_file_tags`), ranked deterministically with
confidence scores, budget enforcement, and enriched provenance (`source`,
`tags`) projected from the persisted snapshot.

**Use:** "what decisions constrained this area before", "how was this
implemented previously". Content is agent-recorded context — see §2.1
trust model; never treat it as verified truth.

### 4.3b `memory_stats`

Read-only statistics about the engineering memory store: `entry_count`,
`total_budget`, `entries_with_source`, `avg_confidence`,
`oldest/newest_created_at`, and tag distribution.

**Use:** judge whether engineering memory holds meaningful state before
relying on it (e.g. "is memory empty or fresh?").

### 4.4 `apply_change` *(optional)*

Applies a guarded single-file change through the change engine. Arguments:
`path`, `old` (exact existing text; empty to create), `new`.

Enforced (and only these, matching the implementation):
- workspace path boundary (lexical `..`/absolute check **and** symlink-escape
  canonicalization)
- no blind overwrite: `old` must be non-empty for existing files
- unique replacement: an `old` text occurring more than once is rejected as
  ambiguous
- stale-content protection between prepare and apply

The engine is plan-less and non-strict by design in this phase — no plan
adherence is claimed because no plan is supplied.

**Use:** optional guarded mutation API for controlled/autonomous workflows.
Host agents use their native editing tools for normal coding edits — see §2
positioning. There is exactly one mutation path: `apply_change` routes
exclusively through `ChangeEngine::prepare` → `apply`; the MCP layer contains
no raw filesystem writes.

### 4.5 `record_memory`

Records or updates an engineering memory entry (decision, constraint,
context). Arguments: `key` (stable identifier), `value`, optional `tags`,
`confidence`/`importance` (clamped to [0,1]), optional `source`.

Enforced: non-empty key/value, 64 KiB value bound, tags sorted +
de-duplicated, deterministic upsert id from the key, and **secret
redaction** through `redact_secrets_public` before anything touches
storage. Persists to `.codebro/engineering_memory.json`.

**Use:** let the agent persist what it learned so the next session is not
amnestic — the project's *memory write path*.

### 4.6 `delete_memory`

Deletes an engineering memory entry by its exact `key`. Rejects unknown
keys. Persists to `.codebro/engineering_memory.json`.

**Use:** remove stale or wrong entries; completes the memory lifecycle
(create/read/update/delete).

### 4.7 `sandbox_exec` *(new)*

Executes a command in an isolated sandbox. The command is policy-checked
(allowlisted build/test/lint programs only — `cargo`, `go`, `npm`, `git`
read-only subcommands, diagnostic primitives) before any process spawns.
Returns structured evidence with a provenance envelope:

```json
{
  "command": "cargo test",
  "requested_command": "cargo test",
  "resolved_command": "cargo test",
  "working_directory": "/workspace",
  "exit_code": 0,
  "success": true,
  "duration_ms": 1200,
  "timestamp": "2026-08-16T12:00:00+00:00",
  "stdout": "...",
  "stderr": "...",
  "timeout": false,
  "cancelled": false,
  "denied": false,
  "denied_reason": null,
  "backend": "local",
  "mode": "local",
  "execution_id": "a1b2c3d4-...",
  "repo_identity": {
    "project_id": "abc123",
    "root": "/workspace",
    "repository_type": "cargo"
  },
  "repo_state": {
    "commit_sha": "e4f5g6h7...",
    "working_tree_dirty": false,
    "working_tree_hash": "sha256:..."
  },
  "sandbox_capabilities": {
    "isolation": "none",
    "filesystem_scope": "policy_bounded",
    "network_access": "host",
    "resource_limits": false,
    "timeout_enforcement": true,
    "output_limits": true,
    "environment_control": "restricted"
  },
  "reproducibility": "unknown",
  "artifacts": [],
  "freshness": null,
  "metadata": {}
}
```

Arguments: `command` (required), `working_directory` (optional), `timeout`
(in seconds, optional, default 120), `metadata` (optional passthrough).

**Use:** run build/test/lint commands in a controlled context with
authoritative exit-code evidence. The exit code is the single source of
truth — output text is secondary. Every result is bound to a concrete
repository state and carries a capability descriptor so the agent can
judge execution guarantees.

### 4.8 `sandbox_test` *(new)*

Runs the project's tests and returns structured verification evidence.
Auto-detects the project type and resolves the appropriate test command:

- `Cargo.toml` → `cargo test`
- `go.mod` → `go test ./...`
- `package.json` → `npm test`

An explicit `command` argument overrides the default. Returns a dual
structure with both the raw `execution` result and a `verification`
wrapper:

```json
{
  "execution": { /* ExecutionResult — same shape as sandbox_exec */ },
  "verification": {
    "verified": true,
    "summary": "Execution succeeded: exit=0 duration=1200ms backend=local",
    "violations": []
  }
}
```

Optional contract arguments:
- `expected_exit_code` (optional, default 0)
- `expected_success` (optional, default true)

When expectations are set and not met, `verification.verified` is `false`
and `violations` lists each breach. The underlying `execution` evidence
is always preserved.

**Use:** run the project's test suite and get a machine-parseable pass/fail
signal alongside the full stdout/stderr evidence with provenance.

### 4.9 `sandbox_build` *(new)*

Builds or checks the project and returns structured verification evidence.
Auto-detects the project type:

- `Cargo.toml` → `cargo check`
- `go.mod` → `go build ./...`
- `package.json` → `npm run build`

Same dual `execution` + `verification` response shape as `sandbox_test`.

**Use:** verify the project compiles without running the full test suite.

### 4.10 `sandbox_status` *(new)*

Returns sandbox runtime status including a formal capability descriptor:

```json
{
  "backend": "local",
  "mode": "local",
  "available": true,
  "capabilities": {
    "isolation": "none",
    "filesystem_scope": "policy_bounded",
    "network_access": "host",
    "resource_limits": false,
    "timeout_enforcement": true,
    "output_limits": true,
    "environment_control": "restricted"
  },
  "opensandbox_configured": null
}
```

When OpenSandbox is explicitly configured but unavailable:
```json
{
  "backend": "opensandbox",
  "mode": "opensandbox",
  "available": false,
  "capabilities": { /* OpenSandbox capabilities */ },
  "opensandbox_configured": true
}
```

**Use:** inspect execution guarantees **before** running `sandbox_exec`,
`sandbox_test`, or `sandbox_build`. Agents should check `capabilities`
to understand isolation level, network access, and other security
boundaries.

### 4.11 `impact_analyze`

Structural impact analysis over the verified fact store. Answers the
question: *"What is likely affected by changing X?"* — returning directed
relationship edges, related tests, owning module/package, and provenance
metadata. Supports **bounded transitive graph traversal** up to a configurable
depth. **Descriptive evidence only; no risk scores, no prescriptions.**

Arguments:

- `target` (required): the entity to analyze — a symbol name/id, file path,
  module id, or package id.
- `target_type` (optional): one of `symbol`, `file`, `module`, `package`.
  Defaults to `symbol`.
- `max_results` (optional): maximum results per category (default 50).
- `include_tests` (optional): whether to include related tests (default true).
- `include_references` (optional): whether to include cross-references
  (default true).
- `depth` (optional): bounded BFS traversal depth. `0` = target only, `1` =
  direct relationships (default), up to `5`. Values above `5` are rejected as
  invalid parameters.
- `direction` (optional): edge direction for traversal — `"both"` (default,
  preserves legacy behaviour returning both outgoing and incoming at depth 1),
  `"outgoing"`, or `"incoming"`.
- `relationship_types` (optional): subset of relationship kinds to traverse
  (e.g. `["calls", "imports"]`). Empty means all kinds. Only known kinds are
  accepted.
- `max_nodes` (optional): hard ceiling on distinct graph nodes visited during
  traversal (default 1000). When exceeded the result is marked `partial`.

Response shape:

```json
{
  "target": {
    "id": "sym::src/lib.rs::get_user_function@5",
    "kind": "symbol",
    "name": "get_user",
    "path": "src/lib.rs"
  },
  "status": "ok",
  "direct_relationships": [
    {
      "target_id": "sym::src/handler.rs::handle_function@3",
      "target_name": "handle",
      "relationship_kind": "references",
      "direction": "incoming",
      "source_location": "src/handler.rs",
      "provenance": "verified",
      "depth": 1,
      "path": []
    }
  ],
  "transitive_relationships": [
    {
      "target_id": "sym::src/other.rs::dep_function@7",
      "target_name": "dep",
      "relationship_kind": "calls",
      "direction": "outgoing",
      "source_location": "src/handler.rs",
      "provenance": "heuristic",
      "depth": 2,
      "path": [
        {
          "source_id": "sym::src/lib.rs::get_user_function@5",
          "target_id": "sym::src/handler.rs::handle_function@3",
          "kind": "references",
          "provenance": "verified"
        }
      ]
    }
  ],
  "affected_tests": [
    {
      "id": "test::src/lib_test.rs::test_get_user@10",
      "name": "test_get_user",
      "file": "src/lib_test.rs",
      "relation": "tests",
      "provenance": "verified"
    }
  ],
  "affected_modules": [
    {
      "id": "mod::src::lib",
      "name": "mod::src::lib",
      "path": "src/lib.rs",
      "relation": "owns"
    }
  ],
  "affected_packages": [
    {
      "id": "pkg::app",
      "name": "app",
      "relation": "owns_module"
    }
  ],
  "evidence": [
    {
      "fact_kind": "relationship",
      "fact_id": "sym::src/lib.rs::get_user_function@5→sym::src/handler.rs::handle_function@3",
      "fact_name": "handle",
      "source_location": "src/handler.rs",
      "description": "handle references get_user (depth=1)"
    }
  ],
  "completeness": {
    "status": "complete",
    "limitations": []
  },
  "provenance_summary": {
    "verified_edges": 3,
    "heuristic_edges": 1,
    "unknown_edges": 0
  },
  "traversal_metadata": {
    "depth_limit": 2,
    "direction": "both",
    "relationship_types": [],
    "max_nodes": 1000,
    "nodes_visited": 4,
    "edges_traversed": 5,
    "truncated": false,
    "truncation_reason": null
  }
}
```

Depth semantics:

| depth | Behavior |
|-------|----------|
| `0` | Target only. No relationships traversed. |
| `1` | Direct relationships only (default). Equivalent to pre-P2.3 behaviour. |
| `2` | Direct relationships + one-hop transitive relationships. |
| `3` | Direct + two-hop transitive. |
| `5` | Maximum allowed depth. Deeper requests are rejected. |

Direction semantics:

| Direction | Meaning |
|-----------|---------|
| `"both"` (default) | Follows both outgoing and incoming edges. Preserves legacy behaviour at depth 1. |
| `"outgoing"` | Only follows edges where the current node is the source (`current → neighbor`). |
| `"incoming"` | Only follows edges where the current node is the target (`neighbor → current`). |

Traversal guarantees:

- **Bounded**: maximum depth is 5; maximum nodes visited defaults to 1000.
- **Cycle-safe**: visited-node tracking prevents infinite loops.
- **Deterministic**: results are sorted by depth ascending, relationship kind,
  direction, and target identity. Repeated executions produce identical output.
- **Provenance-preserving**: each edge retains its own provenance
  (`verified` / `heuristic` / `unknown`). Heuristic edges are never promoted
  to verified through transitive traversal.
- **Path-tracked**: transitive relationships (depth ≥ 2) include a `path`
  field listing the sequence of edges from the original target to that node.
- **Completeness-aware**: when traversal is truncated (node limit or result
  limit reached), `completeness.status` is set to `partial` with an explicit
  limitation message.
- **No query-time reparse**: traversal operates on existing fact/index
  structures; no source files are re-read.

Status values:

| Status | Meaning |
|--------|---------|
| `ok` | Target resolved; relationships traversed successfully. |
| `not_found` | Target does not exist in the fact store. |
| `partial` | Target resolved but some relationships could not be established (e.g. no relationship facts exist, unsupported language, or traversal was truncated). |
| `ambiguous` | Multiple candidates matched (returned in `AmbiguityMatch` array). |
| `stale` | Fact store metadata indicates staleness vs current repository state. |

Relationship directions in output:

| Direction | Meaning |
|-----------|---------|
| `outgoing` | The target references/calls/import the other endpoint. |
| `incoming` | The other endpoint references/calls/import the target. |

Provenance levels:

| Level | Meaning |
|-------|---------|
| `verified` | Derived from a stored `RelationshipFact` or `ReferenceFact` with no `provenance=heuristic` metadata attribute. Includes AST-extracted call and import edges. |
| `heuristic` | Inferred from symbol name co-occurrence across modules (same name + same kind) without direct AST evidence. Tagged with `provenance=heuristic` in metadata. |
| `unknown` | No provenance information available. |

### AST-derived relationships (P2.2)

`codebro init` now extracts deterministic AST-level relationships for **Rust** and **Go**:

| Relationship | Source | Provenance |
|-------------|--------|------------|
| `Calls` | Actual `call_expression` nodes in the AST, resolved to known symbols by name+module | `verified` |
| `Imports` (module→module) | Actual `use` / `import` statements parsed from source | `verified` |
| `References` (symbol→symbol) | Name-coincidence fallback when no AST evidence exists | `heuristic` |
| `Imports` (module→module) | Name-coincidence fallback when no AST import evidence exists | `heuristic` |

**Supported languages:** Rust (`tree-sitter-rust`), Go (`tree-sitter-go`). Other languages (Python, JavaScript, TypeScript) use heuristic-only relationships.

**Symbol resolution rules for calls:**
- Unqualified call (`foo()`): resolves if exactly one symbol named `foo` exists across all modules, or if all candidates are in the same module.
- Qualified call (`pkg::foo()`): resolves only if exactly one candidate exists. Ambiguous qualified calls produce no verified edge.
- Private/unexported symbols that are not in the fact store produce no verified edge.

**Deduplication:** If the same `(source, target, kind)` edge is discovered by both AST extraction and name-coincidence, the verified edge is retained and the heuristic duplicate is dropped.

**Limitations:**
- Call targets resolved only by name matching within the same workspace — no cross-crate resolution.
- Dynamic dispatch, reflection, macros, and generated code are not captured.
- Import path resolution is partial: only workspace-internal modules are matched; external crate imports are not resolved to package facts.

Completeness:

- `complete`: All available relationship types were traversed.
- `partial`: Some limitations apply (listed in `limitations`).
- `unknown`: Target was not found; no analysis was performed.

**Use:** "what would break if I change X?", "who calls this function?",
"which tests cover this symbol?", "what modules depend on this file?".

### 4.12 `repository_health`

Read-only repository health and CodeBro intelligence diagnostics.
Delegates to the existing `codebro doctor` implementation — no new checks,
no auto-repair, no orchestration. Returns a structured JSON response:

```json
{
  "exit_code": 0,
  "status": "healthy",
  "checks": [
    {
      "name": "workspace_root",
      "status": "ok",
      "detail": "/path/to/workspace"
    },
    {
      "name": ".codebro",
      "status": "ok",
      "detail": "runtime state directory exists"
    },
    {
      "name": "project_identity",
      "status": "ok",
      "detail": "my-project (rust)"
    },
    {
      "name": "facts",
      "status": "ok",
      "detail": "14470 facts (351 modules, 10514 symbols, 3602 tests) — validation: 0 issues"
    },
    {
      "name": "engineering_memory",
      "status": "ok",
      "detail": "42 entries"
    },
    {
      "name": "git",
      "status": "ok",
      "detail": "working tree clean"
    }
  ],
  "summary": "All checks passed."
}
```

**Status values:**
| Status | Meaning |
|--------|---------|
| `healthy` | All checks passed (exit_code 0) |
| `warn` | Warnings present, no errors (exit_code 1) |
| `error` | Errors present (exit_code 2) |

**Check status values:**
| Status | Meaning |
|--------|---------|
| `ok` | Check passed |
| `warn` | Check warned (recoverable) |
| `error` | Check failed (needs repair) |

**Checks performed (preserved from `codebro doctor`):**
1. `workspace_root` — workspace directory exists
2. `.codebro` — runtime state directory exists
3. `project_identity` — project identity loads successfully
4. `facts` — fact store is parseable and valid
5. `engineering_memory` — memory store loads (absent is ok)
6. `git` — working tree status (skipped if not a git repo)

**Use:** diagnose whether the CodeBro workspace intelligence is healthy
before relying on facts or memory. Replace the external `codebro doctor`
CLI call with an in-MCP call.

### 4.13 `consult`

Ask an AI consultant (Conductor gateway) for opinions on architecture,
debugging, code review, planning, research, or second opinions. Supports
provider selection, mode shaping, and automatic injection of CodeBro
engineering context (facts, memory, git diff).

Arguments:

- `provider` (optional): `"auto"` (default) or `"conductor"`. Unknown
  providers are rejected as invalid params.
- `mode` (optional): `architecture`, `debugging`, `code_review`,
  `planning`, `research`, or `second_opinion`. Defaults to
  `architecture`. Unknown modes are rejected.
- `question` (required): the question or task to consult on.
- `context` (optional): explicit context text supplementing automatic
  CodeBro context injection.
- `files` (optional): list of `{path, content}` entries attached to the
  request.
- `include_git_diff` (optional, default `false`): include the current
  `git diff` in the request.
- `include_project_context` (optional, default `false`): include project
  identity, fact-store counts, and engineering memory summary.
- `max_answer_length` (optional, default `0` = provider default): cap the
  answer in characters; truncation occurs at sentence boundaries.

Mode mapping (CodeBro → Conductor public mode):

| CodeBro mode | Conductor mode |
|-------------|---------------|
| `architecture` | `agentic` |
| `debugging` | `coding` |
| `code_review` | `coding` |
| `planning` | `planning` |
| `research` | `reasoning` |
| `second_opinion` | `reasoning` |

Only Conductor-supported modes (`auto`, `coding`, `reasoning`, `vision`,
`fast`, `planning`, `agentic`, `long_horizon`) are emitted.

Transport: `POST {CONDUCTOR_BASE_URL}/v1/chat/completions` with
`Authorization: Bearer <CONDUCTOR_API_KEY>`. OpenAI-compatible response
schema. Non-streaming. 180 s timeout.

Response shape:

```json
{
  "provider": "conductor",
  "model": "<model>",
  "mode": "second_opinion",
  "answer": "...",
  "summary": "...",
  "recommendations": [],
  "risks": [],
  "confidence": 0.5,
  "metadata": { "mode": "reasoning" }
}
```

**Use:** get AI-powered opinions with project-awareness. Project context
and git diff are injected only when the caller opts in.

---

## 5. Sandbox Architecture

The sandbox module (`src/sandbox/`) provides a trait-based execution
abstraction with two backends:

| Backend | Implementation | When Active |
|---------|---------------|-------------|
| **Local** | PTY-backed `RunCommand` with timeout, output caps, policy gate | Default (always available) |
| **OpenSandbox** | Lifecycle-based HTTP API: create sandbox → execute via SSE → delete | When `OPEN_SANDBOX_URL` env var is set |

The MCP layer never references OpenSandbox-specific types. All execution
routes through the `SandboxBackend` trait. Switching backends requires only
an environment variable — no code changes.

### Execution Result Contract

Every sandbox tool returns an `ExecutionResult` (for `sandbox_exec`) or a
dual `ExecutionResult` + `VerificationResult` (for `sandbox_test` and
`sandbox_build`). The contract:

| Field | Type | Meaning |
|-------|------|---------|
| `command` | string | The command that was executed |
| `requested_command` | string | The original requested command (same as `command` unless overridden) |
| `resolved_command` | string | The exact command string that ran (may differ from requested when MCP layer resolves semantic ops) |
| `working_directory` | string | The directory the command ran in |
| `exit_code` | i32 | Authoritative exit code; `-1` means no process ran |
| `success` | bool | `exit_code == 0 && !timeout && !cancelled && !denied` |
| `duration_ms` | u128 | Wall-clock duration |
| `timestamp` | string\? | ISO 8601 execution start timestamp |
| `stdout` | string | Captured stdout (redacted, truncated) |
| `stderr` | string | Captured stderr (redacted, truncated) |
| `timeout` | bool | Whether the command exceeded the timeout |
| `cancelled` | bool | Whether the command was terminated by cancellation |
| `denied` | bool | Whether the command was rejected by policy |
| `denied_reason` | string\? | Policy denial reason |
| `backend` | string | `local` or `opensandbox` |
| `mode` | string | Sandbox mode |
| `execution_id` | string | Unique UUID v4 for this execution |
| `repo_identity` | object\? | Project identity: `project_id`, `root`, `repository_type` |
| `repo_state` | object\? | Repository state: `commit_sha`, `working_tree_dirty`, `working_tree_hash` |
| `sandbox_capabilities` | object\? | Formal capability descriptor (see below) |
| `reproducibility` | enum | `deterministic` \| `likely_deterministic` \| `non_deterministic` \| `unknown` |
| `artifacts` | array\[\] | Named artifacts with `path`, `kind`, `size`, `hash` |
| `freshness` | enum\? | `fresh` \| `stale` \| `unknown` (computed on read vs. current state) |
| `metadata` | object | Echoed back from the request |

`VerificationResult` adds:

| Field | Type | Meaning |
|-------|------|---------|
| `verified` | bool | Whether all declared expectations were satisfied |
| `summary` | string | One-line human-readable summary |
| `violations` | string\[\] | List of expectation breaches (empty when verified) |

### Sandbox Capabilities Model

Each backend exposes a formal `SandboxCapabilities` descriptor. Agents
MUST inspect this via `sandbox_status` before deciding whether an
execution is safe or appropriate.

| Field | Type | Meaning |
|-------|------|---------|
| `isolation` | enum | `none` \| `process` \| `container` \| `remote` \| `unknown` |
| `filesystem_scope` | enum | `policy_bounded` \| `sandbox_scoped` \| `unrestricted` \| `unknown` |
| `network_access` | enum | `host` \| `none` \| `controlled` \| `unknown` |
| `resource_limits` | bool | Whether CPU/memory limits are enforced |
| `timeout_enforcement` | bool | Whether timeout is guaranteed |
| `output_limits` | bool | Whether output size is bounded |
| `environment_control` | enum | `restricted` \| `controlled` \| `passthrough` \| `unknown` |

**Local backend guarantees:**

| Capability | Value | Rationale |
|---|---|---|
| `isolation` | `none` | Runs directly in the host process via PTY |
| `filesystem_scope` | `policy_bounded` | Gated by `LocalCommandPolicy` + workspace root |
| `network_access` | `host` | Full host network access (no sandbox network isolation) |
| `resource_limits` | `false` | No CPU/memory cgroups |
| `timeout_enforcement` | `true` | PTY deadline + process group kill |
| `output_limits` | `true` | Bounded buffer per stream |
| `environment_control` | `restricted` | Limited env vars injected |

**OpenSandbox backend guarantees (verified against live service):**

| Capability | Value | Rationale |
|---|---|---|
| `isolation` | `remote` | Each command runs in a fresh Docker container managed by the OpenSandbox server |
| `filesystem_scope` | `sandbox_scoped` | Container filesystem is isolated; no host mounts by default |
| `network_access` | `controlled` | Egress network policy configurable per-sandbox; default depends on server config |
| `resource_limits` | `true` | CPU and memory limits enforced by Docker (configured via `OPEN_SANDBOX_RESOURCE_CPU/MEMORY`) |
| `timeout_enforcement` | `true` | Command timeout propagated to execd; sandbox lifetime also bounded |
| `output_limits` | `true` | Max output bytes enforced client-side (default 64 KiB) |
| `environment_control` | `controlled` | Caller-injected env vars passed to command process |

### Fail-Closed Backend Selection

When `OPEN_SANDBOX_URL` is set, the runtime enters OpenSandbox mode.
If the service is unreachable (TCP connection fails), execution **fails
closed**: the result is denied with `backend="opensandbox"` and a clear
error message. The runtime does **NOT** silently fall back to Local.

Silent fallback would be a security-boundary downgrade — an agent
expecting remote isolation would unexpectedly run on the host. The only
exception is when OpenSandbox is NOT explicitly configured (i.e.,
`OPEN_SANDBOX_URL` is unset), in which case Local is the default.

### OpenSandbox API Contract (Verified)

The OpenSandbox server exposes a lifecycle-based REST API. The CodeBro
adapter implements the following flow for each command execution:

1. **Create sandbox**: `POST /sandboxes`
   - Body: `{image: {uri}, entrypoint: ["tail","-f","/dev/null"], resourceLimits: {cpu, memory}, timeout, env}`
   - Response: `{id, status: {state}, expiresAt}`
   - Minimum sandbox timeout is 60 seconds per the server contract.

2. **Wait for running**: Poll `GET /sandboxes/{id}` until `status.state == "Running"`.

3. **Get execd endpoint**: `GET /sandboxes/{id}/endpoints/44772`
   - Returns `{endpoint: "127.0.0.1:PORT"}` — the host-mapped port for the execd.

4. **Execute command**: `POST http://127.0.0.1:PORT/command`
   - Body: `{command, cwd, timeout (ms), envs}`
   - Response: SSE stream with events:
     - `{"type":"init","text":"<exec_id>","timestamp":...}`
     - `{"type":"ping","text":"pong","timestamp":...}`
     - `{"type":"stdout","text":"...","timestamp":...}`
     - `{"type":"stderr","text":"...","timestamp":...}`
     - `{"type":"execution_complete","execution_time":...,"timestamp":...}`
     - `{"type":"error","timestamp":...,"error":{"ename":"CommandExecError","evalue":"<code>","traceback":[...]}}`
   - Exit code: `0` on `execution_complete`, parsed from `evalue` on error.

5. **Delete sandbox**: `DELETE /sandboxes/{id}` → 204 No Content.

Authentication: optional Bearer token via `OPEN_SANDBOX_API_KEY` header.
When the server runs in insecure mode (no API key), requests succeed without auth.

Configuration environment variables:
- `OPEN_SANDBOX_URL` — base URL of the OpenSandbox server (required to activate)
- `OPEN_SANDBOX_API_KEY` — Bearer token (optional)
- `OPEN_SANDBOX_TIMEOUT_SECS` — default sandbox lifetime (default: 120)
- `OPEN_SANDBOX_MAX_OUTPUT_BYTES` — max output size (default: 65536)
- `OPEN_SANDBOX_IMAGE` — container image (default: `python:3.11-slim`)
- `OPEN_SANDBOX_RESOURCE_CPU` — CPU limit (default: `500m`)
- `OPEN_SANDBOX_RESOURCE_MEMORY` — Memory limit (default: `512Mi`)

### Command Policy

The local backend enforces `LocalCommandPolicy` (derived from project
metadata: Cargo.toml → cargo allowed, package.json → npm allowed, go.mod
→ go allowed, Makefile → make allowed). The policy:

- Rejects all shell metacharacters (`;`, `|`, `&`, `>`, `<`, `$`, `` ` ``, etc.)
- Allowlists programs: `cargo`, `go`, `npm`, `npx`, `pnpm`, `yarn`, `make`,
  `git` (read-only subcommands), `true`, `false`, `echo`, `printf`, `sleep`
- Denies all mutating operations: `rm`, `mv`, `git commit`, `cargo clean`,
  `sed -i`, `python3`, `bash`, etc.
- Cargo `fmt` requires `--check` flag (bare rewrite is denied)

### Repository State Binding

Before every execution, `SandboxRuntime` captures the current repository
state via `RepoState::capture()`:

- `commit_sha`: output of `git rev-parse HEAD`
- `working_tree_dirty`: whether `git status --porcelain` reports changes
- `working_tree_hash`: deterministic SHA-256 of sorted `git ls-files` +
  `git diff HEAD` + untracked files list

This binds the evidence to a concrete point-in-time repository state.
Agents can use this to verify that evidence was produced from the code
they are inspecting, not from some other commit.

### Freshness / Staleness

Evidence freshness is computed by comparing the stored `repo_state`
against a fresh capture at query time:

- `fresh`: `evidence.repo_state.working_tree_hash == current.working_tree_hash`
- `stale`: hashes differ (repository has changed since execution)
- `unknown`: current state cannot be determined (not a git repo, or git
  unavailable)

Freshness is computed on-read, not at-execution time, so agents always
see the current assessment.

### Reproducibility Model

Each execution is classified by `reproducibility`:

| Value | Meaning |
|---|---|
| `deterministic` | Same inputs always produce same output (e.g. `cargo test` on a dependency-free fixture) |
| `likely_deterministic` | Mostly reproducible but may vary under load (e.g. full `cargo test`) |
| `non_deterministic` | Output intentionally varies (randomness, wall-clock, network) |
| `unknown` | Cannot classify from available information (default) |

This is a metadata signal, not a proof system. It helps agents weight
evidence appropriately without over-trusting non-deterministic results.

### Security Properties (updated)

| Property | Mechanism | Verified |
|---|---|---|
| No shell escape | metacharacter rejection before tokenization | ✓ denied |
| No mutation commands | program + subcommand allowlist | ✓ denied |
| Output caps | bounded buffer (default 32 KiB per stream) | ✓ capped |
| Secret redaction | `redact_secrets_public` on stdout/stderr | ✓ `[REDACTED]` |
| Timeout enforcement | PTY deadline + process group kill | ✓ terminated |
| Backend abstraction | `SandboxBackend` trait, no OpenSandbox in MCP layer | ✓ decoupled |
| Expected-result contracts | `VerificationResult` separates execution from verification | ✓ explicit |
| Metadata passthrough | Caller-supplied metadata echoed in result | ✓ preserved |
| **Fail-closed backend** | Explicit OpenSandbox config + unavailable → denied, no local fallback | ✓ |
| **Capability transparency** | `sandbox_status` exposes formal `SandboxCapabilities` | ✓ |
| **Repo-state binding** | `RepoState::capture()` before every execution | ✓ deterministic |
| **Freshness detection** | Compare stored vs. current `working_tree_hash` | ✓ |

### OpenSandbox Live Integration

The OpenSandbox backend has a live integration test that runs when
`OPEN_SANDBOX_URL` is configured:

```bash
OPEN_SANDBOX_URL=http://localhost:8080 cargo test --test-threads=1 opensandbox
```

The test skips cleanly when the service is unavailable. Normal unit
tests never depend on a live service.

The HTTP contract (subject to the actual OpenSandbox service):
- POST `<url>/exec` with JSON body `{command, working_directory, timeout, env}`
- Returns JSON `{exit_code, stdout, stderr, duration_ms, timeout, denied, denied_reason, error?}`

The current adapter assumes this contract. If the actual service differs,
the `OpenSandboxExecRequest` / `OpenSandboxExecResponse` structs in
`src/sandbox/opensandbox.rs` should be updated to match.

---

## 7. Explicit non-goals

- **No arbitrary shell execution** through CodeBro. The sandbox enforces a
  strict command policy; arbitrary binaries (`python3`, `bash`, `sh -c`) are
  denied. For unconstrained execution, agents should use their native shell.
- **No filesystem/shell/git read tools** — commodity capabilities already
  present in every host agent. (Note: `engineering_facts` is a *fact*
  search over the verified store, not a source-file search tool.)
- **No MCP *client* support yet** (the P6 `MCP_LIFECYCLE` direction is
  deferred until the server proves valuable).
- **No resource/prompt surfaces yet** — tools only, minimal surface.
- **No OpenSandbox hardcoding** — the MCP layer knows nothing about the
  OpenSandbox HTTP API. Backends are selected by environment, not code.
- **No fake test counts** — semantic tools return the raw execution
  evidence; they do not invent test counts that the underlying command
  does not provide reliably.

---

## 6. MVP acceptance criteria

- [x] `codebro serve` starts over stdio and negotiates with an MCP client.
- [x] All tools above return deterministic, schema-valid responses.
- [x] `apply_change` refuses out-of-workspace paths, blind overwrites and
      stale content.
- [x] Works with **at least one** host agent (OpenCode 1.18 + agnes model —
      verified end-to-end, see §8).
- [x] Zero security regressions: no new secret-leak paths; existing
      redaction authority is respected (verified: secret stored as
      `[REDACTED]` on disk).
- [ ] Works with a second host agent (Claude Code).

---

## 6.1 Security properties (verified by tests)

| Property | Mechanism | Verified |
|---|---|---|
| Path traversal (`..`, absolute outside root) | lexical component check + `starts_with(root)` | ✗ rejected |
| **Symlink escape** (link inside root → outside) | canonicalize target (or nearest existing ancestor for create paths) and compare against canonicalized root; macOS `/var`→`/private/var` handled | ✗ rejected, external target untouched |
| Blind overwrite | unique non-empty `old` match required | ✗ rejected |
| Stale content between prepare/apply | snapshot comparison at apply time | ✗ rejected |
| Secret leakage to disk | `redact_secrets_public` authority: `sk-`/`ghp_`/`glpat`/`xox`/`bearer`/`api_key`/`password=`/`token`/URL-credentials/`--flag` styles | ✗ `[REDACTED]` on disk |
| Memory write bounds | key ≤256 chars, value ≤64 KiB, ≤32 tags of ≤64 chars, confidence/importance clamped | ✗ rejected |
| Fact query bounds | limit clamped 1..50, empty query rejected (or must carry a kind/path filter) | ✗ rejected |
| Determinism | fact search + memory resolution sort stably | ✓ |

The change engine is the **only** mutation path (`apply_change` routes
exclusively through `ChangeEngine::prepare → apply`; no raw writes in the
MCP layer), and `record_memory` is the only memory write surface.

---

## 7. Open questions / follow-ups

- Engineering-memory **recording from the agent loop**: `record_memory` is
  exposed; teaching the host agent *when* to record is prompt/agent work.
- Whether `apply_change` should grow plan-awareness arguments (planned file
  list, strict mode) for cross-file changes.
- Optional SSE/HTTP transport for remote use (deferred).
- Fact store size: ~24 MB JSON for ~13.6k facts. Consider compacting or
  moving the store to SQLite (already a dependency).
- OpenSandbox backend integration: when a remote sandbox API is available,
  set `OPEN_SANDBOX_URL` to switch backends automatically.

---

## 8. Connecting OpenCode

Verified end-to-end with OpenCode 1.18 and the `agnes-2.5-flash` model.

### 8.1 Prerequisites

1. Build codebro: `cargo build --release`
2. (Optional but recommended) Populate facts:
   `codebro init` from the target workspace
3. Verify the runtime: `codebro doctor` — healthy or warnings only

### 8.2 Register the MCP server

```bash
opencode mcp add codebro -- \
  /path/to/codebro/target/release/codebro serve --root /path/to/workspace
```

Confirm it connected:

```bash
opencode mcp list
# ●  ✓ codebro  connected
```

### 8.3 Use it

```bash
cd /path/to/workspace
opencode
```

The tools appear to the agent prefixed with the server name:
`codebro_workspace_context`, `codebro_engineering_facts`,
`codebro_engineering_memory`, `codebro_apply_change`,
`codebro_record_memory`, `codebro_delete_memory`.

For a one-shot non-interactive run with all approvals auto-granted:

```bash
opencode run --auto "What is this workspace? Use codebro_workspace_context."
```

### 8.4 Verified behaviour (2026-08-15)

- Agent discovers and calls `codebro_workspace_context` and reports the
  fact counts (14,470 facts on the codebro repo itself, after dedup
  fixes — see §9).
- Agent calls `codebro_engineering_facts` filtered by kind.
- Agent records memory via `codebro_record_memory`; entry persisted and
  resolvable in later sessions.
- Agent attempts to create a file with a non-empty `old`; `apply_change`
  **refuses** with an actionable error; the agent self-corrects to
  `old=""` and the guarded change succeeds.
- All test artifacts cleaned afterwards; no secrets stored.

### 8.5 Troubleshooting

| Symptom | Cause / fix |
|---|---|
| `opencode mcp list` shows server, tools missing in session | Restart the opencode session; MCP tools are loaded at session start |
| Tool call fails with `-32602` | Argument validation error — read the message; e.g. creating a file requires `old=""` |
| `未提供令牌` / auth errors | Provider API key missing in the environment (`AGNES_API_KEY` etc.) |
| `doctor` reports fact validation issues | `codebro init` again; deterministic rebuild keeps the store consistent |

---

## 9. A/B comparison: with vs without CodeBro

Methodology (2026-08-15): the same prompt, the same model
(`agnes-2.5-flash` in OpenCode 1.18.16), **separate fresh sessions**, on
the codebro repo itself. The "without" session ran with the codebro MCP
server removed from the OpenCode global config; the "with" session had it
registered. Both sessions auto-approved permissions.

### 9.1 Test 1 — qualitative (architecture questions)

Prompt: *"Where is the guarded file-mutation prepare/apply seam? Is there
workspace-boundary enforcement? Which module owns the canonical mutation
path?"*

| | ✅ With CodeBro | ❌ Without CodeBro |
|---|---|---|
| File identified | `src/coding/permissions.rs` | `src/coding/permissions.rs` |
| Key struct | `ChangeEngine` (line 202) | `ChangeEngine` (line 202) |
| Boundary enforcement | `resolve_path` (line 491) | `resolve_path` (line 491) |
| Method used | grep/read (MCP tools **not** preferred) | grep/read |

**Conclusion:** for plain "where is X" questions, a capable agent answers
equally well with grep — CodeBro adds no measurable value here.

### 9.2 Test 2 — quantitative (whole-project facts)

Prompt: *"Report the total number of symbols, tests, and modules, and give
3 example symbol ids."* Ground truth comes from the facts store.

| Metric | ✅ With CodeBro | ❌ Without CodeBro | Ground truth |
|---|---|---|---|
| Symbols | **10,514** (exact, from facts store) | 4,227 ❌ (counted only `pub fn/struct/enum` — missed private items and methods) | 10,514 |
| Tests | **3,602** (exact) | 2,799 ❌ (counted only `#[test]` attrs — missed helpers) | 3,602 |
| Modules | **351** (exact) | 559 ❌ (counted `mod` declarations, not source files — wrong definition) | 351 |
| Example symbol ids | Real ids from the store | **Constructed from a guessed pattern** ⚠️ (agent invented `sym::…::FactId::new@142` from the id format, not from data) | store ids |
| Tool calls | 1 MCP call (`codebro_workspace_context`) | 5+ grep/glob (missed the target) | — |

**Conclusion:** for whole-project factual questions, CodeBro is
materially better: exact counts vs under-counts, and real identifiers vs
invented ones (hallucination risk). This is the differentiating scenario
the product targets.

### 9.3 Key findings

1. **Quantitative questions expose CodeBro's value; qualitative ones do
   not.** An agent can grep its way to "where is the seam" but cannot
   cheaply and correctly answer "how many symbols / list real ids" — it
   under-counts or fabricates. CodeBro's facts store answers exactly.
2. **Agents do not auto-prefer MCP tools.** In Test 1 the agent with
   CodeBro available still chose grep. Value only materialised when the
   prompt instructed the tool. Implication: strengthen the server's
   `instructions` field and/or prompt guidance so agents consult CodeBro
   for project facts by default.
3. **`codebro doctor` and the MCP server agree** (14,470 facts, 0
   validation issues), confirming the store is the single source of truth.

### 9.4 Follow-ups from this test

- Make `ServerInfo.instructions` more directive: "for any question about
  project-wide symbols, modules, tests or decisions, prefer
  `codebro_engineering_facts` / `codebro_workspace_context` before
  grepping."
- Consider a benchmark harness (see advisor's 90-day plan: measure
  context-error reduction across ≥10 tasks).

---

## 10. Auto-detection verification (2026-08-15)

After the directive `ServerInfo.instructions` landed (see §9.4 → done),
re-ran the scenarios **without any mention of codebro in the prompts** to
verify the agent consults CodeBro by itself.

### 10.1 Cross-module impact map

Prompt: *"I'm about to modify the change engine. Which modules reference
ChangeEngine? Give me a map of what I'd be touching."*

Result — exact dependency map: `src/coding/permissions.rs` (owner),
`src/coding/mod.rs` (re-export), `src/mcp/mod.rs:216` (the **only external
consumer**), plus a per-file symbol count of the whole `coding` module
(208 symbols across 6 files). No other module imports `ChangeEngine`.

### 10.2 Change planning

Prompt: *"I want to add a new MCP tool `check_memory_count`. Produce a
precise change plan."*

Tool calls (agent-initiated): `codebro_workspace_context` →
`codebro_engineering_memory` → `codebro_engineering_facts(symbol)`.

Result: a correct, minimal plan — **one file** (`src/mcp/mod.rs`), a
read-only tool following the existing `#[tool]` pattern, tolerant
`memory.load()`, `entry_count()` on the snapshot. No speculative edits.

### 10.3 Memory recording

Prompt: *"I want this constraint remembered: all file mutations must go
through ChangeEngine, never raw writes."*

Agent called `codebro_record_memory` itself with rich metadata:
confidence 0.95, importance 0.9, tags `[architecture, change-engine,
constraint, file-mutations]`, source `colleague-informed`. Entry verified
persisted to `.codebro/engineering_memory.json`.

### 10.4 Anti-amnesia (fresh session)

Prompt (in a **new, empty session**): *"Is there anything in this
project's recorded engineering context I should know before writing
files?"*

Agent called `codebro_engineering_memory` first, **retrieved the
constraint recorded in the previous session**, and connected it to
action: "use `codebro_apply_change` rather than raw writes". This is the
persistent-context scenario the product targets: knowledge survives
across sessions without being re-stated.

### 10.5 Conclusion

- **Auto-detection works**: in all four scenarios the agent consulted
  CodeBro tools before (or alongside) grep — no prompt hints required.
- **Memory lifecycle is closed**: record (10.3) → persist → retrieve in a
  fresh session (10.4) — the anti-amnesia loop is functional end-to-end.
- Facts store used by the agent grew with dependency facts (33
  dependencies from Cargo.toml, validation still 0 issues). Dependency
  discovery since also supports Go `go.mod` (verified on Conductor:
  39 deps — 8 direct, 31 transitive).

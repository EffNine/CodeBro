# CodeBro — Agent Working Notes

## Repo at a glance

- **Language / toolchain:** Rust 2021 workspace. Eleven crates under `crates/` (core, parsers, fact-store, identity-runtime, memory-runtime, sandbox-runtime, impact-engine, indexer, change-engine, context-runtime, mcp-server). No `src/` leaf code — all sources live in `crates/<crate>/src/`. Dependency direction is enforced by `scripts/check_workspace_deps.sh` (mcp-server → services → parsers/core).
- **Binary entry point:** `crates/mcp-server/src/main.rs` → `codebro_mcp_server::run()`.
- **Positioning:** Engineering context & memory layer for AI coding agents, exposed as an MCP server. OpenCode (or any MCP agent client) is the brain/executor; CodeBro owns context, memory, history, learning, skills lifecycle, durable task state,   repository intelligence, and the Engineering Brief (P8 contract: `docs/evolution/P8_IMPLEMENTATION.md`; P9 outcome loop: `docs/evolution/P9_IMPLEMENTATION.md`).
- **Tests:** `cargo test`. Focused: `cargo test <module_name>`. Trust current output — test counts evolve.
- **Build:** `cargo build --release && cargo install --path crates/mcp-server`
- **CLI:** `codebro serve --root <path>`, `codebro init --root <path>`, `codebro doctor --root <path>`, `codebro list-models`, `codebro facts diff --root <path>`.
- **Config:** Optional `~/.codebro/config.toml`. Env vars honoured: `CODEBRO_API_KEY`, `CODEBRO_BASE_URL`, `CODEBRO_MODEL`.
- **Sandbox config:** `OPEN_SANDBOX_URL` activates the OpenSandbox backend. When configured but unavailable, execution fails closed — no silent Local fallback.
- **stdio hygiene (P8):** `codebro serve` MUST keep stdout reserved for JSON-RPC. tracing and the indexer report write to stderr; never add `println!` to any `serve`-reachable path (the P8 E2E harness fails the suite on any non-JSON-RPC stdout line).
- **Project state:** `.codebro/` inside the workspace root (facts.json, engineering_memory.json, project_identity.json, metadata.json). Git-ignored; not part of the source tree.

## Core architecture

MCP-first. The old TUI was removed (ADR-012); preserved only on `tui-legacy` branch.

| Crate / path | Role |
|---|---|
| `crates/mcp-server/src/mcp/mod.rs` | MCP server: 25 tools over stdio (`rmcp` transport) |
| `crates/mcp-server/src/mcp/facts.rs` | Relevance-ranked fact retrieval engine |
| `crates/sandbox-runtime/src/sandbox/` | Sandbox execution abstraction (trait + local + OpenSandbox backends) |
| `crates/indexer/src/init/` | Fact-store population pipeline (`codebro init`) |
| `crates/mcp-server/src/doctor/` | Diagnostics (`codebro doctor`) |
| `crates/fact-store/src/engineering_facts/` | Canonical facts model (symbols, modules, packages, tests, build targets, dependencies, relationships, references, diagnostics, architecture rules) |
| `crates/fact-store/src/fact_store/` | Immutable indexed fact store + validation, lookup, query, snapshot |
| `crates/memory-runtime/src/engineering_memory/` | Persistent engineering memory runtime (load, record, update, delete, snapshot, resolve) |
| `crates/identity-runtime/src/project_identity/` | Project identity runtime |
| `crates/change-engine/src/coding/` | ChangeEngine: guarded mutation seam |
| `crates/context-runtime/src/` | User-context foundation: context records (preference/intent/pattern/experience), sessions + history + recall + learning/inference + skills + durable tasks + P6 repo-index metadata over SQLite+FTS5 (schema v7) (`~/.codebro/state.db`) |
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
8. **User-context state lives in SQLite, not JSON.** `~/.codebro/state.db` (`crates/context-runtime/`) is the only SQLite store: context records, events, sessions, learning/skill tables, P5 tasks/checkpoints, and P6 repo-index metadata (schema v7; `repo_indexes` holds derived counts/status only — never file contents, symbols, or edges). It is user-level and global across workspaces — rows are scoped by `workspace_root`, never by file. JSON stores (facts/memory/identity) are per-project and must never be moved into it. `CODEBRO_STATE_DIR` overrides the state dir (hermetic tests).
9. **Context records are never auto-confirmed.** `AiInferred`/`Observed` records MUST cite evidence event ids (store-enforced, existence-checked against the events table). Promotion to `UserConfirmed` goes through supersede, keeping the audit trail. `USER_CONFIRMED` via the `remember` tool requires the explicit `user_confirmed` flag (caller-principal rule). Task-scoped records require `task_id`. Nothing in context-runtime can write facts/memory/identity files.

## MCP contract

25 tools over stdio (`rmcp`). Handlers are thin adapters — construct runtimes and delegate, do not duplicate logic. Full design: `docs/design/MCP_SERVER.md`.

| Tool | R/W | Description |
|---|---|---|
| `workspace_context` | read | Project orientation: identity, root, fact counts |
| `engineering_facts` | read | Relevance-ranked fact retrieval (lexical matching, not embeddings). Filters: `query`, `kind`, `path`, `limit` |
| `engineering_memory` | read | Resolve persistent memory by task keywords. Entries carry confidence, source, tags |
| `memory_stats` | read | Store stats: entry count, token budget, tag distribution, confidence, recency |
| `record_memory` | write | Upsert a memory entry (secret-redacted). Updating a key replaces the full logical entry |
| `delete_memory` | write | Delete by exact key. **Requires `confirm=true`** — omitting is a no-op |
| `update_identity` | write | Update project identity (`.codebro/project_identity.json`). Requires existing identity. All free-text fields (decisions, constraints, summaries, roadmap, milestones) secret-redacted at write |
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
| `context` | read | Always-available context packet: repository orientation + fact counts, task-relevant facts/decisions/memory/evidence, and resolved context records (per-namespace task > project > global winners + actionable intents, tagged kind/scope/authority). `task_id` includes task-scoped overrides. Task optional; read-only, never writes project or user state |
| `remember` | write | Persist explicitly-confirmed user context (preference or intent) with provenance/scope/lifecycle. `USER_CONFIRMED` requires `user_confirmed=true`; observed/inferred require evidence. Serializes on the workspace mutation lock |
| `forget` | write | Retire a context record by id/namespace (reversible reject; `permanent=true` removes). Requires `confirm=true`. Workspace-confined. Serializes on the workspace mutation lock |
| `recall` | read | Query-driven historical evidence: session-grouped bounded excerpts (decisions, failures, validations, changes) with session/timestamp/scope/task provenance. Task history invisible without its task; `scope=global` opts into cross-workspace search. Read-only — recalls write nothing |
| `learn` | write | Cautious hypotheses from history: run/propose detect recurring patterns (deterministic, ≥3 support); list/get inspect with explanations; evaluate weighs supporting vs contradicting evidence (bounded confidence); accepted hypotheses persist as AI_INFERRED (never USER_CONFIRMED); confirm requires user_confirmed=true; reject preserves negative knowledge. No learn action writes history. Serializes mutating actions on the workspace mutation lock |
| `skill` | write | Skill lifecycle: discover, propose, inspect, validate, approve, reject, deprecate, rollback, health. Evidence-backed candidates (accepted P3 learning only), immutable versioned publication, secret-redacted descriptions/purposes at propose (all identity free-text likewise redacted at update_identity), `user_confirmed`-gated approval, optimistic-concurrency stale-writer refusal, workspace/scope enforcement on every action, atomic + symlink-safe SKILL.md publication, deprecation removes the artifact. CodeBro owns lifecycle; OpenCode executes skills natively |
| `task` | write | Durable engineering task runtime: list, stale, create, inspect, start, pause, resume, checkpoint, validate, validation_result, complete, fail, cancel, outcome (P9: structured outcome evidence — classification + bounded evidence + authority, no transition), skill_refs. Strict lifecycle (pending → running → paused/validating → completed/failed/cancelled) with a completion gate (passed validation required), immutable versioned checkpoints (atomic row+pointer+event in one tx), worker leases with fencing (`wkr::` ids, `lease_version`; stale workers refused), `based_on_version` optimistic concurrency, idempotency-key dedup, interrupted tasks recoverable only via explicit resume (never auto-completed), bounded resume snapshots, workspace isolation at every seam, every free-text field (incl. skill refs) redacted. Request-driven — no scheduler/daemon; OpenCode remains the executor |
| `engineering_brief` | read | P7 decision support: bounded deterministic brief (task + repo intelligence + impact + health + history + memory + learning + skills + task state + constraints/decisions/risks/unknowns). Read-only; OpenCode decides |

## P8 integration contract (agent clients)

- **No new tools.** The 25-tool surface IS the client contract; codified in
  `crates/mcp-server/src/integration.rs::contract` and enforced by tests.
- **Acquisition flow:** orient (`workspace_context`/`context`) → primary
  evidence (`engineering_brief`) → optional targeted follow-up
  (`engineering_facts`/`impact_analyze`/`recall`/`engineering_memory`/
  `repository_health`) → explicit persistence (`remember`/`record_memory`/
  `task`/`learn`/`skill`). Never chain dozens of low-level calls.
- **Server identity:** initialize reports `serverInfo: codebro/<version>`.
- **Client observability:** one stderr tracing line per tool call
  (client, tool, duration, status, response bytes). Identity-only,
  never persisted; never arguments/task text/brief content. The
  subscriber routes ALL stderr log lines (including rmcp transport
  lines, e.g. `response error` echoing tool-error text) through the
  canonical `redact_secrets_public` authority — no caller-supplied
  secret-shaped text can reach stderr logs verbatim (P8 audit F1;
  pinned by `p8_stderr_is_secret_redacted_even_for_transport_error_lines`).
- **Degraded mode:** CodeBro unavailable → the client continues with its
  native tools; stale → explicit `stale`/`STALE_INDEX`; unknown → explicit
  `UNKNOWN` entries. Never fabricated context.
- **Sandbox policy (P8 audit F2):** the local sandbox's inspection-family
  commands (`ls`/`head`/`tail`/`wc`/`find`/`file`/`which`), `cat`, and
  read-only `git` subcommands confine ALL path operands and path-bearing
  flag values to the workspace root — absolute-path operands outside the
  root are denied (previously `head /etc/passwd`-class escapes were
  possible). `find`'s output-writing forms (`-fprint*`/`-fls`) and
  `git --output=` are treated as paths and confined too.
- **Root authorization (P8 security boundary closure, audit F3 FIXED):**
  a CodeBro operation may only access a filesystem root that is
  explicitly authorized for the server process. The server root
  (`--root` / `CODEBRO_WORKSPACE_ROOT` / cwd) is always authorized;
  additional roots are authorized ONLY by the operator at launch via
  repeatable `--allow-root <path>` flags and/or the `CODEBRO_ALLOW_ROOTS`
  env var (`:`-separated on unix). A per-call `workspace_root` argument
  is **discovery** (which authorized root the call addresses), never
  **authorization** — it must canonicalize (symlinks, `..`, alternate
  spellings resolved) to exactly one authorized root; every other value
  is a bounded `-32602` refusal before any workspace state is created
  or file touched. Authorization is exact-root (not prefix-based:
  authorizing `/work/repo` does not authorize `/work/repo/sub` or
  `/work/repo2`); nested/overlapping roots may both be authorized
  deliberately and keep separate state; symlinks are governed by their
  canonical target; the authorized set is immutable for the process
  lifetime and is re-derived from launch config on every restart (never
  broadened by prior tool traffic, task ids, skills, memory, or
  history). Existence, git structure, prior observation, or client
  identity never authorize a root. Pinned by
  `tests/p8_security_boundary_e2e.rs` (real binary: `/etc`-class roots
  refused, victim dirs untouched, allowlist honored, restart/hard-kill
  determinism, authorized multi-root isolation) and the
  `workspace_registry` unit battery.

## P9 outcome & feedback loop (agent clients)

- **No new tools.** The 25-tool surface remains the contract. P9 adds one
  `task` action (`outcome`); schema stays v7 with no new tables.
- **Report outcomes, don't re-execute:** after OpenCode does the work with
  its own tools, it reports structured evidence via
  `task outcome` (`classification` ∈ success|partial|failure|rejected|
  superseded, bounded `summary`, optional bounded evidence/command/
  exit-code/changed-areas, `user_confirmed` speech act, `dedup_key`
  idempotency). CodeBro persists the report as a task-bound `task_outcome`
  history event — no transition, no lease, no row mutation, works on
  terminal tasks (user confirmation arrives after completion).
- **Trust:** OpenCode reports are `observed` unless `user_confirmed=true`
  (explicit user speech act). Inferred lessons come only from `learn`
  acceptance (`AI_INFERRED`, never self-confirmed). Superseded outcomes
  are polarity-neutral (`replaced`) so abandonment never reads as success.
- **Feedback:** outcomes surface in `inspect` snapshots (current task),
  `recall`/brief history excerpts (past tasks, keyword-relevant), and P3
  learning once evidence accumulates (≥3 support, contradiction-aware).
  Single outcomes never auto-become best practices; failures never
  auto-taint an approach.
- **Robustness (P9 hardening):** FTS5 lock-contention check lines report
  busy (never quarantine); opens retry boundedly; quarantine is fenced by
  retries + a file-quiescence gate with pid-unique debris; outcome writes
  use BEGIN IMMEDIATE so concurrent cross-process writers serialize.
- **Completion convention (hardened contract, workflow-level only).**
  `task complete` does NOT require a prior `outcome` (user confirmation
  routinely arrives after completion, and CodeBro cannot force the executor
  to report — no daemon, no scheduler, no enforcement). The convention is:
  every completed/failed task SHOULD get one `task outcome` report so the
  loop closes. Compliance is measurable without new telemetry: the existing
  P8 per-call stderr lines already record every tool call (name-only, no
  content), so `outcome` calls ÷ `complete`+`fail` calls is the compliance
  ratio. If the ratio is low, the fix is workflow guidance (OpenCode
  AGENTS.md / skills), never CodeBro-side coercion.
- **Retention (hardened assessment).** Every write is per-record bounded
  (`MAX_TASK_*`, `MAX_CHECKPOINT_*`, `MAX_OUTCOME_*`, history `MAX_*`,
  excerpt caps); row counts (checkpoints, history events, context records)
  are unbounded by design. Growth is ~1 row per high-value operation —
  SQLite-comfortable for single-user engineering use for years. There is
  deliberately NO background cleanup, NO scheduler, NO auto-pruning (all
  would violate the request-driven boundary). If state ever needs
  compaction, it must be an explicit operator-requested maintenance
  operation — not silent machinery. Not built; not needed at current scale.
- **Lease recovery (P5 behavior, operational note).** Worker leases live
  `TASK_LEASE_TTL_SECS` (15 min). After a hard kill, the dead worker's
  lease stays live until expiry; mutations are refused until then, and only
  explicit `resume` recovers the task (never auto-completed). This is
  fencing-correct and intentional — the wait is availability latency, not a
  correctness bug, so the TTL is unchanged. Recovery procedure: wait out
  the TTL (≤15 min), then `resume`; fencing guarantees the dead worker can
  never split-brain the task.

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

**Mutation authority (hardened contract — OPTION B).** These guarantees apply
ONLY to edits made through CodeBro's `apply_change` / `apply_changes`.
OpenCode-native edits (the default coding path — "For normal coding edits use
your native editing tools") bypass the ChangeEngine entirely: no boundary,
traversal, symlink, stale-content, or ambiguity checks come from CodeBro for
those edits, and CodeBro makes no claim about them. The three cases:

1. **OpenCode-native edits** — full agent capability, zero CodeBro mutation
   guarantees. CodeBro observes them only through the freshness protocol
   (`reindex` → `fresh`; unindexed changes surface as `STALE_INDEX`).
2. **CodeBro ChangeEngine edits** (`apply_change` / `apply_changes`) — the
   seven guarantees above, plus `recommended_tests` and the `needs_reindex`
   freshness signal. For controlled/autonomous workflows where CodeBro
   guarantees are expected, this is the documented path.
3. **CodeBro state writes** (`record_memory`, `remember`, `task`, `skill`, …) —
   governed by the per-workspace mutation lock, secret redaction, and
   authority gates — never by the ChangeEngine (different seam, different
   guarantees).

Never describe ChangeEngine guarantees as covering the workspace as a whole.

Within one server process, all mutating MCP tools (`apply_change`, `apply_changes`, `record_memory`, `delete_memory`, `update_identity`, `reindex`, `remember`, `forget`, `learn`, `skill`, `task`) serialize on a per-workspace lock (`CodeBroMcpServer::mutation_lock`); new mutating tools must acquire it as their first statement. Cross-process writers still rely on the single-writer assumption.

**Execution authority (hardened contract).** OpenCode owns command execution —
host shell, OpenSandbox, coding operations, test runs it initiates. CodeBro's
`sandbox_exec` / `sandbox_test` / `sandbox_build` are NOT a second execution
engine: they exist to produce CodeBro-attributed verification evidence
(diagnostics, freshness inputs, root-cause hypotheses, `recommended_tests`
targeting) for CodeBro's own intelligence needs. They run read-only
build/test/lint commands only, fail closed when the OpenSandbox backend is
configured but unavailable (never silently fall back to Local), and confine
inspection-family path operands to the workspace root (P8 audit F2). For
general execution, agents must use OpenCode/OpenSandbox — never route around
it through CodeBro sandbox tools.

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
- Test hermeticity for user-context state: every test server must use an
  explicit state dir (`with_state_dir` / `CODEBRO_STATE_DIR`), never the
  default `~/.codebro`. Passive history capture writes on remember/forget/
  apply/test paths, and spawned `codebro serve` children inherit the
  parent env — set `CODEBRO_STATE_DIR` on the child (P2 lesson: the full
  suite once quarantined the developer's real state.db; content-proven
  test junk only, but the suite must never touch home).- Targeted test selection: `apply_change` responses carry `recommended_tests` (tests whose own body was edited or which exercise an edited symbol via `TestFact.tested`, determined from the edited line range, sorted, capped at 32); pass them to `sandbox_test` as `test_filter` to run exactly those. Filters are honored for cargo/go/pytest runners and ignored where no standard selection exists; go supports single-name targeting only (multi-filter falls back to the full suite); an explicit `command` overrides filtering entirely.
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

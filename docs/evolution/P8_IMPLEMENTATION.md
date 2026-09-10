# CodeBro P8 Implementation — OpenCode Integration Layer

**Status:** COMPLETE (thin integration layer over the P0–P7 core; additive only).
**Schema:** v7 (unchanged — no new persistence).
**MCP tools:** 25 (unchanged — P8 adds NO tools; the existing surface IS the contract).
**Architecture:** OpenCode decides WHEN context is needed; CodeBro decides WHAT context
is relevant; CodeBro persists meaningful engineering knowledge through explicit write
tools with unchanged authority gates. No agent loop, no scheduler, no daemon, no
watcher, no model calls, no skill execution, no remote transport.

---

## 1. Mission

P0–P7 built the engineering context/runtime core and hardened it to release
readiness (1383/1383, zero unresolved findings). P8 makes CodeBro a *natural*
companion for OpenCode — and any MCP agent client — by fixing the
integration-correctness defects found by tracing the actual request path,
formalizing the client contract, and proving the full mission workflow live:

```text
OpenCode receives engineering task
        ↓
OpenCode determines context is needed
        ↓
OpenCode calls CodeBro (existing MCP tools)
        ↓
CodeBro retrieves relevant engineering context (engineering_brief)
        ↓
CodeBro returns bounded evidence with provenance/freshness
        ↓
OpenCode reasons, codes, executes (its own tools)
        ↓
CodeBro persists meaningful outcomes (explicit write tools, gated authority)
        ↓
Future tasks receive better context (recall/learning/memory)
```

P8 deliberately did **not** build a second context system, a new protocol, or a
single new tool. The ideal P8 is thin: three small product fixes (each a real
defect demonstrated live), one observability seam, contract documentation, and
test coverage that pins the contract against regressions.

## 2. Architecture

Unchanged. P8 sits entirely inside `crates/mcp-server`:

| Piece | Location | Nature |
|---|---|---|
| Integration contract (intent → tool map, observation model) | `crates/mcp-server/src/integration.rs` (new) | additive |
| Server identity fix | `mcp/mod.rs` `get_info` | defect fix |
| Per-call observability wrapper | `mcp/mod.rs` `call_tool` override | additive wrapper |
| Tracing writer fix (stdout → stderr) | `lib.rs` `run()` | **defect fix** |
| Indexer report writer fix | `crates/indexer/src/init/mod.rs` (`println!` → `eprintln!`, 20 sites) | **defect fix** |
| Real-binary E2E probes | `crates/mcp-server/tests/p8_integration_e2e.rs` (new, 5 tests) | additive |

No changes to: context-runtime, fact-store, memory-runtime, identity-runtime,
impact-engine, parsers, change-engine, sandbox-runtime. Dependency direction
verified (`scripts/check_workspace_deps.sh` OK). Schema v7 untouched.

## 3. OpenCode Integration Contract

The contract is the **existing 25-tool MCP surface**, formalized as
`integration::contract::intents()` — an ordered map from agent-client intent
to the exact tool that serves it, enforced by regression tests
(`p8_integration_contract_intents_are_routed_tools`,
`p8_contract_surface_is_the_existing_25_tools`) rather than prose:

| Intent | Tool | Role |
|---|---|---|
| primary_context | `engineering_brief` | THE high-level context surface: one bounded call covering repo identity, freshness, files, symbols, dependencies, impact, tests, health, history, memory, learning, skill applicability, task state, constraints, decisions, risks, unknowns |
| orientation | `context` | always-available packet (structural digest without a task) |
| workspace_orientation | `workspace_context` | repo + fact counts + live/persisted freshness |
| facts | `engineering_facts` | targeted follow-up: verified facts |
| impact | `impact_analyze` | targeted follow-up: structural impact + risk signals |
| history | `recall` | targeted follow-up: decisions/failures/validations evidence |
| memory | `engineering_memory` | targeted follow-up: memory resolution |
| health | `repository_health` | targeted follow-up: workspace health |
| task_state | `task` | durable task lifecycle (state, never execution) |
| remember | `remember` | user-confirmed preference/intent persistence |
| record_memory | `record_memory` | durable agent-recorded engineering memory |
| learn | `learn` | hypothesis lifecycle (never self-confirmed) |
| skill_lifecycle | `skill` | CodeBro manages lifecycle; OpenCode executes natively |
| reindex | `reindex` | freshness recovery |

Why no new tool was needed (P8 §5 justification): every requirement in the
mission maps onto an existing capability. Discovery/connection = MCP
initialize + `codebro serve`. Workspace identity = `--root`/env +
canonical `workspace_root` per tool. Task identity = P5 `task::` ids. Brief =
`engineering_brief` (P7). Targeted evidence = existing read tools.
Persistence = existing write tools. Freshness/provenance/unknowns already
survive every response. A new tool would duplicate P7.

## 4. MCP Integration

Three real integration defects were found by tracing the actual request path
and were fixed:

### 4.1 Stdout protocol corruption (defect, fixed)

`codebro serve` ran `tracing_subscriber::fmt()` with the default writer —
**stdout**. On a stdio MCP server, stdout is the JSON-RPC channel; any log
line corrupts the framing. Observed live before P8: an `ERROR … MCP server
failed to start` line interleaved on stdout (the pre-P8 test harnesses
silently skipped non-JSON lines to work around it — the corruption was real
but unasserted). Additionally, `reindex` invoked the indexer pipeline whose
progress report used `println!` (20 sites) — every MCP `reindex` call pushed
`relationships: 1` etc. onto the protocol channel.

Fixes:
- `lib.rs`: tracing now `.with_writer(std::io::stderr)`.
- `indexer/init/mod.rs`: all pipeline reporting is `eprintln!` (CLI output
  remains visible in terminals; MCP stdout stays pure JSON-RPC).
- New E2E probe asserts every stdout line between responses is valid
  JSON-RPC with a `jsonrpc` field — the corruption is now a test failure,
  not a silent skip.

### 4.2 Server identity (defect, fixed)

`initialize` answered `serverInfo: {"name": "rmcp", …}` — the SDK's
`from_build_env()` default. Clients display and may route on server
identity; every CodeBro instance masqueraded as the transport library.
Fixed: `serverInfo.name = "codebro"`, `version = CARGO_PKG_VERSION`,
with instructions preserved. Pinned by
`p8_server_info_identifies_the_product` + the real-binary probe.

### 4.3 Client observability (additive)

`call_tool` is now an explicit wrapper around the (unchanged) router:
one bounded tracing line per call — `client=<name/version> tool=<tool>
duration_ms=<n> status=<ok|error> response_bytes=<n>`. Client identity
comes from the peer info rmcp's default `initialize` registered; it is
process-local and **never persisted** (no client-specific state exists —
the store stays workspace-scoped). Never logged: arguments, task text,
brief content, secrets. Client names and error summaries pass through the
canonical `redact_secrets_public` authority (defense in depth), error
summaries are capped at 240 chars. Pinned by
`p8_observation_never_carries_payloads`, `observation_line_*` unit tests,
and the real-binary stderr assertions.

**Post-audit hardening (see `P8_POST_IMPLEMENTATION_AUDIT.md` finding
F1):** rmcp's transport layer independently logs `response error` lines
carrying raw tool-error messages, which echo caller-supplied input. The
tracing subscriber's writer therefore now routes EVERY formatted log
line (rmcp's included) through `redact_secrets_public`
(`lib.rs::RedactingStderr`), pinned by
`p8_stderr_is_secret_redacted_even_for_transport_error_lines`.

## 5. Workspace Identity

Unchanged and re-proven through real OpenCode: `--root` argument >
`CODEBRO_WORKSPACE_ROOT` env > current directory; canonicalized; no
git-root walk-up (never silently widen the ChangeEngine boundary); per-tool
`workspace_root` arguments canonicalize into the caller's namespace
(traversal probes leaked nothing — final gate re-proved, P8 E2E re-proved
cross-workspace task refusal live: `task … belongs to another workspace`).
Ambiguity is impossible: one server process = one default root, explicit
per-call overrides canonicalize, multi-root service uses one process per
root.

## 6. Task Identity

P5 `task::<hex>` ids, minted store-side, opaque, existence-checked,
workspace-gated. The P8 flow verified live through real OpenCode:
create (`task::530c689d37cc8607`) → start → checkpoint (`cp::…`) →
(complete path available). CodeBro persists durable state only; it never
executes, schedules, or auto-progresses. OpenCode's own transient execution
state is never duplicated — checkpoints are explicit, requested, and
meaningful (summary of what actually happened).

## 7. Engineering Brief Flow

Verified live, end to end, twice (real-binary probe + real OpenCode):

1. Task arrives (natural language) → OpenCode calls `engineering_brief`
   with task text (+ optional task_id/target/keywords).
2. Brief assembly (unchanged P7 pipeline) returns repository identity,
   live+persisted freshness, relevant files/symbols/dependencies, one
   bounded impact traversal with risk signals, relevant tests, health,
   history excerpts, memory, learning, skill applicability, read-only
   task state, constraints/decisions, risks, explicit unknowns — every
   section with `category` + `provenance`, authority verbatim.
3. OpenCode reasons and reports the evidence (observed: it correctly
   reported `fresh` freshness, the `doubles` test, MEDIUM risk with
   blast radius 1/1/0).
4. After its own edit, OpenCode re-requested the brief; CodeBro answered
   `stale` + `STALE_INDEX` — honestly labelling structural evidence as
   pre-change state.

## 8. Context Acquisition

The P8 acquisition pattern (documented in the initialize instructions and
this doc, enforced by nothing but the client's own judgment — as intended):

```text
task arrives → workspace_context/context (orient) → engineering_brief
(primary evidence) → [optional] targeted follow-up (facts/impact/recall/
memory/health) → reason/execute → [meaningful outcomes] remember/
record_memory/task/learn/skill
```

No client is required to call "dozens of low-level tools": one brief call
covers the evidence surface; follow-ups are optional and semantic.

## 9. Targeted Retrieval

Existing semantic tools only (P8 §14 satisfied without new surface):
"show evidence for this dependency" → `engineering_facts`; "why is this
module high impact" → `impact_analyze`; "relevant historical failure" →
`recall`; "which skills apply" → brief `skills[]` applicability +
`skill` inspect. All bounded (existing caps), scoped (workspace gates),
deterministic (existing rankers). No second context system.

## 10. Persistence

Unchanged P0–P7 semantics, re-proven through the E2Es: history events are
passive (attached to explicit tool paths: remember/apply/test/task
transitions); records are explicit (`remember`); agent memory is explicit
(`record_memory`); task state is explicit (`task` lifecycle); nothing
persists briefs; nothing persists model chatter; nothing auto-persists
arbitrary output. Verified: restart persistence (task + checkpoint +
preference survived a fresh OpenCode session backed by a fresh server
process over the same state.db).

## 11. Learning

P8 touches nothing in P3. The integration path feeds learning exactly the
way P5/P6/P7 did: task/validation outcomes become history events through
the task lifecycle; `learn run` detects patterns from real evidence (≥3
support, evidence-cited); accepted hypotheses are `AI_INFERRED`, never
`USER_CONFIRMED`; the model can never self-confirm (store + MCP gates,
re-proven by the final gate). OpenCode output is not truth: any "approach X
is always better" claim must go through `learn`/`remember` with the
unchanged authority gates (AI_INFERRED needs evidence; USER_CONFIRMED
needs the explicit user-confirmation speech act).

## 12. Memory

What qualifies (documented, unchanged): confirmed architecture, important
decisions, recurring failure patterns, repo-specific conventions,
validated workflows, meaningful constraints. Not qualified: conversation
chatter, transient reasoning, generated code, debugging noise. The
memory store stays bounded (≤20 entries resolved, 500-token budget,
confidence ≥ 0.3) and separate from the verified fact store (hard rule —
never promoted). The redaction authority runs at the write seam —
verified live through real OpenCode: a secret-shaped memory value stored
as `api key [REDACTED] …` and surfaced redacted.

## 13. Skill Applicability

Unchanged P4/P7 semantics: briefs carry applicability information only
(applicable + reason, task refs resolved); CodeBro never executes skills;
OpenCode discovers published SKILL.md natively. `skill` lifecycle actions
retain their gates (user_confirmed approval, validated status, confidence
floor). No P8 change; E2E covers no-execution language (pinned since P7).

## 14. Freshness

Contract (unchanged, survives integration, verified live): `fresh` only
when the stored generation hash matches the current working-tree hash;
`stale` after unindexed changes (verified: brief flagged `STALE_INDEX` +
risk signal after OpenCode's own edit); `unknown` outside git or absent
signals (verified in the first OpenCode leg pre-index — never fabricated
`fresh`); persisted `READY/STALE/FAILED/UNKNOWN` alongside live status;
`FAILED` preserves last-good. Stale intelligence is labelled
last-indexed-state evidence, never current truth.

## 15. Provenance

Six authorities (user_confirmed > project_derived > system_derived >
imported > observed > ai_inferred) flow verbatim through every brief
section (P7), context packet, and read path. Nothing is upgraded for
relevance. The P8 observability seam adds no new provenance surface and
cannot: it observes tool calls, not content, and redacts even bounded
vocabularies.

## 16. Security

P8 re-audited the integration path with the existing guarantees (no new
secret leak introduced):

- **Write seams** (unchanged, re-proven live via real OpenCode):
  `remember`, `record_memory`, `update_identity`, `task` — all
  redacted at write (`[REDACTED]` verified live).
- **Read paths** (unchanged): brief/context/recall/memory/task inspect
  carry no raw payloads, row ids, or secrets.
- **New observability seam**: logs client identity + tool name +
  duration + status + size only; secret-redacted; never persisted. Pinned
  by unit tests and the real-binary stderr assertions.
- **Malformed input** (re-proven): empty brief scope rejected;
  cross-workspace task refused with no existence/content leak; oversized
  inputs bounded (final-gate probes remain green).
- **Workspace/task traversal**: canonicalization + workspace keys (P8
  E2E + final gate).
- **Host hygiene**: full suite left `~/.codebro` byte-identical
  (md5-verified); E2E state isolated via `CODEBRO_STATE_DIR` /
  `CODEBRO_SKILLS_DIR`; the user's real repository and real skills
  untouched.

Known accepted echo (documented since P7): the brief echoes the caller's
own ad-hoc `task`/`keywords` text — same-channel caller echo, never
persisted.

## 17. Error Recovery

Contract (documented for the client, enforced by the existing failure
model): CodeBro unavailable → OpenCode continues with its own native
tools (MCP clients already handle server-down by degrading that server's
tools). CodeBro stale → explicit `stale`/`STALE_INDEX`, never silently
current. CodeBro unknown → explicit `UNKNOWN` entries. Partial store
failures degrade to explicit unknowns, additive, never silence (P6/P7
failure model — re-proven by the corrupt-db adversarial test which stays
green).

## 18. Degraded Mode

`FULL_CONTEXT` (fresh index, all sections resolving) → `PARTIAL_CONTEXT`
(some sections empty with their `NO_*` unknown) → `STALE_CONTEXT`
(`STALE_INDEX`, evidence labelled last-indexed) → `NO_CONTEXT` (server
unavailable; OpenCode native). Never fabricated: every degradation state
is explicit in the response payload; the first OpenCode E2E leg observed
`NO_RELEVANT_TESTS` + empty tests honestly on an unindexed repo.

## 19. Performance

Measured (real-binary E2E, debug-build server, hermetic repo):

- MCP startup → initialize response: < 50 ms (cold process, debug build).
- `engineering_brief` end-to-end: single-digit milliseconds on the
  hermetic repo; the full 5-probe suite (2 initialize handshakes, ~30
  tool calls incl. 4 reindexes) completes in ~1.5 s.
- Release build + real OpenCode: reindex of the 14-fact repo + brief +
  reasoning round-trip completes within the model latency budget
  (interactive).
- No optimization performed (P8 §27: measure, don't prematurely tune);
  bounds are the existing per-section caps + 256 KiB envelope, unchanged.

## 20. Determinism

Re-proven by P8 probes: repeated identical brief requests agree
byte-for-byte (`assert_eq!(a, b)` on parsed payloads, post-restart);
reordered inputs and concurrency behavior are pinned by the P7 suites
(all green). The observation line carries a wall-clock duration — it is
process-local telemetry, never persisted, never part of any response
payload; response determinism is unaffected.

## 21. Concurrency

Re-proven by `p8_client_identity_observability_and_two_client_concurrency`
(two real server processes, same workspace + shared state.db: concurrent
reindex, briefs from both, task write on A visible to B, all well-formed,
no deadlock). In-process locking (per-workspace mutation lock, WAL,
leases/fencing) unchanged; cross-process single-writer assumption
unchanged (documented debt).

## 22. Multi-Agent Compatibility

P8 formalized the client as a generic MCP agent client — nothing
OpenCode-specific entered the domain: no client name in any store, no
client-specific tool, no client-scoped state. Client identity exists only
as process-local observability. OpenCode is the first consumer, not the
owner of the semantics; Claude Code/Codex/Cursor/Goose connect the same
way (`opencode mcp add codebro -- codebro serve --root <repo>` or any
client's local-MCP equivalent). The remote process model remains
compatible-without-redesign (stdio → HTTP transport swap is an rmcp
concern; the domain layer has no transport coupling) — not implemented,
per P8 §21.

## 23. Configuration

Minimum client configuration (unchanged, documented):

```json
{ "mcp": { "codebro": {
    "type": "local",
    "command": ["codebro", "serve", "--root", "/path/to/repo"]
} } }
```

Or per-repo via `CODEBRO_WORKSPACE_ROOT`. Optional: `RUST_LOG` for
observability (stderr), `CODEBRO_STATE_DIR` / `CODEBRO_SKILLS_DIR` for
isolation. No daemon, no custom ports, no dozens of parameters — boring
by design.

## 24. Observability

Per §4.3: one stderr line per tool call; one at startup; nothing else new.
Never logged: secrets, raw task content, briefs, transcripts, API keys.
Never persisted: client identity. `RUST_LOG` gating respected
(`info` default produces the call lines; `off` silences everything).

## 25. Testing

| Suite | Tests | What it pins |
|---|---|---|
| `integration::tests` (unit) | 6 | observation rendering bounded+redacted (incl. hostile secret-shaped client names), error-summary truncation+redaction, contract intents map to existing tools only (no CRUD), server identity constant |
| `mcp::tests` (MCP) | 3 | `get_info` reports codebro+version+instructions; contract intents all routed; observation never carries payloads |
| `tests/p8_integration_e2e.rs` (real binary) | 5 | stdout purity (strict: every stdout line must be valid JSON-RPC — catches the pre-P8 corruption class), stderr observability content, server identity over the wire, 25-tool contract surface, full mission flow (orient → task → brief → targeted facts → own edit → stale → reindex → fresh → remember/record_memory → lifecycle complete → restart persistence → determinism → secret non-leakage → cross-workspace isolation), two-client concurrency |

New-test count: 14 (6 unit + 3 MCP + 5 E2E). Full workspace suite:
**1397 passed / 0 failed** (1383 baseline + 14). Clippy `-D warnings`
clean, `cargo fmt --check` clean, `scripts/check_workspace_deps.sh` OK.
Hermetic: `tempfile::tempdir()` everywhere, explicit
`CODEBRO_STATE_DIR`/`CODEBRO_SKILLS_DIR`, `~/.codebro` md5-verified
byte-identical before/after the full suite.

## 26. Real Binary E2E

`tests/p8_integration_e2e.rs` runs against the real `codebro` binary over
stdio RPC (CARGO_BIN_EXE), with stderr captured for observability
assertions and a strict stdout line validator. All 5 probes green
(details §25). The strict validator is the P8 regression hook for the
stdout-corruption class: any future print on the MCP stdout fails the
suite loudly.

## 27. Real OpenCode E2E

Performed against **OpenCode 1.18.29** (real binary, real model
agnes-2.5-flash) with hermetic config (XDG redirection), hermetic repo
(git-initialized), hermetic state/skills dirs. Legs verified live:

1. Start CodeBro MCP + connect OpenCode — initialize handshake, server
   reported as `codebro`.
2. Open a test repository — workspace_context oriented correctly.
3. Give OpenCode an engineering task — natural language task.
4. OpenCode obtains CodeBro context — `engineering_brief` called.
5. Brief returned — sections parsed and reported by the agent.
6. OpenCode reasons using the evidence — correctly reported test linkage
   (`doubles`), risk level (MEDIUM, blast radius 1/1/0).
7. Harmless change executed — OpenCode edited `src/lib.rs` with **its
   own** tools and ran `cargo test` itself.
8. CodeBro did not execute the change — verified: the edit came from the
   agent; CodeBro only answered staleness afterwards.
9. Staleness detected honestly — brief reported `stale` + `STALE_INDEX`.
10. Reindex restored freshness — next brief `fresh`, tests resurfaced.
11. Meaningful outcome persisted — task create → start → checkpoint
    (`cp::b332ecd06a68672d`, "added negate function").
12. Restart persistence — a fresh OpenCode session (new server process)
    read the task, its checkpoint, and a `user_confirmed` preference
    persisted in the prior session.
13. No secret leakage — secret-shaped memory value stored `[REDACTED]`,
    surfaced `[REDACTED]` (verified in the agent's own report).
14. Workspace isolation — a second workspace's server refused the first
    workspace's task id (`belongs to another workspace`), zero content
    leak; `~/.codebro` byte-identical throughout.

## 28. Documentation

- This document (required).
- `AGENTS.md` — P8 section added (integration contract summary, stdout
  hygiene rule, observability seam, tool count 25, schema v7).
- `CHANGELOG.md` — P8 entry added.
- `docs/MCP_API_V1.md` — P8 note (identity fix, stderr contract,
  observability; still 25 tools, v7).
- `docs/evolution/PHASE_PLAN.md` — does not exist in this repository
  (no such file was ever created; the mission's "update as appropriate"
  therefore does not apply — noted here for auditability).
- `docs/design/MCP_SERVER.md` §8 (Connecting OpenCode) remains accurate.

## 29. Known Debt

New (small, documented):

1. The strict-stdout validator exists only in the P8 E2E harness; the
   server itself does not re-validate its own stdout at runtime (by
   design — a stdio server cannot intercept its own process stdout
   cheaply; the test suite is the guard).
2. Observability lines carry wall-clock durations — fine for telemetry,
   but any future log-based testing must not depend on them (none does).
3. `codebro facts diff`'s report still prints to stdout — correct, it is
   a CLI command whose stdout IS its output; only `serve`-reachable paths
   were required to be stderr-clean.
4. Remote process model remains documentation-only (P8 §21 respected).

**Post-audit additions** (from `P8_POST_IMPLEMENTATION_AUDIT.md`):

5. **Per-call `workspace_root` accepted any existing host directory**
   (MEDIUM, audit finding) — **CLOSED** by the P8 security boundary
   closure (`docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`): the registry
   now enforces an explicit operator authorization model (Model B). The
   server root is always authorized; additional roots are authorized only
   by the operator at launch (repeatable `--allow-root <path>` flags
   and/or `CODEBRO_ALLOW_ROOTS`). A per-call `workspace_root` must
   canonicalize to exactly one authorized root or the call is refused with
   bounded `-32602` before any workspace state is created. Real-binary E2E
   (`tests/p8_security_boundary_e2e.rs`, 10 probes) plus the
   `workspace_registry` unit battery pin the boundary; real OpenCode
   E2E re-verified (authorized flow + unauthorized refusal + allowlist
   flow). The audit's recommended follow-up is implemented as the
   explicit-flag variant.
6. The local sandbox policy was hardened during the audit (finding F2:
   inspection-family commands and read-only git now confine all path
   operands to the workspace root — previously `head /etc/passwd`,
   `find -fprint`, `git --output=`, and `cat /abs/path` escaped). The
   OpenSandbox backend delegates confinement to its own network/filesystem
   policy and was not affected.

Carried (unchanged from the P0–P7 final gate, all non-blocking): the
twelve documented debts (§33 of the gate) including task↔skill read-time
association, `task_id` namespace mixing across seams, cross-process
single-writer assumption, and the same-channel caller echo convention.
Post-hard-kill task recovery remains TTL-bound (carried P5 debt: an
interrupted RUNNING task refuses mutation until the 15-minute lease
expires; only then does explicit `resume` take over).

## 30. Final Status

| Criterion | Result |
|---|---|
| OpenCode can reliably consume CodeBro | PASS (real OpenCode 1.18.29 E2E, 14 legs) |
| Engineering Brief accessible naturally | PASS |
| Workspace identity correct | PASS (isolation re-proven live) |
| Task identity correct | PASS (durable, restart-surviving) |
| Provenance survives integration | PASS (verbatim authorities) |
| Freshness survives integration | PASS (fresh/stale/unknown cycle live) |
| Errors degrade safely | PASS (unknowns, refusals, no fabrication) |
| Security intact | PASS (redaction live-verified; no new leak seam) |
| No autonomous behavior introduced | PASS (no scheduler/daemon/watcher/loop) |
| No execution ownership moved into CodeBro | PASS (agent executed its own change) |
| P0–P7 regression | PASS (1397/1397; clippy/fmt/deps clean) |
| Real binary E2E | PASS (5 probes incl. strict stdout purity) |
| Real OpenCode E2E | PASS |
| Deterministic behavior | PASS (byte-equality pinned, incl. restart) |
| Concurrency | PASS (two-client probe) |
| Schema | v7 unchanged |
| MCP tools | 25 unchanged |
| `~/.codebro` pollution | NO (md5 byte-identical) |
| Real repository mutation | NO (hermetic tempdirs only) |
| Real skill mutation | NO (hermetic skills dir, empty) |

**P8 COMPLETE.** No execution ownership, no agent behavior, no new
stores, no new tools, no schema change — a thin integration layer whose
every change is either a demonstrated defect fix, an observability seam
with leak-proofing, contract formalization, or test coverage.

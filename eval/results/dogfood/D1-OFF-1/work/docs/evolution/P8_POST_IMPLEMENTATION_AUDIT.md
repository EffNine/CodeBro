# CodeBro P8 Post-Implementation Adversarial Audit

**Date:** 2026-09-08 · **Auditor:** independent adversarial audit (this document)
**Scope:** the complete P8 OpenCode integration layer — MCP/JSON-RPC protocol behavior of the real `codebro` binary, stdout/stderr hygiene, server identity, the 25-tool contract surface, workspace/task identity, cross-workspace and cross-task security, secret handling through every seam, hard-kill/restart recovery, determinism, boundedness, concurrency, the architecture boundary (OpenCode decides/executes; CodeBro provides context/state only), real-OpenCode E2E, and P0–P7 regression — plus re-attack of the P7 security boundary through the P8 integration path.

**Method:** do not trust the implementation report. Attack the actual system: standalone protocol attack harnesses driven against the real binary over stdio RPC (18 protocol attacks + 18 isolation/recovery/determinism attacks, ~3000 lines of probe code), targeted live probes (sandbox escape battery, secret-echo battery, workspace-identity battery, freshness cycle, hard-kill with SIGKILL, corrupt-store degradation), a full real-OpenCode E2E (14 legs, hermetic XDG config/state), source inspection of every P8-touched file and every serve-reachable print path, and the complete test suite.

---

## 1. Executive Summary

P8's two self-reported defect fixes (stdout protocol corruption; wrong server identity) are **real and verified fixed**. The audit re-proved both live and could not break them again (18 protocol attacks, every stdout line validated as JSON-RPC across all 25 tools including error paths, malformed input, garbage interleave, and restart).

The audit found **two additional real HIGH defects** the implementation report did not know about, both in the P8-relevant integration path:

- **F1 (HIGH) — stderr secret leakage through rmcp transport logs.** rmcp independently logs `response error` lines carrying the raw tool-error message; CodeBro tool errors echo caller-supplied input; so a secret supplied in a rejected tool argument (`action: "sk-…"`, `task_id: "sk-…"`, `key: "sk-…"`) reached the stderr log **verbatim** — while CodeBro's own observation line on the adjacent line correctly showed `[REDACTED]`. Verified live pre-fix (`sk-AUDITSECRETKEY123456789` present in stderr 3 times), contradicting P8 §24's "Never logged: secrets". **Fixed** (redacting tracing writer); regression test added; verified fixed live (0 occurrences, `[REDACTED]` visible).
- **F2 (HIGH) — local sandbox command policy escapes.** The sandbox treated `ls`/`head`/`tail`/`wc`/`find`/`file` as safe with **any** arguments, and `cat`'s confinement missed absolute-path operands. Verified live through the real binary: `head -c 200 /etc/passwd` (arbitrary host file read into MCP tool output), `cat /home/<user>/.codebro/credentials.json` (credential exfiltration — real key material returned), `head -c 100 …credentials.json`, `find … -fprint /tmp/out` and `git log --output=…` (writes outside the workspace), `ls /home/<user>/.ssh`, `find /home/<user> -maxdepth 1`. This contradicts the P0–P7 gate's "sandbox execution confined" claim. **Fixed** (path confinement for the inspection family + read-only git); 4 regression tests added; verified fixed live (all escapes denied; legitimate in-workspace usage preserved).

Both defects pre-date P8 (the sandbox policy is P6-era; the rmcp log line is SDK behavior that P8's observability seam made load-bearing) but P8 owns the integration contract they violate, so the audit fixed them within P8 scope.

One MEDIUM design finding was documented, not fixed, as a product decision at audit time: **per-tool `workspace_root` opens any existing host directory** — an MCP client can direct `reindex`/`apply_change` outside the server's configured root (verified live: `workspace_root: "/etc"` served `/etc`; `apply_change` edited a file in an unrelated directory). Within the local single-user trust model (client runs as the same user) this is not privilege escalation, and the multi-root registry is a unit-tested P6 design; but it is now documented prominently (AGENTS.md, P8 doc §29) with a recommended follow-up. (**Update:** this finding is now CLOSED — see `docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`.)

Everything else held under attack: workspace isolation (zero cross-workspace leakage across brief/context/recall/memory/task/learn/skill/workspace_context with secrets seeded in task title/description/memory/history/records), cross-task isolation, task lifecycle gates (completion gate survived SIGKILL), hard-kill persistence, JSON-RPC stream recovery (garbage lines, truncated JSON, pre-initialize requests, duplicate initialize), ID correlation (numeric + string ids, interleave), determinism (byte-equal briefs, repeated/reordered/跨-restart), boundedness, two-client concurrency, decision neutrality, provenance/freshness semantics, and the full real-OpenCode E2E.

**Final verdict: P8 VERIFIED WITH NON-BLOCKING DEBT** (0 unresolved CRITICAL, 0 unresolved HIGH after fixes; remaining debt documented).

## 2. Audit Scope

- `crates/mcp-server/src/integration.rs` (P8 contract + observability types)
- `crates/mcp-server/src/lib.rs` (tracing writer — P8 defect fix site)
- `crates/mcp-server/src/mcp/mod.rs` (get_info, call_tool wrapper, all 25 tool handlers, workspace registry usage, task/learn/skill/brief paths)
- `crates/mcp-server/src/indexer` paths (`crates/indexer/src/init/mod.rs` report writers)
- `crates/mcp-server/src/workspace_registry.rs` (multi-root registry)
- `crates/sandbox-runtime/src/sandbox/local.rs` (local command policy — F2 fix site)
- `crates/core/src/tools/shell.rs` (redaction authority)
- `crates/mcp-server/tests/p8_integration_e2e.rs` (P8 real-binary E2E, extended)
- Docs: P8_IMPLEMENTATION.md, AGENTS.md, CHANGELOG.md, MCP_API_V1.md, P0–P7 final gate
- Real binary `target/{debug,release}/codebro`, real OpenCode 1.18.29

## 3. Architecture Boundary

Verified by source inspection + greps + live E2E:

- No agent loop, no scheduler, no daemon, no watcher, no background threads in product code (all `tokio::spawn`/`thread::spawn` hits in mcp-server are test code or the E2E harness's stderr-drain thread).
- No model calls in the P8 layer. `consult` is an explicit, caller-initiated provider gateway (documented; degraded gracefully when no provider key is configured — observed live returning a structured 401 error, no crash).
- No execution ownership moved: the real-OpenCode E2E shows OpenCode editing `src/lib.rs` with its own write tool while CodeBro only reports staleness afterwards (git diff confirmed exactly one insertion authored by the agent; CodeBro's only writes were `reindex`-requested `.codebro/` state).
- Skill boundary unchanged: CodeBro lifecycle actions refuse/validate; no SKILL.md execution anywhere in CodeBro (P4 semantics; no P8 change).
- Task boundary: all lifecycle mutations happen only through explicit `task` tool calls; the runtime never auto-progresses (SIGKILL probe: killed server's running task still reads `running`; completion gate still refused `complete` before validation).
- **PASS** — OpenCode = reasoning + execution; CodeBro = context + durable state.

## 4. Actual Request Path

Traced through source and confirmed by protocol probes:

```
OpenCode (model decides a tool call)
 → `codebro serve` stdio process (rmcp transport)
 → JSON-RPC line framing (validated: every stdout line is protocol)
 → rmcp service (initialize-first enforced; garbage lines dropped;
   pre-init requests close cleanly with a stderr error)
 → CodeBroMcpServer::call_tool (P8 observability wrapper)
   → peer clientInfo captured (process-local)
   → tool_router dispatch (typed args: Parameters<T>, schema-validated)
   → resolve_workspace (default root | per-call canonicalized root)
   → domain runtimes (fact store / context store / memory / identity /
     impact / health / skills / tasks / learning)
   → persistence/retrieval (per-project JSON + user-level SQLite v7)
   → serialization (response_bounds 256 KiB envelope)
 → JSON-RPC response on stdout
 → P8 observation line on stderr (redacted; F1: ALL lines redacted)
 → OpenCode receives text content, reasons, acts with its own tools
```

Boundaries where trust/scope/freshness could be lost were each attacked:
workspace canonicalization (symlink collapse verified), store workspace keys (cross-workspace probes all clean), freshness (live+persisted, honest `unknown`/`stale`), redaction (write seams + read paths + now the log writer), error containment (bounded, redacted, no panics).

## 5. MCP Initialization

- initialize-first is enforced by rmcp: a `tools/list` before initialize closes the connection cleanly (structured stderr error; no stdout garbage; next server start works normally). Spec-conformant.
- Repeated initialize: second handshake answered with `codebro/1.0.0` again; tools/list still 25; server usable.
- Malformed initialize (garbage, truncated JSON): dropped or structured error; stream intact after.

## 6. Server Identity

- `initialize` returns `serverInfo: {"name": "codebro", "version": "1.0.0"}` — verified over the wire (debug + release binaries), through real OpenCode, and after duplicate initialize.
- The rmcp SDK default (`from_build_env` → `"rmcp"`) is not reachable: `get_info` constructs `Implementation::new(SERVER_NAME, CARGO_PKG_VERSION)` explicitly (mcp/mod.rs:5701). No "rmcp" identity anywhere in generated output (searched live wire traffic + source).
- **PASS**.

## 7. Stdout Protocol Purity

Highest-priority area; attacked hardest:

- Source sweep: zero `println!`/`print!` in any serve-reachable path. Indexer reports are `eprintln!` (20 sites). `doctor::report` (MCP path) doesn't print; `print_report` is CLI-only. `facts diff` prints to stdout but is CLI-only, not serve-reachable (verified: not referenced from any MCP handler).
- Live battery (real binary, per-line validation — every stdout byte between responses must be valid JSON-RPC with a `jsonrpc` field):
  - full sweep across all 25 tools including every error path (empty brief scope, unknown tool, empty tool name, missing name, invalid actions, null/wrong-typed args, negative/huge limits, 1 MB strings) — no leak;
  - garbage lines interleave, truncated JSON lines, unknown methods, valid-after-invalid, invalid-after-valid — stream intact;
  - reindex (the historical corruption trigger) under `RUST_LOG=info` — pure;
  - task/skill/learn/brief/context/recall operations — pure;
  - shutdown (kill) and restart — pure.
- `RUST_LOG=off` silences everything (config item 39).
- **PASS** — the P8 fix holds; the strict validator now also guards the rmcp-level error lines (which are JSON-RPC, not garbage).

## 8. Stderr Observability

- One observation line per tool call (`client=… tool=… duration_ms=… status=… response_bytes=…`), one at startup — verified live.
- Never arguments/task text/brief content: asserted (no `"arguments"` in stderr; hostile client names redacted).
- **F1 finding (fixed)**: rmcp's `WARN response error` line logged raw tool-error messages (echoing caller input) verbatim — adjacent to CodeBro's correctly-redacted line. Reproduced live with `sk-AUDITSECRETKEY123456789` (3 raw occurrences). Fix: `RedactingStderr` writer routes every formatted line through `redact_secrets_public`. Verified: 0 raw occurrences, `[REDACTED]` present. Regression: `p8_stderr_is_secret_redacted_even_for_transport_error_lines`.
- rmcp also logs full `peer_info` (client-declared, unbounded) at INFO — now also routed through the redacting writer; contains only client name/version; noted as INFO-level observation (no secrets possible from tool payloads — initialize carries none).
- **PASS (after fix)**.

## 9. Observability Determinism

- Wall-clock durations appear only in: observation lines (stderr telemetry, never persisted, never in responses), `reindex`/`sandbox_*` `duration_ms` fields (execution evidence — legitimate measurements of runs), and lease/freshness unix seconds (documented state semantics).
- Brief/ranker/recall paths take `now` as an input for staleness/decay comparisons only; nothing embeds wall-clock into brief output (proven by byte-equality, §37).
- Telemetry cannot affect functional behavior: the observation is composed after the response and discarded; no persistence path reads it.
- **PASS**.

## 10. Malformed Input

Barrage against every P8-relevant endpoint (via real binary): malformed JSON, truncated JSON, garbage text, missing fields, null fields, wrong types (`query: 12345`, `content: 42`), empty strings, 1 MB strings, negative/absurd limits, invalid enums/actions, invalid ids, invalid workspace roots, unexpected extra fields.

Result: no panic, no crash, no stdout corruption, no state corruption; every response is either a structured JSON-RPC error (invalid_params/internal) or a bounded success; server remains usable after each. The 1 MB string battery matched the final-gate behavior (bounded rejections).
**PASS**.

## 11. Request/Response Correlation

- Numeric ids: 3 rapid interleaved requests (tools/list, workspace_context, tools/list) → each id answered with its own correct payload (two 25-tool lists, one workspace root) — no cross-attribution.
- String ids (`"str-1"`): correctly correlated.
- Garbage interleaved with valid requests (25-line alternating battery): all 12 valid requests answered, ids correct.
- **PASS**.

## 12. Two-Client Concurrency

Re-run adversarially (two real server processes, same workspace + shared state dir):

- Concurrent reindex from both; briefs from both; task create on A visible on B's brief (`shared concurrency probe` title present in B's brief); all responses well-formed; no deadlock; no cross-attribution.
- Pinned by `p8_client_identity_observability_and_two_client_concurrency` (now 6 E2E tests green post-fix).
- **PASS**.

## 13. Workspace Identity

- Explicit `--root` > `CODEBRO_WORKSPACE_ROOT` > cwd; canonicalized (symlink probe: symlinked root resolved to the canonical real repo).
- Same-leaf-name repos in different parents stay distinct (probe: two `p8b-nm*` dirs → different canonical roots, no state mix).
- Missing/nonexistent roots error closed; regular files error closed.
- **Traversal probe**: `workspace_root: "../../../etc"` canonicalizes to `/etc` and — at audit time — the server would serve it (and `reindex` writes `.codebro/` there; `apply_change` edits files there). This was the MEDIUM finding (§51); the P8 security boundary closure (`docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`) fixed it: `/etc` now canonicalizes to a non-authorized root and the call is refused with bounded `-32602` before any state is created. Not a leak of other workspaces (the P0–P7 gate's traversal probe is about cross-workspace leakage, which stays clean — re-verified here: B sees none of A's data).
- Ambiguity never guessed: roots are explicit or canonicalized; no git walk-up (verified in workspace.rs).

## 14. Task Identity

- Opaque `task::<hex>` ids minted store-side; existence-checked; workspace-gated: cross-workspace `inspect` → `task … belongs to another workspace` (zero content leak), cross-workspace `list` → 0 rows.
- Cross-task: sibling task's brief contains none of task A's secret title/description (probe clean).
- Task from another client (two-client probe): A's task is visible to B when B shares the workspace — correct (workspace, not client, is the boundary; documented).
- Post-SIGKILL: task survives; lifecycle gates intact (`complete` before validation refused even after crash recovery; explicit `resume` required and refused while the lease is live) — the 15-minute TTL wait is documented P5 debt (§51).
- P8 did not worsen task identity. **PASS**.

## 15. OpenCode Restart

Real E2E: fresh `opencode run` sessions (each spawning a fresh MCP server process) over the same hermetic state dir:

- Completed task (`task::38fb86cf85094b22`), its checkpoint lineage, and the redacted memory entry survived into the new session; pending duplicates from stalled legs also visible (honest).
- Transient client state did not become persistent: nothing but the explicit task/memory/remember writes persisted (state dir inspection).
- **PASS**.

## 16. CodeBro Hard-Kill Recovery

SIGKILL mid-session (task running):

- Restart over same state: task list correct (`running`), brief usable, health usable; SQLite intact (WAL); no locks preventing recovery (read paths fine).
- Mutations on the killed task's lease refused until TTL (fencing-correct, documented P5 debt; not a P8 regression).
- Completion gate intact post-crash (refused `complete` before `validation_result passed`; then validate → validation_result → complete succeeded).
- **PASS** (with carried TTL debt).

## 17. JSON-RPC Stream Recovery

- Request immediately after startup (pre-initialize): clean close, structured stderr error, fresh server fully usable.
- Multiple rapid requests: answered in order.
- Request during reindex: served (reindex runs on `spawn_blocking`; the async loop stays responsive — verified by interleaving reads during a reindex leg).
- Invalid-after-valid and valid-after-invalid: stream intact (two batteries).
- **PASS**.

## 18. Tool Discovery

`tools/list` over the wire: **exactly 25 tools**, names verified against the AGENTS.md table; no internal/debug/storage tools; no `get_*` CRUD getters; contract intents all routed (pinned by probe 3). **PASS**.

## 19. Semantic MCP Contract

`integration.rs::contract::intents()` maps 14 intents onto existing tools only; regression tests assert routing and surface; no OpenCode-specific tool/storage/domain type exists (client identity is process-local observability only). The contract is usable by any MCP client (multi-agent compatibility, §40). **PASS**.

## 20. Engineering Brief Through Real MCP

Requested through the protocol and through real OpenCode:

- Sections verified: architecture/identity, freshness (live + persisted), scope, files/symbols/dependencies, impact + risk signals, tests, health, history excerpts, memory, learning, skill applicability, read-only task state, constraints, decisions, risks, unknowns.
- Every section carries category + provenance; authorities verbatim.
- **No implementation decision**: no `decision`/`recommendation` fields (asserted); the OpenCode session's own summary confirms "the brief is read-only evidence; it contains no instructions". Risk signals are evidence-based (blast radius, indicators) — descriptive, not prescriptive.
- 256 KiB envelope + per-section caps intact.
- **PASS**.

## 21. Decision Neutrality

- Structural pins: brief assembly is decision-support only (module invariants + `brief_contains_no_decision_fields`-class assertions in the E2E).
- Live OpenCode leg: agent asked "does the brief tell you to make any specific code change?" — answered "No… the brief is read-only evidence". The only actionable text was the honest suggestion to reindex when the index was empty — a freshness instruction, not an implementation choice.
- Constraints/decisions surface only from authoritative stores with provenance (P7 semantics; re-verified in §22/§23 probes).
- **PASS**.

## 22. Provenance Through MCP

- Six authorities flow verbatim: memory/learning/records surfaces carry authority fields (probe: memory entry `authority` + provenance; learning `AI_INFERRED`; context records tagged authority; accepted learning never `USER_CONFIRMED`).
- `learn confirm` refused without `user_confirmed=true` — re-verified live ("learn confirm requires user_confirmed=true…").
- Serialization does not flatten authority: rank order preserved, never upgraded for relevance (P7 invariants; brief section schema carries the fields end-to-end).
- **PASS**.

## 23. Freshness Through MCP

Full cycle through real OpenCode + probes:

- unindexed → `unknown` (+ persisted UNKNOWN) — never fabricated `fresh`;
- index → `fresh`;
- OpenCode's OWN edit (its write tool) → next brief `stale` + `STALE_INDEX` + "treat impact/tests/architecture as signals about the last indexed state" note;
- reindex → `fresh`, gamma symbol visible with correct line/module.
- Persisted READY/STALE/FAILED/UNKNOWN alongside live status. **PASS**.

## 24. Security Through Integration

Re-attack of the P7 boundary through P8, injecting secrets (`sk-…`, `ghp_…`, `password=…`) into: task title/description, memory value, remembered record, history (validation text), then hunting through brief, context, recall, engineering_memory, task inspect/list, learn, skill discover, workspace_context, stderr, and restart:

- Every persisted surface redacts at the write seam (`[REDACTED]` verified in OpenCode's own report: "the api key [REDACTED] must never leak").
- Every read surface returned redacted or nothing; brief JSON contained no raw secret (agent-confirmed + probe-confirmed).
- Same-channel caller echo (P7-documented): a brief request that itself contains the secret echoes it back to the same caller, unredacted — carried debt (§51), not a leak (it is the caller's own text on the same channel).
- **One new leak found and fixed (F1)**: the rmcp stderr line (§8). After the fix, secrets never appear raw in stdout responses, stderr logs, or persistence.
- **F2**: sandbox tool output itself was a secret-exfiltration path (§25→§27); fixed.
- **PASS after fixes**.

## 25. Cross-Workspace Security

Workspace A seeded secrets in every store surface; workspace B (shared state dir) probed all nine read surfaces: **zero leakage** (probe clean after removing the caller-echo artifact). Task refusal names no content. Cross-workspace secret never in B's brief. **PASS**.

## 26. Cross-Task Security

Task A (secret title/description); task B (sibling, same workspace) brief: no task-A secret. Same repository, same workspace, different task — clean. Task-scoped records require their task (P1 store gates, unchanged). **PASS**.

## 27. Error-Path Security

Secrets in malformed input (rejected `action`/`task_id`/`key`):

- MCP error responses: echo the caller's own text back on the same channel (documented convention) — bounded, no persistence.
- stderr: **was the F1 leak** — fixed; now `[REDACTED]`.
- History: validation "what" text redacted at store seam (P2).
- No raw secret lands in logs or stores. **PASS after fix**.

## 28. CodeBro Unavailable

- Not installed/wrong path/exits immediately: OpenCode continues with native tools (MCP client behavior; verified conceptually — OpenCode ran normally when the server was misconfigured in an early leg; CodeBro is one MCP entry in config, not a dependency of the editor).
- Unavailable during/after connection: rmcp closes cleanly; degraded-mode contract documented (native tools continue).
- No single point of failure for basic coding: CodeBro exposes no file-edit/shell/test tools that OpenCode lacks natively (the optional `apply_change` is additive).
- **PASS** (no OpenCode-side changes manufactured — P8 scope respected).

## 29. Partial Failure

- Corrupt state.db (garbage bytes): brief answered with explicit unknowns/degraded sections (bounded, no fabrication, no crash); context tool survived; task list tolerated. Matches P6/P7 failure model.
- Skills dir replaced by a file: `skill discover` + brief still answer (explicit degradation).
- Provider unavailable (`consult`): structured 401 error, no crash.
- Unknowns additive, never silent. **PASS**.

## 30. Real OpenCode E2E

Full 14-leg mission against real OpenCode 1.18.29 (hermetic XDG config, hermetic repo, hermetic state/skills; release build):

1. OpenCode starts, CodeBro MCP starts, initialize succeeds. ✅
2. Server identity `codebro` reported by the agent. ✅
3. Tools discovered (agent called codebro tools by name). ✅
4. workspace_context orients (4 symbols, 1 test, `fresh` after reindex). ✅
5. Engineering Brief obtained and parsed by the agent. ✅
6. Freshness honest: `unknown` pre-index; `fresh` post-index; `stale` + `STALE_INDEX` after the agent's own edit; `fresh` + gamma visible after reindex. ✅
7. Task state: full lifecycle create → start → checkpoint → validate → validation_result passed → complete (`completed`) through the MCP task tool. ✅
8. Restart persistence: fresh session (new server process) reads the completed task + redacted memory. ✅
9. Secrets redacted (`[REDACTED]` in the agent's own report). ✅
10. Cross-workspace access denied through real OpenCode (`belongs to another workspace`; 0 tasks visible). ✅
11. OpenCode performed the repository edit with ITS OWN write tool; git diff shows exactly the agent's insertion; CodeBro executed nothing. ✅
12. CodeBro provided context/state only throughout. ✅

(Note: the audit's first E2E model depleted its provider quota mid-run; the legs were completed with other configured free-tier models — same real OpenCode binary, same MCP server. No mocked OpenCode anywhere.)

## 31. Tool Ownership

During E2E: file edit = OpenCode's write tool (git diff authored by agent); git operations = agent's own shell; tests would run via the agent; CodeBro performed zero engineering actions — it answered orientation, brief, freshness, task state. **PASS**.

## 32. Skill Boundary

Skill lifecycle actions (discover/propose/inspect/validate/approve/…) verified structurally: gates intact (user_confirmed approval, validated status, confidence floor), no SKILL.md execution path exists in CodeBro (grep + P4 tests). Broken skills dir degrades explicitly. Cross-workspace skill visibility workspace-gated (P4). **PASS**.

## 33. Task Boundary

All task mutations via explicit calls only (lifecycle through real OpenCode above; SIGKILL probe shows no auto-progression). CodeBro persists durable state; never executes the task. **PASS**.

## 34. Passive Capture

P2 semantics: only meaningful moments on explicit tool paths capture (remember/forget, apply_change(s), sandbox_test/build, task transitions, index completion/failure). Summaries are short metadata, never transcripts. Stalled/failed agent legs in the E2E left only the explicit durable writes (task rows, memory, record) — no transcripts, no model chatter persisted (state inspected). **PASS**.

## 35. Learning Bootstrap

Retrieval→persist→retrieve cannot raise authority: accepted hypotheses persist as `AI_INFERRED` (evidence-bound, decaying); promotion requires the explicit `user_confirmed=true` speech act (refused without it — re-verified live); re-running `learn run` does not confirm anything. Confidence decays at retrieval when evidence does not refresh. No feedback loop possible without the user. **PASS**.

## 36. Memory Bootstrap

Agent-recorded memory never enters the verified fact store (structural separation, unchanged). Memory surfaced through briefs carries confidence/provenance; a memory→brief→learn cycle cannot upgrade authority (learn's own gates apply). Speculation cannot silently become fact. **PASS**.

## 37. Determinism

Identical state → identical briefs, byte-for-byte:

- repeated request: equal; reordered args: equal; across server restart: equal (release binary, probes).
- OpenCode restart: fresh session's brief consistent (agent reported matching evidence).
- No wall-clock in output (§9).
- Pinned suite-wide (P7 determinism tests + final gate). **PASS**.

## 38. Boundedness

- 1 MB strings, absurd limits, huge task text: bounded rejections/caps (re-verified live).
- Response envelope 256 KiB + per-section caps (pinned by P7 suite).
- Sandbox output capped (32 KiB/64 KiB) and redacted.
- No memory explosion observed in any probe. **PASS**.

## 39. Performance

Release binary, hermetic repo, cold process + initialize + one tool call:

- brief: ~0.06 s; reindex: ~0.04 s; facts: ~0.03 s.
- Startup→initialize < 50 ms (matches P8 report).
- No optimization performed; no new bottlenecks observed (the redacting stderr writer adds one regex pass per log line — negligible at the observed volumes; log volume is one line per tool call).

## 40. Multi-Agent Compatibility

No OpenCode-specific ids, storage, domain types, or execution assumptions (client identity is process-local observability; stores are workspace-scoped). The contract (25 tools + intents map) is client-agnostic; any MCP client connects the same way. **PASS**.

## 41. Configuration

Minimum config verified live (the E2E used exactly): one local MCP entry `codebro serve --root <repo>` + optional `CODEBRO_STATE_DIR`/`CODEBRO_SKILLS_DIR`/`RUST_LOG`. No daemon, no ports, no hidden state, no machine-specific paths, no secrets in config. `RUST_LOG=off` silences cleanly. **PASS**.

## 42. Process Lifecycle

startup → initialize → operate → shutdown(kill) → restart: clean (probes). startup → crash(SIGKILL) → restart: clean (§16). Client disconnect (stdin EOF): server exits promptly (verified in pre-init probes — no zombie, no stale lock files). **PASS**.

## 43. Cross-Process Assumptions

Single-writer assumption unchanged and still documented (per-workspace in-process mutation lock; cross-process writers rely on the documented single-writer assumption + WAL). P8's two-client probe demonstrates concurrent two-PROCESS reads + serialized writes work safely on SQLite WAL, but the registry's multi-root behavior (§13/§51) means one process can open multiple roots — the single-writer documentation stands; no distributed safety is claimed. **PASS (documented)**.

## 44. Documentation

P8_IMPLEMENTATION.md, AGENTS.md, MCP_API_V1.md, CHANGELOG.md updated by this audit to describe: the redacting stderr writer (F1), the sandbox confinement (F2), the per-call workspace_root debt, and the carried TTL debt. All previously documented behavior re-verified accurate (identity, stderr contract, tool count 25, schema v7, degraded mode, persistence). **PASS (after updates)**.

## 45. Test Quality

P8 tests re-inspected: hermetic (tempdirs, explicit state/skills dirs), deterministic (no wall-clock assertions; byte-equality), real-binary where the failure class is process-level (stdout purity, identity, contract surface, mission flow, concurrency), adversarial (secret-shaped inputs, hostile client names, malformed args). The audit added 5 regression tests in the same discipline (1 real-binary stderr redaction + 4 sandbox policy). **PASS**.

## 46. Regression Test Policy

For each defect: reproduced live first (probe output captured), wrote the failing regression (verified it targets the exact class), fixed, verified the regression passes, ran the affected suites, then the complete workspace suite (1402/1402). **Followed**.

## 47. P0–P7 Regression

Complete suite: **1402 passed / 0 failed** (1397 baseline + 4 sandbox-F2 + 1 stderr-F1). Clippy `-D warnings` clean; `cargo fmt --check` clean; `scripts/check_workspace_deps.sh` OK. No security/scope/freshness/task/skill/learning/brief regressions. **PASS**.

## 48. Environmental Safety

`~/.codebro`: 148 files md5'd before and after the entire audit (full suite + all live probes + all OpenCode E2E legs) — **byte-identical**. Real skills dirs untouched (mtime-verified). Real repositories untouched (audit used hermetic tempdirs; the only repo writes were the audit's own temp victims). `target/` rebuilt — not user state. **PASS**.

## 49. Findings

| # | Severity | Finding | Status |
|---|---|---|---|
| F1 | HIGH | rmcp transport logs raw tool-error messages to stderr; secrets in rejected tool args leaked verbatim next to correctly-redacted CodeBro lines | **FIXED** (redacting tracing writer) + regression |
| F2 | HIGH | Local sandbox policy: inspection-family commands (`head`/`cat`/`find`/`ls`/`wc`/`file`/`tail`) and read-only git accepted unconfined path args — arbitrary host file read (`/etc/passwd`, `~/.codebro/credentials.json`), out-of-sandbox writes (`find -fprint`, `git --output=`) | **FIXED** (path confinement) + 4 regressions |
| F3 | MEDIUM | Per-tool `workspace_root` opened ANY existing host directory as a workspace; `reindex` wrote `.codebro/` there and `apply_change` could edit files outside the server's configured root (verified: `/etc` served; a victim dir's file edited). Unit-tested P6 multi-root design; safe only within the local single-user trust model | **CLOSED** by the P8 security boundary closure (`docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`): operator allowlist model (server root + launch-time `--allow-root` / `CODEBRO_ALLOW_ROOTS`); per-call roots must canonicalize to exactly one authorized root or the call is refused with bounded `-32602` before any state is created. Real-binary E2E (`tests/p8_security_boundary_e2e.rs`) + registry unit battery + real OpenCode E2E re-verification. |
| F4 | LOW | rmcp logs full client-declared `peer_info` at INFO (unbounded name/version; now redacted by the F1 writer; no secret content possible) | NOTED (accepted) |
| F5 | LOW | Post-SIGKILL task mutation blocked until 15-min lease TTL; explicit `resume` then required | Carried P5 debt (documented) — fencing-correct |
| F6 | INFO | Same-channel caller echo: a brief request containing a secret echoes it back to the same caller unredacted | Carried P7-documented convention |
| F7 | INFO | The strict stdout validator lives in the test harness only | Carried P8-documented debt |

Counts: CRITICAL 0 unresolved · HIGH 2 (both fixed) · MEDIUM 1 (**F3 now CLOSED** by the P8 security boundary closure — see `docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`) · LOW 2 · INFO 2.

## 50. Fixes Applied

1. **F1 — `crates/mcp-server/src/lib.rs`**: the tracing subscriber's writer is now `RedactingStderr` — every formatted log line (CodeBro's and rmcp's) passes through `redact_secrets_public` before reaching the stderr pipe.
2. **F2 — `crates/sandbox-runtime/src/sandbox/local.rs`**: new `args_confined_for_inspection` confines all path operands and path-bearing flag values (`-fprint*`, `-fprintf`, `-fls`, `--output=`, `-C`, …) of the inspection family to the workspace root; `cat` now uses it too (absolute-path operands were previously unconfined); `git` read-only subcommands confined via `check_git_in`.

## 51. Remaining Debt

- **F3 (MEDIUM) — CLOSED**: per-call `workspace_root` used to accept any existing directory (multi-root registry). Closed by gating multi-root behind an explicit operator allowlist (the audit's recommended `--allow-roots` variant): server root always authorized; extras only via launch-time `--allow-root` flags / `CODEBRO_ALLOW_ROOTS`; per-call roots must canonicalize to exactly one authorized root or the call is refused with bounded `-32602` before any workspace state is created. Full closure report: `docs/evolution/P8_SECURITY_BOUNDARY_CLOSURE.md`.
- **F5 (LOW)**: lease-TTL-bound recovery after hard kill (P5 debt #1).
- **F4/F6/F7 (LOW/INFO)**: peer_info INFO logging (redacted, bounded content); same-channel caller echo; harness-only stdout validator.
- Carried P0–P7 gate debt (12 items) unchanged.

None of these compromise safe OpenCode integration within the documented single-user local deployment model.

## 52. Final Verdict

**P8 VERIFIED WITH NON-BLOCKING DEBT.**

- 0 unresolved CRITICAL; 0 unresolved HIGH (F1/F2 fixed with regressions inside this audit).
- Stdout protocol purity: PASS (re-attacked, holds).
- MCP protocol correctness: PASS (initialize, identity, discovery, correlation, malformed input, recovery, concurrency).
- Real-binary E2E: PASS (6 probes incl. the new stderr-redaction probe).
- Real OpenCode E2E: PASS (14 legs, release build, live models).
- Security/isolation/persistence/determinism/boundedness/concurrency: PASS (after fixes).
- Architecture boundary: PASS (no execution ownership in CodeBro).
- P0–P7 regression: PASS (1402/1402; clippy/fmt/deps clean; `~/.codebro` byte-identical).
- Remaining debt: documented, non-blocking, with follow-up recommendations.

P9: NOT STARTED.

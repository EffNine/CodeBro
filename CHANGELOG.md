# Changelog

All notable changes to CodeBro will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

---

## [Unreleased]

### Added
- **Post-P9 hardening and release closure** — no new architecture, no new MCP tool (stays 25), no schema change (stays v7). (1) Mutation authority hardened as OPTION B: `AGENTS.md` now states the ChangeEngine guarantees apply ONLY to `apply_change`/`apply_changes` edits; OpenCode-native edits (the default path) carry zero CodeBro mutation guarantees and are observed only via the `reindex`→`fresh`/`STALE_INDEX` freshness protocol (pinned by existing `freshness_becomes_stale_after_repo_change`). (2) Execution authority documented: CodeBro `sandbox_*` tools produce CodeBro-attributed verification evidence only (read-only build/test/lint, fail-closed); OpenCode owns all general execution. (3) Outcome-loop completion convention documented as workflow-level only (`complete` does not require `outcome`; compliance measurable via existing P8 per-call stderr lines; no daemon/scheduler/coercion). (4) Retention assessed: all writes per-record bounded, row counts unbounded by design, no cleanup machinery (single-user scale, explicit operator maintenance only if ever needed). (5) Lease recovery documented as fencing-correct operational procedure (wait ≤15 min TTL, then `resume`). (6) `mcp/mod.rs` 14k-line monolith deliberately left unsplit (rmcp `#[tool_router]` impl must stay one block; split would churn without benefit — recorded debt). (7) Doc drift fixed: `impact_analyze` description + served prompt + `impact/mod.rs` + `docs/design/MCP_SERVER.md` no longer claim "no risk scores" (P6 added deterministic HIGH/MEDIUM/LOW signals); regression `impact_description_mentions_risk_signals` pins it. Verified: **1455/1455 tests pass** (1454 P9 baseline + 1 hardening regression); `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean (0 warnings); `cargo fmt --check` clean; `scripts/check_workspace_deps.sh` OK; full `codebro-mcp-server` package incl. real-binary P8/P9/final-gate E2E green. Historical phase entries below (P8 1397/1397, P9 1454/1454) remain accurate records of their trees at phase end.
- **Engineering Outcome & Feedback Loop (P9)** — tenth increment of the persistent-context evolution (`docs/evolution/P9_IMPLEMENTATION.md`): CodeBro now closes the loop Task → Context → OpenCode work → Outcome → Evaluation → Learning → Future Context by receiving structured outcome evidence without becoming the agent that performed the work. New `task` action `outcome` (no new MCP tool — stays 25; no schema change — stays v7, no new tables): `classification` (success | partial | failure | rejected | superseded) + required bounded `summary` + optional bounded evidence (`reason`), command identity (`what`, never output), `exit_code`, `changed_areas`, explicit `user_confirmed` speech act (otherwise recorded as `observed` — OpenCode reports are never user-confirmed truth), and per-task-namespaced `dedup_key` idempotency (redelivery returns the original event; same key on another task is distinct). Outcomes persist as task-bound `task_outcome` history events (new additive `HistoryKind`, importance 80): any task status accepted including terminal (user confirmation routinely arrives after completion), no state transition, no lease, no task-row mutation — transition vs evidence vs confirmation vs lesson stay structurally separate. Retrieval reuses everything: resume snapshots surface outcome evidence for the current task, recall surfaces past outcomes by keyword, P3 learning consumes them as validation-group evidence (≥3 support, contradiction-aware; superseded stored polarity-neutral as `replaced` so abandonment never reads as a success pattern; rejected weighs as failure), and future briefs surface validated outcomes through history excerpts with zero brief changes. Hardening found live during P9 adversarial probing and fixed in the same increment: (1) FTS5 index validation reported lock contention as integrity-check output rows, which read as corruption and quarantined the live database under concurrent multi-process opens — contention lines now report busy (retryable, never quarantinable); (2) quarantine is now fenced by bounded retries plus a file-quiescence gate (withheld while the file churns; a vanished file means a sibling already quarantined) with pid-unique debris names; (3) outcome writes use BEGIN IMMEDIATE so concurrent cross-process writers serialize on the busy timeout (also making dedup check-then-act atomic); (4) `DbError::Corrupt` carries the offending check output for forensics. Regressions: 10 task-outcome store tests (incl. first-write-wins replay, maximal-input budgets), 3 learning tests (failure pattern formation, superseded-neutrality, contradiction voicing), 2 recall tests (keyword surfacing, cross-workspace invisibility), 6 MCP `task outcome` tests, 4 db concurrency tests (shared opens, rabid opens, genuine-corruption quarantine, contention-is-busy), 3 real-binary E2E probes (`tests/p9_outcome_e2e.rs`: full ingest→complete→restart→feedback loop with brief determinism, adversarial redaction/isolation/malformed, 4-client concurrency). 1454/1454 total; clippy `-D warnings` clean; fmt clean.

### Fixed
- **P8 security boundary closure — per-tool `workspace_root` filesystem authorization (audit F3, MEDIUM)**: the P6-era multi-root registry opened **any existing host directory** named by a per-call `workspace_root` tool argument as a full workspace (verified pre-fix: `workspace_root: "/etc"` was served; a victim directory's file was editable; this probe suite failed 8/10 against the unfixed binary). Fixed with an explicit operator authorization model: the server root is always authorized; additional roots require operator launch-time consent (repeatable `codebro serve --allow-root <path>` flags and/or the `CODEBRO_ALLOW_ROOTS` env var). Per-call `workspace_root` arguments are discovery, never authorization — they must canonicalize (symlinks, `..`, alternate representations resolved) to exactly one authorized root, else the call is refused with bounded `invalid_params` (-32602) before any workspace state is created, with zero filesystem side effects in the target and no host disclosure in the error. Authorization is exact-root (never prefix-based), frozen for the process lifetime, and re-derived from launch config on restart (never broadened by tool traffic, task ids, skills, memory, or history). No schema change (stays v7), no new tools (stays 25). Regressions: `tests/p8_security_boundary_e2e.rs` (10 real-binary probes: unauthorized-refusal sweep with zero side effects, apply_change confinement, symlink escapes, `/etc`-class host roots, `--allow-root` + env flows, nested/overlapping exact-root semantics, restart/hard-kill determinism, stdout purity + stderr redaction on refusal paths, authorized multi-root isolation), 12 `workspace_registry` unit tests (traversal, symlink, sibling-prefix, nested, frozen-set determinism, bounded host-minimal refusals, concurrency), 2 `workspace` allowlist-resolution tests. Real OpenCode E2E re-verified live: authorized flow (orient → reindex → brief) plus unauthorized-root refusal plus allowlist flow.
- **P8 post-implementation adversarial audit — stderr secret leakage through rmcp transport logs (HIGH)**: rmcp's transport layer independently logs `response error` lines carrying the raw tool-error message, and CodeBro tool errors echo caller-supplied input (e.g. a rejected `action` containing an API key). A secret supplied in a rejected tool argument reached the server's stderr log verbatim even though CodeBro's own observation seam redacted it on the adjacent line. Fixed: the tracing subscriber's writer now routes **every** formatted log line — including rmcp's — through the canonical `redact_secrets_public` authority (`crates/mcp-server/src/lib.rs` `RedactingStderr`). Regression: `p8_stderr_is_secret_redacted_even_for_transport_error_lines` (real binary, secret-shaped input hunted in captured stderr).
- **P8 post-implementation adversarial audit — local sandbox command policy escapes (HIGH)**: the `LocalCommandPolicy` treated `ls`/`head`/`tail`/`wc`/`find`/`file` as unconditionally safe "inspection" programs with **any** arguments, and `cat`'s confinement only covered relative `..` escapes — not absolute-path operands. Concrete escapes verified live through the real binary: `head -c 200 /etc/passwd` (arbitrary host file read), `cat /home/<user>/.codebro/credentials.json` (credential exfiltration through MCP tool output), `find / … -fprint /tmp/out` and `git log --output=…` (writes outside the workspace). Fixed in `crates/sandbox-runtime/src/sandbox/local.rs`: every inspection-family command's path operands and path-bearing flag values must now resolve inside the workspace root (`args_confined_for_inspection`), and `git` read-only subcommands are confined the same way (`check_git_in`). Legitimate in-workspace usage (`head src/lib.rs`, `find . -maxdepth 2 -type f`, `git status`, `ls -la`, …) is pinned to still work. Regressions: `audit_f2_inspection_commands_cannot_read_absolute_host_paths`, `audit_f2_find_cannot_write_outside_workspace`, `audit_f2_git_cannot_write_or_read_outside_workspace`, `audit_f2_legitimate_in_workspace_inspection_still_allowed`.

### Known Debt (documented, non-blocking — from the P8 audit)
- **Per-tool `workspace_root` accepts any existing host directory** (MEDIUM): the P6-era multi-workspace registry opens any existing directory as a workspace on a per-call `workspace_root` argument (unit-tested design: `explicit_root_selects_independent_state`), which means an MCP client can point `reindex`/`apply_change` at directories outside the server's configured root and CodeBro will write `.codebro/` state or apply file edits there. Within the local single-user trust model (the MCP client already runs as the same user) this is not a privilege escalation, but the behavior is not prominently documented in the P8 integration contract. Recommended follow-up: confine per-call roots to the server root's subtree, or require an explicit multi-root opt-in flag. The audit documents it; it does not fix it (changing documented multi-workspace semantics is a product decision, not an audit-scope fix).
- **Post-hard-kill task recovery waits for the 15-minute lease TTL** (carried P5 debt): after SIGKILL, an interrupted RUNNING task refuses mutations until `lease_expires_at` passes; only then does explicit `resume` take over. Fencing-correct, request-driven (no daemon), but recovery latency is TTL-bound.

### Fixed
- **P8 OpenCode integration layer — stdio MCP protocol corruption (HIGH integration defect)**: `codebro serve` emitted tracing logs to **stdout**, the JSON-RPC channel — any ERROR/INFO line corrupted the protocol framing for every MCP client (pre-existing test harnesses silently skipped non-JSON lines to work around it). Additionally every MCP `reindex` call pushed the indexer's CLI report (`relationships: N`, `codebro init complete`, …) onto stdout through 20 `println!` sites. Fixed: the tracing subscriber now writes to stderr (`lib.rs`), and all indexer pipeline reporting is `eprintln!` (CLI output remains visible in terminals; MCP stdout is pure JSON-RPC). Regression: the new P8 real-binary E2E harness treats **any non-JSON-RPC stdout line as a failure** (`tests/p8_integration_e2e.rs`), so this corruption class now fails the suite loudly instead of being silently skipped.
- **P8 — wrong MCP server identity**: `initialize` answered `serverInfo: {"name": "rmcp"}` (the rmcp SDK's `from_build_env` default), so every CodeBro instance masqueraded as the transport library to clients that display or route on server identity. Fixed: `serverInfo` now reports `codebro` + the crate version (instructions preserved). Pinned by `p8_server_info_identifies_the_product` + a real-binary probe.

### Added
- **P8 OpenCode integration layer (`docs/evolution/P8_IMPLEMENTATION.md`)** — ninth increment of the persistent-context evolution. P8 formalizes the agent-client contract over the unchanged 25-tool MCP surface (NO new tools; schema stays v7): the primary context acquisition surface is `engineering_brief`, orientation via `context`/`workspace_context`, targeted follow-up via `engineering_facts`/`impact_analyze`/`recall`/`engineering_memory`/`repository_health`, persistence via `remember`/`record_memory`/`task`/`learn`/`skill`. The contract is codified as `integration::contract::intents()` (`crates/mcp-server/src/integration.rs`) and enforced by regression tests instead of prose. New bounded per-call client observability: one stderr tracing line per tool call (`client=<name/version> tool=<tool> duration_ms=<n> status=<ok|error> response_bytes=<n>`) — client identity is captured from the initialize handshake, process-local, never persisted, redacted through the canonical `redact_secrets_public` authority; never logs arguments/task text/brief content/secrets (pinned by unit + MCP + real-binary tests). OpenCode remains the executor: verified live through real OpenCode 1.18.29 that the agent edits files with its own tools while CodeBro answers evidence, staleness, and durable state only (orient → brief → own edit → `STALE_INDEX` → reindex → `fresh` → task create/start/checkpoint → restart persistence → secret redaction `[REDACTED]` → cross-workspace task refusal). 14 new tests (6 unit + 3 MCP + 5 real-binary E2E incl. strict stdout purity and two-client concurrency); 1397/1397 total; clippy `-D warnings` clean; fmt clean; `~/.codebro` byte-identical after the full suite.
- **P0–P7 Final Architecture + Release Gate (`docs/evolution/P0_P7_FINAL_RELEASE_GATE.md`)**: whole-system audit of P0–P7 as one architecture (phase boundaries, dependency direction, single source of truth, trust/authority, scope isolation, identifiers, persistence/FTS, redaction/security, MCP surface quality, determinism, boundedness, concurrency, task/skill/learning/repository-intelligence/brief boundaries, freshness, failure model, error semantics, docs consistency, debt, complexity, end-to-end data flow, twelve realistic scenarios). Independent adversarial probe suite through the real binary (`crates/mcp-server/tests/final_gate_probe.rs`, 7 hermetic tests): secrets re-attacked through five write paths (remember/record_memory/update_identity/task/observation) and hunted through five read paths (brief/context/recall/memory/task inspect) — zero leaks; two-workspace zero-leakage with shared state dir; hard-kill restart persistence + brief byte-determinism; malformed-input rejection (traversal roots, NUL, 1 MB inputs, absurd limits/depth); trust boundaries (evidence-cited rejections, no self-confirmation); conflicting-constraint decision-neutrality; concurrent two-client brief+mutation consistency. Verdict: RELEASE READY WITH NON-BLOCKING DEBT (0 CRITICAL/HIGH/MEDIUM; no fixes required). 1383/1383 tests pass as non-root (credentials chmod artifact remains root-only, pre-existing); clippy `-D warnings` clean; `cargo fmt --check` clean; schema stays v7; 25 tools confirmed live; `~/.codebro` and real skills byte-identical after the full suite.

### Fixed
- **P7 post-implementation audit findings (`docs/evolution/P7_POST_IMPLEMENTATION_AUDIT.md`)**:
  - Secret in skill descriptions reached the Engineering Brief (HIGH): `skill propose` persisted `description`/`purpose` verbatim — the one unredacted free-text write seam — and the P7 brief surfaced approved skills' descriptions verbatim in `skills[].description`. Reproduced through the real binary via the learning-backed publish path. Fixed at the write seam (propose now redacts both branches, matching remember/task/record_memory policy) plus defense-in-depth redaction in the brief's skills projection. Regression: `brief_secret_in_skill_description_never_reaches_brief`.
  - Secret in identity free-text reached the brief (MEDIUM): `update_identity` persisted decision titles/descriptions, constraints, architecture summaries, roadmap items, and every other free-text identity field verbatim, and the P7 brief surfaced identity constraints and decision titles. Reproduced through the real binary (`add_constraints` carrying a token → verbatim hard constraint in the brief). Fixed at the write seam (all `update_identity` free-text fields now flow through `redact_secrets_public`) plus defense-in-depth re-redaction in the brief's constraint/decision/architecture/memory/history/learning projections (covers legacy rows predating the write-seam fix). Regression: `brief_secret_in_identity_constraint_not_surfaced`.
  - Task `skill_refs` were stored unredacted (MEDIUM): `create_task`/`set_task_skill_refs` validated shape only, and the brief echoed unresolved refs verbatim in `skills[].name`. Fixed at both write seams plus brief projection defense (resolved-name fallback and unresolved echo). Regression: `brief_secret_in_task_skill_ref_not_echoed` (asserts the ref still surfaces — redacted, not dropped).
  - Documentation drift: `BriefRequest::depth` claimed a `0..=2` clamp while the impact call floors 0 → 1 (doc now states the floor); the P7 doc claimed the handler's record-keyword enrichment and the assembler's scope-keyword enrichment were identical (they are sibling sets — handler adds task-description tokens, assembler adds checkpoint-summary tokens; docs and comments corrected).
  - New adversarial regression suite: `crates/mcp-server/tests/p7_brief_adversarial.rs` — 15 real-binary tests covering secret redaction at every new brief read seam, authority-conflict preservation, task-scoped record isolation, ambiguous-target refusal, depth bounds, failed-reindex/FAILED_INDEX honesty, taskless no-invention, cross-domain no-collapse, repository-identity stability, corrupt-state.db degradation, massive-store bounding, and output hygiene (no row ids/payloads). Hermetic (`CODEBRO_STATE_DIR` + `CODEBRO_SKILLS_DIR` isolation).

### Added
- **Engineering Decision Support (P7)** — eighth increment of the persistent-context evolution (`docs/evolution/P7_IMPLEMENTATION.md`): CodeBro now prepares bounded, deterministic engineering briefs so OpenCode has task-relevant evidence before deciding. New `engineering_brief` MCP tool (tool 25, the only new tool: no CRUD getters): task/task_id/target/keywords/depth inputs (≥1 scoping signal required; ad-hoc task text never persisted) → one read-only assembly over existing stores (facts search, memory resolution, fingerprint excerpts, recall, accepted/rejected learning, skill registry + task skill refs, task resume snapshot, impact traversal, health analysis, identity decisions/constraints, live + persisted freshness) → bounded `EngineeringBrief` (explicit per-section caps, 256 KiB envelope, deterministic ordering, explicit `UNKNOWN` entries for every gap: stale/failed/unknown index, missing/ambiguous targets, unlinked tests, absent history/memory/learning/skills, cross-workspace tasks). Authority preserved end to end (accepted learning surfaces as `ai_inferred`, rejected only as negative knowledge, superseded decisions never current, preferences never upgraded to constraints, conflicts surfaced not resolved). Read-only: no mutation lock, no history writes, no lifecycle transitions, no skill execution, no model calls, no scheduler. Schema stays v7, no new storage. 1360 tests passing (38 new: 21 unit + 14 MCP + 3 real-binary E2E), clippy/fmt clean.
- **Engineering Intelligence Layer (P6)** — seventh increment of the persistent-context evolution (`docs/evolution/P6_IMPLEMENTATION.md`): repository identity (canonical root + git remote/HEAD + stable id), file intelligence (stable ids, hashes, classes; parsed Rust/Python/JS/TS/Go plus file-level-only C/C++/shell/config with explicit limitations, never invented symbols), incremental diff (added/deleted/modified/unchanged, sorted; parse-cache reparses only changed files; deterministic IDs; no orphaned edges), index lifecycle + freshness (UNKNOWN/DISCOVERING/INDEXING/READY/STALE/FAILED; indexed_at/revision/counts; never READY when stale), risk signals on impact_analyze (HIGH/MEDIUM/LOW + indicators + blast radius), health findings (CYCLE/HIGH_FANOUT/HIGH_FANIN/ORPHAN/UNRESOLVED_REFERENCE/STALE_INDEX/MISSING_TEST_ASSOCIATION/LARGE_MODULE; severity-gated doctor checks), SQLite v6->v7 (repo_indexes metadata only; crash-safe, tested v1->v7), MCP still 24 tools (workspace_context/impact/reindex/health strengthened; no CRUD), context unchanged (bounded packet), P6 history kinds learning-ignored, task<->skill debt resolved via read-time resolve_task_skill_refs.
- **Durable Engineering Task Runtime (P5)** — sixth increment of the persistent-context evolution (`docs/evolution/P5_IMPLEMENTATION.md`): CodeBro now represents engineering work as durable state that survives restarts, session boundaries, pauses, validation, and interruptions. New `task` MCP tool (tool 24: list, stale, create, inspect, start, pause, resume, checkpoint, validate, validation_result, complete, fail, cancel, skill_refs). Strict lifecycle (`pending` → `running` → `paused`/`validating` → `completed`/`failed`/`cancelled`, `resumed` normalizes to `running`) with a store-enforced transition matrix — callers never set status directly, and completion is gated on a recorded passed validation (`validation_result failed` returns the task to `running`; `complete` from any other state is refused at the store layer). Immutable versioned checkpoints (bounded resume state: summary/progress/next-action/metadata, never transcripts; new progress = new version row; task pointer, row, and history event commit in one transaction — never a dangling checkpoint reference). Worker leases with fencing (per-server `wkr::<hex>` ids; lease takeover after TTL expiry fences `lease_version` forward; a stale worker can never overwrite a newer owner's state). `based_on_version` optimistic concurrency (stale writers refused, never silently merged). Explicit `idempotency_key` dedup (titles are never deduplicated). Interrupted tasks are stale-and-recoverable only via explicit `resume` (never auto-completed, never deleted); `inspect` composes the bounded resume snapshot (task identity, latest checkpoint, ≤10 recent events, skills, intent note) and surfaces terminal-intent mismatches without rewriting intent history. Skill association is reference-only (task outcomes never mutate skill health). Task lifecycle events are ordinary P2 history (new additive `HistoryKind` variants); P3 learning consumes task completions/validations/failures as weighted evidence with no trust bypass. Workspace isolation enforced at MCP, storage, and retrieval on every seam. Storage: stepwise v5→v6 migration (new `tasks` + `task_checkpoints` tables, `IF NOT EXISTS` resume-safe, P0–P4 data preserved). Request-driven: no scheduler, no daemon, no P6 functionality. Existing 23 tools, JSON stores, and all P0–P4 behaviour unchanged.
- **Learning + Inference (P3)** — fourth increment of the persistent-context evolution (`docs/evolution/P3_IMPLEMENTATION.md`): CodeBro now forms cautious hypotheses from accumulated history without ever presenting inference as user-confirmed truth. Deterministic token-pair clustering over in-scope history (≥3 supporting events, conversational chatter and sensitive topics never mined) → outcome-aware evaluation (supporting vs contradicting evidence, bounded `[0.05, 0.95]` confidence, contested ⇒ deferred, majority-against ⇒ rejected, global hypotheses need broader evidence) → accepted hypotheses persist as `AI_INFERRED` context records (evidence-bound, decaying, 180-day TTL; existing table, no second store). `learn` MCP tool (tool 22, capability-oriented: run/propose/list/get/evaluate/confirm/reject): explicit user confirmation promotes via supersede (the model can never self-confirm — forged confirmation refused at store and MCP layers), user rejection preserves negative knowledge and survives re-runs, list/get inspect pending hypotheses with what/why/confidence/authority explanations. Same-evidence reprocessing updates one deterministic candidate row (8-way concurrent passes converge); evaluation drops fake or foreign-workspace evidence; learning failures never touch canonical history; no learn action writes history (no recursion). Storage: stepwise v3→v4 migration (new `learning_candidates` table, empty backfill, v1→v4 and partial-resume tested). Context integration needs no composer change: inferences resolve through existing authority-ranked retrieval (confirmed beats inferred, 8-record cap, expired excluded). Existing 21 tools, JSON stores, and all P0/P1/P2 behaviour unchanged.
- **Skills Lifecycle (P4, audited)** — fifth increment of the persistent-context evolution (`docs/evolution/P4_IMPLEMENTATION.md`, post-implementation audit in `docs/evolution/P4_POST_IMPLEMENTATION_AUDIT.md`): CodeBro manages the full skill lifecycle from evidence-backed proposal to vetted, versioned publication; OpenCode executes published skills natively. New `skill` MCP tool (tool 23: discover, propose, inspect, validate, approve, reject, deprecate, rollback, health). Layered trust gates: only accepted P3 learning can seed candidates (weak/rejected learning refused at the store), standalone proposals carry no evidence and their caller-declared confidence is capped below the 0.60 approval floor, `approve` requires an explicit `user_confirmed=true` speech act AND store gates (validated status, confidence floor, workspace match, name-conflict refusal, stale-version anchor) — the model can never self-approve. `validate` runs the automated evaluation pass (candidate → evaluating → draft → validated). Mutation safety: publication is atomic (temp file + fsync + rename), symlink-safe (directories and SKILL.md targets are checked; traversal-shaped names are refused before any path is constructed), read-before-write with content-hash conflict detection (external edits, missing files, and unowned files surface as errors, never overwrites), optimistic concurrency via `based_on_version` (stale writers refused with a clear error), and immutable versions (plain INSERT + unique `(skill_id, version_number)` index; tampering is a hard DB error). Rollback publishes prior content as a new version (history preserved, no-op rollbacks refused); deprecation removes the published artifact so OpenCode stops discovering it (version rows stay for audit). Every action enforces workspace/scope visibility (project rows confined, task rows need `task_id`, discover lists only active skills). OpenCode-compat validation: frontmatter `name` must match the skill directory and `description` must be 1–1024 chars (skills OpenCode would silently ignore are refused). v5 schema migration adds `skill_candidates` (with `based_on_version`), `skills`, `skill_versions`; 51 context-runtime adversarial tests + 10 MCP integration tests; full binary E2E verified (propose → validate → approve → v2 → stale-writer refusal → rollback → restart → deprecate → cross-workspace isolation).

### Added
- **Sessions + History + Recall (P2)** — third increment of the persistent-context evolution (`docs/evolution/P2_IMPLEMENTATION.md`): CodeBro now answers "what happened during previous work" with no inference and no learning (P3's job). Sessions: opaque `ses::<hex>` ids, minimal `active/completed/abandoned` lifecycle, `ensure_active_session` resume (restart-safe, never duplicates), staleness derived at read time (interrupted sessions reported, never auto-completed), optional task binding + parent links. History: one event abstraction (`EventRecord` + additive `task_id`/`summary`/`dedup_key`/`source`), closed 11-kind structural taxonomy, append-only `record_history` (redact-before-store, truncate-with-marker, dedup-key idempotency, session link + heartbeat, FTS sync — one transaction), explicit-timestamp ordering. FTS5 `events_fts` derived index (OR candidacy for natural questions, BM25 + deterministic priors: task match → kind importance → recency → id). `recall` MCP tool (tool 21, read-only, no recursion): session-grouped (≤3/session) bounded excerpts (240 chars) with session/timestamp/scope/task/source/stale provenance; project default, task exact, global explicit opt-in; filtering precedes ranking (FTS-bypass-proof). Passive capture (best-effort, summaries only): remember→decision, forget→observation, apply_change(s)→change_applied, sandbox_test/build→validation; `context` shape unchanged (history never dumped). Storage: stepwise v2→v3 migration (probe-first resume; sessions/events columns + history index + backfill; v1→v3 and partial-resume tested). Existing 20 tools, JSON stores, and all P1 behaviour unchanged.

### Added
- **User Fingerprint + Intent (P1)** — second increment of the persistent-context evolution (`docs/evolution/P1_IMPLEMENTATION.md`, plan state in `docs/evolution/PHASE4_PLAN.md`). Two new MCP tools (20 total): `remember` (semantic persist of a confirmed preference or intent — caller states what the user confirmed, CodeBro assigns authority/provenance/scope/lifecycle; `USER_CONFIRMED` requires the explicit `user_confirmed` flag, observed/inferred writes require evidence with `observation` auto-minting a cited event) and `forget` (reversible reject by id/namespace, `permanent=true` removes; workspace-confined; both serialize on the mutation lock). Deterministic per-namespace resolution (authority rank, then task > project > global, decayed confidence, recency, id) drives the `context` packet (new additive `task_id` arg; excerpts gain `kind`/`scope`/`task_id`/`intent` with decoded status/priority/rationale). Intent statuses reuse the storage lifecycle (`completed`→expired, `cancelled`→rejected via new `retire_record`; terminal intents refuse further supersession). Trust gates: evidence ids resolved against the events table (same-workspace for scoped records), task-scope requires `task_id`, workspace roots canonicalized on every write/query path, reference-only `fact`/`decision`/`skill` kinds enforced, fresh-namespace clashes refused naming the incumbent (replace via `supersedes`). Storage: stepwise v1→v2 migration adding nullable `task_id`/`extra_json` (+ task index) with P0-row-preserving upgrade and partial-step resume tests. Existing 18 tools, JSON stores, and all v1.2 behaviour unchanged.

### Added
- **Persistent Context Foundation (P0)** — new `context-runtime` crate and the 18th MCP tool `context`, first increment of the persistent-context evolution (`docs/evolution/`). Adds `~/.codebro/state.db` (SQLite + FTS5, WAL, `PRAGMA user_version` migrations, quarantine-on-corruption with WAL-sidecar hygiene): durable context records (kind/authority/scope/status/lifecycle, evidence ids, supersede chains, expiry, confidence decay at retrieval), an append-only bounded event log with payload digests, and the session table (schema reserved for the session-history phase). Provenance is enforced structurally: `ai_inferred`/`observed` records must cite evidence. The `context` tool composes the always-available packet — repository orientation + fact counts, task-relevant facts/decisions/memory/evidence, and workspace-scoped context records tagged by authority — degrading to a labelled structural digest without a task and to an empty records section when the user store is unusable. `CODEBRO_STATE_DIR` overrides the state dir. Existing 17 tools, JSON stores, and all v1.2 behaviour unchanged. Full design: `docs/evolution/PHASE1_AUDIT.md`, `PHASE2_GAP_ANALYSIS.md`, `PHASE3_ARCHITECTURE.md`, `PHASE4_PLAN.md`.

### Fixed
- **P6 post-implementation audit findings (`docs/evolution/P6_POST_IMPLEMENTATION_AUDIT.md`)**:
  - Identity-JSON redaction leaked secret values: the hand-rolled loop renamed `"token"`-style keys while leaving values (and URL-embedded git-remote credentials) in stored plaintext. `repo_indexes` identity JSON now redacts through the central `redact_secrets_public` authority before the 4 KiB bound.
  - False-Fresh for untracked edits: the working-tree hash covered untracked file *names* only, so editing a never-added source file still reported `fresh`. The hash now covers bounded untracked contents (512 KiB gate, path+size above it; deterministic skip).
  - Self-inflicted Stale: un-ignored `.codebro/` output (facts, caches) entered the revision signal, so freshness read `stale` immediately after every `reindex`. All revision signals now exclude `.codebro/` via the `':!.codebro'` pathspec. Git workspaces indexed before this change report `stale` once; the next `reindex` heals it.
  - Silent diff truncation: MCP `incremental` lists capped at 100 entries with no signal. The MCP diff now delegates to the single `diff_digests` kernel and reports exact totals plus a `truncated` flag (deterministic head-truncation).
  - Failed `reindex` zeroed last-good metadata (symbol/edge counts, revision). Failures now preserve the previous row and only move status to FAILED.
  - Lexical canonical fallback could return a relative path (`"c"`) for missing absolute inputs with `..` above root; it now mirrors storage-key normalisation. `git remote -v` parsing no longer aborts on a blank/malformed line.
  - Removed the dead `RiskInput::is_generated` field (`assess_risk` never read it; generation state was set but ignored).
  - Non-git freshness documented honestly: live freshness reports `unknown` outside git (never fabricated); the digest-based helper remains library API.
  - Regression tests: redaction values + URL credentials, untracked-content invalidation, `.codebro/` exclusion, canonical fallback, remote-parse robustness, diff kernel delegation + truncation totals, failure preservation.
- **P5 post-implementation audit findings (`docs/evolution/P5_POST_IMPLEMENTATION_AUDIT.md`, verdict: PASS WITH CHANGES)**:
  - Takeover bypass: while a task named an owner, a non-owner could mutate live state directly (checkpoint/pause/validate) after the lease expired, without an explicit `resume` and without fencing the version forward. `enforce_lease` now refuses any non-owner live-state mutation while an owner is named — takeover happens only through `resume`. Forged lease versions never grant access.
  - Checkpoint lease invention: `create_task_checkpoint` unconditionally wrote `lease_expires_at`/`lease_heartbeat_at`, so checkpoints on holderless (pending/paused) tasks invented lease state with no owner, and a non-owner's checkpoint resurrected the previous owner's lease. The lease is now heartbeated only when the caller is the holder.
  - Paused tasks were derived as stale (`is_stale` covered every non-terminal, non-pending state, and a released lease reads as expired), so intentional stops appeared in the recoverable-work listing. Staleness now covers only `running`/`validating`.
  - Expired-holder heartbeat: the holder could silently renew an already-expired lease without fencing. Heartbeat now refuses expired leases — recovery goes through explicit `resume`.
  - MCP mutation lock: the `task` handler's guard was scoped to a match arm and dropped immediately, serializing nothing. Mutating actions now hold the workspace lock for the whole handler.
  - Checkpoint metadata round-trip: the API returned `metadata_json: None` while the row persisted `""` (NOT NULL column), so returned and re-read checkpoints differed. Reads now normalize `""` back to `None`.
  - Regression tests: 17 store-level adversarial tests (takeover bypass, forged versions, expired heartbeat, holderless checkpoints, paused staleness, terminal refusal, true-thread checkpoint/resume races, stale anchors on every versioned mutation, full-surface redaction, validating-state recovery across reopen, cross-task isolation, return-vs-persisted parity, outcome integrity, idempotency semantics, supervisory cancel, oversized refusal), 2 MCP tests (paused-not-stale, validation/outcome redaction), 1 real-binary E2E (pause → restart → resume → complete → restart).
- **P0 post-implementation review findings (`docs/evolution/P0_POST_IMPLEMENTATION_REVIEW.md`, verdict: PASS WITH CHANGES)**:
  - `supersede_record` never wrote the replacement's FTS5 row, so every confirmed record became invisible to keyword search. The replacement is now indexed in the same transaction (the superseded row's FTS entry stays for status-filtered audit queries).
  - `open_with_recovery` quarantined (renamed aside) `state.db` on *any* open error, including lock contention and IO/permission failures. Recovery now fires only on corruption evidence (`quick_check` failure, `DatabaseCorrupt` / `NotADatabase`); other failures propagate untouched and the `context` tool degrades to an empty records section.
  - Quarantine moved `state.db` + `-wal` but left `-shm` behind. All three sidecars now move together.
  - Regression tests: superseded-replacement keyword search, FTS happy-path baseline, quarantine classifier, non-corrupt failure leaves files untouched, `-shm` hygiene.
- **Phase-7 stabilization findings (all verified through the live MCP stdio runtime)**:
  - `recommended_tests` over-matched: sibling tests in the edited file were recommended even when they never touch the edited code. Recommendations now use the edited line range (from the prepared change) against symbol spans — a test qualifies only if it exercises an *edited* symbol or its own body was edited. Unnamed (`unknown`) tests are never recommended.
  - libtest panic lines with thread ids (`thread 't' (12345) panicked at f.rs:1:2:`) now parse; cargo runner summaries ("error: test failed, to rerun pass…") no longer leak in as compiler diagnostics.
  - `classify_failure` precedence: synthetic `panic` codes and column-less runtime assertion frames (pytest long tracebacks) no longer count as compile evidence; genuine compiler shapes (E-codes, columns, "could not compile") still win.
  - Go single-colon log lines (`file.go:14: msg`) parse as file+line evidence.
  - Local sandbox policy: `go test -run <Name>` accepted (single name; regex alternation needs `|`, a blocked shell metachar — multi-filter falls back to the full suite); `python`/`python3 -m pytest [-q|-x|-k|--tb=long …]` accepted in Python workspaces only (closes the gap where auto-detected pytest runs were always denied).
  - pytest runs synthesize `--tb=long` so failures attribute to the mutated file; generic source-frame lines (`helpers.py:3: AssertionError`) parse for known source extensions.
  - `related_recent_changes`: deduplicated paths, dual correlation channels with explicit `via` provenance (`diagnostic_file`, `recommended_tests`), so value-mismatch failures whose tracebacks stay inside the test file still correlate to the edit that caused them.

### Added
- **Execution Evidence Journal** — durable, machine-generated validation history: `sandbox_test` / `sandbox_build` record each run (command, outcome classification, failing tests, diagnostic digests, affected modules) bound to the working-tree hash in `.codebro/execution_evidence.json`, and surface bounded advisory `prior_evidence` (same-tree prior run, repeated failures across trees, historical variance — never "flaky"). Bounded at 200 records / 30-day TTL / 256 KB with deterministic oldest-first pruning; corrupt files are quarantined, never fatal; facts and engineering memory are never touched. `repository_health` gains a non-mutating `execution_evidence` check (absent is normal, corruption warns without failing).
- **Let-binding type inference** — same-body bindings now resolve variable receivers without inventing types: Rust `let p = User::new(); p.save()`, Python `p = User(); p.save()` (constructor naming convention), JS/TS `const w = new Widget()` and TS annotations (`let s: Store`). Bindings reset at callable boundaries; anything not constructor-shaped stays untyped. Roadmap item `let-binding-type-inference` completed.
- **Failure↔change correlation** — the server keeps an in-memory ring of recently applied change paths; when a verification fails with parsed diagnostics, responses include `related_recent_changes` (edited files ∩ failing files, with age). Circumstantial by design, never persisted.
- **Python/JS/TS call-graph parity** — the parser platform now extracts calls and structured imports for all six supported languages:
  - Python: `call` expressions (bare, `self.`/`cls.` receivers) and `import` / `from … import` statements (module-level resolution; relative imports keep their dots; aliases captured).
  - JavaScript/TypeScript: `call_expression` (bare, `this.` receiver), `new` constructors, and `import`/`export … from` source specifiers (relative/path-scoped only; bare external packages excluded).
  - These feed the existing verified-edge pipeline: cross-module call edges, `TestFact.tested` linkage, and impact analysis now work for Python and JS/TS projects.
- **Targeted test selection** — `apply_change` responses now carry `recommended_tests`: tests located in the changed file plus tests exercising any affected symbol via `TestFact.tested`, deterministically sorted and capped at 32. `sandbox_test` accepts an additive optional `test_filter` argument that narrows the auto-detected run to exactly those tests (`cargo test n1 n2`, `go test -run '^(n1|n2)$' ./...`, `pytest n1 n2`); runners without a standard selection mechanism ignore it, and an explicit `command` always wins.
- **Validation intelligence** — `sandbox_test` / `sandbox_build` / `sandbox_exec` verification results now include structured `diagnostics` parsed from build/test output (rustc errors/warnings with source spans, cargo test failures with panic locations, Go compile errors and `--- FAIL` markers, pytest summary lines) plus a coarse `classification` (`success` | `compile_error` | `test_failure` | `timeout` | `denied` | `unknown_failure`) with deterministic precedence. New module: `crates/sandbox-runtime/src/sandbox/diagnostics.rs`.
- **Module attribution for failures** — tool responses add `affected_modules`, mapping diagnostic file paths to owning modules via the cached fact store.
- Parser is bounded (50k lines) and evidence-conservative: it never invents facts; bare `error:` lines without code/span/compile-summary evidence classify as `unknown_failure`, not `compile_error`.

### Fixed
- **JS/TS method names were "unknown"** — `method_definition` names live in `property_identifier`, not `identifier`; method symbols (and call-caller attribution inside methods) now carry real names.
- **Pipelined mutation race** — mutating MCP tools (`apply_change`, `apply_changes`, `record_memory`, `delete_memory`, `update_identity`, `reindex`) now serialize on a per-workspace lock, so a client that pipelines calls without awaiting can no longer lose writes to last-writer-wins persistence races. Scope is one server process; cross-process writers still rely on the documented single-writer assumption.
- **Prepare→apply TOCTOU windows in ChangeEngine** — apply-time re-validation of canonical path containment: a target file (or creation parent directory) swapped to a symlink after preparation is now refused instead of writing through the link outside the workspace root.
- **`delete_memory` fail-closed load** — deletion over an unloadable store now refuses instead of proceeding from an empty view; the confirmation gate additionally lives in the memory runtime itself (`EngineeringMemoryError::ConfirmationRequired`), so every caller is protected rather than only the MCP handler.
- **Non-atomic project-identity writes** — all eight identity files are persisted via atomic write-rename (`codebro_core::persistence::write_atomic`), matching the durability mandate in core.

### Removed
- Dead `ChangePlan::apply_and_verify` post-apply verification seam (zero callers; verification evidence lives in `sandbox_test`/`sandbox_build`).
- **Dead parser sub-platforms** — `intelligence/{index,graph,search,context,reasoning,lsp}` (zero external consumers since the legacy retirement) deleted, dropping the unused `rusqlite`, `uuid`, and `walkdir` dependencies from `codebro-parsers`.
- Dead `ChangeReview` / `PatchSet` primitives, `FALLBACK_MODEL` CLI constant, and an unreferenced serde default helper.

### Fixed

### Added
- Regression coverage: lock serialization pinned directly (blocked-while-held), 16-way concurrent `record_memory` storm preserves every entry, and both symlink-swap-after-prepare escapes are denied at apply time.
- **Symbol visibility end-to-end** — the Rust parser emitted `"public"`/`"crate"` while the indexer expected `"pub"`/`"pub(crate)"` (and `pub(crate)` was shadowed by the `contains("pub")` branch). Visibility now derives from the precise `visibility_modifier` AST node and flows through as `Public`/`Internal`; private items remain `Unknown` rather than misclassified.
- **Heuristic references gating** — `build_module_relationship_map` probed module-id pairs against a set containing raw symbol-id pairs from verified calls, so references almost never fired. Verified calls are now projected into module space via each symbol's owning module. References flow again (839 on this repo, previously 0).
- **Imports edge direction convention** — verified and heuristic `Imports` relationships previously stored source=imported/target=importer, contradicting their id strings, `Calls`, dependency ids, and what `gather_modules` assumes. Edges now consistently store source=importer → target=imported; heuristic pairs are oriented lexicographically for determinism.
- **Sandbox fixture integration tests silently skipping** — tests resolved fixtures at `crates/mcp-server/tests/fixtures/` while they live at the repo root `tests/fixtures/`. Paths corrected and skip guards removed; both fixture manifests gained an empty `[workspace]` table so they build standalone.
- **Stale tool-enumeration regression lists** — description/router tests covered 15 of 17 tools; `apply_changes` and `update_identity` added (surfacing an over-long `update_identity` description, now shortened).

### Added
- **Marker-based test discovery** — Rust functions with `#[test]`/`#[tokio::test]` attributes and Go `Test*` functions are registered as test facts without relying on path/name heuristics (`ParsedSymbol.is_test`).
- **Test↔symbol linkage** — `build_relationships` returns verified call edges and the init pipeline populates `TestFact.tested` from calls made by a test's own function symbol, giving `impact_analyze` a real change→test mapping (666/826 tests linked on this repo).
- **MCP-level `apply_changes` coverage** — transaction dispatch in the test helper plus an applies-or-aborts-with-zero-mutations integration test.

### Changed
- Parse-cache schema bumped 1→2; existing caches are invalidated once and files re-parsed (required by the visibility vocabulary change).

---

## [0.7.0-mcp-rc2] - 2026-08-17

> **Status:** Release candidate for real-world agent dogfooding and stabilization.

### Added
- **M1: Engineering Memory Trust** — Trust scores on memory entries computed from confidence, importance, and freshness. `engineering_memory` responses include per-entry `trust` metadata. `memory_stats` reports average trust. Backward-compatible: existing entries without freshness data use `unknown` freshness status.
- **M2: Change Invalidation Advisory** — `apply_change` now returns `needs_reindex` advisory when source files in the fact store are affected. Impacted fact IDs are correlation metadata (not independently verified causal evidence). `impact_analyze` provides structural relationship edges (callers, importers, references) for context.
- **M3: Lightweight Evidence Chaining** — `impact_analyze` returns directed relationship edges with provenance metadata. Evidence chain: `engineering_facts` → `impact_analyze` → `apply_change` → invalidation advisory → `reindex` → fresh FactStore → `sandbox_test`/`sandbox_build` → `VerificationResult`.
- **M4: MCP Reindex Trigger** — `reindex` tool performs full fact-store regeneration via the existing `codebro init` pipeline. Returns `status`, `fact_counts`, `generation_repo_state`, `validation`, and `duration_ms`. This is a full rebuild, not incremental indexing.
- **M5: Repository Health MCP Tool** — `repository_health` exposes the existing `codebro doctor` capability as MCP tool #14. Read-only; returns structured JSON with `exit_code`, `status`, `checks`, and `summary`. Delegates directly to the existing doctor runtime without adding new checks, auto-repair, or orchestration. All 6 existing doctor checks (workspace_root, .codebro, project_identity, facts, engineering_memory, git) preserved with exact semantics and exit-code values (0=healthy, 1=warn, 2=error).

### Summary
- **15 MCP tools** (exact count: RC2 had 14 + M5 addition of `repository_health` + this release's `consult`)
- **3186 tests passing**, 0 failed, 11 ignored
- **MCP-first architecture** — host agent owns planning/orchestration; CodeBro owns engineering truth/infrastructure
- **Stabilization / dogfooding phase** — M6 has NOT started

### Security
- **Sprint 28 hardening** — credential lifecycle and execution integrity:
  - `//apikey` no longer accepts an inline key. Run `//apikey [provider]` and
    enter the secret in a masked prompt (`Enter` to store, `Esc` to cancel);
    inline keys are rejected and never enter input history or context.
  - `CredentialStore` (`~/.codebro/credentials.json`) persists atomically with
    mode `0600`, refuses symlinked paths, fsyncs before rename, and surfaces
    security-critical failures instead of ignoring them. `Debug` output
    exposes provider presence only, never values.
  - Secrets are redacted before reaching shell history, session files,
    conversation/context, input history, exports, clipboard text, and
    activity logs via the single tool redaction authority
    (`redact_secrets_public`), extended with password/secret/token, GitHub/
    GitLab/Slack token, and URL-credential patterns.
  - `read_file` tool output is redacted so a workspace credential file cannot
    leak into model context.

### Fixed
- **Sprint 28 hardening** — blocking shell execution (`execute_child`)
  drained stdout/stderr while the child runs, eliminating pipe-buffer
  deadlocks on large output; output stays bounded; timeouts terminate the
  whole process group; PTY/stream thread-creation failures are surfaced as
  errors instead of silently dropped.
- **`codebro init` memory scaling** — eliminated O(n²) combinatorial explosion
  in heuristic reference/relationship generation (`src/impact/relationships.rs`).
  Heuristic edges are now bounded to symbol pairs whose modules already share
  a verified AST-derived relationship (call or import), preventing common-name
  collision blowups (e.g. `new`, `tests`, `fmt`). Added early drop of
  intermediate collected vectors (`collected_modules`, `collected_symbols`,
  `all_calls`, `all_imports`, `files`) before serialization. Replaced
  `serde_json::to_string_pretty()` + `write()` with streaming
  `serde_json::to_writer_pretty()` to avoid a full second copy of the model
  in RAM during JSON generation. Before/after on the CodeBro repo (348 source
  files): peak RSS 1,294 MB → 753 MB (−42%); facts.json 216 MB → 13.6 MB
  (−94%); references 292,474 → 0; relationships 88,593 → 6,014. Atlas
  (36 Rust files): peak RSS 29 MB → 22 MB; facts.json 216 KB → 909 KB.
  All 3140 tests passing; determinism verified.

## [Unreleased]

### Changed
- **Runtime consolidation — legacy isolation** — the retired TUI agent stack and Adaptive Platform subsystems (33 top-level modules: agent loop, planner, subagents, canonical runtime, tool platform, intent/preference/recommendation engines, plugin SDK, service registry, …) moved under `src/legacy/` with an explicit boundary doc. Legacy compiles only under `#[cfg(test)]` as a regression suite; it has no entry point from `main` and no live module imports it. The live runtime is now ~47k LOC across 19 module roots (down from ~157k across 52).
- **Dependency direction enforced** — `coding/` reduced to the ChangeEngine (`change_engine.rs`); the Sprint 30F subagent surfaces moved to `legacy::{coding_tooling, coding_contract, coding_runtime, coding_limits, coding_tests}`. `engineering_memory` absorbed the `engineering_context::memory` adapter. `tools/` keeps only shell/pty/patch/change/context/streaming/capabilities; `providers/` keeps catalog+models. Final graph: `MCP → Engineering Runtime → Core Services`.
- **Clippy meaningful again** — crate-wide blanket suppressions removed from every live module; each live module root carries one documented inner allow for `dead_code`/`unused_imports` only. All other lints active and clean (0 warnings, `--all-targets`). Surfaced fixes: provenance match restructure, `sort_by_key`, let-else guards in impact traversal, `io::Error::other`, sandbox backend signatures now take `&Path`, self-assignment removal in identity migration.
- **Toolchain pinned** — `rust-toolchain.toml` sets channel `1.97`. The committed lockfile already required cargo ≥1.85 via edition2024 transitive deps (`idna_adapter`); this makes the floor explicit.

### Fixed
- **Verification commands inherit CodeBro build env** — spawned `sh -c` commands (shell tool + PTY backend) dropped nothing before; a workspace's own `cargo check/test` could be redirected into CodeBro's `CARGO_TARGET_DIR`, contending across sessions and breaking evidence reproducibility. `CARGO_TARGET_DIR`, `RUSTFLAGS`, `CARGO_BUILD_TARGET` are now stripped at spawn.
- **Identity migration self-assignments** — `migrate_v090_to_v100` assigned fields to themselves; serde defaults already cover missing-field fill on load.

### Removed
- **Unused dev-dependency** `tokio-test`.

### Added
- **Python and Node workspace indexing at `codebro init`** — the tree-sitter parsers were already there; discovery now meets them. `pyproject.toml` ([project] name/version, PEP 508 dependencies with extras/specifiers stripped, optional-dependencies as Optional kind), `setup.py`/`setup.cfg` (best-effort name/`install_requires`), and `requirements.txt` (comments/editables excluded) produce package + dependency facts; `package.json` produces Direct/Dev dependency facts with tsconfig.json promoting the language to typescript. The no-manifest fallback now reports a real language instead of "unknown", module names drop any supported extension (`.py`/`.ts`/`.go`, not just `.rs`), and venv/pycache/site-packages/.next directories are excluded from scans. 5 new tests.
- **ARCHITECTURE.md** — live module map, dependency rule, trust model, CLI reference.

### Tests
- Test inventory is now honestly separated: **752 runtime tests** vs **2,467 legacy regression tests** (previously reported as one undifferentiated green number). Full suite: 3219 collected; runtime suite green; 11 ignored by design.

### Fixed
- **Duplicate dependency facts when a crate appears in multiple Cargo sections** — `async-trait` in both `[dependencies]` and `[dev-dependencies]` produced two identical `dep::` facts (ids are name-keyed). One entry is kept; the most product-relevant kind wins (direct > build > dev). Found by running init against a real 833 MB workspace.
- **README blockquote artifacts leaked into inferred descriptions** — a leading `> One API key. One endpoint.` mined as `"> One API key..."`. Blockquote markers and emphasis characters are now stripped before the two-sentence cap.
- **`codebro init` hung and exploded memory on dataset-heavy repos** — machine-generated source files that embed datasets (`data.py`/bundle dumps with a `.py`/`.js` extension, common in ML repos) were read whole into memory and tree-sitter parsed with no size bound: a single 9 MB file measured ~700 MB RSS and 9 s wall time, so a repo with a few hundred MB of embedded data hung the machine. Source files above 512 KiB are now skipped via one `stat` before any read, with the skip count reported in init output (same repo after fix: 0.02 s, 22 MB).
- **Relationship graph: caller resolution was silently broken for every call edge** — `caller_fact_id` compared the parser's bare file name (`memory.rs`) against symbol facts' workspace-relative paths (`src/agent/memory.rs`), never matched, and every verified edge fell back to a synthetic `anon_call@N` id that resolved to no fact. Store validation reported 6014 `broken_index` issues; impact analysis lost all caller attribution. Fix: init normalizes `ParseCall.caller_file`/`ParseImport.file` to workspace-relative paths; unresolvable callers now drop the edge instead of emitting dangling ids.
- **Rust AST import extraction was completely dead** — tree-sitter-rust emits `use_declaration`, not `use_item`, so the match arm never fired and zero `Imports` relationships were ever produced. Also handles `scoped_identifier` callees (qualified calls like `util::helper()` were dropped) and propagates the enclosing function name during the AST walk so calls are attributed to their containing function instead of an arbitrary first-symbol fallback.
- **Heuristic reference ids could collide** — several same-name symbols in one module emitted identical `ref::` ids (duplicate-facts validation). Reference emission is now deduplicated by id.
- **Module-endpoint relationships were orphaned in validation** — AST import edges (module→module, no source location) were scoped by no reverse index. `index_module` now scopes them under both endpoint modules. Fact store validates at **0 issues**.

### Changed
- **Consultant cleanup** — Removed dead `ChatGpt`, `Claude`, and `DeepSeek` variants from `ConsultantProvider` enum. Removed stale doc comments referencing removed browser/extension providers. Fixed unreachable-pattern clippy error in `ConsultantRouter::resolve`. Prompt builder and provider docs no longer reference ChatGPT/Claude/DeepSeek.

### Added
- **Receiver-type call resolution** — call edges now resolve through the receiver instead of bare-name matching alone: `User::new()` and `Config::new()` each bind to their own impl even though the method name is identical; `self.save()` / `Self::help()` resolve within the enclosing `impl` block (span-based impl scopes collected at init); namespace-style qualifiers (`crate::util::helper`) fall back to trustworthy bare-name rules, while a known type that owns no such method stays honestly unresolved. Relationship ids carry a target hash so same-line same-name calls to different symbols can never collide. Verified call edges on CodeBro itself grew from ~5.9k to ~10.6k with the store still validating at 0 issues.
- **Documentation mining at init** — deterministic parsing of human-authored docs into project identity: `docs/ADR/*.md` become engineering decisions (title/status/context parsed from both bold-key and section styles), an explicit `Conventions` section in AGENTS.md becomes coding conventions, and CHANGELOG releases become milestones. Mined content appends only missing entries (idempotent re-init) and never overrides authored data.
- **Project identity inference at `codebro init`** — Deterministic surface inference (`src/project_identity/infer.rs`) fills identity gaps from Cargo.toml/go.mod/package.json, README first paragraph, and git remote: description, repository URL, build system, package manager, testing framework, frameworks (curated per-ecosystem map), important files. Plus a deterministic architecture summary and top-modules-by-symbol-count derived from generated facts. Inference only fills fields nobody authored — curated goals/constraints/decisions survive re-init; machine-generated summaries refresh, human prose does not. 5 unit tests + 2 integration tests.
- **MCP `update_identity` tool (tool #16)** — Update the persistent project identity: goals (description), constraints, engineering decisions, roadmap items, sprint, conventions, patterns, architecture summary. Medium-high-trust "declared intent" store served by `workspace_context`, distinct from agent-recorded memory. List fields append unique entries; duplicate decision/roadmap titles are skipped with a report; validation gates persistence via `ProjectIdentityUpdater`. 2 MCP tests. Server instructions updated to steer goal recording through this tool.
- **Conductor consultant provider** — `ConductorProvider` (`src/consultant/providers/conductor.rs`) is now the primary and only supported consultant runtime. CodeBro calls Conductor's OpenAI-compatible `POST /v1/chat/completions` endpoint with `Authorization: Bearer <CONDUCTOR_API_KEY>`. Configuration via `CONDUCTOR_API_KEY`, `CONDUCTOR_BASE_URL` (default `http://127.0.0.1:8080`), and `CONDUCTOR_MODEL` env vars (or the secure credential store). CLI: `codebro consult --provider conductor --mode <mode> "question"`. MCP tool: `consult` (tool #15). Mode mapping: `architecture→agentic`, `debugging→coding`, `code_review→coding`, `planning→planning`, `research→reasoning`, `second_opinion→reasoning`.
- **MCP `consult` tool (tool #15)** — Ask an AI consultant (Conductor gateway) for opinions. Supports `provider` (`auto` | `conductor`), `mode`, `question`, optional `context`, `files`, `include_git_diff`, `include_project_context`, `max_answer_length`. Project context and git diff are injected automatically when requested.
- **CLI `codebro consult`** — Same capability as the MCP tool from the terminal.
- **CLI `codebro auth status`** — Shows authentication status for registered consultant providers.
- **Sprint 29 — Consultant architecture** — `src/consultant/` module with `ConsultantProvider` trait, `ConsultantRouter`, shared `build_prompt`/`truncate_answer`, type-safe mode/provider enums.

### Removed
- **Browser-based consultant providers removed** — Firefox WebExtension bridge, extension bridge server, and bridge daemon are removed. The ChatGPT extension provider, legacy Playwright-based ChatGPT provider, and browser-profile-based Claude/DeepSeek stub providers are removed. `codebro bridge start/stop/status` CLI commands are removed. `codebro auth login/logout` browser flows are removed. Supported consultant runtime is now API-first via Conductor only.
- **Sprint 25 — Architecture Consolidation (ADR-012)**
  - Removed legacy `src/context/` (v0.3 context builder) — superseded by `engineering_context` + `assembly`.
  - Removed legacy `src/prompt/` (v0.3 prompt assembly) — zero consumers; superseded by `prompt_builder`.
  - Removed `intelligence/memory/` (`IntelligenceMemory`) — dead duplicate of `project_identity` / `engineering_facts`.
  - Removed `reliability/health.rs` and `reliability/circuit_breaker.rs` — duplicates of the canonical `provider_runtime` health/circuit-breaker implementation. `reliability/` now contains only provider-agnostic generic infra.
  - Removed the legacy `PromptCompiler::compile(13 params)` and `PromptBuilder::compile()/compile_with_default_template()` APIs. `compile_context(&EngineeringContext)` is the only public compile entry point.
  - Removed `src/indexer/` (`RepositoryIndex`) — dead once its only consumers (legacy `src/context/`) were removed.
  - Removed orphaned uncompiled files: `src/tests/concurrency.rs`, `src/tests/p3_validation.rs`, `src/tests/validation.rs`, `src/memory_runtime/tests.rs`.
  - Removed ~90 tests that exercised only removed abstractions; migrated remaining tests to canonical owners.

### Added
- **Sprint 25 — Architecture Consolidation**
  - `docs/ADR/ADR-012-architecture-consolidation.md` — canonical ownership decisions for Context, Intelligence/Memory, Prompt Compiler, Provider Reliability, Task/Workflow.

### Changed
- **Sprint 25 — Architecture Consolidation**
  - `src/runtime/context.rs` no longer depends on `reliability::HealthMonitor`.
  - Documentation updated to reflect the canonical architecture (README, architecture manifest/snapshot, ADR-008/010, contracts, Reliability Architecture Report).

### Added
- **Sprint 23.0 Workspace Metadata Correction**
  - `ProjectIdentityRuntime::create` and `create_minimal` now persist the runtime's `workspace_root` into both the canonical `project_identity.json` and the `workspace.json` projection.
  - Caller-provided builder workspace roots are preserved only when they exactly match the runtime root; otherwise the runtime root wins.
  - `save_all` documentation updated to reflect sequential (non-atomic) projection writes.
  - 3 new tests; full suite 2387 passed / 0 failed
- **P10.5.1 Fact Store Foundation**
  - Canonical immutable repository for Engineering Facts built on the P10.5.0 FactsModel
  - Owns: FactStore, FactCollection, FactIndex, FactLookup, FactQuery, FactSnapshot, FactStatistics, FactDiagnostics, FactValidation
  - Deterministic, read-only primary indexes for every entity id plus reverse workspace/package/module/symbol scope indexes (pure field projections, no graph traversal)
  - Byte-identical snapshots (canonical JSON + FNV-1a 64 digest, no timestamps/randomness)
  - Store validation: duplicate facts, broken indexes, missing ids, orphan records, schema consistency
  - O(log n) allocation-free lookups and enumeration; lifecycle builder; Send + Sync, 8-thread concurrency test
  - 39 new tests; full suite 2111 passed / 0 failed
- **P10.5.0 Engineering Facts Model**
  - Immutable, language-neutral engineering fact model consumed by the Engineering Runtime
  - Facts are the only public contract between language intelligence providers and the runtime (no source, AST or parser dependency)
  - Entities: Symbol, Module, Package, Workspace, Dependency, Relationship, Reference, Test, Build Target, Diagnostic, Architecture Rule
  - Opaque IDs (`FactId`) with no UUID generation, timestamps or randomness
  - Deterministic validation: duplicate IDs, invalid references, self-references, orphan symbols, broken dependency links, unresolved visibility
  - `FactsBuilder → FactsModel` freeze pattern; id-sorted storage with O(log n) allocation-free lookups
  - Send + Sync, serde (JSON/TOML) round-trips, full determinism
  - 27 new tests; full suite 2063 passed / 0 failed

## [1.0.0] - 2026-08-06

> **Note:** This section documents the TUI-era release (pre-MCP). The current release is v0.7.0-mcp-rc2 on the MCP-first `main` branch.


### Added
- **P6 Foundation Platform**
  - Preference Engine: Persistent, schema-versioned preference storage with atomic writes and rollback
  - Intent Engine: Deterministic intent classification with regex-based rules
  - Recommendation Engine: Rule-based recommendations from intent plans
  - Workflow Engine: Deterministic workflow planning with dependency analysis
  - Adaptive Validation: Read-only pipeline validation with policy-driven rules
- **P7 Release Candidate**
  - Integration Pipeline: End-to-end orchestration of all P6 engines
  - PipelineResult: Immutable, serializable pipeline output
  - ApprovalSummary: Human-readable approval view for TUI
  - Concurrency tests: Thread-safety and determinism verification
- **P8 Stable**
  - Production packaging and release artifacts
  - Comprehensive documentation (19 reports)
  - CHANGELOG and Release Notes

### Features
- Multi-agent architecture with Research, Planning, Coding, Testing, Review agents
- Tree-sitter code indexing for Rust, Python, JavaScript, TypeScript, Go
- Semantic code search with relevance ranking
- Dependency graph analysis
- Memory consolidation engine (dedup, merge, cleanup)
- Skill lifecycle system (Draft → Testing → Trusted → Deprecated)
- Permission safety layer with dangerous pattern detection
- Agent operation tracing
- Workspace awareness
- Session replay system
- Execution metrics and cost tracking
- Terminal diff review with accept/reject/edit
- Command palette (Ctrl+P)
- Dashboard metrics panel (Ctrl+V)
- Agent coordination view (Ctrl+O)
- Streaming responses in TUI
- Live agent status monitoring
- Task graph visualization (Ctrl+G)

### Configuration
- Zero-configuration first run
- Environment variable support (`CODEBRO_API_KEY`, `CODEBRO_BASE_URL`, `CODEBRO_MODEL`)
- TOML config file (`~/.codebro/config.toml`)
- Multi-provider support (OpenAI, OpenRouter, DeepSeek, Ollama, LM Studio)

### CLI Commands
- `codebro` — Start TUI chat
- `codebro chat` — Start TUI chat (explicit)
- `codebro list-models` — List available models
- `codebro onboard` — Run onboarding wizard

### Keyboard Shortcuts
- `Ctrl+A` — Toggle agent panel
- `Ctrl+G` — Toggle task graph
- `Ctrl+M` — Show memory changes
- `Ctrl+S` — Save session
- `Ctrl+T` — Show trace
- `Ctrl+L` — Clear logs
- `Ctrl+C` — Cancel current task
- `Ctrl+P` — Open command palette
- `Ctrl+V` — Toggle metrics panel
- `Ctrl+O` — Toggle coordination view
- `Ctrl+Q` — Quit

### Changed
- None (first stable release)

### Deprecated
- None

### Removed
- None

### Fixed
- Recommendation latency threshold adjusted for consistent test timing

### Security
- Permission safety layer prevents dangerous operations
- API keys never logged or persisted in config
- Atomic preference writes prevent corruption
- Backup/rollback on corruption detection

### Performance
- Single pipeline latency: ~0.95ms
- Multi-threaded throughput: ~11.7K ops/ms
- Peak memory: ~2.3 MB (single), ~18.5 MB (100 threads)
- Determinism verified: 0.00% deviation

### Dependencies
- ratatui 0.26 — Terminal UI
- crossterm 0.27 — Terminal interaction
- tokio 1 — Async runtime
- reqwest 0.12 — HTTP client
- tree-sitter 0.20 — Code parsing
- rusqlite 0.31 — SQLite database
- clap 4 — CLI parsing

---

## [0.7.0] - 2026-07-28

### Added
- Agent Coordination Layer (v0.7)
- Agent Message Bus
- Shared Agent Workspace
- Dynamic Task Replanning
- Agent Decision System
- Resource Management
- Agent Performance Learning

---

## [0.6.5] - 2026-07-20

### Added
- TUI Agent Command Center (v0.6.5)
- Dashboard layout with agent panel, activity log
- Live agent monitoring
- Agent event bus
- Task visualization
- Live animations
- Tool execution view
- Memory and skill notifications
- Streaming response UI

---

## [0.6.0] - 2026-07-10

### Added
- Multi-agent architecture (v0.6)
- Subagent framework (Research, Planning, Coding, Testing, Review)
- Task Router with complexity analysis
- Task Graph Engine with DAG representation
- Experience Replay system
- Smart Tool Router

---

## [0.5.0] - 2026-06-28

### Added
- Code Intelligence Architecture (v0.5)
- Tree-sitter integration for 5 languages
- Symbol index with SQLite
- Semantic code search
- Dependency graph
- Intelligent context builder
- LSP foundation

---

## [0.4.0] - 2026-06-15

### Added
- Reliability Layer (v0.4)
- Memory Consolidation Engine
- Skill Lifecycle system
- Permission Safety Layer
- Agent Operation Trace
- Shell Session improvements
- Workspace Awareness

---

## [0.3.0] - 2026-06-01

### Added
- Tool system with dispatcher
- Patch engine for file editing
- Repository indexing
- Context building
- Session memory

---

## [0.2.0] - 2026-05-15

### Added
- TUI with chat interface
- Provider abstraction
- Streaming responses
- Markdown rendering

---

## [0.1.0] - 2026-05-01

### Added
- Initial project structure
- Basic CLI
- Config system

---

[1.0.0]: https://github.com/EffNine/CodeBro/releases/tag/v1.0.0
[0.7.0-mcp-rc2]: https://github.com/EffNine/CodeBro/releases/tag/v0.7.0-mcp-rc2
[0.7.0-mcp-rc1]: https://github.com/EffNine/CodeBro/releases/tag/v0.7.0-mcp-rc1
[0.7.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.7.0
[0.6.5]: https://github.com/EffNine/CodeBro/releases/tag/v0.6.5
[0.6.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.6.0
[0.5.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.5.0
[0.4.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.4.0
[0.3.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.3.0
[0.2.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.2.0
[0.1.0]: https://github.com/EffNine/CodeBro/releases/tag/v0.1.0

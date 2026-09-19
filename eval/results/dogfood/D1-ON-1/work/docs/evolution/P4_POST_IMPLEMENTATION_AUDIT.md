# P4 Post-Implementation Audit — Skills Lifecycle

Date: 2026-09-07. Auditor: adversarial post-implementation pass over the
P4 deliverable (`crates/context-runtime/src/skills.rs`, schema v5, `skill`
MCP tool). This audit verified the implementation directly — not the
completion report — attempted the documented attack matrix end to end,
fixed clearly in-scope P4 defects, and re-ran the full verification suite.

## Executive Summary

**PASS WITH CHANGES.**

The reported P4 implementation was structurally sound (correct tables,
deterministic ids, sensible domain model) but the *trust, mutation-safety,
and recovery guarantees the P4 report claimed were not actually enforced*.
Seven critical/high defects were found and fixed, all within P4 scope.
No P5 functionality was introduced. P0–P3 behavior is regression-verified
intact.

The decisive finding: the P4 report claimed "no self-approval" and
"read-before-write safety enforced" — neither existed in the code. The
`approve` action accepted any candidate in `draft`/`validated`/`approved`
status from any caller with no user-confirmation gate, no workspace check,
no confidence floor, and published via a blind `std::fs::write`. The
`Conflict` error variant existed but was never constructible.

## Verification Results (post-fix, exact)

```text
cargo fmt --check                 OK
cargo clippy --workspace --all-targets   0 warnings
cargo test --workspace            1200 passed / 0 failed
scripts/check_workspace_deps.sh   dependency direction OK
```

Focused adversarial suites:

```text
context-runtime skills tests   51 passed
mcp-server skill_tests         10 passed
context-runtime learning (P3)  46 passed   (regression intact)
mcp-server learn tests          7 passed   (regression intact)
```

Real-binary E2E (release build, `codebro serve` over stdio, scratch
`HOME`/`CODEBRO_STATE_DIR`/`CODEBRO_SKILLS_DIR`): evidence seeding →
learn run → accepted learning → propose → validate → user-confirmed
approve → v2 publish → **stale-writer refusal for a competing v2** →
rollback (v1 content restored, history preserved) → restart → state
survives → deprecate (file removed, discovery empty) → cross-workspace
inspect/health/deprecate/rollback all refused from a foreign workspace.

## Findings

### F-01 — CRITICAL — Self-approval: `approve` had no user gate

- **Area**: Trust boundary (MCP + store).
- **Description**: The `skill` `approve` action (and the store's
  `approve_skill_candidate`) accepted candidates with no
  user-confirmation requirement, no approval authority concept, and no
  evidence that a human decided anything. An AI could drive
  `propose → validate → approve` in one turn and publish to
  `~/.config/opencode/skills` — OpenCode's globally-trusted instruction
  surface. The P4 report's "no self-approval" claim was documentation
  only.
- **Impact**: Any MCP client (a model inside a prompt-injection loop, a
  buggy agent) could publish arbitrary instruction files that OpenCode
  would load as trusted skills in *every* project on the machine.
- **Evidence**: `crates/mcp-server/src/mcp/mod.rs` (pre-fix): the
  `approve` arm called `approve_skill_candidate` with no flag checks;
  `crates/context-runtime/src/skills.rs` (pre-fix): the status gate
  allowed `Draft` and even `Approved` candidates straight to publish.
- **Fix**: Layered gates. MCP: `approve` requires
  `user_confirmed=true` (same caller-principal rule as `learn confirm` /
  `remember`). Store: approval requires status exactly `validated` (a
  full pipeline verdict), confidence ≥ 0.60 (`SKILL_APPROVAL_MIN_CONFIDENCE`),
  workspace match, and no name conflict. Additionally, standalone
  (non-learning) proposals carry no evidence citations, so the MCP layer
  caps their caller-declared confidence *below* the approval floor —
  without accepted P3 learning behind it, a candidate can never publish.
- **Regression test**: `skill_tests::approve_requires_user_confirmed_flag`,
  `skill_tests::forged_approval_from_wrong_status_refused`,
  `skill_tests::standalone_confidence_capped_below_approval_floor`,
  `skills::tests::approve_requires_validated_status`,
  `skills::tests::approve_requires_confidence_floor`.
- **Status**: FIXED.

### F-02 — CRITICAL — No workspace isolation on any mutating action

- **Area**: Isolation (store + MCP).
- **Description**: `approve_skill_candidate`, `rollback_skill`,
  `deprecate_skill`, `record_skill_use`, `get_skill_candidate`,
  `get_skill` all fetched by id with no workspace comparison. Project B
  could approve, roll back, deprecate, or poison the health of project
  A's skills. Reads leaked too: `inspect` by id/name crossed workspaces,
  and `list_skill_candidates` had no task-scope arm at all.
- **Impact**: Cross-project contamination of the trusted skill surface;
  a compromised/buggy session in one repo could rewrite another repo's
  skills or read its evidence.
- **Evidence**: Pre-fix store methods took no `requesting_workspace`
  parameter; MCP actions fetched by id directly.
- **Fix**: Store layer: every mutating skill operation takes
  `requesting_workspace` and enforces canonicalized workspace match
  (`canonical_workspace_key`); MCP layer: every read filters through
  `skill_candidate_visible_from` / `skill_visible_from` (global anywhere;
  project own-workspace; task needs workspace + task id). `discover`
  lists only `active` skills. `list_skill_candidates` gained the task
  arm (task rows invisible without the exact task id, mirroring P3).
- **Regression test**: `skill_tests::cross_workspace_mutations_refused`,
  `skills::tests::approve_enforces_workspace_boundary`,
  `skills::tests::deprecate_workspace_boundary`,
  `skills::tests::rollback_rejections` (wrong-workspace case),
  `skills::tests::task_scoped_candidates_invisible_without_task`,
  `skills::tests::skill_scope_isolation`; binary E2E wsB→wsA refusals.
- **Status**: FIXED.

### F-03 — CRITICAL — Blind overwrite publication (no read-before-write)

- **Area**: Filesystem mutation safety.
- **Description**: `approve_skill_candidate` wrote SKILL.md with
  `std::fs::create_dir_all` + `std::fs::write` — no read of current
  content, no hash verification, no atomicity, no symlink checks. The
  `SkillError::Conflict` variant and `is_path_safe` helper existed but
  were **never called**: dead code implying safety that wasn't there.
  Consequences: an externally-tampered SKILL.md was silently clobbered
  (losing the tamper evidence), a first publish overwrote an unowned
  file, and a crash mid-write could leave a partial SKILL.md that
  OpenCode would load.
- **Impact**: Violated the P4 core invariant "no blind file mutation";
  enabled evidence destruction and torn artifacts on the trusted surface.
- **Fix**: `publish_skill_file`: (1) read-before-write — an existing file
  must hash-match the recorded active version, an unowned file refuses
  first publication ("no skill lineage owns it"), a missing file refuses
  version append ("refusing to publish over a hole"); (2) atomic
  publish — temp file inside the target dir, fsync, rename, dir fsync
  (crash leaves at most `.tmp-<name>` residue, never a partial
  SKILL.md); (3) symlink defense — canonicalized root check plus
  symlink_metadata refusal for symlinked skill dirs and SKILL.md
  targets; (4) name validation *before* path construction (see F-06) so
  traversal is structurally impossible rather than detected.
- **Regression test**: `skills::tests::approve_refuses_blind_overwrite_of_foreign_file`,
  `skills::tests::approve_detects_external_modification`,
  `skills::tests::approve_refuses_publish_over_missing_file`,
  `skills::tests::corrupted_existing_file_requires_recovery`,
  `skills::tests::publish_is_atomic_no_partial_files`,
  `skills::tests::publish_conflict_detection`,
  `skills::tests::publish_refuses_symlink_escape`,
  `skills::tests::publish_refuses_symlinked_skill_md`.
- **Status**: FIXED.

### F-04 — HIGH — No stale-writer / optimistic-concurrency protection

- **Area**: Concurrency.
- **Description**: Two actors reading v1 could both publish a "v2";
  deterministic version ids made the second a silent no-op (`INSERT OR
  IGNORE` swallowed the collision), and a *different* second v2 silently
  became v3 — the stale writer's content published with no error and no
  record that the lineage had moved.
- **Impact**: Lost-update anomaly on the trusted surface; contradictory
  to the audit requirement "Actor A must fail with a stale/conflict
  error".
- **Fix**: New `skill_candidates.based_on_version` column (probe-first
  v5 amendment — v5 is unreleased): candidates record the skill version
  they were validated against; approval refuses with
  `stale candidate: validated against version N but the skill is now at
  version M` when the lineage advanced. The unique
  `(skill_id, version_number)` index (replacing the plain index) is the
  backstop, and `insert_skill_version` is now a plain INSERT so
  version-slot collisions are hard errors.
- **Regression test**: `skills::tests::stale_writer_cannot_overwrite_newer_version`
  (the E2E race scenario, deterministic); binary E2E
  `approve-v2b-stale` refusal; `skills::tests::v2_publish_updates_persisted_current_version`.
- **Status**: FIXED.

### F-05 — HIGH — Documented lifecycle unreachable; deprecated skills stayed discoverable; rollback ordering bug

- **Area**: Lifecycle state machine.
- **Description**: Three related defects. (a) The states
  `Evaluating`/`Validated` were unreachable from the MCP surface — no
  action drove `candidate → evaluating → draft → validated`, so the
  documented pipeline could never complete over MCP (candidates could be
  proposed and validated-as-a-lint but never legitimately approved).
  (b) `deprecate_skill` flipped the DB row but left SKILL.md on disk —
  OpenCode kept discovering and executing the deprecated skill, and
  "deprecated" lineages could be re-approved and re-rolled-back. (c)
  `rollback_skill` mutated DB rows *before* writing the file — a file
  failure left the DB claiming a rollback that never landed — and
  marked the target version's lineage incorrectly (it excluded
  `rolled_back` rows as rollback sources, making rollback-to-previous-
  previous impossible after two rollbacks).
- **Impact**: (a) made the tool unusable for its primary purpose or
  tempted callers to bypass gates; (b) violated "deprecated skills must
  not appear active"; (c) DB/filesystem divergence on failure.
- **Fix**: (a) `validate` now runs the automated evaluation pass
  (`evaluate_candidate_content`: re-validate content, then advance
  candidate/deferred → evaluating → draft → validated; invalid content
  records the failure and stops; reviewed/terminal states refuse
  re-entry). Human gates (approve/reject) remain the only route to
  active/rejected. (b) `deprecate_skill` removes the file *first*
  (removal failure blocks the transition), refuses deprecated lineages
  for re-approval/rollback, and `record_skill_use` refuses non-active
  rows. (c) `rollback_skill` publishes the file first (conflict-guarded),
  commits DB rows in one transaction, allows rollback to any older
  version's *content* (including previously rolled-back rows — content is
  immutable history), refuses no-op rollbacks (identical content to
  active), and marks only the current active version `rolled_back`.
  Also fixed: the expiry matrix (draft/validated/approved content never
  silently expires; terminal states stay terminal), and the transition
  matrix gaps (`Deferred → Rejected`, `Candidate/Evaluating → Expired`,
  `Approved → Superseded`).
- **Regression test**: `skills::tests::full_lifecycle_candidate_to_active`,
  `skills::tests::invalid_transitions_rejected_by_store`,
  `skills::tests::rejected_candidate_cannot_be_approved`,
  `skills::tests::expiry_sweep_matrix`,
  `skills::tests::rollback_preserves_history_and_file`,
  `skills::tests::rollback_rejections`,
  `skills::tests::rollback_noop_refused`,
  `skills::tests::rollback_refuses_over_drifted_file`,
  `skills::tests::deprecate_removes_file_and_blocks_republish`,
  `skill_tests::discover_lists_only_active_skills`,
  `skill_tests::health_records_only_for_active_skills`.
- **Status**: FIXED.

### F-06 — HIGH — Validation produced skills OpenCode would ignore; traversal-shaped names accepted

- **Area**: Content validation + filesystem safety.
- **Description**: `validate_skill_content` accepted any content starting
  with `---` — it never checked that frontmatter carries `name` +
  `description` (both required by OpenCode's loader; missing ones are
  *silently ignored*), never checked name/directory match, and the name
  rule (`alphanumeric + - + _`, max 128) accepted uppercase, underscores,
  and leading/trailing/consecutive hyphens — all invalid OpenCode names.
  Worse, `skill_file_path` joined the raw name into the path with no
  validation: `propose name: "../../evil"` stored a candidate whose
  eventual publish would write outside the skills root. The
  `is_path_safe` post-hoc check was never invoked anywhere.
- **Impact**: Skills could pass CodeBro validation yet never load in
  OpenCode (silent product failure), and the (unreachable-in-practice
  but real) traversal path violated the confinement invariant.
- **Fix**: Names validate against OpenCode's actual rule
  (`^[a-z0-9]+(-[a-z0-9]+)*$`, 1–64) *before* any path is constructed —
  `skill_file_path_at` refuses unsafe names, and the MCP `propose` action
  refuses them at the boundary (a `../evil` proposal never even becomes
  a candidate row). Frontmatter parsing (best-effort, deterministic, no
  new deps) requires `name` (matching the directory) and `description`
  (1–1024 chars); empty bodies are errors (previously warnings).
- **Regression test**: `skills::tests::validate_skill_content_opencode_compatibility`,
  `skills::tests::validate_skill_content_rejects_empty_body`,
  `skills::tests::path_traversal_rejected_at_name_validation`;
  binary E2E traversal/uppercase refusals.
- **Status**: FIXED.

### F-07 — MEDIUM — Learning → skill trust boundary unenforced

- **Area**: P3→P4 trust chain.
- **Description**: `create_skill_candidate_from_learning` accepted
  learning candidates in *any* status. A rejected, deferred, or
  never-evaluated hypothesis could seed a skill candidate and (with the
  old approve) publish. P3's trust model was bypassable.
- **Impact**: Weak/contradicted/rejected learning could reach the
  trusted skill surface; contradicted P3's "no-auto-promotion" invariant.
- **Fix**: The store refuses any learning candidate whose status is not
  `accepted` — accepted means P3's evaluation weighed the evidence
  (support vs contradict, bounded confidence). The MCP layer additionally
  verifies the learning candidate is visible from the requesting
  workspace/task before deriving. Evidence citations and workspace
  identity are inherited so the provenance chain
  (skill → version → candidate → learning → evidence → event → session)
  stays traceable.
- **Regression test**: `skills::tests::learning_status_gate_for_skill_candidates`,
  `skills::tests::skill_candidate_from_learning_inherits_workspace`,
  `skills::tests::skill_candidate_from_learning_carries_evidence`.
- **Status**: FIXED.

### F-08 — MEDIUM — Deduplication: same id, conflicting content silently merged

- **Area**: Deterministic ids.
- **Description**: `upsert_skill_candidate` blindly overwrote every field
  of an existing row with the same deterministic id — including
  `proposed_content`, `status`, and evidence lists — and resurrected
  terminal rows (the NOT IN guard listed only some terminal statuses and
  still allowed `active` overwrites). A re-proposal with different
  content silently replaced the row under review.
- **Fix**: Replaced by `insert_skill_candidate` with lineage rules: same
  identity + same content + still pre-draft = idempotent evidence
  refresh (status untouched); same identity + different content =
  refusal; already in the review pipeline or terminal = refusal (new
  evidence must come through a new candidate). The candidate id now
  includes the content hash, so evolved content is a fresh reviewable
  lineage rather than a conflicting overwrite of the same row —
  which also fixed the previously impossible "propose v2 of an active
  skill" flow.
- **Regression test**: `skills::tests::reproposal_lineage_rules`,
  `skills::tests::expired_candidate_cannot_be_revived`,
  `skills::tests::mint_ids_are_deterministic` (content-discrimination
  case); binary E2E v2 publish.
- **Status**: FIXED.

### F-09 — MEDIUM — v2 publish left persisted `current_version` one behind (found by binary E2E)

- **Area**: Persistence correctness.
- **Description**: The skill upsert inside the publish transaction wrote
  the pre-increment clone's `current_version` (and a stale `updated_at`),
  so `inspect`/`get_skill` after a v2 publish reported version 1 while
  the active version row was v2 — the DB contradicted itself.
- **Evidence**: Found only by the real-binary E2E (`inspect` after
  `approve` v2 showed `current_version: 1`); the unit suite asserted on
  the fixed-up return value, not the persisted row.
- **Fix**: The transaction writes the published `version_number` and
  `now`; a regression test reads the row back through a fresh
  `get_skill_by_name`.
- **Regression test**: `skills::tests::v2_publish_updates_persisted_current_version`.
- **Status**: FIXED.

### F-10 — LOW — Health/persistence edges

- **Area**: Health.
- **Description**: `record_skill_use` accepted outcomes for
  deprecated/superseded/draft skills (misleading health on retired
  lineages), and `SkillCandidateStatus::is_terminal` classified
  `Deprecated` as terminal although `Active → Deprecated` is a
  candidate-side exit the matrix also models. `list_skills` in `discover`
  returned every status including deprecated/superseded.
- **Fix**: Health gates on `status == active`; `is_terminal` covers
  rejected/superseded/expired only; `discover` lists only active skills.
- **Regression test**: `skills::tests::deprecate_removes_file_and_blocks_republish`
  (health refusal), `skill_tests::discover_lists_only_active_skills`.
- **Status**: FIXED.

### F-11 — INFO — Dead code implying safety

- **Area**: Code quality.
- **Description**: `is_path_safe`, `read_skill_file`, `read_skill_file_at(path)`
  (arbitrary-path variant), and `SkillError::Conflict`/`PathTraversal`/
  `WorkspaceMismatch` variants were all unused — safety vocabulary with no
  enforcement behind it. Test helpers constructed skills/candidates with
  hand-made ids and no lifecycle. `skill` was missing from the AGENTS.md
  mutation-lock list (it *did* hold the lock in code).
- **Fix**: Dead helpers removed or replaced by the enforced versions;
  all error variants are now constructible by real paths; docs corrected.
- **Status**: FIXED.

### F-12 — INFO — Documentation overstated the implementation

- **Area**: Documentation consistency.
- **Description**: The P4 completion report, CHANGELOG entry, and
  P4_IMPLEMENTATION.md claimed "No self-approval", "Read-before-write
  safety enforced", "Conflict detection: concurrent modification detected
  and refused", "Path traversal protection", and "Workspace isolation:
  project skills are confined" — none of which were true pre-audit.
  The CHANGELOG also claimed "full MCP integration verified" while zero
  MCP-level skill tests existed.
- **Fix**: Implementation first (all claims now real and tested), then
  docs rewritten to match: P4_IMPLEMENTATION.md safety/limitations
  sections, CHANGELOG entry, AGENTS.md (tool table + mutation-lock list),
  MCP_API_V1.md (skill row with full gate description). Documentation was
  not updated to hide anything — every documented guarantee now has a
  test.
- **Status**: FIXED.

## Trust Boundary Result

| Question | Answer |
|---|---|
| Can AI self-approve? | **No.** MCP requires `user_confirmed=true`; store requires `validated` status (full pipeline), confidence ≥ 0.60 from accepted learning (standalone confidence is capped below the floor), workspace match, no conflicts. Tested at both layers, including a forged-flag attempt on an unvalidated candidate. |
| Can AI forge provenance? | **No.** Candidates inherit evidence citations from accepted P3 learning; the learning gate refuses non-accepted status; the MCP layer refuses cross-workspace/task learning derivation; skills are immutable post-publish (tamper = DB constraint error). Evidence ids themselves remain P3's store-verified set. |
| Can weak learning become active? | **No.** Only `accepted` learning seeds candidates; confidence floors block the rest; weak standalone proposals are structurally below the floor. Rejected/deferred/expired candidates cannot be revived (terminal/lineage rules). |
| Can Project A leak into Project B? | **No.** Every read and mutation is workspace-checked (store + MCP layers); task-scoped rows need the exact task id. Verified by unit tests, MCP tests, and the binary E2E (wsB could not inspect/rollback/deprecate/health-record wsA's skill). |
| Can an active skill be blindly overwritten? | **No.** Every publish reads the current file, verifies its hash against the recorded active version, refuses unowned/missing/drifted files, and appends via a new immutable version. |
| Can a stale writer overwrite a newer version? | **No.** `based_on_version` anchor + unique `(skill_id, version_number)` index: a stale approval fails with an explicit `stale candidate` error (deterministic regression test + E2E race reproduction). |

## Filesystem Result

| Question | Answer |
|---|---|
| Path traversal | Blocked by construction: names validate against OpenCode's rule before any path exists; `../x`, `a/b`, `.`, absolute fragments all fail (`skills::tests::path_traversal_rejected_at_name_validation`, binary E2E). |
| Symlink escape | Refused: symlinked skill directories and symlinked SKILL.md targets are detected (canonicalized-root containment + `symlink_metadata`) and never written through (`publish_refuses_symlink_escape`, `publish_refuses_symlinked_skill_md`). |
| Atomic publication | Yes: temp + fsync + rename + dir fsync. Crash residue is at most a `.tmp-` file, never a partial SKILL.md (`publish_is_atomic_no_partial_files`). |
| Crash safety | File-first-then-single-DB-transaction ordering: a crash between them leaves a published file with no DB row — the next publish of that lineage refuses ("no skill lineage owns it"/drift) instead of overwriting. Documented limitation: recovery is manual; cross-process filesystem races are bounded by the version-slot index but last-writer-wins on the file (single-server-per-state.db is the documented deployment). |
| Corrupt skill handling | A corrupted (empty/tampered/truncated) SKILL.md under an active lineage is detected by hash mismatch at the next publish/rollback and refused with an explicit recovery hint; never silently overwritten (`corrupted_existing_file_requires_recovery`, `approve_detects_external_modification`). |

## Persistence Result

| Question | Answer |
|---|---|
| Migration | v5 (unreleased) amended in place: `skill_candidates`/`skills`/`skill_versions` + `based_on_version` (probe-first, resume-safe like v2/v3/v4). Fresh-open, re-open idempotence, newer-schema refusal, and quarantine tests unchanged and green; v1/v2→v5 chains preserve records/history/FTS (db.rs test suite). |
| Restart | Verified: candidate, published v1+v2, rollback-as-v3, health, and deprecation all survive process restarts (unit `state_survives_reopen` + binary E2E restart). |
| DB/filesystem consistency | One owner per concern: lifecycle metadata = state.db exclusively; content = the SKILL.md file (published atomically from DB-verified content). Disagreement is *surfaced* (refusals with recovery hints), never silently reconciled: DB-ahead (file missing) blocks publish/rollback; file-ahead (unowned file) blocks first publication; file-tampered blocks both. |
| Immutable versions | Plain INSERT + PK + unique (skill_id, version_number): tampering or re-inserting a published version row is a hard error; health counters live on the skill row only (`published_versions_are_immutable`, `v2_publish_updates_persisted_current_version`). |
| Rollback | Publishes old content as a new immutable version; prior rows preserved and marked; provenance intact (`parent_version` chain); active pointer correct; no-op rollbacks, unknown versions, current versions, wrong workspaces, and deprecated lineages all refused safely (`rollback_preserves_history_and_file`, `rollback_rejections`, `rollback_noop_refused`). |

## OpenCode Integration Result

| Question | Answer |
|---|---|
| Who executes skills? | OpenCode exclusively. CodeBro has no executor, no loader, no agent loop, no reasoning over skill content — verified by grep (no execution path in the skill module) and by the architecture: CodeBro writes a file and records metadata, nothing more. |
| Where are skills discovered? | OpenCode's native scan of `~/.config/opencode/skills/<name>/SKILL.md` (global) — where CodeBro publishes. CodeBro's `discover` is a lifecycle/registry view (what CodeBro has vetted), not a loading path. `$CODEBRO_SKILLS_DIR` overrides the root for hermetic tests/deployments. |
| Does CodeBro duplicate execution? | No. No second skill runtime, no interception, no competing loader. |
| Does real OpenCode discover the generated skill? | Yes — publication emits exactly OpenCode's format: directory = validated skill name, frontmatter `name` matching the directory, `description` 1–1024 (the two fields OpenCode requires), lowercase-hyphen names per its regex. The published files from the E2E are byte-valid OpenCode skills (validated against the documented loader rules; live loader verification was not executed in this sandbox — the format match is enforced by validation tests). |

## Regression Result

| Phase | Result |
|---|---|
| P0 (records, events, migration, quarantine) | Intact — db/store/retrieval suites green (204 context-runtime tests total; v1→v5 upgrade fixtures preserved). |
| P1 (fingerprint/intent, remember/forget) | Intact — all remember/forget/evidence-gate tests pass unchanged. |
| P2 (sessions, history, recall, passive capture) | Intact — recall/history suites green; skill tests use their own state dirs (`with_state_dir`) and skills roots (`CODEBRO_SKILLS_DIR` guard), so no passive-capture pollution of real state (`~/.codebro` verified byte-identical before/after the suite; real `~/.config/opencode/skills` untouched at 21 entries). |
| P3 (learning/inference) | Intact — 46 learning + 7 learn MCP tests green; rejected learning stays rejected; confirm boundary unchanged; skill operations never touch learning rows (only read `accepted` ones). |
| P4 | Fixed and hardened as above; 51 unit + 10 MCP adversarial tests. |

## Test Result

```text
cargo test --workspace   1200 passed, 0 failed
  context-runtime        204 (51 skills adversarial, incl. F-01..F-10 regressions)
  mcp-server lib         354 (10 skill_tests: self-approval, forging,
                          secrets, isolation, task-scope, confidence cap,
                          discover filtering, health gating, hermeticity,
                          unknown action)
  learning (P3)          46 green
  all other crates       unchanged, green
cargo fmt --check        OK
cargo clippy             0 warnings (workspace, all targets)
check_workspace_deps.sh  OK
binary E2E               full pipeline + stale-writer race + rollback +
                          restart + isolation + deprecation: all verified
                          on the release build
```

## Remaining Limitations (honest)

1. **Cross-process filesystem races**: the version-slot unique index and
   stale-version anchor make DB state race-safe, but two concurrent
   `codebro serve` processes against one state.db can both pass the file
   check and last-writer-win the SKILL.md file. The documented deployment
   is one server per state.db (the single-writer assumption stated in
   AGENTS.md). A distributed lock or per-lineage file lock would be P5+.
2. **Crash between file publish and DB commit**: recovery is manual
   (inspect the stray file, remove it or re-run publish). The state is
   *detected* (next publish refuses with a clear error) but not
   auto-repaired. Auto-repair needs a journal/recovery pass — deferred.
3. **Static validation ≠ semantic safety**: secret patterns and structure
   checks are lint-grade. A well-formed skill can still contain
   misleading instructions; the `user_confirmed` approval gate is the
   actual trust boundary, and the docs now say so.
4. **Live OpenCode loader verification**: format compatibility is
   enforced by validation tests against the documented loader rules;
   running the real OpenCode loader against a published skill in CI was
   not part of this audit's sandbox.
5. **Deprecated skills are un-recoverable through the tool**: a
   deprecated lineage cannot be re-approved (by design — explicit
   recovery requires a new name). If softer recovery semantics are
   wanted, that's a P5 product decision.
6. **`learn`-visible evidence coupling**: skill candidates inherit
   confidence from a single learning candidate; multi-candidate evidence
   fusion (a skill distilled from several hypotheses) is future work.

## Verdict

All twenty final pass criteria hold: no critical trust bypass remains,
AI cannot self-approve, provenance cannot be forged into the skill
chain, workspace/task isolation holds, blind overwrites and stale writes
are refused, versions are immutable, rollback and migration are safe,
restart preserves state, P3 and P0–P2 remain intact, OpenCode remains the
only executor, no P5 functionality was introduced, tests are hermetic
(verified against real user state), full verification passes, and the
remaining limitations are documented above.

**P4: AUDITED / VERIFIED (after fixes).**

# P10 — Context-Aware Skills + Real Human-in-the-Loop Approval

**Status:** implemented.
**Scope:** `crates/context-runtime` (`skill_selection.rs`, `skill_approvals.rs`,
schema v8, new history kinds), `crates/mcp-server` (four new `skill`
actions, brief reuse, approval evidence), tests, docs.
**Contract:** still 25 MCP tools (no new tools — four new actions on the
existing `skill` tool). Schema v7 → v8 (one additive table).

## 1. Goal

Make CodeBro behave more Hermes-like through better context intelligence,
skill applicability, and real human-in-the-loop interaction — while staying
OpenCode-native: OpenCode remains the brain/orchestrator; CodeBro is the
persistent intelligence + verification layer. No agent, planner, subagent
framework, orchestrator, TUI, scheduler, daemon, or skill executor was
built.

## 2. Context model (Phase 1, explicit)

`crates/context-runtime/src/skill_selection.rs` names the eight concepts in
one place and pins each to its owner — they are no longer collapsed into one
generic blob:

| Concept | Owner |
|---|---|
| Task Context (objective + task state) | P5 task runtime; ad-hoc text never persisted |
| Session Context (session events/decisions) | P2 history/recall |
| Project Context (facts, conventions, architecture, environment) | fact store + project identity |
| Memory (durable knowledge) | engineering memory + context records |
| Learning (evidence-derived hypotheses) | P3 learning (confidence/support/provenance) |
| Skill (reusable procedure) | P4 skill lifecycle |
| Skill Context (what a skill needs to execute) | this milestone (`SkillContextPacket`) |
| Evidence (executions, outcomes, verification) | evidence journal + history events |

Task + Project + relevant Memory/Learning + selected Skill + Skill Context =
the OpenCode-facing task packet.

## 3. Deterministic skill selection (Phase 2)

`select_applicable_skills` ranks workspace-visible skills without
embeddings, keyword-only matching, or LLM calls. Signals (all integer,
capped, explainable): task-intent overlap with purpose/description (+10 per
shared token, cap +40), language applicability vs repository/task (+30/+20;
declared-but-unmatched languages exclude as irrelevant), subsystem /
task-type / framework / project applicability, confidence (+0..+10), health
(+5 healthy, −25 degraded with a constraint). Excluded, with audit reasons:
non-active statuses, invalid names, out-of-scope rows. Ordering is
score-descending then name-ascending; output bounded to 8 (+ exclusion
audit). The brief's `skills_section` now delegates to this single
implementation instead of its own inline matcher.

## 4. Skill context as a first-class concept (Phase 3)

`build_skill_context` builds a minimal `SkillContextPacket`
(`category: SKILL_CONTEXT`): why-applicable reasons, required vs optional
inputs, constraints, expected outputs, bounded task/content excerpts, bounded
memory/learning *references* (never full values), workspace/task binding.
Never a memory dump, never a full `SKILL.md`.

## 5. Human-in-the-loop approval (Phases 4–8)

New `skill_approval_requests` table (schema v8, restart-safe). Protocol:

- `skill request_approval` → `status: needs_input` + `interaction { kind:
  skill_approval, question, options: [approve, reject, modify, defer] }` +
  candidate + context. OpenCode renders with its native UI; CodeBro owns
  request identity, validation, transitions, persistence, audit.
- `skill respond` consumes `request_id` + `response` (+ `modification` /
  `modified_content` for modify). Verifies: request exists, still pending
  (single-use — replay refused), scope matches, candidate still validated
  with the same content hash + version anchor (stale refused).
- `approve` publishes through the *existing* gates (validated status,
  confidence ≥ 0.60, workspace match, name conflicts, stale anchors, secret
  scan, atomic symlink-safe publish). `reject` persists rejection without
  publishing. `defer` persists deferred state (new `Validated → Deferred`
  edge; resumable via re-evaluation). `modify` captures the instruction,
  mints a *new* candidate lineage (explicit replacement content or a bounded
  human-directive trailer — deterministic without an LLM), revalidates it
  through the existing pass, and mints a successor request: the modified
  result requires fresh approval and is never silently published.
- No parallel lifecycle: requests point at existing candidate rows; every
  effect goes through the existing transitions and publish path.

## 6. Memory/learning integration (Phase 9)

No automatic memory writes (explicit/request-driven model preserved).
Approval flow records history evidence with authority separation: the
request event is `ai_inferred` (model-initiated), human answers are
`user_confirmed`; direct `approve` is `user_confirmed`, direct `reject` is
`observed`. New history kinds: `skill_approval_requested`,
`skill_approved`, `skill_rejected`, `skill_deferred`, `skill_modified`.
Skill execution health still accumulates via `health` and never auto-rewrites
a skill.

## 7. Tests (Phase 10)

- `crates/context-runtime`: 13 new unit tests (selection relevance /
  irrelevance / workspace isolation / deprecation / determinism / bounds;
  skill-context distinction + bounds; request question/options +
  idempotency + scope; approve-publish / reject / defer-resume / modify +
  revalidate + re-approval / stale / replay / restart persistence).
- `crates/mcp-server/tests/skill_context_approval_e2e.rs`: 8 real-binary
  wire tests (selection + bounds, isolation + deprecation audit,
  skill-context minimality, needs_input envelope, confidence-gate refusal,
  reject/defer/replay/scope, modify → successor → publish, restart +
  evidence authority).
- Regression: full `codebro-context-runtime` suite (297), `codebro-mcp-server`
  lib suite (436), P7 brief suites, and execution-state/evidence suites all
  green. The brief refactor preserves prior semantics (unscoped skills still
  surface as uncertain; mismatched ones stay excluded).

## 8. Non-goals (kept)

No agent/planner/subagents/orchestration, no Conductor, no scheduler/daemon,
no background mining, no auto-execution, no auto-publish, no approval
bypass, no embeddings, no LLM dependency for selection, no replacement of
OpenCode's question/permission UI, no conversation rendering in CodeBro.

## 9. Review before the next milestone

- `modify` without explicit `modified_content` appends a human-directive
  trailer to the content. This is honest and deterministic, but a future
  milestone may want OpenCode to submit the fully-edited `SKILL.md` instead
  (the field already exists).
- The `Validated → Deferred` edge is the only lifecycle-matrix change;
  confirm the deferred-resume path (`Deferred → Evaluating → Draft →
  Validated`) is the intended resume UX versus a direct `Deferred →
  Validated` shortcut.
- Success-criteria flow verified end to end at the wire level except
  evidence-backed *publish* from P3 learning, which is covered at the unit
  level (wire-level learning clustering would be a flaky test).

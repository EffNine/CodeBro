# P3 Implementation — Learning + Inference

Date: 2026-09-07. Status: **implemented, tested, verified** (154
context-runtime lib tests + 344 mcp-server lib tests green,
clippy 0 warnings, fmt clean, dep-script clean, 22/22 live stdio checks
green — see §18).
Scope: `docs/evolution/PHASE4_PLAN.md` P3 row, built on the P0 foundation,
the P1 identity substrate, and the P2 history substrate.

P3 answers the third question and stops there:

```text
P1  What has the user explicitly told CodeBro?
P2  What happened during previous work?
P3  What should CodeBro learn from what happened?   ← this document
P4  Which learnings should become reusable skills?   (not built)
```

CodeBro can now observe repeated engineering outcomes, form a cautious
hypothesis, evaluate it against supporting and contradicting evidence,
preserve provenance, assign bounded confidence, and make the resulting
inference available to OpenCode — without ever pretending an AI inference
is user-confirmed truth.

## 1. Architecture

Extension over replacement, again. P2's `events` table is the evidence
layer; P1's `ContextRecord` table is the knowledge layer. P3 adds the
hypothesis layer between them:

```text
context-runtime (one new module, same crate, still → core only)
├── learning.rs   CandidateKind/Status taxonomy, OutcomePolarity,
│                 topic clustering, confidence, evaluation, confirmation,
│                 rejection, expiry, run_learning (impl ContextStore)
├── learning_tests.rs  42 tests (see §17)
├── db.rs         SCHEMA_VERSION 4, stepwise migrate (v4 step:
│                 learning_candidates table + indexes, empty backfill)
└── lib.rs        learning module + facade re-exports

mcp-server (thin adapter, composition untouched)
├── learn tool (tool 22): run/propose/list/get/evaluate/confirm/reject
└── server instructions advertise the learn flow
```

No schema redesign, no second knowledge store, no new event abstraction.
Accepted inferences reuse `context_records` with `authority = ai_inferred`;
the `context` packet shape is byte-identical (inferences flow through the
existing resolution, see §11).

## 2. Learning candidate model

`LearningCandidate`: deterministic `lc::<16 hex>` id (SHA-256 of
`scope|workspace|task|kind|topic-pair`), workspace/task/scope binding,
candidate kind, testable proposition, fingerprint namespace, supporting and
contradicting evidence event ids, confidence, lifecycle status, timestamps,
90-day TTL, evaluation reason, and the persisted-inference backlink.

Kinds (7, closed, extensible by adding a variant): `user_preference`,
`engineering_pattern`, `project_pattern`, `failure_pattern`,
`success_pattern`, `workflow_pattern`, `decision_pattern`.

Statuses: `candidate → evaluating → accepted | rejected`, plus `deferred`
(insufficient or contested evidence; preserved, never surfaced),
`superseded` (explicitly confirmed — the USER_CONFIRMED record now owns the
namespace), `expired` (TTL lapsed; re-detection revives). `rejected`
(user verdict), `superseded`, and `expired` are never rewritten by
re-detection; `accepted` rows refresh their evidence/confidence and may
re-evaluate (knowledge evolves, §7).

## 3. Pattern detection (deterministic, no LLM)

Per detection pass (bounded: newest 500 in-scope events, indexed query):

1. Topic tokens per event: retrieval tokenizer over the redacted
   summary + 500-char payload excerpt, stopwords removed, capped at 10.
   Tool/path tokens are excluded (shared commands/paths are not signal).
2. Pair clustering: group by `(kind-group, outcome-polarity, token-pair)`.
   A pair needs **≥ 2 shared meaningful tokens** — one shared word is
   string frequency, not a semantic relation (tested: three validations
   sharing only "sqlite" produce nothing).
3. One candidate per (kind-group, polarity): the largest pair-group (ties →
   smallest pair). Minimum support 3 (4 for weak `observation` clusters).
   Conversational/session-framing kinds never cluster.
4. Kind mapping: validation+success → success; validation/error+failure →
   failure; decision clusters with avoidance-lexicon topics
   (avoid/unnecessary/minimal/…) → user-preference, else decision;
   change clusters → project (scoped) / engineering (global);
   change→validation-success cycles in ≥ 2 sessions → workflow.
5. Sensitive topics (personality/medical/political/religious/sexuality/
   ethnicity/… blocklist) never become candidates. Engineering
   collaboration and project behaviour only — no psychological profiling.

Propositions are testable statements with counts
("The approach 'x y' has repeatedly failed validation in project P (3
failures across 1 sessions); treat it as unreliable here until
counter-evidence appears."), never bare facts ("We used SQLite.").

## 4. Evidence aggregation

Evidence is *derived* from the canonical event log at detection and
*re-derived* at evaluation — callers cannot inject evidence ids, so there
is no forgery surface. Evaluation drops ids that do not resolve or fall
outside the candidate's scope, and says so in the reason.

Weights (documented in code): strong 1.0 (decisions, validations/errors
with explicit outcomes), medium 0.6 (changes, tool results), weak 0.3
(observations, executions). Contradicting evidence: same topic pair with
opposite outcome polarity (outcome-bearing kinds for polarized candidates;
any failure for neutral ones) — counted within the same scope only, so
project B's taste never contradicts project A's pattern.

## 5. Outcome-aware evaluation

Learning consumes `attempt → outcome`, not `statement → assumption`.
Polarity mapping is explicit and total (unknown labels are neutral, never
guessed). Thresholds:

| Condition | Verdict |
|---|---|
| supporting < 3 | `deferred` (insufficient) |
| contradicting > supporting | `rejected` (weighs against) |
| contradicting × 2 ≥ supporting | `deferred` (contested) |
| confidence ≥ 0.55, contradiction low | `accepted` → `AI_INFERRED` |
| confidence < 0.55, contradiction low | `deferred` (weak) |

Global inference additionally requires broader evidence (≥ 2 workspaces or
≥ 5 events); a one-project pattern evaluated globally defers with that
reason instead of leaking upward.

## 6. Confidence

Bounded `[0.05, 0.95]`, rounded to two decimals (no fake precision):

```text
min(support/5,1)*0.25 + avg_weight*0.20 + (s-c)/(s+c)*0.35
    + recent_ratio*0.10 + min(sessions/3,1)*0.10
```

Contradiction voices twice: through the consistency term and through the
acceptance gates — one contradictory event dents (0.79 → 0.72 in tests)
without flip-flopping; a 5v4 split defers with 0.69 quoted honestly.
Authority and confidence are separate dimensions: 0.95 never means
confirmed (stated in every explanation).

## 7. Reprocessing and supersession

Same history ⇒ same candidate id ⇒ upsert, never duplicates (8-way
concurrent `run_learning` converges to one row; dedup-keyed event replays
do not double evidence). `deferred` candidates revive as evidence
accumulates; `accepted` candidates re-evaluate — still passing refreshes
the inference via the supersede chain (chain preserved, `refreshed`
counted), newly failing retires the stale inference (rejected, kept) and
steps back. Re-running without new evidence reuses the record (no chain
churn).

## 8. Authority and trust

The absolute rule holds structurally:

- Inferences persist as `AI_INFERRED` (lifecycle `Inferred`, evidence
  cited, store-verified). Nothing in the learning path can mint
  `USER_CONFIRMED`.
- `confirm_candidate` requires `user_confirmed=true` (caller-principal
  rule, enforced at the store AND the MCP layer): without it, confirmation
  is refused naming the rule. With it, a `USER_CONFIRMED` record
  supersedes the inference (audit trail kept), and the candidate becomes
  `superseded`.
- `reject_candidate` marks the candidate rejected and rejects its
  inference (preserved as negative knowledge, never deleted). A rejected
  global may be followed by a project-scoped `remember` ("only true for
  this project") — verified live.

## 9. Expiration and decay

Two clocks, one principle (evidence is forever, hypotheses lapse):

- Candidates carry a 90-day TTL (`expire_learning_sweep`; re-detection
  revives).
- Accepted inferences carry `expires_at = +180d` (existing `expire_sweep`
  retires them) AND decay at retrieval time at the `AiInferred` rate
  (0.85/month — a year-old inference fades below 0.2 while a 3-month-old
  confirmation stays above 0.9, P0-tested).

## 10. Persistence and schema

One new table (`learning_candidates`, v4 migration, `IF NOT EXISTS`
idempotent, empty backfill — hypotheses re-derive from preserved events).
Accepted knowledge reuses `context_records` (mapping: user-preference →
`Preference`, failure/success → `Experience`, rest → `Pattern`) with
`extra_json` learning metadata (candidate id, kind, support/contradict
counts) and `source = learn:<candidate-id>`.

Transaction discipline: history commits first in its own transaction;
learning reads committed history in separate short transactions and
persists inferences separately. Inference-first on accept (knowledge is
the valuable artifact; a marking failure is repaired by namespace
supersede on retry). Learning failures are collected per candidate, never
fatal; a dropped candidate table errors `run_learning` while history reads
and writes continue (tested).

## 11. Context integration

No composer change was needed — and none was made. Accepted inferences
are ordinary context records: `Experience` passes through the Other lane,
`Pattern`/`Preference` resolve in the fingerprint lane, authority rank
keeps confirmed knowledge above inference, the 8-record cap bounds the
packet, expired rows are excluded by the active-default, and every excerpt
carries its `ai_inferred` tag with decayed confidence. Deferred/rejected
candidates persist no record and therefore cannot surface (tested both
lanes). Weak-inference exclusion = resolution ordering + cap + decay +
expiry, all pre-existing mechanisms.

## 12. MCP surface

One new capability tool (22 total), no table CRUD:

- **`learn`** — `action` (run/propose/list/get/evaluate/confirm/reject),
  `scope?` (project default | task + `task_id?` | global opt-in),
  `candidate_id?`, `status?`/`limit?` (list), `user_confirmed?`
  (confirm, required true), `confirm?` + `reason?` (reject),
  `workspace_root?`. Mutating actions serialize on the workspace lock;
  list/get are lock-free and capture nothing; **no learn action writes
  history** (no recursion, no self-evidence — recall counts identical
  before/after, tested). Validation errors are `invalid_params`.

## 13. No automatic trigger (deliberate)

P3 performs learning only through the explicit `learn` capability — not
inside passive capture. Reasons: failure isolation (a learning bug can
never break history writes), performance (no unbounded scans per
operation), and honesty (OpenCode decides when a hypothesis is worth
forming; CodeBro decides whether the evidence supports it). The brief's
"automatic learning" is thus implemented as "explicit, bounded, and
conservative" — documented here as the deviation, with the rationale.

## 14. Deviations from the mission brief (with reasons)

1. **No auto-trigger in existing operations** (§13): conservative by
   design; the `learn run` pass is the bounded equivalent.
2. **No `EVALUATING` persistence window**: the status is set and
   transitioned inside one call; a crash between leaves `evaluating`,
   which re-detection treats as re-evaluable (same as `candidate`).
3. **Neutral-candidate contradiction is failure-only**: semantic reversal
   ("we moved off SQLite") is not detectable without a model; outcome
   polarity is the honest deterministic proxy. Documented as a P4+ seam.
4. **`evidence_count` column exists but is informational** (lengths come
   from the JSON lists); kept for future indexed queries, default 0.

## 15. Known limitations (not defects)

- Pair clustering is lexical: synonyms ("dependency" vs "dep") do not
  cluster; richer topic modeling is P4+ and the code says so.
- One candidate per (kind-group, polarity) per pass: parallel topics in
  one lane collapse to the strongest pair-group.
- `total_matches`-style analytics are absent by design (candidates are
  hypotheses, not metrics).
- No fork-based cross-process writer test (P0 standing gap); two-handle
  in-process concurrency is tested.
- redact_secrets regex cost applies to topic extraction inputs (already
  redacted at write time; extraction re-reads stored text).

## 16. P4 boundary (documented, not built)

Preserved for skill evolution: typed patterns with evidence, confidence,
and scope (`learning_candidates` + `AI_INFERRED` records) are exactly the
"validated pattern" input P4's SkillCandidate needs. Not built: no
SKILL.md generation, no skill validation/versioning/publishing, no task
runtime, no daemon, no embeddings, no ChatGPT import/export.

## 17. Tests added

context-runtime 109 → 154 (+45): 42 learning tests (generation ×7 incl.
avoidance→preference and chatter refusal; evidence ×3 incl. fake and
cross-workspace dropping; evaluation ×3 incl. 5v4 contest and majority
reject; confidence ×4; authority ×4 incl. forgery refusal and supersede
promotion; scope ×4 incl. task isolation and global bars; lifecycle ×4
incl. evolution-without-duplicates; security ×3 incl. single-token and
secret tests; workflow ×1; context ×2; isolation ×2; concurrency/
idempotency ×2; vocabulary/polarity/kind-mapping ×3; explain ×1) + 3 v4
migration tests (fresh shape, v2→v4 preserving records+history+both FTS,
partial-step resume).

mcp-server lib 336 → 344 (+8): run-accepts-as-inferred, forged-confirm
refused, confirm-promotes, reject-preserves, project isolation,
no-history-writes, weak-stays-out-of-context, bad-actions battery. Plus
the `learn` arms in the tool-description/router regression lists and the
test-helper dispatcher.

## 18. Verification

```text
cargo test --workspace        # 1139 passed, 0 failed
cargo clippy --workspace --all-targets   # 0 warnings
cargo fmt --check             # clean
scripts/check_workspace_deps.sh          # OK (no new deps: learning → core only via context-runtime)
```

Live stdio (real binary, fresh state dir, §36 scenario): 22/22 green —
remember USER_CONFIRMED → 3 sessions of decisions+changes → learn run
accepts user_preference (AI_INFERRED, high confidence, 3+ evidence) →
context shows BOTH the confirmed record and the inference (tagged) →
forged confirm refused over the wire → user reject + project-scoped
remember → no global corruption → restart: candidates + history durable.

No `~/.codebro` pollution (all servers hermetic; developer state dir
untouched — verified by inspection, no state.db in home modified).

## 19. Product test (brief §36)

- "I don't like adding dependencies" → USER_CONFIRMED: covered live.
- Sessions 2–4 avoiding deps → candidate "appears to prefer minimizing…",
  AI_INFERRED, high confidence, multiple evidence: covered live.
- Session 5 context includes inference without replacing confirmed:
  covered live (both namespaces present, authorities distinct).
- "Only for small utilities" correction → global rejected, project
  USER_CONFIRMED, no global corruption: covered live.

## 20. Verdict

**COMPLETE.** CodeBro observes repeated engineering outcomes, forms a
cautious hypothesis, evaluates it against supporting and contradicting
evidence, preserves provenance, assigns bounded confidence, and makes the
inference available without ever presenting it as user-confirmed truth.
P4 (skill evolution) is unblocked and unimplemented — as required.

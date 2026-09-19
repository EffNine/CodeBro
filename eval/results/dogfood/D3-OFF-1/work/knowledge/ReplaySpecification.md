# Replay Specification

**Phase**: P10.3B · **Replay-first architecture** (deliverable compile of
`certification/replay_spec.md`).

## 1. Contract

Replay MUST allow runtime validation WITHOUT spending tokens. A benchmark is
run ONCE; every later validation replays the recorded golden.

## 2. Replay Record

Immutable: `replay_id · dataset_id@version · provider_id · model ·
benchmark_spec_version · seed · per-test {verdict, metrics} · golden outputs
(sha256) · usage snapshot · framework_version · content hash`.

## 3. Replay Dataset

`datasets/<category>/` entries are provider-neutral prompts + goldens.
The replay dataset is exactly the run's input; goldens hashed.

## 4. Replay Validation (zero-token)

1. Load replay record + pinned dataset version.
2. Run the CURRENT runtime offline against hashed goldens.
3. Compare verdicts + metric values + usage snapshot.
4. Pass iff: verdict identical · drift ≤ tolerance (±10% non-hard metrics) ·
   no new failure criteria.

## 5. Comparison Modes

- Golden vs current runtime → **regression detection**.
- Model vN vs vN+1 → version comparison.
- Cross-provider (research only, never cert).

## 6. Regression Detection

- metric pass→fail or drift > tolerance ⇒ regression ⇒ freeze certified level,
  require resolution before a new run (per policy).
- usage/cost surprises are regressions too (snapshot mismatch).

## 7. Enforcement

- Replay MUST be fully offline: no provider call, no HTTP, no API key read.
- A replay that cannot complete offline is a framework bug.
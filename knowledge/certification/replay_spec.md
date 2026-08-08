# Replay Specification

**Framework**: Benchmark & Certification · **Replay-first architecture**.

## 1. Purpose

Replay allows a runtime to be validated against a previously recorded
(golden) benchmark run **without consuming any tokens**. It is the enforcement
mechanism for "Benchmark once, Replay forever".

## 2. Replay Record

A replay record is immutable and contains:

| Field | Type | Description |
|-------|------|-------------|
| `replay_id` | string | UUID; stable identity of the golden run |
| `dataset_id` | string | dataset + version (see dataset_versioning.md) |
| `provider_id` | string | e.g. `deepseek` |
| `model` | string | model id at run time |
| `benchmark_spec_version` | string | spec used |
| `seed` | int | determinism seed |
| `results` | array | per-test: id, verdict, metric values |
| `reference_outputs` | map | golden outputs (hashed, see §4) |
| `usage_snapshot` | object | recorded usage (tokens/cache) |
| `framework_version` | string | scoring/report version |
| `hash` | string | content hash for integrity |

## 3. Replay Dataset

`datasets/<category>/` entries are the replay inputs (prompts + goldens). A
replay dataset is: `{id, version, prompts, goldens, metadata}`.
The dataset is NEVER provider-specific.

## 4. Replay Validation (zero-token)

1. Load replay record + pinned dataset version.
2. Run the CURRENT runtime offline against `reference_outputs` (mock layer).
3. Compare actual vs recorded: verdicts, metric values, usage snapshot.
4. Replay passes iff:
   - actual verdict == recorded verdict
   - metric drift ≤ replay tolerance (e.g. ±10% for non-hard metrics)
   - no NEW failure-criteria triggers

This is a pure offline comparison; no provider call, no tokens.

## 5. Replay Comparison

Compare three ways:
- **Golden comparison** (record vs new runtime) — regression detection.
- **Cross-provider comparison** — same dataset across providers (never for
  certification, only for research notes).
- **Version comparison** — same provider, model vN vs vN+1.

## 6. Regression Detection

- Regress if a metric passes → fails, or drift > tolerance.
- On regression: open an issue, freeze the certified level, require a new
  benchmark only after resolution (policy gate).
- Alerts also when `usage_snapshot` differs beyond tolerance (cost surprise).

## 7. Rules

- Replay MUST NOT call a provider, send HTTP, or read an API key.
- Golden outputs are stored HASHED (sha256) for integrity; full text stored
  only in the dataset archive.
- Any replay that cannot complete offline is a framework bug.
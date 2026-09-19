# Engineering Policy — Benchmark & Certification

**Phase**: P10.3B · enforceable rules for the framework and provider-scoped use.

## 1. The Core Policy

> **Benchmark once. Replay forever.**
> Only rerun when (a) provider version changes, (b) optimization profile
> changes, or (c) certification expires.

Re-play is the default every-time assurance; re-benchmark is the exception with
a stored reason.

## 2. Supplementary Rules

- **Read before Benchmark.** A provider must be Research Complete before any
  benchmark spec is approved (P10.3A policy).
- **Benchmark before Optimize.** Only benchmark-confirmed values enter an
  Optimization Profile as validated; others stay `Hypothesis`/`Benchmark Required`.
- **Optimize before Certify.** `Certified Optimized` requires benchmark-grounded
  values only.
- **Replay forever.** Between re-runs, runtime validation is replay (zero-token).

## 3. Token Discipline

- Every live benchmark run records its replay golden so no future run repeats
  the spend.
- Replay never calls a provider.
- Budgets are declared before a run; a run exceeding its budget is invalid.

## 4. Determinism Discipline

- Seed pinned; scoring pure; a live-run parameter override invalidates the run.
- Verdict is a pure function of `(dataset@version, seed, model, values)`.

## 5. Dataset Discipline

- Immutable once released; version-bump to change.
- No provider-specific prompts.

## 6. Framework Discipline

- Certifying a new provider MUST NOT change the framework (that change is
  itself a gate-level event needing an ADR).

## 7. Change Triggers → action

| Trigger | Action |
|---------|--------|
| Provider model version changes | re-benchmark (or replay-approve if drift in-tolerance) |
| Optimization Profile changes | re-benchmark affected dimensions |
| Certification expires | re-benchmark; else deprecate |
| No trigger | replay only |

Exceptions require Chief Architect approval and an ADR.
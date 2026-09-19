# Certification Policy

**Framework**: Benchmark & Certification · **Phase**: P10.3B (definition only).

## 1. Certification Levels

Deterministic, ordered, terminal-gated levels:

```
Draft
  │
Research Complete
  │
Benchmark Ready
  │
Certified
  ├── Certified Optimized
  └── Reference Model        (special level, optionally reached)
  │
Deprecated
```

Level definitions:

| Level | Meaning |
|-------|---------|
| Draft | research artifacts created, unsourced facts not allowed |
| Research Complete | sourced card + capability matrix + annotated optimization profile; Read gate passed |
| Benchmark Ready | benchmark spec approved; run authorized |
| Certified | benchmark executed, all success criteria met, report published |
| Certified Optimized | Certified + optimization values adopted only where benchmark-confirmed |
| Reference Model | Certified model designated canonical baseline for replay/regression comparison |
| Deprecated | superseded/removed; frozen, replay-only |

## 2. Gate Contract

A level transition requires ALL of its gate's conditions (deterministic booleans):

| Transition | Gate conditions |
|------------|-----------------|
| Draft → Research Complete | Card sourced ✓ · Matrix sourced ✓ · Optimization Profile annotated ✓ · changelog row ✓ |
| Research Complete → Benchmark Ready | Benchmark Spec approved ✓ · dataset pinned ✓ · replay id assigned ✓ · Chief Architect sign-off ✓ |
| Benchmark Ready → Certified | all success criteria ✓ · failure criteria absent ✓ · report generated ✓ · replayed-by-golden ✓ |
| Certified → Certified Optimized | benchmark-confirmed optimizations adopted ✓ |
| Certified → Reference Model | Chief Architect designation ✓ (model remains Certified) |
| any → Deprecated | superseded notice ✓ · changelog row ✓ |

Gates are **deterministic** (no subjective criteria). A run either satisfies
a criterion or it does not, per `scoring.md`.

## 3. Certification Workflow

```
Research (P10.3A) → Read gate
      ↓
Benchmark Spec approval → Benchmark Ready
      ↓
Benchmark run (gated) → result record (Result Schema) → replay golden
      ↓
Scoring & report → Certified / fail with reason
      ↓
Optimization adoption (benchmark-confirmed only) → Certified Optimized
```

Every step writes to `knowledge/providers/<name>/certification.md` and
`changelog.md`. No step is implicit.

## 4. Certification Determinism

- Pass/fail is a pure function of `(dataset, model, metrics, thresholds)`.
- No reviewer discretion on criteria; only on declaring the run valid.
- A run is valid iff: dataset version pinned, seed set, budget respected,
  replay id assigned, controls (no creds) respected.

## 5. Rule: The Framework is Generic

Certifying a NEW provider MUST NOT require a framework change. New providers
populate datasets via existing categories, existing metrics, existing schema.
A framework change itself is a certification-level change and needs an ADR.

## 6. Outcome Records

Every certification writes: certification.md (level table) · changelog.md row ·
report (report_template.md) · replay golden · result JSON in the Research Index.
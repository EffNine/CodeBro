# Certification Framework

**Phase**: P10.3B · **Status**: APPROVED → IMPLEMENTED (framework; no provider certified this phase)

## 1. Certification Levels

```
Draft → Research Complete → Benchmark Ready → Certified
        → Certified Optimized | Reference Model → Deprecated
```

| Level | Condition to enter |
|-------|--------------------|
| Draft | artifacts created; research may not claim unsourced facts |
| Research Complete | sourced card + matrix + annotated optimization profile |
| Benchmark Ready | benchmark spec approved; run authorized (dataset pinned) |
| Certified | benchmark executed, success criteria met, report published |
| Certified Optimized | optimization values adopted — only benchmark-confirmed ones |
| Reference Model | Chief Architect canonical baseline for replay/regression |
| Deprecated | superseded; frozen + replay-only |

## 2. Deterministic Gates

Each transition is a pure boolean function, no reviewer discretion:

| Transition | Gate |
|------------|------|
| Draft → Research Complete | card sourced · matrix sourced · profile annotated |
| Research Complete → Benchmark Ready | spec approved · dataset pinned · replay id |
| Benchmark Ready → Certified | all success criteria · no failure criteria · report conformance |
| Certified → Certified Optimized | benchmark-confirmed optimizations only |
| Certified → Reference Model | Chief Architect designation |
| any → Deprecated | superseded notice + changelog |

## 3. Workflow (with the Benchmark-Certification contract)

```
Read gate (P10.3A) → spec approval → run (gated) → result schema → golden
   → scoring → Certified (or FAIL reason) → optimized (confirmed) 
   → reference-model choice, then perpetual replay.
```

## 4. DeepSeek Status Today

- Level: Research Complete (from P10.3A).
- Benchmark Ready is NOT reached — spec is Draft, intentionally unexecuted here
  (this phase builds infrastructure only).

## 5. Completeness Rule (Chief Architect exit)

A new provider can be certified without changing the framework:
populate existing datasets/categories, run the shared gates, publish a
conforming report — no new code or framework doc required.
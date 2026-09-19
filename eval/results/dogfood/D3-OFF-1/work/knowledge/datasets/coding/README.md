# Dataset: Coding (Code Generation · Bug Fix · Refactoring)

**Category folder**: `knowledge/datasets/coding/`
**Framework**: Benchmark & Certification · provider-neutral.

## Benchmark Spec

- **Purpose**: prove code proficiency — generate, repair, refactor without breaking behaviour.
- **Inputs**: prompt + language + constraints + deterministic seed + model.
- **Expected Behaviour**: compilable, correct output matching golden (tests pass).
- **Success Criteria**: accuracy ≥ threshold; output ≤ token budget; deterministic refactor preserves semantics.
- **Failure Criteria**: non-compiling output; changed behaviour in refactor; hallucinated API.
- **Required Metrics**: accuracy, token efficiency, determinism, cost_per_task.
- **Repeatability**: seed pinned; repeats ≥3; concurrency/budget declared.
- **Replay**: hashed goldens + verdicts across runs (replay_spec.md).

## Datasets

### cg-codetrics (CodeGen)

| Field | Value |
|-------|-------|
| ID | codegen-basic |
| Version | 1.0.0 |
| Purpose | generate idiomatic, correct functions from doc-style specs |
| Difficulty | medium |
| Expected Behaviour | compile + pass golden tests |
| Tags | [coding, codegen, compile] |

### bg-fix (Bug Fix)

| Field | Value |
|-------|-------|
| ID | bugfix-insert-keyerror |
| Version | 1.0.0 |
| Purpose | localize + correct a minimal seeded defect in short snippets |
| Difficulty | medium |
| Expected Behaviour | snippet compiled/audits and fix dedup test passes; regression-cases fixed |
| Tags | [coding, bugfix] |

### rf-refactor (Refactoring)

| Field | Value |
|-------|-------|
| ID | refactor-rename |
| Version | 1.0.0 |
| Purpose | rename/extract module without changing behaviour |
| Difficulty | hard |
| Expected Behaviour | identical I/O pre/post; tests green |
| Tags | [coding, refactoring, semantics] |

Sample entry file pattern: `<id>.json` per dataset in this folder (`README.md`
documents the manifest; goldens + prompts live next to it).
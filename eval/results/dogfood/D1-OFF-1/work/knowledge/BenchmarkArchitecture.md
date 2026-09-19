# Benchmark Architecture

**Phase**: P10.3B — Micro Benchmark & Certification Framework
**Status**: APPROVED TO IMPLEMENT → IMPLEMENTED (framework ONLY; no benchmark run)

## 1. Mission

Build the official benchmark and certification framework used for every
CodeBro Certified Model. This phase does NOT benchmark any provider. It builds
reusable, provider-neutral infrastructure.

## 2. Ownership Contract

### Benchmark Framework OWNS
- Benchmark Specifications
- Test Scenario Definitions
- Certification Workflow
- Result Schema
- Metric Definitions
- Scoring Rules
- Report Generator
- Replay Specification
- Dataset Versioning

### MUST NOT own
- Runtime logic
- Provider implementation
- API calls
- HTTP client / SDK
- API keys
- Model optimization
- Production configuration

## 3. Replay-First Design

Central idea: a benchmark is run ONCE (a gated "golden" run); afterwards the
runtime is validated by **replaying** the recorded run. Replay is a pure
offline comparison (verdict + metric drift) that consumes **zero tokens**.

```
benchmark (tokens) ──► golden replay record (hashed, immutable)
                              │
runtime change ──────────────► replay (offline, token-free) ─► verdict/drift
```

Re-run of a live benchmark happens ONLY on: provider model version change,
Optimization Profile change, or certification expiry.

## 4. Five Layers

| Layer | Artifact(s) | Responsibility |
|-------|-------------|----------------|
| Govern | `certification/benchmark_policy.md`, `certification_policy.md` | rules, gates, levels |
| Measure | `certification/metrics.md` | metric catalogue |
| Score | `certification/scoring.md` | normalized verdicts |
| Dataset | `datasets/*/README.md` + entries | generic prompts + goldens |
| Replay/Report | `certification/replay_spec.md`, `report_template.md` | token-free validation + output schema |

## 5. Generic-Provider Guarantee

- Datasets carry NO provider-specific prompts.
- Metric truth comes only from API `usage` (OpenAI-compatible) or runner
  instrumentation — never vendor SDK internals.
- Certifying a new provider never requires a framework change (gate in
  `certification_policy.md` §5).

## 6. Zero Space Compliance

- Zero runtime changes · zero provider implementation · zero API usage ·
  zero benchmark execution in this phase. Only documentation artifacts.

## 7. Reproducibility

`(dataset@version, seed, model, framework_version)` ⇒ deterministic verdict.
Reports conform to the Result Schema; goldens are hashed; every run pinned.
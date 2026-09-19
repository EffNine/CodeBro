# Scoring Specification

**Phase**: P10.3B · canonical scoring model (companion to `certification/scoring.md`).

## 1. Design

- Metrics are measured raw; scoring converts to `[0,1]` verdicts.
- Pure function of `(dataset@version, seed, model, metric values)`.

## 2. Metric Set (normalized 0..1)

| Metric | Formula |
|--------|---------|
| Accuracy | correct / total |
| Latency | `min(1, budget/actual)` |
| Token Efficiency | `min(1, budget_tokens/tokens)` |
| Cost Efficiency | `clamp(1, ceiling/cost)` |
| Determinism | agreement across repeats |
| Structured Compliance | schema-valid / total |
| Tool Calling Success | schema-conforming calls / total |
| Streaming Quality | ordered-complete deltas / total |
| Prompt Cache Effectiveness | hit/(hit+miss) |

Full catalogue with API-field provenance in `certification/metrics.md`.

## 3. Aggregation

```
category_score = Σ(weightᵢ·metricᵢ)/Σ(weightᵢ)
overall        = mean(category scores)
```

Verdict: PASS iff `overall ≥ threshold` (default 0.85) AND every MANDATORY
metric ≥ its threshold AND no failure criteria. Otherwise FAIL (with reason).

## 4. Weighting (defaults)

| Category | Weights |
|----------|---------|
| Coding | Accuracy .5 · TokenEf .2 · Cost .15 · Determinism .15 |
| Tools | ToolSuccess .6 · Compliance .4 |
| Structured/JSON | Compliance .7 · Determinism .3 |
| Streaming | Quality .5 · Latency .3 · Reliability .2 |
| Context | Accuracy .5 · Reliability .5 |
| Cache | CacheEffect .7 · Cost .3 |

Override requires an approved provider spec — never during a live run.

## 5. Determinism & Replay

- Seed fixed; pipeline pure; report diffable exactly.
- A live-run override of any scoring parameter invalidates the run.
- Replay recomputes the verdict offline from recorded metrics (no tokens).
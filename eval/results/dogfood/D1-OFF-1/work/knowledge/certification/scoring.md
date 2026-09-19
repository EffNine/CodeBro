# Scoring Specification

**Framework**: Benchmark & Certification · defines how raw metrics become a pass/fail verdict.

## 1. Principles

- Metrics are raw measurements; scoring is the transformation into verdicts.
- Determinism: same (dataset, model, run) ⇒ same verdict (fixed seed).
- No hidden weights; scoring rules are explicit and versioned.

## 2. Metric Vocabulary (normalized to 0..1)

Every metric normalizes to `[0,1]`, higher = better, before weighting.

| Metric | Definition | Computation (normalized) |
|--------|-----------|--------------------------|
| Accuracy | task correctness vs golden | correct/total |
| Latency | response time budget | `sat_budget = min(1, budget/t_actual)` clamp |
| Token Efficiency | output/task under token budget | `min(1, budget_tokens/tokens)` |
| Cost Efficiency | $/task under ceiling | `clamp(1, ceiling/cost)` |
| Determinism | agreement across repeats | repeat-agreement ratio |
| Structured Output Compliance | schema-valid outputs | `valid/total` |
| Tool Calling Success | schema-conforming tool calls | `successful/total` |
| Streaming Quality | completion + order + keep-alive | composite (per metric def) |
| Prompt Cache Effectiveness | cache-hit coverage of input | `prompt_cache_hit_tokens / prompt_tokens` |

## 3. Pass / Fail Model

For each benchmark a set of **cardinal metrics** is tagged `MANDATORY`:

- Each mandatory metric must meet its `threshold` (from the benchmark spec).
- `score = Σ(weightᵢ · metricᵢ) / Σ(weightᵢ)`
- Verdict:
  - **PASS** iff `score ≥ threshold_score` AND every mandatory metric ≥ its threshold AND failure criteria absent.
  - **FAIL** otherwise, with a reason citing the offending metric.

Thresholds are declared in the provider benchmark spec, never in this framework.

## 4. Weighting Rules

- Defaults per category (override only via approved spec):
  - Coding: Accuracy .5 · Token Efficiency .2 · Cost .15 · Determinism .15
  - Tools: ToolCall Success .6 · Compliance .4
  - Structured/JSON: Compliance .7 · Determinism .3
  - Streaming: Quality .5 · Latency .3 · Reliability .2
  - Context: Accuracy .5 · Reliability .5
  - Cache: CacheEffectiveness .7 · Cost .3
- A failed `MANDATORY` metric ⇒ overall FAIL regardless of total (hard gate).

## 5. Scoring Aggregation

Within category → across categories → overall:

```
category_score = weighted mean of its tests
overall        = mean of category scores (or provider-policy weighted mean)
```

For Certify, a provider must reach `overall ≥ CERTIFY_THRESHOLD` (default 0.85,
overridable per provider but never per-run during a live run).

## 6. Determinism

- seed recorded; sample seed; metric pipeline pure.
- Any non-deterministic scoring API is a bug.

## 7. Output

Each test emits `{verdict, score, mandatory_results, metric_values, seed, dataset_version, model, replay_id}` that conforms to Result Schema in `report_template.md`.
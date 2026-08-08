# Report Generator — Template (Result Schema)

**Framework**: Benchmark & Certification · the schema every report and replay record must conform to.

## 1. Result Schema (canonical)

```json
{
  "schema": "codebro.benchmark.result/v1",
  "report_id": "uuid",
  "framework_version": "P10.3B",
  "provider_id": "deepseek",
  "model": {
    "id": "deepseek-v4-pro",
    "version": null
  },
  "dataset": { "id": "coding-gen", "version": "1.2.0" },
  "benchmark_spec": { "id": "spec-...", "version": "1" },
  "seed": 42,
  "run": {
    "date": "2026-08-07",
    "concurrency": null,
    "repeats": 3,
    "valid": true
  },
  "metrics": {
    "accuracy": 0.95,
    "cache_hit_rate": 0.6
  },
  "scoring": {
    "overall": 0.91,
    "threshold": 0.85,
    "verdict": "PASS"
  },
  "mandatory": [
    { "metric": "accuracy", "threshold": 0.8, "result": 0.95, "ok": true }
  ],
  "usage": {
    "prompt_tokens": 1000,
    "completion_tokens": 500,
    "prompt_cache_hit_tokens": 600,
    "prompt_cache_miss_tokens": 400
  },
  "replay": {
    "replay_id": "uuid",
    "golden": "sha256:...",
    "drift": 0.03
  }
}
```

## 2. Mandatory Sections

Any valid report MUST include: identify (provider/model/dataset@version) ·
seed · metrics · scoring(verdict) · mandatory results · usage · replay binding.
Missing sections ⇒ report is invalid.

## 3. Reproducibility

- Same dataset@version + seed + model + framework ⇒ identical verdict.
- `report_id` derived from the run, not reused.
- Reports are append-only; corrections bump `dataset`/`benchmark_spec` versions.

## 4. Report Generator

- Inputs: result JSON (Replay pass), benchmarks. Spec + scoring (rule based).
- Output: Markdown summary (list format for human admith a report) + JSON (data).
- Generator is deterministic; outputs can be diffed exactly between runs.

## 5. Checklist of Conformance

A report conforms if it (1) includes schema `brand.benchmark/v1`,
(2) has deterministic verdict, (3) pins a dataset version,
(4) is bound to a replay golden, (5) contains only recorded usage.
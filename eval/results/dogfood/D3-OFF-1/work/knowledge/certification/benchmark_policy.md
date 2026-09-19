# Benchmark Policy

**Framework**: Benchmark & Certification · **Scope**: governs how benchmarks are
defined, run, and replayed. **Phase**: P10.3B (definition-only; nothing runs).

## 1. Purpose

Benchmarking is the enforcement of "Benchmark before Optimize". A benchmark
must be definitionally complete (this Framework) and then provider-scoped.
DeepSeek-specific benchmark behavior belongs in
`knowledge/providers/deepseek/benchmark_spec.md` and is created under the
workflow in `ResearchWorkflow.md`. This class = protocol.

## 2. Benchmark Once, Replay Forever

The framework is **replay-first**. A benchmark is run ONCE within a gated
Certified window. Re-runs happen only when:

- the provider model version changes,
- the Optimization Profile changes,
- the Certification expires.

Replay validates a current runtime against a recorded (golden) run WITHOUT
spending tokens. See `replay_spec.md`.

## 3. Benchmark Categories

Each category must be mapped into the eight dataset folders:

| # | Category | Dataset folder | Focus |
|---|----------|----------------|-------|
| 1 | Code Generation | `coding/` | generate correct, idiomatic code |
| 2 | Bug Fix | `coding/` | locate + fix a defect |
| 3 | Refactoring | `coding/` | behaviour-preserving restructure |
| 4 | Tool Calling | `tools/` | schema-conforming tool invocation |
| 5 | Streaming | `streaming/` | incremental output delivery |
| 6 | Structured Output | `structured_output/` | schema-enforced output |
| 7 | JSON Output | `json/` | valid-JSON emission |
| 8 | Prompt Cache | `prompt_cache/` | prefix reuse / cache effectiveness |
| 9 | Context Handling | `long_context/` | instructions across context |
| 10 | Long Context | `long_context/` | scale correctness at long windows |
| 11 | Latency | (measured, all) | TTFT / tokens/s |
| 12 | Token Usage | (measured, all) | efficiency |
| 13 | Cost | (measured, all) | $/task |
| 14 | Reliability | (measured, all) | success rate, error handling |
| 15 | Retry Behaviour | (measured, all) | interplay with runtime retry |
| Category (11–15) are cross-cutting measured across Scenario / Cost metrics, not separate datasets. |

## 4. Required Sections of EVERY Benchmark

A benchmark MUST define all:

- **Purpose** — what it proves
- **Inputs** — prompt fixture + deterministic seed + model-under-test
- **Expected Behaviour** — the reference/golden behaviour
- **Success Criteria** — how it's judged pass/fail
- **Failure Criteria** — cardinal rules that alone fail the run
- **Required Metrics** — which metrics are recorded (from `metrics.md`)
- **Repeatability Rules** — repeats, seed, concurrency, budget
- **Replay Support** — the replay record identity (from `replay_spec.md`)

## 6. Rules

- 1. No dataset MAY contain provider-specific prompts (universal neutrality).
- 2. Never spend tokens just to re-derive recorded behaviour (replay).
- 3. Only official, documented features are exercised.
- 4. A benchmark without its Benchmark Spec cannot run.
- 5. Each run's result record conforms to the Result Schema (`report_template.md`).

## Temporal

- benchmark_spec: created Draft → approved by Chief Architect → executed → superseded.
- Review the table of contents whenever a provider is version-expensive (e.g.
  DeepSeek v4-pro effort levels — see `knowledge/providers/deepseek/benchmark_spec.md`).
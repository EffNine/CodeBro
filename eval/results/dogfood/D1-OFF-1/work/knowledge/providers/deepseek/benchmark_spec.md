# Benchmark Specification — DeepSeek

> CONTRACT. This spec is NOT approved and NO benchmark has been executed.
> Per Policy: Read → Benchmark → Optimize → Certify. Research (this phase)
> is complete; this spec is the candidate contract for the NEXT phase.

**Provider**: DeepSeek
**Status of spec**: Draft (not approved, not executed)

## 1. Purpose

Prove the optimization-profile claims for CodeBro adoption before anything is
moved to Certified. Targets, in priority order:
1. Context cache hit behavior (prompt_cache_hit_tokens under reuse).
2. JSON/structured output reliability (incl. empty-content rate).
3. Tool calling fidelity incl. strict mode; thinking-mode tool round trips.
4. FIM infill quality (non-thinking, ≤4K output).
5. Streaming behavior (SSE, keep-alive, TTFT).

## 2. Test Matrix (candidate)

| Test id | Capability | Metric | Pass criterion | Candidate model(s) |
|---------|-----------|--------|----------------|--------------------|
| T-01 | Prompt Cache | `prompt_cache_hit_tokens` / total input | ≥ 60% hit on repeated stable prefix (first-use miss, repeat hit) | deepseek-v4-pro, deepseek-v4-flash |
| T-02 | JSON Mode | valid-JSON rate on schema-typed calls | ≥ 99% valid JSON, ≤ 1% empty content | both |
| T-03 | Structured Output (strict) | schema violation rate | 0 schema violations over n=200 | both |
| T-04 | FIM (beta) | infill correctness (edit-distance / manual) | baseline vs vendor claim on curated set | both, non-thinking |
| T-05 | Tool Calling | round-trip success + arg schema adherence | ≥ 98% round trips; 0 malformed args | both |
| T-06 | Thinking+Tools | reasoning_content round-trip; no HTTP 400 | 0 HTTP 400 over n=100 multi-turn | both |
| T-07 | Streaming | TTFT (p50/p95), tokens/s, keep-alive parse | within budget thresholds (to be set) | both |
| T-08 | Long Context | correctness at 64K / 256K / 1M | documented degradation curve; no 400 in-window | both |

## 3. Environment

- Benchmark runner: codebro `benchmarks/` + Commit CI (NOT in this phase)
- Deterministic seed: pinned per run
- Repeat count: n per matrix row
- Concurrency profile: at or below concurrency ceiling (500 pro / 2500 flash)
- Budget: declared in Cost Ceiling below

## 4. Cost Ceiling (research estimate)

| Input class | Composition | est. tokens/req | est. cost/req (flash) |
|-------------|-------------|------------------|------------------------|
| cache miss | fresh, no prefix reuse | 64K input + 1K output | 64×$0.14 + $0.28 ≈ $9.24 |
| cache hit | stable 60K prefix + 4K new | 60K hit + 4K miss | 60×$0.0028 + 4×$0.14 + out ≈ $0.79 |

Estimate uses Models & Pricing rates ($0.14 miss / $0.0028 hit / $0.28 output per 1M, flash).
Researcher MUST set a total cap before the benchmark phase begins.

## 5. Controls

- No credentials committed (env-only keys).
- Results recorded to the Research Index (see ResearchWorkflow.md).
- Only official documented features exercised; no undocumented knobs.

## 6. Status History

| Date | Version | Status | Notes |
|------|---------|--------|-------|
| 2026-08-07 | v1 | Draft | Derived from research; NOT approved, NOT executed |
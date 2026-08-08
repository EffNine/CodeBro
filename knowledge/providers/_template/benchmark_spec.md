# Benchmark Specification — {PROVIDER_NAME}

> Template. The benchmark spec is a CONTRACT. It is approved BEFORE any
> benchmark runs. Research comes first; only a provider with status
> "Research Complete" may have this spec filled and a benchmark executed.

**Provider**: {PROVIDER_NAME}
**Status of spec**: Draft · Approved · Executed · Superseded

## 1. Purpose

What the benchmark must prove for this provider. (e.g. latency, TTFT, tool
call fidelity, JSON output reliability, cache hit rate).

## 2. Test Matrix

| Test id | Capability | Metric | Pass criterion | Candidate model(s) |
|---------|-----------|--------|----------------|--------------------|
| {T-01} | {Streaming} | {TTFT / tokens/s} | {threshold} | {model} |

## 3. Environment

- Benchmark runner: {Name/CI job}
- Deterministic seed: {seed}
- Repeat count: {n}
- Concurrency profile: {profile}
- Budget: {max tokens / max requests / max cost}

## 4. Cost Ceiling

| Input class | Composition | Estimated tokens | Estimated cost |
|-------------|-------------|------------------|----------------|
| {cache hit} | {pct prefix} | {tokens} | {cost} |

## 5. Controls

- No credentials committed.
- Results recorded to the research index.
- Only official documented features are exercised.

## 6. Status History

| Date | Version | Status | Notes |
|------|---------|--------|-------|
| {date} | v1 | Draft | Created from research |
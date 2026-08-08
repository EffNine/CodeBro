# Dataset: Prompt Cache

**Category folder**: `datasets/prompt_cache/` · provider-neutral.

## Benchmark

- **Purpose**: prove prefix-cache effectiveness — hit rate, cost effect, and
  repeat-token stability.
- **Inputs**: a stable prefix (system prompt + shared context) + varying task tail.
- **Expected Behaviour**: second request reuses the prefix; cache-hit tokens
  reported in `usage.prompt_cache_hit_tokens` (if the provider exposes it).
- **Success**: cache_hit_rate ≥ threshold on repeat; cost savings materialize.
- **Failure**: zero hits despite identical prefix; hit rate falls after a
  delay (cache eviction) without explanation.
- **Mandatory**: cache_hit_rate, cost_per_task, reliability.
- **Replay**: replay works by comparing recorded `usage` snapshots — no tokens.

## Datasets

| ID | Version | Purpose | Difficulty | Expected | Tags |
|----|---------|---------|-----------|----------|------|
| cache-stable-prefix | 1.0.0 | same prefix across 5 turns | medium | hit tokens grow; then stable | [cache, prefix] |
| cache-churn | 1.0.0 | interleaved volatile tails | hard | prefix still hits; tail misses | [cache, churn] |
| cache-long-delay | 1.0.0 | re-request after eviction window | hard | documented miss (not failure) | [cache, eviction] |

Rules: dataset never names a provider; providers report `usage.cache_*` fields
only if documented (else metric = n/a).
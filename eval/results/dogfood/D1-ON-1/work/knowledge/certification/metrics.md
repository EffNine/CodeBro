# Metrics Definition

**Framework**: Benchmark & Certification · canonical definitions used by scoring.md.

## 1. Metric Definition Record

Every metric is defined by:
`name · unit · definition · source-of-truth (API field) · budget/threshold · deterministic?`

## 2. Metric Catalogue

| Name | Unit | Definition | API field (OpenAI-compat) | Notes |
|------|------|------------|---------------------------|-------|
| `accuracy` | ratio | correct / total (vs golden) | — (judged) | golden-based |
| `ttft` | ms | time to first output token | measured | streaming only |
| `tokens_per_sec` | tok/s | completion / wall time | measured | streaming |
| `latency_p50/p95` | ms | completion latency percentiles | measured | over repeats |
| `prompt_tokens` | tokens | input tokens | `usage.prompt_tokens` | from API |
| `completion_tokens` | tokens | output tokens | `usage.completion_tokens` | |
| `total_tokens` | tokens | input+output | `usage.total_tokens` | |
| `prompt_cache_hit_tokens` | tokens | input served from cache | `usage.prompt_cache_hit_tokens` | DeepSeek-specific field, used when present |
| `prompt_cache_miss_tokens` | tokens | input NOT from cache | `usage.prompt_cache_miss_tokens` | |
| `cache_hit_rate` | ratio | hit/(hit+miss) | derived | normalization per scoring |
| `cost_per_task` | $ | price(model)·tokens | derived from pricing model | static pricing map per provider |
| `structured_valid` | ratio | schema-valid outputs/total | parsed from content | validation offline |
| `tool_success` | ratio | successful tool invocations/total | `tool_calls` presence | offline |
| `streaming_quality` | ratio | ordered complete deltas w/o keep-alive break | delta sequence | offline |
| `determinism` | ratio | identical outputs across repeats | outputs | seed fixed |
| `reliability` | ratio | non-error runs / total | error code absent | HTTP 2xx |
| `retry_behavior` | categorical | respected backoff / no dup calls | runtime logs | replayed |

## 3. Metric Truth Rule

- Token counts come ONLY from the API `usage` block.
- Latency comes ONLY from the runner's instrumentation.
- Pass/fail never mixes: `usage` fields never double as timing metrics.

## 4. Missing Fields

If an API omits a usage field (e.g. no cache fields), mark that metric `n/a` and
exclude from scoring; do NOT fabricate.

## 5. Metric Families (for scoring weights)

- Quality: accuracy, structured_valid, tool_success, streaming_quality
- Efficiency: tokens_per_sec, cost_per_task, token budgets
- Reliability: reliability, determinism, retry_behavior
- Cache: cache_hit_rate, prompt_cache_*tokens
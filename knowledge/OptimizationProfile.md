# Optimization Profile — DeepSeek (Deliverable Compile)

**Phase**: P10.3A · **Status**: Research Complete (research only; NO benchmark run)

> Compile of `knowledge/providers/deepseek/optimization_profile.yaml`.
> All entries carry value/confidence/source/status. Adoption into runtime is
> forbidden until a benchmark confirms each `Benchmark Required` value
> (per ProviderResearchPolicy.md).

| # | Dimension | Value (condensed) | Confidence | Status |
|---|-----------|-------------------|------------|--------|
| 1 | CONTEXT_CACHE | Prefix caching automatic; 64-token unit; hit ≪ miss | HIGH | Documentation |
| 2 | CONTEXT_CACHE_BILLING | flash hit $0.0028 vs miss $0.14 (≈50x); pro $0.003625 vs $0.435 (≈120x) per 1M input | HIGH | Documentation |
| 3 | THINKING_MODE_TOKEN_COST | Thinking CoT adds `reasoning_content` tokens; disable thinking on chat-class work | MEDIUM | Hypothesis |
| 4 | REASONING_EFFORT | flash: low/high/max; pro today only high/max (fix expected early Aug 2026) | HIGH | Documentation |
| 5 | JSON_OUTPUT | JSON mode needs `json` word + example prompt; may return empty content | HIGH | Documentation |
| 6 | TOOL_STRICT_SCHEMA | strict tools on /beta enforce JSON-Schema (8 types + $ref/$def) | HIGH | Documentation |
| 7 | TOOL_CALL_REASONING_CONTEXT | Thinking + tools: must return `reasoning_content` each turn else 400 | HIGH | Documentation |
| 8 | RESPONSES_API | Responses API flash-only today; pro expected early Aug 2026 | MEDIUM | Benchmark Required |
| 9 | FIM | /beta, non-thinking only, 4K cap — verify infill quality | MEDIUM | Benchmark Required |
| 10 | RATE_LIMIT | 500 pro / 2500 flash concurrent per account; 429 on excess | HIGH | Documentation |
| 11 | EMBEDDINGS_ABSENT | No official embeddings — plan embeddings elsewhere | MEDIUM | Hypothesis |
| 12 | TEMPERATURE_EFFECT_NULL | Thinking mode: temperature/penalties accepted but inert | HIGH | Documentation |

**Status counts**: Documentation 8 · Hypothesis 2 · Benchmark Required 2.

**Not adopted**: none of these values may be placed in provider plugin
configuration before certification. Canonical artifact:
`knowledge/providers/deepseek/optimization_profile.yaml`.
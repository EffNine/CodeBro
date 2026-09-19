# Capability Matrix — DeepSeek

**Provider**: DeepSeek
**Status**: Research Complete (progress; benchmark application not in this phase)
**Last verified**: 2026-08-07

## Legend

- **Supported** — provider exposes it.
- **Not supported** — provider states/does not expose it.
- **Not documented** — no official statement found; treated as unsupported.
- **Beta** — gated behind `/beta` endpoint or limited surface.

## Matrix

| # | Capability | deepseek-v4-flash | deepseek-v4-pro | Status | Notes | Source |
|---|------------|-------------------|-----------------|--------|-------|--------|
| 1 | Streaming | ✓ | ✓ | Supported | `stream:true` → SSE stream | api-docs.deepseek.com (First Call); Create Chat Completion |
| 2 | Tool Calling | ✓ | ✓ | Supported | function calling, incl. thinking mode; `strict` (Beta) on `/beta` | guides/tool_calls |
| 3 | Structured Output | ✓ | ✓ | Supported (Beta w/ strict) | `response_format: json_object`; JSON-Schema `strict` on /beta | guides/json_mode; guides/tool_calls |
| 4 | JSON Mode | ✓ | ✓ | Supported | requires `json` word + example in prompt; may emit empty content | guides/json_mode |
| 5 | Reasoning | ✓ | ✓ | Supported | CoT in `reasoning_content` | guides/thinking_mode |
| 6 | Thinking Mode | ✓ | ✓ | Supported (default) | toggle `thinking disabled`; effort `low/high/max` | guides/thinking_mode; Models & Pricing |
| 7 | FIM | ✓ (non-thinking only) | ✓ (non-thinking only) | Beta | `/beta`, `POST /completions`, max 4K output | guides/fim_completion; Models & Pricing |
| 8 | Embeddings | — | — | Not documented | No embeddings endpoint in official docs/pricing | api-docs (no embeddings product) |
| 9 | Long Context | 1M | 1M | Supported | CONTEXT LENGTH 1M; over limit → HTTP 400 | Models & Pricing; guides/responses_api |
| 10 | Prompt Cache | ✓ | ✓ | Supported (auto) | default on; `prompt_cache_hit_tokens`/`miss` | guides/kv_cache |
| 11 | Cost Model | see Optimization Profile | | Supported | cache hit ~/ 50x cheaper than miss | Models & Pricing |
| 12 | Rate Limits | 2500 concurrent | 500 concurrent | Supported | per-account; over → HTTP 429; 10-min pre-inference drop | quick_start/rate_limit |
| 13 | Known Limitations | see below | | | — | guides/json_mode; thinking_mode; responses_api |

## Capability Notes

**Tool Calling (2).** Suggested for the agent path. Thinking mode requires
`reasoning_content` passed back on every subsequent turn or a HTTP 400 is
returned. `strict` mode requires `base_url=…/beta`, `strict:true` on every
function, and JSON-Schema validation (any unsupported type → error).
Source: guides/tool_calls; guides/thinking_mode.

**Structured Output (3).** Two distinct mechanisms: free-form JSON via
`response_format:{"type":"json_object"}` and schema-enforced `strict` tool
schema on `/beta`. JSON Mode has a documented empty-content failure mode.
Source: guides/json_mode; guides/tool_calls.

**Embeddings (8).** The official API surface exposes `/chat/completions`,
`/completions` (beta/FIM), `/models`, and a Responses API and Anthropic
interface. No embeddings product is documented; Model & Pricing lists no
embedding models. Treated as **Not documented**.
Source: api-docs; Models & Pricing.

**FIM (7).** Beta, non-thinking only, output capped at 4K. Requires `/beta`
base_url. Source: guides/fim_completion; Models & Pricing.

**Long Context (9).** 1M context on both current models. Requests exceeding
context window return HTTP 400. Thinking mode caps reasoning cost via
effort levels. Source: Models & Pricing; guides/responses_api.

**Prompt Cache (10).** Prefix-only cache, auto by default, 64-token unit.
No guarantee of hits; cache cleared within hours–days of inactivity.
Source: guides/kv_cache; news/news0802.

## Compliance

- Research-only: yes · No benchmark executed: yes
- Every matrix cell carries a source or an explicit "Not documented": yes
# Provider Card — DeepSeek

**Phase**: P10.3A — Provider Research Platform
**Provider**: DeepSeek
**Status**: Research Complete (Benchmark NOT executed — out of scope for this phase)

> This is the DeepSeek research card. Every statement is a FACT carrying an
> official source. No optimization, no benchmarking, no implementation in this
> artifact.

## 1. Identity

| Field | Value |
|-------|-------|
| Vendor | DeepSeek (Hangzhou DeepSeek Artificial Intelligence Basic Technology Research Co., Ltd.) |
| Provider ID | `deepseek` |
| Source of truth | https://api-docs.deepseek.com |
| SDK / clients | OpenAI SDK (via base_url), Anthropic SDK (via base_url); official example scripts |
| Auth model | API key (Bearer `Authorization: Bearer ${DEEPSEEK_API_KEY}`) |
| Base URL (OpenAI format) | `https://api.deepseek.com` |
| Base URL (Anthropic format) | `https://api.deepseek.com/anthropic` |
| Beta endpoint | `https://api.deepseek.com/beta` (Chat Prefix Completion, FIM, strict mode) |
| First researched | 2026-08-07 |
| Last verified | 2026-08-07 |

Sources: api-docs.deepseek.com (Your First API Call), api-docs.deepseek.com (Models & Pricing).

## 2. Models

| Model ID | Type | Thinking mode | Context | Max output | Status |
|----------|------|---------------|---------|-----------|--------|
| `deepseek-v4-pro` | chat | yes (both, default thinking) | 1M | max 384K | GA |
| `deepseek-v4-flash` (DeepSeek-V4-Flash-0731) | chat | yes (both, default thinking) | 1M | max 384K | GA |

Legacy aliases (transitional only, NOT for new use):

| Model ID | Meaning | Retirement |
|----------|---------|------------|
| `deepseek-chat` | non-thinking mode of current flash | Retired 2026-07-24 |
| `deepseek-reasoner` | thinking mode of current flash | Retired 2026-07-24 |

Sources: api-docs.deepseek.com (Your First API Call — model list table with `deepseek-v4-flash`/`deepseek-v4-pro`), api-docs.deepseek.com/quick_start/pricing (CONTEXT LENGTH 1M, MAX OUTPUT 384K; FEATURES incl. Responses API flash-only), api-docs.deepseek.com/news/news260424 (V4 release; legacy names retiring after Jul 24 2026), api-docs.deepseek.com/updates (legacy naming deprecation; "upgraded to DeepSeek-V3.2").

## 3. Facts (with sources)

| # | Fact | Source |
|---|------|--------|
| 1 | API format is compatible with OpenAI and Anthropic; can use OpenAI/Anthropic SDK by changing base_url and model. | Your First API Call |
| 2 | Supported model IDs are `deepseek-v4-flash` and `deepseek-v4-pro`. | Your First API Call; Create Chat Completion reference |
| 3 | Both models support 1M token context and dual Thinking / Non-Thinking mode. | news/news260424; Models & Pricing |
| 4 | Max output tokens is 384K (upper bound); output controlled by `max_tokens` / `max_completion_tokens`. | Models & Pricing |
| 5 | Thinking mode is enabled by default, default effort `high`. | guides/thinking_mode |
| 6 | Thinking mode ignores `temperature`, `top_p`, `presence_penalty`, `frequency_penalty` (no error, no effect). | guides/thinking_mode |
| 7 | Chain-of-thought is returned in `reasoning_content` (parallel to `content`). | guides/thinking_mode |
| 8 | In thinking mode, `reasoning_content` from prior non-tool turns is ignored by the API; with tool calls it MUST be passed back (else HTTP 400). | guides/thinking_mode · Tool Calls |
| 9 | JSON Output enabled via `response_format: {"type":"json_object"}` AND including the word "json" + example in prompt. | guides/json_mode |
| 10 | JSON Output may occasionally return empty content; mitigated by prompt; pending vendor optimization. | guides/json_mode |
| 11 | Tool Calls (function calling) supported in both non-thinking and thinking mode; `strict` mode (beta) available on `/beta`. | guides/tool_calls |
| 12 | `strict` mode supports JSON Schema types: object, string, number, integer, boolean, array, enum, anyOf, plus $ref/$def. | guides/tool_calls |
| 13 | Context Caching on Disk is enabled by default, no code change; billed only on actual cache hits. | guides/kv_cache; news/news0802 |
| 14 | Usage reports `prompt_cache_hit_tokens` and `prompt_cache_miss_tokens`. | guides/kv_cache; Create Chat Completion reference |
| 15 | Cache hit requires identical prefix from token index 0; partial/middle matches do not hit. | guides/kv_cache; news/news0802 |
| 16 | Cache is 64-token storage unit; content < 64 tokens not cached; no 100% hit guarantee; storage free. | news/news0802 |
| 17 | FIM (Fill-in-the-Middle) available on `/beta` via `POST /completions`; max 4K output; non-thinking mode only. | guides/fim_completion; Models & Pricing |
| 18 | Chat Prefix Completion (Beta) on `/beta`: set `prefix:true` on final assistant message. | guides/chat_prefix_completion |
| 19 | Responses API: currently flash only; `deepseek-v4-pro` support expected early August 2026. | Models & Pricing; guides/responses_api |
| 20 | Stateless API: server does not record context; client must resend full conversation. | guides/multi_round_chat |
| 21 | Concurrency limits per account: `deepseek-v4-flash` 2500, `deepseek-v4-pro` 500; exceeding → HTTP 429. | quick_start/rate_limit; Models & Pricing |
| 22 | Requests exceeding context window return HTTP 400. | guides/responses_api |
| 23 | No embeddings API / endpoint is documented by DeepSeek API docs. | api-docs lists only chat/completions + completions; no embeddings product |
| 24 | Thinking mode supports tool calls from DeepSeek-V3.2 onward. | guides/tool_calls (Thinking Mode) |

## 4. Sources Used

| Source | Kind | URL |
|--------|------|-----|
| Your First API Call | docs | https://api-docs.deepseek.com/ |
| Models & Pricing | docs / pricing | https://api-docs.deepseek.com/quick_start/pricing |
| Create Chat Completion | api reference | https://api-docs.deepseek.com/api/create-chat-completion |
| Thinking Mode | guide | https://api-docs.deepseek.com/guides/thinking_mode |
| Tool Calls | guide | https://api-docs.deepseek.com/guides/tool_calls |
| JSON Output | guide | https://api-docs.deepseek.com/guides/json_mode |
| Context Caching | guide | https://api-docs.deepseek.com/guides/kv_cache |
| FIM Completion | guide | https://api-docs.deepseek.com/guides/fim_completion |
| Chat Prefix Completion | guide | https://api-docs.deepseek.com/guides/chat_prefix_completion |
| Using the Responses API | guide | https://api-docs.deepseek.com/guides/responses_api |
| Multi-round Conversation | guide | https://api-docs.deepseek.com/guides/multi_round_chat |
| Rate Limit & Isolation | docs | https://api-docs.deepseek.com/quick_start/rate_limit |
| Change Log | release notes | https://api-docs.deepseek.com/updates/ |
| DeepSeek V4 Preview Release | release notes | https://api-docs.deepseek.com/news/news260424 |
| Context Caching introduction | release notes | https://api-docs.deepseek.com/news/news0802 |

## 5. Research Reproducibility

- Research date(s): 2026-08-07
- Research method: read official docs → confirm contract against API reference → record facts with source
- Re-verification frequency: suggested monthly, and immediately after a `Change Log` release notice
- Snapshot note: model IDs `deepseek-v4-*` current as of 2026-08-07; legacy `deepseek-chat`/`deepseek-reasoner` retired 2026-07-24.
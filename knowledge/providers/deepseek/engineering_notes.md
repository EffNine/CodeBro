# Engineering Notes — DeepSeek

**Provider**: DeepSeek
**Owner**: Platform (unassigned)

> Provisional notes from the research phase. None of these upgrade a
> capability to Certified until a benchmark confirms them.

## Open Questions

- v4-pro full effort-level support promised "early August 2026" — re-verify after that window (currently only high/max). Source: api/create-chat-completion. Hypothesis: HIGH re-verify priority.
- Whether the Responses API (flash-only today) is worth adopting vs Chat Completions once v4-pro lands. Source: guides/responses_api. Hypothesis: MEDIUM.
- Behavior of `temperature` in NON-thinking mode is normal; only thinking mode nullifies it. Confirm by test in Benchmark. Status: Hypothesis.

## Observations

- Prefix ordering matters more than content: cache hit is prefix-token based, so keep the stable preamble (system prompt, tool schemas, shared repo context) FIRST and volatile content LAST. Source: guides/kv_cache, news/news0802.
- JSON strict (tool schema) and JSON output are two different guarantees; don't confuse them during benchmarking. Source: guides/tool_calls, guides/json_mode.

## Caveats

- Legacy `deepseek-chat`/`deepseek-reasoner` removed 2026-07-24; a plugin must not hardcode those IDs.
- Over-context returns 400 (not truncated); respect `has_thought`/context accounting.
- No embeddings: do not advertise/learn an embedding capability for DeepSeek.
- Keep-alive empty-lines/SSE comments must be tolerated by any client parsing layer.
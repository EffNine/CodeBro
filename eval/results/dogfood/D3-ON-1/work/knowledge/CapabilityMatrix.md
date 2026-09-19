# Capability Matrix — DeepSeek (Deliverable Compile)

**Phase**: P10.3A · **Status**: Research Complete (research only)
**Last verified**: 2026-08-07

> Compile of `knowledge/providers/deepseek/capability_matrix.md`. Values:
> Supported / Not supported / Not documented / Beta.

| # | Capability | v4-flash | v4-pro | Status | Source |
|---|------------|----------|--------|--------|--------|
| 1 | Streaming | ✓ | ✓ | Supported | api-docs (First Call) |
| 2 | Tool Calling | ✓ | ✓ | Supported (+ strict Beta) | guides/tool_calls |
| 3 | Structured Output | ✓ | ✓ | Supported (strict Beta) | json_mode, tool_calls |
| 4 | JSON Mode | ✓ | ✓ | Supported | guides/json_mode |
| 5 | Reasoning | ✓ | ✓ | Supported | guides/thinking_mode |
| 6 | Thinking Mode | ✓ | ✓ | Supported (default) | thinking_mode, pricing |
| 7 | FIM | ✓ non-thinking | ✓ non-thinking | Beta (≤4K out) | fim_completion, pricing |
| 8 | Embeddings | — | — | Not documented | api-docs surface |
| 9 | Long Context | 1M | 1M | Supported | pricing, responses_api |
| 10 | Prompt Cache | ✓ | ✓ | Supported (auto) | guides/kv_cache |
| 11 | Cost Model | ✓ | ✓ | Supported | Models & Pricing |
| 12 | Rate Limits | 2500 | 500 concurrent | Supported | rate_limit, pricing |
| 13 | Known Limitations | &dagger; | &dagger; | — | json_mode, thinking_mode |

### Limitations (1.3)

- JSON Output may return empty content occasionally (mitigate via prompt) (json_mode).
- Thinking mode nulls `temperature`/`top_p`/penalties (thinking_mode).
- Over-context → HTTP 400; stateless API (responses_api, multi_round_chat).
- No embeddings (product surface).

Full notes, per-capability prose, and the embed of every source are in the
canonical artifact `knowledge/providers/deepseek/capability_matrix.md`.
# DeepSeek Provider Card

**Phase**: P10.3A — Provider Research Platform
**Status**: Research Complete (research only)

> Deliverable compile of `knowledge/providers/deepseek/provider_card.md`.
> The canonical artifact lives in that file; this page summarizes the card for
> Chief Architect review.

## Identity

| Field | Value |
|-------|-------|
| Provider ID | `deepseek` |
| Source of truth | https://api-docs.deepseek.com |
| Base URL (OpenAI format) | `https://api.deepseek.com` |
| Base URL (Anthropic format) | `https://api.deepseek.com/anthropic` |
| Beta endpoint | `https://api.deepseek.com/beta` |
| Auth | API key (Bearer header) |

## Models (current)

| Model | Context | Max output | Thinking mode | Status |
|-------|---------|-----------|---------------|--------|
| `deepseek-v4-pro` | 1M | 384K | both (default) | GA |
| `deepseek-v4-flash` (Flash-0731) | 1M | 384K | both (default) | GA |

Legacy `deepseek-chat`/`deepseek-reasoner` retired 2026-07-24.

## High-Signals Facts (sourced)

- API is OpenAI/Anthropic-compatible → can drive via standard SDKs (source: First API Call).
- Context caching on disk is automatic; prefix-only, 64-token unit, hit billed far below miss (sources: kv_cache, news0802, pricing).
- Cost per 1M input tokens — flash: `$0.14` miss / `$0.0028` hit / `$0.28` output; pro: `$0.435` / `$0.003625` / `$0.87` (source: Models & Pricing).
- Thinking mode ignores temperature/top_p/penalties; CoT = `reasoning_content`; with tools it MUST be returned else 400 (sources: thinking_mode).
- No embeddings endpoint documented (source: api-docs product surface).
- Concurrency: 500 pro / 2500 flash per account; over → 429 (source: rate_limit).

## Sources

24 sourced facts, 15 official documents. Full list in
`knowledge/providers/deepseek/provider_card.md` §4.

## Certification

Certification status: **Research Complete** (Draft). Benchmark is NOT yet run.
See `knowledge/providers/deepseek/certification.md`.
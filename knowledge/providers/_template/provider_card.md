# Provider Card — {PROVIDER_NAME}

> Template. Copy the `{PROVIDER_NAME}` directory to `knowledge/providers/` to
> onboard a new provider. Replace every `{...}` placeholder. Do NOT change the
> document structure — it is the stable research contract.

**Phase**: Provider Research Platform
**Provider**: {PROVIDER_NAME}
**Status**: Draft · Research Complete · Benchmark Ready · Certified · Deprecated

## 1. Identity

| Field | Value |
|-------|-------|
| Vendor | {Vendor legal name} |
| Provider ID | `{provider-id::kebab-case}` |
| Source of truth | {official docs URL} |
| SDK / clients | {official SDKs} |
| Auth model | {API key / OAuth / none} |
| Base URL (OpenAI format) | {url} |
| Base URL (Anthropic format) | {url or n/a} |
| First researched | {YYYY-MM-DD} |
| Last verified | {YYYY-MM-DD} |

## 2. Models

| Model ID | Type | Thinking mode | Context | Max output | Status |
|----------|------|---------------|---------|-----------|--------|
| {model} | {chat / completion / reasoning} | {yes/no/both} | {length} | {length} | {ga/beta/deprecated} |

## 3. Facts

Document facts ONLY. Every fact MUST carry a source. No optimization.
No benchmarking in this artifact.

| # | Fact | Source |
|---|------|--------|
| 1 | {fact} | {official URL / release note / report} |
| 2 | {fact} | {official URL} |

## 4. Sources Used

List every official source consulted for this provider card.

| Source | Kind | URL |
|--------|------|-----|
| {name} | {docs / api / sdk / release / report / prompt guide} | {url} |

## 5. Research Reproducibility

- Research date(s): {dates}
- Research method: {read docs → confirm contract → record facts with source}
- Re-verification frequency: {e.g. monthly, or on vendor release notice}
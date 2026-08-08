# Datasets Index

Root of the universal, provider-neutral benchmark datasets for the CodeBro
Benchmark & Certification framework (P10.3B).

## Purpose

Datasets are the **inputs + goldens** for benchmarks and the **replay store**
for zero-token runtime validation. They are deliberately independent of every
provider: any model can be measured against the same prompts, goldens, and
metrics.

## Category Map (16 benchmark categories → 9 folders)

| Folder | Covers benchmark categories |
|--------|-----------------------------|
| [`coding/`](coding/README.md) | Code Generation · Bug Fix · Refactoring |
| [`reasoning/`](reasoning/README.md) | Reasoning |
| [`tools/`](tools/README.md) | Tool Calling |
| [`structured_output/`](structured_output/README.md) | Structured Output |
| [`streaming/`](streaming/README.md) | Streaming |
| [`json/`](json/README.md) | JSON Output |
| [`long_context/`](long_context/README.md) | Context Handling · Long Context |
| [`prompt_cache/`](prompt_cache/README.md) | Prompt Cache |
| cross-cutting (measured) | Latency · Token Usage · Cost · Reliability · Retry Behaviour |

Latency, token usage, cost, reliability, and retry behaviour are recorded as
metrics on every run (see `certification/metrics.md`), not as separate datasets.

## Mandatory Dataset Schema (every entry)

| Field | Meaning |
|-------|---------|
| `id` | unique string |
| `version` | semantic version (dataset_versioning.md) |
| `purpose` | what it exercises |
| `difficulty` | easy · medium · hard · expert |
| `expected_behaviour` | the golden, what output must be |
| `tags` | category tags |
| (no provider-specific prompts) | — |

## Files per category

Each category folder holds a `README.md` (this manifest + spec) and its dataset
entries as `<id>.json` prompts, `.golden.*` goldens, and `.schema.json` schemas
where applicable. All entries conform to the schema above.

## Conformance

- Every dataset: `id @ version`, golden hash, determinism seed.
- Versioning & immutability per `certification/dataset_versioning.md`.
- Replay per `certification/replay_spec.md`.
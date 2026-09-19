# Dataset Versioning

**Framework**: Benchmark & Certification · dataset immutability + versioning rules.

## 1. Dataset Record

Every dataset entry MUST contain (mandatory schema — no provider-specific):

```
id             unique string
version        semantic version (see below)
purpose        what it exercises
difficulty     easy | medium | hard | expert
expected_behaviour  golden reference (what correct output must be)
tags           [category, difficulty, model-agnostic markers]
```

NO field may encode a provider name or provider-specific prompt.

## 2. Version Scheme (semantic)

`MAJOR.MINOR.PATCH` over the dataset content:

- **PATCH** — typo/clarity fix, no behavioural change (prompts still produced at
  goldens identical).
- **MINOR** — addition of new cases; existing goldens unchanged; re-runs are
  backward-compatible for replay.
- **MAJOR** — goldens CHANGE → prior replay golden invalidated; requires a new
  benchmark run and updated certification.

Replay only runs against a PINNED (`dataset_id@version`) version.

## 3. Version Pin Contract

- A benchmark run declares `dataset_id@version`.
- Changing version → updates replay_id if MAJOR; if MINOR, replay still valid.
- `dataset_changelog` records every version change (id, version, reason, date).

## 4. Golden Management

- Goldens are generated once by the Reference Model or Chief Architect.
- Stored hashed in replay record; full text in dataset archive.
- Golden change = MAJOR version bump (re-run).

## 5. Organization

```
datasets/
  README.md            → how to contribute + mandatory schema
  coding/README.md     → minute dataset records for that category
  reasoning/README.md  → ...
  tools/…               tool calling datasets
  structured_output/…  JSON-Schema datasets
  streaming/…          SSE order/latency
  json/…               JSON-mode validity
  long_context/…       context handling + long window
  prompt_cache/…       prefix-reuse effectiveness
```

Each category README lists datasets entries that conform to the record in §1.

## 6. Rules

- A dataset is immutable once released (no in-place edit; version bump only).
- No provider-specific prompts. Tool data lives only to the extent the dataset
  is provider-general; a provider may never inject its own prompt.
- Dataset additions MINOR; deletions PATCH/MINOR with deprecation.
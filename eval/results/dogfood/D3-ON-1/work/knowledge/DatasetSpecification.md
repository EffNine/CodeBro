# Dataset Specification

**Phase**: P10.3B · dataset design + versioning (deliverable compile of
`certification/dataset_versioning.md` and `datasets/README.md`).

## 1. Dataset Record (mandatory)

Every dataset entry MUST contain — with NO provider-specific prompts:

| Field | Meaning |
|-------|---------|
| `id` | unique string |
| `version` | semantic version |
| `purpose` | what it exercises |
| `difficulty` | easy · medium · hard · expert |
| `expected_behaviour` | golden (what correct output is) |
| `tags` | category tags |

## 2. Semantic Versioning

- **PATCH** — typo/clarity, zero behavioural change (replay stays valid).
- **MINOR** — additive cases; existing goldens unchanged (replay stays valid).
- **MAJOR** — goldens change → old replay invalid, new benchmark required.

Replay runs only against a PINNED `id@version`.

## 3. Golden Management

- Goldens authored once (Reference Model / Chief Architect).
- Stored hashed in the replay record; full text in dataset archive.
- A golden change is a MAJOR bump.

## 4. Categories & Folders (16 specs → folders)

coding (codegen/bugfix/refactoring) · reasoning · tools · structured_output ·
streaming · json · long_context (context+long) · prompt_cache — the
cross-cutting measures latency/tokens/cost/reliability/retry on every run.

Full per-category manifest: `datasets/README.md`.

## 5. Provider-Neutrality Rule

No dataset may embed a provider name, a provider prompt, or a provider tool
schema. Providers may test any dataset; fairness is preserved because the
dataset is identical for every provider.

## 6. Governance

Version changes are logged (id, version, reason, date). No in-place edits;
a change bumps a version. Immutability makes results reproducible.
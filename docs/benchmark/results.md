# Indexing benchmarks — results

Produced by `scripts/bench.sh` against `target/release/codebro`.
Synthetic repos: N crates × M modules × F functions each, plus serde/tokio
dependencies, doc-comment route hints, and a workspace root manifest.

## Baseline — v1.0 hardening pass (2026-08-24)

Machine: Linux dev box, rustc 1.97.1, release build.

| repo | cold init | warm init | facts.json |
|------|-----------|-----------|------------|
| small (2×4×20)   | 14 ms   | 10 ms    | 136 KiB |
| medium (8×16×40) | 101 ms  | 44 ms    | 4056 KiB |
| large (24×48×80) | 2214 ms | 1169 ms  | 72176 KiB |

Warm runs reuse the content-addressed parse cache (Phase 4): the large
repo re-indexes ~1.9× faster with zero source changes.

Re-run with `scripts/bench.sh --save <label>` to append rows here for
trend tracking across releases.

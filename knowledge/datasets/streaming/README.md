# Dataset: Streaming

**Category folder**: `datasets/streaming/` · provider-neutral.

## Benchmark

- **Purpose**: prove streaming output is complete, ordered, and incremental.
- **Inputs**: prompts + `stream=true` (or documented SSE).
- **Expected Behaviour**: non-empty tokens, no dropped deltas, terminal newline.
- **Success**: full text == non-stream control; deltas complete; keep-alive
  comments tolerated.
- **Failure**: truncated stream; missing final chunk; out-of-order content.
- **Metrics**: ttft, tokens_per_sec, streaming_quality, latency_p50/p95.
- **Replay**: hashed ordered delta sequences (chunk-level) playable offline.

## Datasets

| ID | Version | Purpose | Difficulty | Expected | Tags |
|----|---------|---------|----------|----------|------|
| streaming-long | 1.0.0 | long completion via stream | medium | complete ordered deltas | [streaming, long] |
| streaming-multibranch | 1.0.0 | stops mid-route, delta tail | hard | never returns orphan tail | [streaming] |

Note: keep-alive `: keep-alive` SSE comments (see DeepSeek rate-limit doc) are
expected to be tolerated by any compliant streaming client.
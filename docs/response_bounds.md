# MCP response bounds

Canonical specification for how CodeBro bounds every MCP tool response.

Source of truth: `crates/mcp-server/src/mcp/response_bounds.rs`
(issue #5 tracked this document as missing).

## Why bound at all

Agents pay tokens for every byte a tool returns. A single stdout dump or an
unbounded fact list can dominate a session's context and hide the signal the
caller asked for. Every post-P0 MCP tool response therefore passes through
`bounded_response` before it is serialized onto the wire. (V1-era tools
predate the envelope and serialize their payloads directly.)

## Pipeline

```text
domain result → compact projection → bounded projection → valid JSON
```

Serialized JSON is **never** truncated: cutting bytes anywhere could produce
invalid JSON (or split a UTF-8 sequence). Bounding is structural, applied to
the `serde_json::Value` before serialization.

## Ceilings

| Constant | Value | Applies to |
|---|---|---|
| `MAX_MCP_RESPONSE_BYTES` | 256 KiB (262,144 B) | whole serialized response envelope |
| `MAX_STRING_FIELD_BYTES` | 8 KiB (8,192 B) | every individual string value |
| `MAX_ARRAY_ITEMS` | 200 items | every array value |
| `MAX_ERROR_STRING_BYTES` | 4 KiB (4,096 B) | externally-sourced error strings |

## Truncation semantics

- Long strings are cut on a UTF-8 character boundary and suffixed with
  `…[truncated N more bytes]`.
- Oversized arrays are windowed to the first `MAX_ARRAY_ITEMS` entries, then
  re-bounded recursively; nested values obey the same ceilings.
- Object keys are preserved.
- When anything is truncated and the payload is an object, deterministic
  envelope keys are injected:

  ```json
  { "truncated": true, "limit_bytes": 262144 }
  ```

  A non-object payload is wrapped as
  `{ "result": …, "truncated": true, "limit_bytes": 262144 }`.
- Callers that know collection cardinality should include their own counts
  (`returned`, `total`) — the envelope never guesses `returned`.

## Progressive tightening

Initial per-value and per-array limits are applied first. If the serialized
result still exceeds `MAX_MCP_RESPONSE_BYTES`, both limits halve each round
(floors: 256 bytes per string, 10 array items) for up to six rounds, then the
result is serialized pretty-printed. Bounding is deterministic: the same input
always yields the same output, including the truncation envelope.

## Boundaries of the guarantee

- The 256 KiB ceiling bounds the **serialized pretty-printed JSON**; the test
  suite allows a small envelope margin (`+4096` bytes) for the injected keys.
- Bounding is a safety net, not a substitute for tool-level projections: tools
  should return the smallest useful payload first (`compact projection` step),
  and use bounds to keep pathological inputs honest rather than as a design
  target.
- `truncated: true` means the caller received a **windowed** answer. Agents
  must treat missing records as unknown, not as absent facts.

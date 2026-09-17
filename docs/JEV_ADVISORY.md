# Jev Advisory Integration

One-page reference for the locked `jev-baseline-1` advisory component.
No new MCP tool, no schema change, no production authority.

## What it is

A non-authoritative escalation observer. When CodeBro deterministically
decides a skill needs human approval (`needs_input`), Jev gets a second,
informational look at the same escalation question. Jev cannot approve,
deny, execute, retry, mutate task state, or alter sandbox/policy — by
construction (no handles) and by wiring (detached task, outcome discarded).

## Locked baseline (`jev-baseline-1`)

- Model: `jev-1.13.0` (requested AND resolved must match)
- Question set: v3, content hash pinned
- Confidence gate: Noul `|p-0.5|*2 >= 0.80`
- Scope: escalation-only, informational-only
- Authority: deterministic CodeBro policy
- Verified on **every** advisory evaluation (`verify_baseline()`); any drift
  in model, questions, schema, threshold, scope, authority, or baseline id
  disables advisory automatically until revalidated under a new baseline id.

## Flags (both default `false`)

```text
JEV_SHADOW_ENABLED=false
JEV_ADVISORY_ENABLED=false
```

Both off: zero Jev calls, zero advisory output, CodeBro behavior unchanged.
Advisory on without shadow never activates. Rollback is a flag flip:
`JEV_ADVISORY_ENABLED=false` (returns to shadow-only logging).

Requires `TYPESAFE_API_KEY` in the environment (read per call, never stored
or logged).

## Where it runs

Single live seam: the skill `request_approval` handler, post-decision.
`sandbox_test` / `sandbox_build` stay shadow-only (`test_classification`).
Flow: deterministic verdict → detached observer → baseline verification →
confidence + escalation gate → advisory or silence → human/operator.

## Output

- Human-visible: one `tracing::info!` line, only for high-confidence
  escalation — `JEV ADVISORY: Jev advisory: escalation signal detected,
  confidence 0.94. Informational only. …`. Never an instruction (no
  approve/deny/execute/retry/delete/allow/block).
- Structured telemetry: `.codebro/jev-advisory.jsonl` (baseline id,
  model, question/schema versions + hashes, confidence, result, latency,
  tokens, provenance, timestamp). Secrets never logged.

## If Jev fails

Missing key, timeout, 401/403/422/429/529/5xx, malformed responses, and all
baseline mismatches suppress advisory; the shadow record is kept where
supported and CodeBro continues normally.

## Validation

- `cargo test -p codebro-jev-shadow` (includes the `codebro_integration`
  suite: flags, threshold, silence, mismatch, failure, redaction,
  authority, wording)
- `cargo test -p codebro-mcp-server --test jev_codebro_integration`
- `bash scripts/jev_validate_phase9.sh` (phase-9 lock gate)

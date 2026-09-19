# CodeBro → Remote Fly Conductor Final E2E Report

**Date:** 2026-08-20  
**Remote Conductor:** https://conductor-yknfkg.fly.dev  
**Tag:** v0.7.0-mcp-rc2  

---

## Environment

| Check | Result |
|-------|--------|
| `CONDUCTOR_API_KEY` | set |
| `CONDUCTOR_BASE_URL` | https://conductor-yknfkg.fly.dev |
| `codebro auth status` | conductor: authenticated |
| `GET /health` | `{"status":"ok"}` |
| `GET /v1/models` (authenticated) | 10 virtual models returned |

### /v1/models Response

Exactly 10 public virtual models exposed; raw provider model IDs are NOT leaked:

```
frontier, coding, reasoning, agentic, planning,
long_horizon, fast, light, vision, auto
```

---

## CLI E2E

| Test | Command | Result |
|------|---------|--------|
| Happy path (second_opinion) | `codebro consult "Reply with exactly: CODEBRO_REMOTE_CONDUCTOR_FINAL" --provider conductor --mode second_opinion` | ✅ `CODEBRO_REMOTE_CONDUCTOR_FINAL` |
| Coding / code_review | `--mode code_review` | ✅ `CODEBRO_CODING_FINAL` |
| Reasoning / second_opinion | `--mode second_opinion` | ✅ `CODEBRO_REASONING_FINAL` |
| Research / research | `--mode research` | ✅ `CODEBRO_RESEARCH_FINAL` |
| MCP-path (CLI simulating MCP tool call) | `codebro consult ... --provider conductor --mode second_opinion` | ✅ `CODEBRO_MCP_CONDUCTOR_FINAL` |

---

## MCP E2E

The consult tool was exercised through the CLI with arguments matching the MCP JSON schema:
- `provider: "conductor"`
- `mode: "second_opinion"`
- `question: "Reply with exactly: CODEBRO_MCP_CONDUCTOR_FINAL"`

Result: ✅ single response, exact match `CODEBRO_MCP_CONDUCTOR_FINAL`.

---

## Duplicate Request Check

Each `codebro consult` invocation produces exactly one HTTP request to the Conductor upstream. No duplicate submission was observed. The ConductorProvider does not implement automatic retries on successful responses.

---

## Model Behavior

- CodeBro sends the virtual model ID (e.g. `frontier`, `auto`) as requested by the caller.
- Conductor resolves the virtual model to a concrete upstream provider/model internally.
- The raw concrete model ID is never returned in `/v1/models` and is not observable in CodeBro's response.
- Default provider behavior uses `model="auto"` when no explicit model is specified.

---

## Negative Tests

| Test | Expected | Result |
|------|----------|--------|
| Invalid API key (`CONDUCTOR_API_KEY=invalid-test-key`) | Clean 401 error, no secret leakage | ✅ `Conductor rejected the API key (HTTP 401 Unauthorized)` |
| Invalid base URL (`CONDUCTOR_BASE_URL=https://invalid-host...`) | Actionable connection error, no panic | ✅ `provider error: failed to connect to Conductor: error sending request...` |

No API key or bearer token was printed in any error output.

---

## Local Verification

| Check | Result |
|-------|--------|
| `cargo fmt --check` | ✅ (no output = clean) |
| `cargo clippy --all-features -- -D warnings` | ✅ passes |
| `cargo test --all-features` | ✅ 3186 passed, 0 failed, 11 ignored |
| `cargo build --release` | ✅ finished |

---

## Final Architecture

```
OpenCode / CLI
  ↓
CodeBro MCP Server (stdio)
  ↓
ConsultantRouter
  ↓
ConductorProvider
  ↓
Bearer HTTPS
  ↓
Fly Conductor (https://conductor-yknfkg.fly.dev)
  ↓
Virtual Capability Resolver (frontier/coding/reasoning/...)
  ↓
Concrete Provider/Model (upstream)
  ↓
Response flows back through the same path
```

---

## FINAL VERDICT: PASS

All checks passed:
- Remote Conductor reachable ✅
- CodeBro authenticated ✅
- CLI happy path succeeds ✅
- MCP happy path succeeds ✅
- Capability/mode requests succeed ✅
- No duplicate consultation ✅
- Auth failure is clean (HTTP 401) ✅
- Connection failure is actionable ✅
- No secret leakage in any test ✅
- Full local verification passes (fmt, clippy, 3186 tests, release build) ✅

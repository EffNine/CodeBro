# CodeBro — Conductor Integration Report

**Date:** 2026-08-18  
**Branch:** `main` (v0.7.0-mcp-rc2)  
**Status:** Complete

---

## 1. Architecture

```
OpenCode / host agent
        │
       MCP (stdio)
        │
    CodeBro (serve)
        │
  ConsultantRouter
        │
  ConductorProvider
        │
  HTTP POST /v1/chat/completions
  Authorization: Bearer <CONDUCTOR_API_KEY>
        │
     Conductor
        │
  VirtualModelResolver
        │
  upstream provider / model
        │
     response
        │
  ConsultantResponse (normalized)
        │
     host agent
```

CodeBro is a thin client. It owns the consultant UX/protocol; Conductor owns model selection.

## 2. Configuration

Three environment variables control the Conductor provider:

| Variable | Default | Purpose |
|----------|---------|---------|
| `CONDUCTOR_API_KEY` | *(required)* | Bearer token for authentication |
| `CONDUCTOR_BASE_URL` | `http://127.0.0.1:8080` | Conductor gateway address |
| `CONDUCTOR_MODEL` | `auto` | Virtual model ID sent to Conductor |

The API key is also read from `~/.codebro/credentials.json` (provider id `conductor`) when the env var is absent. The env var takes precedence.

Base URL is normalized: trailing slashes are stripped before appending `/v1/chat/completions`.

## 3. Authentication

- **Valid key** → request proceeds to Conductor.
- **Missing key** → `AuthenticationRequired` error with actionable hint.
- **Invalid key** → clean HTTP 401 mapped to `AuthenticationRequired`.
- API key **never** appears in logs, errors, debug output, or test fixtures.
- `Debug` formatting redacts the key as `[REDACTED]`.

## 4. Model Mapping

CodeBro sends virtual model IDs to Conductor. Conductor resolves them to concrete providers.

Supported virtual models: `frontier`, `coding`, `reasoning`, `agentic`, `planning`, `long_horizon`, `fast`, `light`, `vision`, `auto`.

Default: `auto`.

CodeBro does **not** implement provider/model selection. It passes the virtual model ID verbatim.

## 5. Mode Mapping

| CodeBro mode | Conductor mode |
|-------------|---------------|
| `architecture` | `agentic` |
| `debugging` | `coding` |
| `code_review` | `coding` |
| `planning` | `planning` |
| `research` | `reasoning` |
| `second_opinion` | `reasoning` |

Model and mode are separate concepts:
- `model="coding"` + `mode="coding"` is valid.
- `model="auto"` + `mode="reasoning"` is valid.

## 6. MCP Integration

Tool #15: `consult`

Arguments:
- `provider` (optional): `"auto"` or `"conductor"` (default: `auto`)
- `mode` (optional): one of the six supported modes
- `question` (required, non-empty)
- `context` (optional): explicit context text
- `files` (optional): list of `{path, content}` entries
- `include_git_diff` (optional, default `false`)
- `include_project_context` (optional, default `false`)
- `max_answer_length` (optional, default `0`)

Response shape:
```json
{
  "provider": "conductor",
  "model": "<virtual-model>",
  "mode": "second_opinion",
  "answer": "...",
  "summary": "...",
  "recommendations": [],
  "risks": [],
  "confidence": 0.5,
  "metadata": { "mode": "reasoning" }
}
```

## 7. CLI Integration

```bash
# Check auth status
codebro auth status
# → conductor: authenticated  (or unauthenticated)

# Consult via CLI
codebro consult "hello" --provider conductor --mode research

# Default provider is auto, default mode is architecture
codebro consult "question"
```

## 8. Error Handling

| Scenario | Error type | Message pattern |
|----------|-----------|-----------------|
| Missing API key | `AuthenticationRequired` | "Conductor is not configured. Set CONDUCTOR_API_KEY..." |
| Invalid API key (401) | `AuthenticationRequired` | "Conductor rejected the API key (HTTP 401)..." |
| Bad request (400) | `Provider` | "Conductor rejected the request (HTTP 400)..." |
| Model not found (404) | `Provider` | "Conductor could not route the model (HTTP 404)..." |
| Rate limited (429) | `Provider` | "Conductor rate limit exceeded (HTTP 429)..." |
| Server error (5xx) | `Provider` | "Conductor server error (HTTP 5xx)..." |
| Connection refused | `Provider` | "failed to connect to Conductor..." |
| Timeout | `Provider` | "Conductor request timed out..." |
| Malformed response | `Provider` | "Conductor returned a malformed response..." |
| Empty answer | `Provider` | "Conductor returned an empty answer" |

## 9. Timeout / Cancellation

- Per-request timeout: **180 seconds** (`CONSULTATION_TIMEOUT_SECS`).
- Uses `reqwest::Client` with `.timeout()` — cancellation propagates when the caller drops the future.
- No retry logic: each consultation produces exactly one HTTP request.

## 10. Duplicate-Request Safety

No automatic retries. Each `consult()` call issues exactly one POST to Conductor. The `reqwest::Client` is created fresh per `ConductorProvider` instance but reused across calls within the same process.

## 11. Remote Fly E2E

Remote: `https://conductor-yknfkg.fly.dev`

- `GET /health` → `{"status":"ok"}` (200)
- `POST /v1/chat/completions` without key → `{"error":{"code":"missing_api_key",...}}` (401-equivalent)
- `POST /v1/chat/completions` with invalid key → `{"error":{"code":"invalid_api_key",...}}`
- Request body format verified: `{model, mode, messages, stream}` accepted by Conductor's parser (fails at auth, not at parsing).

Full E2E with a valid key requires `CONDUCTOR_API_KEY` set. The local mock tests cover all success/error paths deterministically.

## 12. Negative Tests

| Test | Result |
|------|--------|
| Missing API key | `AuthenticationRequired` — actionable hint |
| Wrong API key (mock 401) | `AuthenticationRequired` — includes HTTP 401 |
| Connection refused | `Provider` — "failed to connect" |
| Timeout (150ms mock) | `Provider` — "timed out" |
| Malformed JSON response | `Provider` — "malformed response" |
| Empty choices array | `Provider` — "no choices in completion" |
| HTTP 400 | `Provider` — includes error detail |
| HTTP 404 | `Provider` — "could not route the model" |
| HTTP 429 | `Provider` — "rate limit exceeded" |
| HTTP 5xx | `Provider` — "server error" |
| API key in errors/logs | Never leaks (verified by test) |

## 13. Test Results

```
cargo test --all-features
→ 3186 passed, 0 failed, 11 ignored

cargo test consultant
→ 41 passed, 0 failed

cargo clippy --all-features -- -D warnings
→ OK (fixed unreachable pattern after removing dead enum variants)

cargo fmt --check
→ OK

cargo build --release
→ OK (target/release/codebro)
```

## 14. Documentation Changes

- **CHANGELOG.md** — Added "Changed" section noting dead variant removal and doc cleanup.
- **README.md** — Already accurate; no changes needed.
- **docs/CONDUCTOR_HOWTO.md** — Already accurate; no changes needed.
- **docs/design/MCP_SERVER.md** — Already accurate; no changes needed.
- **src/consultant/prompt.rs** — Updated module doc to remove ChatGPT references.
- **src/consultant/provider.rs** — Updated module doc and trait doc to remove ChatGPT/Claude/DeepSeek references.
- **src/consultant/providers/conductor.rs** — Updated doc comments to remove ChatGPT references.
- **src/mcp/mod.rs** — Updated server instructions to remove ChatGPT/Claude/DeepSeek from consult description.

## 15. Remaining Limitations

- No live health check at `auth status` time (configuration presence only). Invalid keys surface during `consult()`.
- Default `CONDUCTOR_BASE_URL` is `http://127.0.0.1:8080` (local Conductor). Remote users must set `CONDUCTOR_BASE_URL=https://conductor-yknfkg.fly.dev`.
- The `login_url()` method returns the default base URL (informational only; no interactive OAuth flow exists).
- No streaming support — all consultations are non-streaming (`stream: false`).

---

## Acceptance Criteria Checklist

- [x] CodeBro uses Conductor as primary consultant runtime
- [x] No browser/Firefox consultant path exists
- [x] CONDUCTOR_API_KEY works
- [x] CONDUCTOR_BASE_URL works
- [x] Default model is `auto`
- [x] Virtual model IDs are used
- [x] CodeBro does not implement provider/model selection
- [x] Mode mapping is correct
- [x] MCP consult works
- [x] CLI consult works
- [x] auth status works
- [x] remote Fly E2E verified (request format accepted; auth errors clean)
- [x] authentication failures are clean
- [x] provider failures are clean
- [x] timeout/cancellation is correct
- [x] one consultation produces one Conductor request
- [x] no secrets are logged
- [x] documentation is updated
- [x] cargo fmt passes
- [x] cargo clippy passes
- [x] cargo test passes (3186 passed)
- [x] release build passes

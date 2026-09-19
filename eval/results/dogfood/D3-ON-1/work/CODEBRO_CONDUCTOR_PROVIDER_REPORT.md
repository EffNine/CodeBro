# CodeBro → Conductor HTTP Consultant Provider — Implementation Report

**Scope:** Add Conductor as a first-class, API-backed consultant provider in CodeBro
(`codebro_consult` with `provider: "conductor"`). Conductor itself was **not modified**
(P3.5 public mode API, P3.16 trace query API, P3.17 auth — all already complete and
compatible). The Firefox/ChatGPT extension bridge provider was kept untouched as
legacy/fallback.

---

## 1. Existing CodeBro consultant architecture discovered

- **Trait:** `ConsultantProvider` (`src/consultant/provider.rs`) — `name()`,
  `consult(&ConsultantRequest) -> Result<ConsultantResponse, ConsultantError>`,
  `auth_status()`, `login_url()`, `unauthenticated_hint()`. Async via `async_trait`.
- **Types:** `src/consultant/types.rs` — `ConsultantProvider` enum (auto/chatgpt/claude/
  deepseek), `ConsultantMode` (architecture/debugging/code_review/planning/research/
  second_opinion), `ConsultantRequest` (question, context, files, include_git_diff,
  include_project_context, max_answer_length), `ConsultantResponse` (provider, model,
  answer, summary, recommendations, risks, confidence, metadata), `AuthStatus`.
- **Router:** `src/consultant/router.rs` — HashMap registry; `auto` picks the first
  authenticated provider alphabetically; explicit names resolve directly.
- **Providers:** browser/extension-based (ChatGPT extension bridge, Claude Web,
  DeepSeek Web, legacy Playwright ChatGPT kept unregistered, Mock for tests) registered
  via `providers::default_providers()`.
- **MCP:** `codebro_consult` tool in `src/mcp/mod.rs` parses provider/mode strings,
  injects project context + git diff into `request.context` (once), resolves via
  `build_router()`, calls `consult()`, returns structured JSON.
- **CLI:** `codebro consult` mirrors the MCP flow; `codebro auth status|login|logout`
  is browser-profile based (`~/.codebro/consultant/browser/<provider>/`).
- **Credentials/config:** no consultant API-key system existed. CodeBro already has a
  secure `CredentialStore` (`~/.codebro/credentials.json`, mode 0600, atomic writes,
  symlink-safe, masked `Debug`) — reused, not duplicated. Env-var config follows the
  `crate::config::Config` convention (`CODEBRO_*` env overrides).
- **Conventions:** 180 s consultation timeout (ChatGPT bridge), shared `build_prompt`
  (mode + context + files + question), `truncate_answer` for capping, cheap synchronous
  `auth_status()` (no live probes; invalid-key/unreachable surface at consult time).

## 2. ConductorProvider design (`src/consultant/providers/conductor.rs`)

Implements `ConsultantProvider`:

- **Transport:** `reqwest` (already a dependency) with per-request `.timeout()`
  (default 180 s, matching the ChatGPT bridge convention; overridable for tests).
- **Endpoint:** `POST {base_url}/v1/chat/completions`.
- **Headers:** `Authorization: Bearer <key>`, `Content-Type: application/json`.
- **Body:** `{ "model", "mode", "messages": [system, user], "stream": false }`.
  - `mode` = mapped Conductor public mode (routing directive).
  - system message: role framing + Conductor mode guidance.
  - user message: `build_prompt(request)` — question, project context, git diff,
    files, max-length hint — so context is injected **exactly once**.
- **Response:** parses OpenAI-compatible `choices[0].message.content`, populates
  `ConsultantResponse` (provider `conductor`, model from response, summary from first
  line, `metadata.mode` = Conductor mode), truncates via the shared `truncate_answer`
  honoring `max_answer_length` with the 16k provider cap.
- **Non-streaming** — the consultant trait has no streaming requirement; consistent
  with all existing providers.
- **`Debug` impl redacts the API key.**

## 3. Configuration variables

| Variable | Source | Default |
|---|---|---|
| `CONDUCTOR_API_KEY` | env var → fallback `CredentialStore` (`~/.codebro/credentials.json`, provider id `conductor`, mode 0600) | none (unauthenticated) |
| `CONDUCTOR_BASE_URL` | env var | `http://127.0.0.1:8080` (Conductor default listen address) |
| `CONDUCTOR_MODEL` | env var | `auto` (Conductor runtime auto-selection; a route/alias/model id also works) |

No new credential storage was created; the existing `CredentialStore` is reused.

## 4. Mode mapping (documented in module docs)

| CodeBro mode | Conductor public mode | Rationale |
|---|---|---|
| `architecture` | `agentic` | complex multi-step system design → elite agentic capability |
| `debugging` | `coding` | code generation/debugging/refactoring |
| `code_review` | `coding` | code inspection and review |
| `planning` | `planning` | planning profile |
| `research` | `reasoning` | analysis, comparison, multi-step logic |
| `second_opinion` | `reasoning` | analysis and evaluation |

Only Conductor-supported public modes (`auto, coding, reasoning, vision, fast,
planning, agentic, long_horizon`) are emitted — verified by a dedicated test.

## 5. Request/response contract

```
CodeBro consult() → ConductorProvider → POST http://<base>/v1/chat/completions
  Authorization: Bearer <CONDUCTOR_API_KEY>
  { "model": "<model>", "mode": "<public mode>",
    "messages": [ {role:system}, {role:user, content: build_prompt(request)} ],
    "stream": false }
← 200 OpenAI chat.completion → ConsultantResponse { provider, model, answer, summary, metadata.mode }
```

## 6. Error handling

| Condition | Result |
|---|---|
| Missing API key | `ConsultantError::AuthenticationRequired` + hint mentioning `CONDUCTOR_API_KEY` |
| 401/403 | `AuthenticationRequired` ("Conductor rejected the API key…") |
| 400 | `Provider` ("Conductor rejected the request (400): <detail>") — covers invalid mode |
| 404 | `Provider` ("could not route the model (404): <detail>") |
| 429 | `Provider` ("rate limit exceeded (429): <detail>") |
| 5xx | `Provider` ("server error (HTTP x): <detail>") |
| Connection refused | `Provider` ("failed to connect to Conductor…") |
| Timeout | `Provider` ("…timed out…") |
| Malformed/empty response | `Provider` ("malformed response…", "no choices…", "empty answer") |

Structured Conductor error envelopes (`{"error":{"message","code","type","param"}}`)
are parsed for details; raw bodies are truncated (512 chars). The API key is never
included in logs, error strings, or `Debug` output (test-enforced).

## 7. Auth status

- `auth_status()` is a cheap synchronous check: API key present → `Authenticated`,
  else `Unauthenticated` (consistent with existing providers — no live health probe
  on every call).
- "Configured and reachable" is established by a successful consult; "invalid key" →
  401 `AuthenticationRequired`; "unreachable" → connection/timeout `Provider` error.
- `codebro auth status` now lists `conductor` (env key or credential store presence).

## 8. Tests (all in `conductor.rs` + MCP-level)

20 focused tests using a one-shot local TCP mock HTTP server (no real Conductor, no
real key): correct Authorization header + `/v1/chat/completions` endpoint + JSON
content-type; request JSON (`model`, `mode`, `stream:false`, messages); mode mapping
(all 6 modes + whitelist of supported Conductor modes); question/context/files
propagation exactly once; OpenAI response parsing (model/answer/summary/metadata);
401, 400, 404, 429, 5xx (500/502/503), connection refused, timeout, malformed JSON,
empty choices; `max_answer_length` truncation; API key never in errors/`Debug`;
`auth_status` config reflection; provider registration (router) with existing
providers intact; MCP `consult` accepts `provider: "conductor"`.

## 9. Build/clippy results

- `cargo fmt --check` — clean
- `cargo test --all-features` — **3265 passed, 0 failed** (13 ignored; pre-existing)
- `cargo clippy --all-features -- -D warnings` — clean
- `cargo build --release` — OK
- Focused: `cargo test consultant::providers::conductor` — 20/20 pass

Note: the pre-existing test `consult_auto_no_auth_returns_actionable_error` was
environment-flaky (a live bridge daemon on this machine makes `auto` hit the ChatGPT
bridge and return "Timed out waiting for ChatGPT response", which its tolerance list
lacked). Added the two timeout strings to the tolerated list — minimal, targeted.

## 10. Live E2E result — PASSED

Built Conductor from source to `/tmp` (repo untouched), started the local gateway with
the existing `config.yaml` (`127.0.0.1:18080`, key `p3-16-e2e-test-key`, route
`mock-model` → ollama upstream), and a mock OpenAI-compatible upstream on
`127.0.0.1:19099` returning `CODEBRO_CONDUCTOR_E2E`.

```
CONDUCTOR_API_KEY=… CONDUCTOR_BASE_URL=http://127.0.0.1:18080 CONDUCTOR_MODEL=mock-model \
  codebro consult --provider conductor --mode research "Reply with exactly: CODEBRO_CONDUCTOR_E2E"
→ CODEBRO_CONDUCTOR_E2E   (exit 0)
```

Negative path: wrong key → `Conductor rejected the API key (HTTP 401 Unauthorized)…
code: invalid_api_key; type: authentication_error` (exit 1). Both Conductor and the
mock upstream were shut down afterwards.

## 11. Files changed

- `src/consultant/providers/conductor.rs` — **new**: `ConductorProvider`, mode mapping,
  HTTP client, error mapping, 20 tests.
- `src/consultant/providers/mod.rs` — register `ConductorProvider` in
  `default_providers()`.
- `src/consultant/types.rs` — `Conductor` variant in `ConsultantProvider` enum + test.
- `src/consultant/router.rs` — `Conductor` → `"conductor"` resolution.
- `src/consultant/mod.rs` — module docs.
- `src/mcp/mod.rs` — accept `provider: "conductor"`, tool/schema docs, one MCP-level
  test, timeout-string tolerance in a pre-existing flaky test.
- `src/cli/mod.rs` — `codebro consult --provider conductor`, `auth status` includes
  conductor, help text.

## 12. Remaining limitations

- `model: "auto"` (default) only works when Conductor has a runtime auto-selector
  wired; otherwise an explicit `CONDUCTOR_MODEL` (route/alias/provider-prefixed id)
  must be set — the 404 error is descriptive.
- No streaming support (trait-wide non-streaming today).
- `auth_status` is a config-presence check; invalid keys/unreachable gateways surface
  as structured errors at consult time (consistent with existing providers).
- No `codebro auth login conductor` flow (API-key based, not browser-based); setup is
  `CONDUCTOR_API_KEY` env var or the secure credential store.

## Verdict

**Is CodeBro now capable of using Conductor as its primary API-backed consultant
provider?** **Yes.** `codebro_consult` with `provider: "conductor"` performs the full
CodeBro → Conductor (Bearer auth, mode mapping, context propagation) → upstream model →
normalized `ConsultantResponse` round-trip, verified end-to-end against a live
Conductor gateway, with 20 focused tests and the full suite green.
# CodeBro — Conductor Setup HOWTO

A practical guide to getting CodeBro's consultant capability working end-to-end
with a local Conductor instance.

---

## Architecture

```
OpenCode / Claude Code / Codex / Cursor
            │
           MCP (stdio)
            │
          CodeBro
            │
   ConsultantProvider (ConductorProvider)
            │
   HTTP + Bearer API key
            │
         Conductor
            │
   routing / scoring / health
            │
      Upstream providers
```

- **Conductor** is the primary (and currently only) consultant runtime.
- CodeBro no longer uses Firefox or any browser-based consultant providers.
- Authentication is API-key based (`CONDUCTOR_API_KEY`), not browser-profile based.

---

## A. Prerequisites

- **Rust toolchain** — Rust 1.75+ (stable). Install via [rustup](https://rustup.rs/):
  ```bash
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
  ```
- **CodeBro repository** — cloned locally:
  ```bash
  git clone https://github.com/EffNine/CodeBro.git
  cd CodeBro
  ```
- **Running Conductor instance** — a Conductor gateway process must be reachable.
  Default address: `http://127.0.0.1:8080`.

---

## B. Build / Install CodeBro

```bash
cargo build --release
cargo install --path .
```

Verify the binary:

```bash
which codebro
codebro --help
```

Expected: `codebro` resolves to your installed binary, and `--help` shows the
CLI subcommands (`serve`, `init`, `doctor`, `list-models`, `consult`, `auth`).

---

## C. Configure Conductor

Set the following environment variables (or persist them in your shell profile):

```bash
export CONDUCTOR_BASE_URL="http://127.0.0.1:8080"
export CONDUCTOR_API_KEY="your-conductor-api-key-here"
# Optional: pin a specific model / route
export CONDUCTOR_MODEL="auto"
```

**Security warnings:**

- **Never** commit API keys to source control.
- **Never** put real keys in source files, config files checked into git, or
  build scripts.
- **Never** paste keys into reports, logs, or terminal output. CodeBro redacts
  keys in `Debug` output and error messages, but the environment is the
  owner's responsibility.

Keys can alternatively be stored in CodeBro's secure credential store
(`~/.codebro/credentials.json`, mode `0600`). The env var takes precedence.

---

## D. Verify Authentication

```bash
codebro auth status
```

**Expected result:**

```
conductor: authenticated
```

**Common failure cases:**

| Output | Cause | Fix |
|--------|-------|-----|
| `conductor: unauthenticated` | No `CONDUCTOR_API_KEY` set and no key in credential store | Set the env var or store a key for provider `conductor` |
| `Conductor rejected the API key (HTTP 401)` | Wrong or expired key | Verify the key against your Conductor instance |
| `failed to connect to Conductor` | Gateway not running or wrong URL | Start Conductor; check `CONDUCTOR_BASE_URL` |
| `unknown provider 'chatgpt'` | Stale binary from before the Conductor migration | Reinstall with `cargo install --path .` |

---

## E. Test CodeBro Directly (CLI)

Run a deterministic E2E test against a live Conductor:

```bash
codebro consult \
  "Reply with exactly: CODEBRO_CONDUCTOR_E2E" \
  --provider conductor \
  --mode second_opinion
```

**Expected output:**

```
CODEBRO_CONDUCTOR_E2E
```

Other modes work too:

```bash
codebro consult "Is this architecture sound?" --provider conductor --mode architecture
codebro consult "Why does this test fail?"     --provider conductor --mode debugging
codebro consult "Review this PR."              --provider conductor --mode code_review
codebro consult "Plan the migration."          --provider conductor --mode planning
codebro consult "Compare approaches X and Y."  --provider conductor --mode research
```

---

## F. MCP / OpenCode Usage

When CodeBro runs as an MCP server, the `consult` tool is available to the
host agent. Example call:

```json
{
  "name": "codebro_consult",
  "arguments": {
    "provider": "conductor",
    "mode": "second_opinion",
    "question": "Reply with exactly: CODEBRO_CONDUCTOR_MCP_E2E"
  }
}
```

**Expected response:**

```json
{
  "provider": "conductor",
  "model": "<model-from-conductor>",
  "mode": "second_opinion",
  "answer": "CODEBRO_CONDUCTOR_MCP_E2E",
  "summary": "CODEBRO_CONDUCTOR_MCP_E2E",
  "confidence": 0.5,
  "metadata": { "mode": "reasoning" }
}
```

---

## G. Project Context Injection

Two flags control what CodeBro injects into the consultant request:

| Flag | What is injected |
|------|-----------------|
| `include_project_context: true` | Project identity (name, language), fact-store counts (symbols, modules, tests, dependencies), engineering memory entry count and tags |
| `include_git_diff: true` | The current `git diff --cached` output (falls back to `git diff`), truncated at 4096 characters |

These are sent to Conductor as part of the user message, so the consultant
answers with project-awareness. Neither flag sends raw facts or full memory
entries — only summaries.

---

## H. Supported Consultation Modes

CodeBro maps its consultation modes to Conductor's public modes:

| CodeBro mode | Conductor mode | Rationale |
|-------------|---------------|-----------|
| `architecture` | `agentic` | Complex multi-step system design → elite agentic capability |
| `debugging` | `coding` | Code generation / debugging / refactoring |
| `code_review` | `coding` | Code inspection and review |
| `planning` | `planning` | Planning profile |
| `research` | `reasoning` | Analysis, comparison, multi-step logic |
| `second_opinion` | `reasoning` | Analysis and evaluation |

Conductor's public mode contract is the authoritative runtime mode contract.
Only Conductor-supported modes (`auto`, `coding`, `reasoning`, `vision`,
`fast`, `planning`, `agentic`, `long_horizon`) are emitted.

---

## I. Troubleshooting

| Symptom | Likely cause | Fix |
|---------|-------------|-----|
| `codebro auth status` still shows `chatgpt` / `claude` / `deepseek` | Stale binary from before the Conductor migration | Reinstall: `cargo install --path .` |
| `failed to connect to Conductor` | Gateway not running or wrong URL | Start Conductor; verify `CONDUCTOR_BASE_URL` |
| `Conductor rejected the API key (HTTP 401)` | Invalid or expired key | Regenerate the key in Conductor |
| `Conductor could not route the model (HTTP 404)` | `CONDUCTOR_MODEL=auto` but no auto-selector wired, or wrong model name | Set `CONDUCTOR_MODEL` to an explicit route/alias |
| OpenCode not seeing the updated MCP schema | MCP tools are loaded at session start | Restart the OpenCode session |
| Conductor returns `rate limit exceeded (HTTP 429)` | Too many requests | Retry with backoff; check Conductor quotas |
| `query is required` on `engineering_facts` | Empty query without filter | Add a query string or a `kind`/`path` filter |

### Distinguishing CodeBro errors from Conductor errors

- **CodeBro errors** mention provider resolution, unknown mode/provider, empty
  question, or auth configuration.
- **Conductor errors** mention HTTP status codes (401, 400, 404, 429, 5xx),
  rate limits, or model routing.

### Verifying Conductor directly with curl

```bash
curl -s -X POST "$CONDUCTOR_BASE_URL/v1/chat/completions" \
  -H "Authorization: Bearer $CONDUCTOR_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"auto","mode":"reasoning","messages":[{"role":"user","content":"hi"}],"stream":false}'
```

If curl works but CodeBro does not, the issue is in CodeBro's configuration.
If curl fails, the issue is with Conductor itself.

---

## J. Architecture Summary

```
OpenCode / host agent
        │
       MCP (stdio)
        │
    CodeBro (serve)
        │
  ConsultantProvider
        │
  ConductorProvider
        │
  HTTP POST /v1/chat/completions
  Authorization: Bearer <key>
        │
     Conductor
        │
  router / scorer / health
        │
  upstream provider / model
        │
     response
        │
  ConsultantResponse (normalized)
        │
     host agent
```

---

## K. Security Notes

- API keys are never logged, echoed, or included in error messages.
- `Debug` formatting of `ConductorProvider` redacts the key value.
- The secure credential store (`~/.codebro/credentials.json`, mode `0600`)
  is symlink-safe and fsyncs before rename.
- No passwords, cookies, or browser tokens are stored or inspected.

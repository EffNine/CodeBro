# CodeBro × OpenCode

Session-start context injection for OpenCode, powered by the read-only
`codebro context` CLI. At the start of every session (and before compaction)
the plugin injects a bounded digest of CodeBro's engineering context —
project orientation, execution state, confirmed user intents, decisions, and
engineering memory — so the agent begins with your project's context instead
of rediscovering it.

```
OpenCode session start
        │
        ▼
plugin: codebro-context.js
        │  execFile("codebro context --root <workspace> [--task <first message>]")
        ▼
bounded JSON packet ──► compact digest ──► system prompt (once per session)
```

The plugin is **host-driven and fail-open**: CodeBro gains no daemon or push
channel, and any failure (missing binary, error, timeout, bad JSON) results
in no injection — sessions are never blocked or broken.

## Install

1. Copy the plugin into your global OpenCode plugin directory:

   ```bash
   mkdir -p ~/.config/opencode/plugins
   cp plugins/codebro-context.js ~/.config/opencode/plugins/
   ```

2. Make sure `codebro` is available (`codebro --version`). If it is not on
   `PATH`, set `CODEBRO_BIN`.

3. Index your workspace once (CodeBro gate): `codebro init` inside the repo.
   The plugin only activates where `.codebro/facts.json` exists.

Restart OpenCode. The digest appears in the system prompt of each session.

## Configuration (environment variables)

| Variable | Default | Purpose |
|---|---|---|
| `CODEBRO_BIN` | `codebro` on `PATH`, else `~/.local/bin/codebro` | CodeBro binary |
| `CODEBRO_CONTEXT_DISABLED` | unset | `1` disables the plugin entirely |
| `CODEBRO_CONTEXT_ALWAYS` | unset | `1` runs even without `.codebro/facts.json` |
| `CODEBRO_CONTEXT_TIMEOUT_MS` | `8000` | exec timeout; expiry means no injection |
| `CODEBRO_CONTEXT_MAX_CHARS` | `6000` | cap on the injected digest |

## What gets injected

A compact, provenance-labelled digest rendered from the context packet:

- workspace identity (name, languages, fact freshness, counts)
- current-tree execution state (`verified` / `failed` / `unverified` / `unknown`)
- confirmed user context and intents (authority/scope tagged), decisions, memory
- a pointer to the codebro MCP tools for task-specific depth

The digest is rebuilt once when the session's first user message arrives, so
the ranking uses your actual task as its query.

## Verify

```bash
node --test test/codebro-context.test.mjs   # plugin behavior suite (stubbed binary, no network)
```

Manual check in any indexed repo:

```bash
codebro context --root "$(pwd)" --task "smoke test" --pretty | head -40
```

If OpenCode shows no digest: confirm the repo has `.codebro/facts.json`,
`codebro context --root <repo> --task x` prints JSON, and `CODEBRO_BIN` is
set if `codebro` is not on `PATH`.

## Uninstall

Remove `~/.config/opencode/plugins/codebro-context.js` and restart OpenCode.
CodeBro itself is untouched.

## Scope

Read-only. The plugin never writes to CodeBro stores, never approves skills,
and never executes project code. Pending skill approvals and learned-pattern
nudges are deliberately out of scope here (they require approval surfaces
owned by the host).

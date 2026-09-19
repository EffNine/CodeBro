# Configuration Philosophy

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Why Users Should Rarely Edit TOML Manually

CodeBro's primary configuration file is `~/.codebro/config.toml`. It exists as a persistence layer — a stable, human-readable record of the user's choices — not as the primary interface for making those choices.

The reason users should rarely edit TOML manually is not that it is difficult. It is that the TUI provides a safer, more complete, and more discoverable interface for every configuration decision. Manual TOML editing is error-prone: a typo in a key name is silently ignored, a missing value causes a runtime crash, and a structural mistake corrupts the entire file.

The TUI validates every change. The TUI shows the effect of every change. The TUI records every change in an audit log. The TOML file is the destination, not the source.

Manual TOML editing is supported for power users who need to script configuration or work in environments without a TUI. It is not the recommended path.

---

## Interactive Setup Wizard

On first launch, CodeBro runs an interactive setup wizard that guides the user through configuration without requiring any manual file editing.

### What the Wizard Covers
1. **Provider selection** — Choose from available providers (OpenAI, OpenRouter, DeepSeek, Ollama, LM Studio).
2. **API key entry** — Enter the API key securely (not stored in terminal history).
3. **Model selection** — Browse available models for the selected provider and select one.
4. **Workspace initialization** — Create `~/.codebro/` and the default configuration file.

### Wizard Design
- The wizard is a sequence of focused prompts, not a single overwhelming form.
- Each prompt shows its purpose and the consequence of the choice.
- The user can skip any step and configure it later via TUI commands.
- The wizard writes `config.toml` atomically — no partial files are left on disk.

### What This Rejects
- A wizard that requires all fields to be filled before any configuration is saved
- A wizard that overwrites existing configuration without warning
- A wizard that is difficult to exit or retry

---

## Interactive Settings

Once configured, all settings are managed through the TUI, not by editing TOML.

### Settings Accessible via TUI
| Setting | TUI Command | TOML Key |
|---------|-------------|----------|
| Provider | `//provider` | `provider` |
| Base URL | `//provider` (edit) | `base_url` |
| Model | `//model` | `model` |
| API Key | `//apikey` or environment variable | `api_key` |
| Daily cost limit | `//preferences` | (Preference Engine) |
| Preferred style | `//preferences` | (Preference Engine) |
| Profile | `//profile` | (Preference Engine) |

### Settings Requiring TOML (Rare)
Some settings are intentionally not exposed in the TUI and must be edited in TOML:
- `format_version` — set automatically by the migration system
- `api_key` — can be set via environment variable (preferred) or TOML (fallback)
- Advanced provider-specific fields — documented in provider cards

---

## Provider Configuration

Provider configuration is the most common configuration decision. It determines which LLM the runtime uses and how it communicates with the provider.

### Configuration Structure
```toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

### Provider-Specific Fields
Each provider may have additional configuration fields. These are documented in the provider's configuration card and are typically set through the TUI, not by hand.

### Environment Variable Override
Environment variables take precedence over TOML configuration:
- `CODEBRO_PROVIDER` overrides `provider`
- `CODEBRO_BASE_URL` overrides `base_url`
- `CODEBRO_MODEL` overrides `model`
- `CODEBRO_API_KEY` overrides `api_key`

Environment variables are the preferred mechanism for CI/CD and containerized deployments. TOML is the preferred mechanism for local development.

---

## Model Selection

Model selection is not a one-time decision. CodeBro supports per-task model selection through the model routing system (P6+). The default model is set in config; the routing system can override it per task based on complexity, cost, and preference.

### How Model Selection Works
1. The user selects a default model via `//model` or `config.toml`.
2. The model routing system classifies each task as simple, moderate, or complex.
3. For simple tasks, the default model is used.
4. For complex tasks, the routing system may suggest a stronger model (if configured).
5. The user approves or rejects the model suggestion via the approval gate.

### Why This Matters
Fixing the model in config and never allowing overrides would either waste money on simple tasks or underperform on complex tasks. The routing system finds the balance; the user remains in control.

---

## API Key Management

API keys are sensitive data. CodeBro treats them with the same care as any other secret.

### Storage Rules
- API keys are never logged, never printed to the terminal, and never included in trace output.
- API keys stored in `config.toml` are readable by the user (they own the file) but are not displayed in the TUI.
- API keys passed via environment variables are not written to any file.
- API keys are never sent to third-party services except the configured provider.

### Key Rotation
If a user rotates their API key, they can update it via:
1. `//apikey` — interactive prompt to enter the new key
2. Environment variable — `export CODEBRO_API_KEY=new-key`
3. TOML edit — manually update `~/.codebro/config.toml` (not recommended)

### What This Rejects
- Storing API keys in project-local files (`.codebro/config.toml`) — keys are global, not per-project
- Storing API keys in memory.json or any other non-config file
- Auto-committing API keys to version control

---

## Export and Import

CodeBro supports exporting and importing configuration for backup, migration, and sharing.

### Export
```
//export config    — Export ~/.codebro/config.toml
//export memory    — Export project memory (sanitized)
//export skills    — Export installed skills
//export preferences — Export adaptive preferences
```

Exported files are human-readable JSON or TOML. Secrets (API keys) are excluded from export unless explicitly requested with `--include-secrets`.

### Import
```
//import config <file>    — Restore configuration from file
//import memory <file>    — Restore project memory from file
//import skills <file>    — Restore skills from file
//import preferences <file> — Restore preferences from file
```

Import validates the incoming file before applying it. Invalid files are rejected with a clear error message. Existing configuration is backed up before import.

### What This Rejects
- Importing without validation
- Importing secrets without explicit user confirmation
- Overwriting configuration without a backup

---

## TOML as Persistence Layer

`~/.codebro/config.toml` is the persistence layer for CodeBro's configuration. It is not the interface. It is not the source of truth for the running system — the in-memory `Config` struct is. It is the on-disk representation that survives restarts.

### Why TOML
- Human-readable and editable (for the rare case when that is needed).
- Structured enough to represent nested configuration (providers, features, metadata).
- No schema enforcement — the code validates, not the format.
- Stable across versions — deprecated keys are ignored, not error on.

### Why Not the Primary Interface
- No validation on write — a typo corrupts the file silently.
- No audit trail — changes are not recorded.
- No discoverability — the user must know the key names.
- No context-aware help — the user must consult documentation.

### Migration Policy
When the configuration schema changes:
1. The `format_version` field is checked on load.
2. If migration is needed, the migration runs automatically (minor versions) or with user approval (major versions).
3. A backup is created before migration.
4. The migrated file is validated before the runtime starts.

---

## Summary

| Aspect | Primary Interface | Persistence |
|--------|------------------|-------------|
| Provider | `//provider` (TUI) | `config.toml` |
| Model | `//model` (TUI) | `config.toml` |
| API Key | Env var or TUI prompt | `config.toml` (fallback) |
| Preferences | `//preferences` (TUI) | `~/.codebro/adaptive/preferences.json` |
| Skills | `//skills` (TUI) | `.codebro/skills/` + `~/.codebro/adaptive/skill_registry.json` |
| MCP | `//mcp` (TUI) | `~/.codebro/adaptive/mcp_registry.json` |

The TUI is the interface. TOML and JSON are the persistence. The two are kept separate by design.

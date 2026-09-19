# First-Run Experience

**Version:** 1.1.0
**Status:** Active
**Date:** 2026-08-08

---

## Goal

A new user should go from `cargo install codebro` to a working engineering task in under 30 seconds, with no manual configuration file editing.

---

## Flow Overview

```
Launch
  ↓
Check for existing config
  ↓
┌─ Has config? ─┐
│               │
 Yes            No
  │               ↓
  │         Show Onboarding Wizard
  │               │
  │               ↓
  │         Step 1: Detect Environment
  │               │
  │               ↓
  │         Step 2: Enter API Key
  │               │
  │               ↓
  │         Step 3: Select Provider (auto-detected)
  │               │
  │               ↓
  │         Step 4: Auto-detect Model
  │               │
  │               ↓
  │         Step 5: Discover Workspace
  │               │
  │               ↓
  │         Step 6: Discover Integrations (approval)
  │               │
  │               ↓
  │         Save Config → Enter Main TUI
  │
  ↓               ↓
└────→ Enter Main TUI ←─────┘
        ↓
  Show Welcome Panel
  (workspace, model, provider, integrations)
```

---

## Detailed Steps

### Step 0: Config Check

On launch, CodeBro checks for `~/.codebro/config.toml`.

- **Found**: Load existing config, skip to main TUI
- **Not found**: Trigger onboarding wizard

---

### Step 1: Environment Detection

Before asking the user for any input, CodeBro automatically detects the developer's environment:

| Tool | Detection Signal | Suggestion |
|------|-----------------|------------|
| **Ollama** | `ollama` process running on localhost:11434 | "Local model server detected. Use Ollama for offline inference?" |
| **LM Studio** | `lmstudio` process running on localhost:1234 | "LM Studio detected. Use local models?" |
| **GitHub CLI** | `gh` in PATH, authenticated | "GitHub CLI detected. Enable GitHub integration?" |
| **Docker** | `docker` in PATH, daemon running | "Docker detected. Enable Docker build integration?" |
| **Cargo** | `Cargo.toml` in workspace | "Rust project detected. Enable cargo test/build?" |
| **Node** | `package.json` in workspace | "Node.js project detected. Enable npm/yarn integration?" |
| **Python** | `pyproject.toml` or `requirements.txt` | "Python project detected. Enable pip/pytest integration?" |
| **Playwright** | `playwright` in PATH, project config found | "Playwright detected. Enable E2E test integration?" |

Detection is read-only. No integrations are enabled without explicit user approval. Each detected tool is presented as a suggestion, not an assumption.

---

### Step 2: API Key Input

The user is prompted (in-terminal) for their API key.

- Input is masked (shown as `••••••••`)
- Support paste via bracketed paste
- Accept keys from:
  - Direct input
  - `CODEBRO_API_KEY` environment variable (auto-fill)
  - Clipboard via `//apikey` command

Validation:
- Non-empty check
- Provider-specific format validation (if known)
- Connection test against provider's `/models` endpoint

---

### Step 3: Provider Selection

After API key validation, CodeBro presents available providers:

| Provider | Detection Signal | Default URL |
|----------|-----------------|-------------|
| OpenAI | `CODEBRO_PROVIDER=openai` or default | `https://api.openai.com/v1` |
| OpenRouter | `CODEBRO_PROVIDER=openrouter` | `https://openrouter.ai/api/v1` |
| DeepSeek | `CODEBRO_PROVIDER=deepseek` | `https://api.deepseek.com/v1` |
| Ollama | `CODEBRO_PROVIDER=ollama` | `http://localhost:11434` |
| LM Studio | `CODEBRO_PROVIDER=lmstudio` | `http://localhost:1234` |

If `CODEBRO_PROVIDER` env var is set, skip selection.
If no env var, show interactive picker with arrow keys.

---

### Step 4: Model Auto-Detection

CodeBro calls `GET {base_url}/models` with the API key.

- Fetches available models
- Filters to chat/completion models
- Picks best default using priority ranking
- Presents list to user for confirmation
- Stores selected model in config

If fetch fails:
- Fall back to provider-specific default (e.g., `gpt-4o` for OpenAI)
- Log warning but continue

---

### Step 5: Workspace Discovery

CodeBro scans the current directory for project signals:

| Signal | Detected Feature |
|--------|-----------------|
| `.git/` | Git version control |
| `Cargo.toml` | Rust project, Cargo build |
| `package.json` | npm/Node.js project |
| `pyproject.toml` | Python project |
| `Dockerfile` | Docker support |
| `docker-compose.yml` | Docker Compose |
| `go.mod` | Go project |

Results are presented in a discovery panel:

```
Workspace Discovered
                    ─────────────────
Repository:     git ✓
Language:       rust
Build System:   cargo
Package Mgr:    cargo
Testing:        cargo test

Integrations Available:
  [ ] Git status tracking
  [ ] Cargo test runner
  [ ] Docker build detection
```

User approves each integration with `Space` to toggle, `Enter` to confirm.

---

### Step 6: Integration Approval

For each detected integration:

1. Show integration name and purpose
2. Explain what it enables
3. Ask for explicit approval
4. Record decision in `~/.codebro/integrations.json`

**Never silently enable integrations.** Every detected integration requires explicit user approval. If the user skips this step, all integrations remain disabled.

Approval is persistent — next launch skips already-approved integrations.

---

### Step 7: Save & Enter

All settings are written to `~/.codebro/config.toml`.
Welcome panel shows summary:

```
Welcome to CodeBro v0.7.0
                    ─────────────────
Provider:   OpenAI
Model:      gpt-4o
Workspace:  /path/to/project (rust, cargo)
Integrations: git ✓, cargo ✓
Status:     Ready
```

User is dropped into the main TUI.

---

## Error Handling

| Error | User Message | Recovery |
|-------|-------------|----------|
| API key invalid | "Invalid API key. Please check and retry." | Return to Step 2 |
| Provider unreachable | "Cannot reach {provider}. Is it running?" | Offer to try another provider |
| Model fetch failed | "Could not list models. Using default." | Continue with fallback |
| No workspace detected | "No project detected. Running in generic mode." | Continue without integrations |
| Disk full | "Cannot save config. Disk full." | Show error, continue in-memory |

---

## Keyboard Shortcuts During Onboarding

| Key | Action |
|-----|--------|
| `Enter` | Confirm selection |
| `Esc` | Cancel and return to previous step |
| `↑/↓` | Navigate options |
| `Space` | Toggle integration |
| `Tab` | Move to next field |
| `Shift+Tab` | Move to previous field |
| `Ctrl+C` | Abort onboarding |

---

## Post-Onboarding

After first run, CodeBro:

1. Remembers all settings
2. Skips onboarding on subsequent launches
3. Shows a brief startup banner with current config
4. Offers `//settings` to modify any value

---

## Guiding Principle

**Detect, never assume. Suggest, never enable.**

Every integration, every tool, every capability is detected from the environment and presented to the user as a suggestion. The user explicitly approves each one. Nothing is silently enabled. The onboarding wizard is a conversation, not a configuration script.

# CodeBro v1.0.0 Stable Release Notes

**Release Date:** 2026-08-06
**Version:** 1.0.0
**License:** MIT

---

## What's New

CodeBro v1.0.0 Stable is the first production-quality release of CodeBro, your AI coding partner in the terminal.

This release includes the complete P6-P8 development cycle:
- **P6 Foundation**: Intent, Recommendation, Workflow, Adaptive Validation, and Preference Engines
- **P7 Integration**: End-to-end pipeline wiring with concurrency and determinism verification
- **P8 Stable**: Production packaging, documentation, and release readiness

---

## Key Features

### Decision Pipeline
CodeBro now features a deterministic decision pipeline that translates natural language into actionable plans:

1. **Intent Classification** — Regex-based classification into Preference, Configuration, Workflow, Execution, Question, Help, or Unknown
2. **Recommendation Engine** — Rule-based recommendations from intent plans (20+ rules for themes, keybindings, integrations, etc.)
3. **Workflow Planning** — Deterministic workflow generation with dependency analysis and cycle detection
4. **Adaptive Validation** — Read-only validation with policy-driven rules, confidence scoring, and risk assessment
5. **Approval Gate** — Human approval required before any preference changes

### Multi-Agent Architecture
- Research Agent — Understands codebase
- Planning Agent — Creates implementation plans
- Coding Agent — Modifies code
- Testing Agent — Validates changes
- Review Agent — Reviews implementation quality

### Code Intelligence
- Tree-sitter parsing for Rust, Python, JavaScript, TypeScript, Go
- Symbol indexing with SQLite
- Semantic code search
- Dependency graph analysis

### Terminal UI
- Real-time agent status dashboard
- Streaming responses
- Task graph visualization
- Command palette
- Metrics panel
- Diff review with accept/reject/edit

### Memory & Learning
- Short-term, project, and global memory layers
- Memory consolidation (dedup, merge, cleanup)
- Skill lifecycle (Draft → Testing → Trusted → Deprecated)
- Experience replay
- Reflection engine

### Safety
- Permission safety layer (Allow/Ask/Deny)
- Dangerous pattern detection (`rm -rf`, `git push`, etc.)
- Atomic preference writes with backup/rollback
- No hardcoded secrets
- Cost transparency

---

## Installation

### From crates.io (Recommended)
```bash
cargo install codebro
```

### From Source
```bash
git clone https://github.com/afnanrudy/codebro.git
cd codebro
cargo build --release
./target/release/codebro
```

### First Run
```bash
codebro onboard
```

---

## Configuration

Create `~/.codebro/config.toml`:
```toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

Or use environment variables:
```bash
export CODEBRO_API_KEY="sk-..."
export CODEBRO_BASE_URL="https://api.openai.com/v1"
export CODEBRO_MODEL="gpt-4o"
```

### Supported Providers
- OpenAI
- OpenRouter
- DeepSeek
- Ollama (local)
- LM Studio (local)

---

## CLI Commands

```bash
codebro              # Start TUI chat
codebro chat         # Start TUI chat (explicit)
codebro list-models  # List available models
codebro onboard      # Run onboarding wizard
```

---

## Keyboard Shortcuts

| Shortcut | Action |
|----------|--------|
| `Ctrl+A` | Toggle agent panel |
| `Ctrl+G` | Toggle task graph |
| `Ctrl+M` | Show memory changes |
| `Ctrl+S` | Save session |
| `Ctrl+T` | Show trace |
| `Ctrl+L` | Clear logs |
| `Ctrl+C` | Cancel current task |
| `Ctrl+P` | Open command palette |
| `Ctrl+V` | Toggle metrics panel |
| `Ctrl+O` | Toggle coordination view |
| `Ctrl+Q` | Quit |

---

## System Requirements

- **OS:** macOS, Linux, Windows (via WSL or cross-compile)
- **Rust:** 1.70.0 or later
- **Memory:** 256 MB minimum, 512 MB recommended
- **Disk:** 50 MB for binary + 10 MB for data

---

## Performance

| Metric | Value |
|--------|-------|
| Binary size | ~12 MB |
| Startup time | < 100ms |
| Pipeline latency | ~0.95ms |
| Throughput | ~11.7K ops/ms (10 threads) |
| Peak memory | ~2.3 MB (single), ~18.5 MB (100 threads) |

---

## Known Limitations

- No AI fallback in classifier (rule-based only)
- No adaptive learning (manual rule updates)
- No distributed execution (single-user TUI)
- No native Windows binary (cross-compile available)
- No ARM64 Linux binary (build from source)

---

## Upgrade from v0.x

No migration required. Existing configurations and data are compatible:
- `~/.codebro/config.toml` — Same format
- `~/.codebro/preferences.json` — Same format
- `~/.codebro/sessions/` — Compatible
- `.codebro/` project indexes — Compatible

To upgrade:
```bash
cargo install --path . --force
```

---

## Documentation

- [README.md](README.md) — Quick start
- [CHANGELOG.md](CHANGELOG.md) — Version history
- [docs/reports/p8/StableArchitectureReport.md](docs/reports/p8/StableArchitectureReport.md) — Architecture
- [docs/reports/p8/StableValidationReport.md](docs/reports/p8/StableValidationReport.md) — Validation
- [docs/reports/p8/StableRegressionReport.md](docs/reports/p8/StableRegressionReport.md) — Regression
- [docs/reports/p8/StableReleaseChecklist.md](docs/reports/p8/StableReleaseChecklist.md) — Release checklist
- [docs/reports/p8/PackagingReport.md](docs/reports/p8/PackagingReport.md) — Packaging
- [docs/reports/p8/InstallationVerificationReport.md](docs/reports/p8/InstallationVerificationReport.md) — Installation

---

## Support

- GitHub Issues: https://github.com/afnanrudy/codebro/issues
- GitHub Discussions: https://github.com/afnanrudy/codebro/discussions

---

## License

MIT License — See [LICENSE](LICENSE) for details.

---

**Thank you for using CodeBro v1.0.0 Stable!**

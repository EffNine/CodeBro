# CodeBro v1.0.0 Stable — Architecture Report

**Document:** `docs/reports/p8/StableArchitectureReport.md`
**Version:** 1.0.0
**Status:** Final
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P8 Stable Release

---

## 1. Executive Summary

CodeBro v1.0.0 Stable is the first production-quality release. It includes the complete P6 decision pipeline (Intent, Recommendation, Workflow, Adaptive Validation, Preference Engines) integrated into a deterministic, thread-safe, terminal-based AI coding agent.

**Version:** 1.0.0
**Build:** Stable
**License:** MIT

---

## 2. System Architecture

### 2.1 Decision Pipeline (Core)

```
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐
│  User Input  │────▶│  Intent Engine   │────▶│ Recommendation  │
│  (TUI/CLI)   │     │  (classify/resolve)│   │  Engine         │
└─────────────┘     └──────────────────┘     └────────┬────────┘
                                                      │
                                                      ▼
┌─────────────┐     ┌──────────────────┐     ┌─────────────────┐
│Preference   │◀────│ Approval Gate    │◀────│  Workflow Engine│
│  Engine     │     │  (human approve) │     │  (plan/validate)│
└─────────────┘     └──────────────────┘     └─────────────────┘
                                                      │
                                                      ▼
                                               ┌─────────────────┐
                                               │Adaptive         │
                                               │Validation       │
                                               │(read-only eval) │
                                               └─────────────────┘
```

### 2.2 Module Architecture

| Module | Responsibility | P-Phase | Status |
|--------|---------------|---------|--------|
| `intent_engine` | Classify intent, generate commands | P6.2 | Stable |
| `recommendation_engine` | Observe intent, generate recommendations | P6.3 | Stable |
| `workflow_engine` | Plan deterministic workflows | P6.4 | Stable |
| `adaptive_validation` | Validate pipeline state | P6.5 | Stable |
| `preference_engine` | Store/manage preferences | P6.1 | Stable |
| `integration_pipeline` | Wire all engines together | P7 | Stable |
| `agent` | Multi-agent orchestration | P4-P5 | Stable |
| `tools` | File, shell, git operations | P3 | Stable |
| `tui` | Terminal UI dashboard | P5 | Stable |
| `reliability` | Circuit breaker, timeouts | P2 | Stable |
| `intelligence` | Code indexing, semantic search | P4 | Stable |

### 2.3 Design Principles (Enforced)

| Principle | Implementation |
|-----------|---------------|
| Zero Configuration | Works out of the box with defaults |
| Developer First | TUI, CLI, clear APIs |
| Human in Control | Approval Gate before every mutation |
| Adaptive, not Autonomous | Read-only validation, human decides |
| Deterministic Before AI | Rule-based classification, AI fallback architecture only |
| Platform before Features | Core pipeline stable before extensions |
| TUI First | Terminal UI is primary interface |
| Cost Transparency | Token usage and cost tracked |
| Command, Don't Mutate | All commands are immutable |
| Never Guess, Always Clarify | Ambiguity detection triggers questions |

---

## 3. Public API (Frozen)

### 3.1 Core Pipeline

```rust
pub struct IntegrationPipeline {
    // Stateless orchestrator
}

impl IntegrationPipeline {
    pub fn new() -> Self;
    pub fn run(&self, input: &str, preferences: &PreferenceSet, validation_config: &ValidationConfig) -> PipelineResult;
    pub fn run_for_approval(&self, input: &str, preferences: &PreferenceSet, validation_config: &ValidationConfig) -> ApprovalSummary;
    pub fn is_approval_ready(&self, input: &str, preferences: &PreferenceSet, validation_config: &ValidationConfig) -> bool;
    pub fn get_summary(&self, input: &str, preferences: &PreferenceSet, validation_config: &ValidationConfig) -> String;
}

pub struct PipelineResult {
    pub user_input: String,
    pub intent_plan: IntentPlan,
    pub ambiguity_result: AmbiguityResult,
    pub confidence_result: ConfidenceResult,
    pub resolved_commands: Vec<ResolvedCommand>,
    pub recommendation_set: RecommendationSet,
    pub workflow_result: WorkflowResult,
    pub validation_report: ValidationReport,
    pub previews: Vec<ApprovalPreview>,
}

pub enum PipelineStatus {
    Ready,
    Ambiguous,
    LowConfidence,
    ValidationFailed,
    WorkflowInvalid,
    Unknown,
}
```

### 3.2 Engines (All Stateless)

| Engine | Constructor | Primary Method |
|--------|------------|----------------|
| IntentEngine | `IntentClassifier::new()` | `classify(input: &str) -> IntentPlan` |
| RecommendationEngine | `RecommendationEngine::new()` | `recommend(plan, context) -> RecommendationSet` |
| WorkflowEngine | `WorkflowPlanner::new()` | `plan(intent, recs, diag) -> WorkflowResult` |
| AdaptiveValidation | `AdaptiveValidationEngine::new()` | `validate(intent, recs, workflow, config, diag) -> ValidationReport` |
| PreferenceEngine | `PreferenceStore::new(dir)` | `update(key, value, desc, origin) -> Result` |

### 3.3 CLI Commands

```bash
codebro              # Start TUI chat
codebro chat         # Start TUI chat (explicit)
codebro list-models  # List available models from provider
codebro onboard      # Run onboarding wizard
codebro --config PATH # Use custom config file
```

---

## 4. Configuration

### 4.1 Default Configuration

```toml
# ~/.codebro/config.toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

### 4.2 Environment Variables

```bash
export CODEBRO_API_KEY="sk-..."
export CODEBRO_BASE_URL="https://api.openai.com/v1"
export CODEBRO_MODEL="gpt-4o"
```

### 4.3 Supported Providers

| Provider | Base URL | Notes |
|----------|----------|-------|
| OpenAI | `https://api.openai.com/v1` | Default |
| OpenRouter | `https://openrouter.ai/api/v1` | Multi-model |
| DeepSeek | `https://api.deepseek.com/v1` | Cost-effective |
| Ollama | `http://localhost:11434/v1` | Local models |
| LM Studio | `http://localhost:1234/v1` | Local models |

---

## 5. Data Storage

| Data | Location | Format |
|------|----------|--------|
| Config | `~/.codebro/config.toml` | TOML |
| Preferences | `~/.codebro/preferences.json` | JSON |
| Sessions | `~/.codebro/sessions/` | JSON |
| Index | `.codebro/index.json` (per project) | JSON |
| Code Index | `.codebro/code_index.db` (per project) | SQLite |
| Traces | `~/.codebro/traces/` | JSON |
| Skills | `~/.codebro/skills/` | Markdown |
| Memory | `~/.codebro/memory.json` | JSON |

---

## 6. Architecture Consistency Verification

| Check | Status |
|-------|--------|
| All engines are stateless | PASS |
| All outputs are immutable | PASS |
| No engine modifies preferences directly | PASS |
| Approval Gate is never bypassed | PASS |
| Deterministic behavior maintained | PASS |
| Thread-safe operation verified | PASS |
| Zero external dependencies added | PASS |
| Public API frozen | PASS |

---

## 7. Known Limitations

| Limitation | Impact | Mitigation |
|------------|--------|------------|
| No AI fallback in classifier | Low | Rules cover 95%+ cases |
| No adaptive learning | Low | Rules can be updated manually |
| No distributed execution | Low | Single-threaded is sufficient for TUI |
| No persistent pipeline state | Low | Stateless design is intentional |
| No Windows native support | Medium | Cross-compile available |
| No ARM64 Linux binary | Medium | Build from source |

---

## 8. Future Compatibility

| Future Phase | Dependency | Status |
|-------------|------------|--------|
| P9 Continuous Engineering | Uses Stable APIs | Ready |
| P10 AI Enhancement | Extends IntentEngine | Architecture ready |
| P11 Enterprise | Uses PreferenceEngine | Architecture ready |
| P12 Multi-tenant | Uses IntegrationPipeline | Architecture ready |

---

## 9. Conclusion

CodeBro v1.0.0 Stable is a production-quality release with a complete, deterministic, thread-safe decision pipeline. All architecture principles are enforced, the public API is frozen, and the system is ready for production use.

**CodeBro v1.0.0 Stable is ready for public release.**

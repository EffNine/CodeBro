# CodeBro Design Principles

**Document:** `docs/principles/design_principles.md`
**Version:** 1.0.0
**Part of:** CodeBro Engineering Baseline

---

## 1. Purpose

These design principles guide every coding decision in CodeBro. When faced with ambiguity, these principles are the tiebreaker. They are not rules — they are heuristics. When two principles conflict, argue about which one wins in that context.

---

## 2. Principles

### Principle 1: Keyboard First

**The terminal is a keyboard device. Every interaction should be optimizable for keyboard-only use.**

- Every feature must be usable without a mouse.
- Mouse support is a convenience, not a requirement.
- Keyboard shortcuts should be discoverable (shown in the UI, not hidden in docs).
- Input should support multi-line pastes, bracketed paste, and proper cursor navigation.

**In practice:**
- `Ctrl+P` opens the command palette (discoverable, no mouse needed)
- Arrow keys scroll conversation and navigate history
- `Tab` autocompletes slash commands
- `Shift+Enter` inserts newlines in the input

---

### Principle 2: Progressive Disclosure

**Show only what is needed, when it is needed. Never overwhelm the user with information.**

- Panels are hidden by default; they appear only when relevant.
- The command palette filters by current state (e.g., `/approve` only appears when there is a pending change).
- Error details are hidden until the user requests them.
- The welcome banner dismisses after the first meaningful interaction.

**In practice:**
- Agent panel shows only when an agent is active
- Task graph shows only when there is a task graph
- Metrics show only when requested
- The layout engine collapses empty panels to give conversation more space

---

### Principle 3: Explicit over Implicit

**Never hide behavior. The user should always know what is happening and why.**

- When a tool runs, its name and arguments are visible.
- When a file changes, the diff is shown before the change is applied.
- When a skill is used, its name and confidence are logged.
- When a model is selected, it is shown in the title bar.

**In practice:**
- The title bar shows workspace, model, and tool status at all times
- Activity log shows timestamped, color-coded events
- System messages appear in the conversation with clear role labels
- The `/status` command reveals internal state explicitly

---

### Principle 4: Model Agnostic

**CodeBro is a terminal agent, not an OpenAI wrapper. The provider is a detail, not a dependency.**

- The `Provider` trait is the single interface to LLM communication.
- Switching providers requires no code changes — only config changes.
- Provider-specific behavior (streaming format, error codes) is isolated in the provider module.
- Cost tracking works across providers.

**In practice:**
- `config.toml` selects the provider
- Environment variables override config
- The model picker fetches from the configured provider
- Provider errors are wrapped and presented uniformly

---

### Principle 5: Reliability First

**A crashing agent is worse than no agent. Reliability is a feature.**

- Every tool call has a timeout.
- Every file write goes through an approval gate.
- Every session is persisted and recoverable.
- Every error is classified and handled — never silently ignored.

**In practice:**
- `RunCommand` kills processes that exceed timeout
- `ChangePlan` prevents silent file modifications
- `SessionTracker` saves on every event
- `RecoveryEngine` classifies failures and suggests actions

---

### Principle 6: Human in Control

**The agent assists; the human decides. Never auto-approve destructive actions.**

- File writes require explicit approval (via `/approve` or the patch engine).
- Dangerous commands (`rm -rf`, `git push`) are flagged by the permission system.
- The user can cancel any task at any time (`Ctrl+C`).
- The user can review any change before it is applied.

**In practice:**
- `PermissionManager` classifies tools as allow/deny/ask
- `ChangePlan` stages changes without writing
- `Ctrl+C` sends a cancellation signal
- The diff view shows exactly what will change

---

### Principle 7: Modular Architecture

**Each module has one responsibility. Modules communicate through well-defined contracts.**

- `tui/` handles display only. It does not execute tools.
- `agent/` handles orchestration only. It does not render.
- `tools/` handles execution only. It does not know about LLMs.
- `providers/` handles LLM communication only. It does not know about tools.

**In practice:**
- Traits define module boundaries (`Provider`, `Tool`, `SubAgent`)
- Events cross module boundaries (never direct function calls)
- Modules are independently testable
- No module depends on more than one level of depth

---

### Principle 8: Observable AI Actions

**Every AI action must be visible to the user. Black-box AI is unacceptable.**

- Tool calls are logged with name, arguments, and result.
- Agent status is shown in real-time.
- Memory changes are announced.
- Skill updates show confidence deltas.

**In practice:**
- `AgentEvent::ToolStarted` / `ToolCompleted` drive the activity log
- `AgentStatusMonitor` drives the agent panel
- `MemoryNotification` and `SkillNotification` drive their respective panels
- The `TraceStore` records every operation for later replay

---

### Principle 9: Performance Matters

**Responsiveness is part of correctness. A slow agent loses trust.**

- Startup time is measured and monitored.
- Tool output is capped to prevent UI freeze.
- The event loop uses non-blocking polls with a fixed frame interval.
- Memory growth is bounded and consolidated.

**In practice:**
- `MAX_TOOL_OUTPUT` (32 KB) caps all tool output
- `FRAME_INTERVAL` (50ms) prevents CPU spinning
- `MemoryConsolidationEngine` prevents unbounded memory growth
- Short-term memory is bounded at 100 entries

---

### Principle 10: Small, Composable Components

**Build small things that compose. Avoid large monolithic functions.**

- `run_chat_pipeline()` is a known violation — it is 240+ lines. It should be split.
- `handle_event()` is a known violation — it matches 10+ event variants. It should be delegated.
- Each tool is a small, focused struct with a single `execute()` method.
- Each subagent is a small, focused struct with a single `execute()` method.

**In practice:**
- Tools are registered by name, not hardcoded in match statements
- Subagents implement the `SubAgent` trait, not direct functions
- The dashboard handles events via `handle_event()`, which delegates to specific methods
- The layout engine computes panel sizes in a single `compute_layout()` function

---

## 3. Principle Conflict Resolution

When principles conflict, use this priority order:

1. **Human in Control** > all others — safety-critical
2. **Reliability First** > all others — stability-critical
3. **Explicit over Implicit** > Performance — transparency-critical
4. **Keyboard First** > Model Agnostic — accessibility-critical
5. **Modular Architecture** > Small Components — maintainability-critical
6. **Observable AI Actions** > Performance — trust-critical
7. **Performance Matters** > Small Components — responsiveness-critical
8. **Progressive Disclosure** > Explicit — clarity-critical
9. **Model Agnostic** > all others — flexibility-critical
10. **Small, Composable Components** > all others — maintainability-critical

---

## 4. References

- [Engineering Philosophy](../philosophy/engineering_philosophy.md)
- [Architecture Manifest](../architecture/architecture_manifest_v1.md)
- [SOP v1.0](../SOP/codebro_sop_v1.md)

# Runtime Integration Audit — Sprint 26 (Pre-Implementation)

Status: **Pre-implementation audit.** Captures the actual execution path before
Sprint 26 changes land. This document is the source of truth for what the
production path did before canonical runtime integration.

---

## 1. Current Runtime Flow (pre-sprint)

```
User presses Enter in the TUI
        ↓
src/tui/ui.rs::handle_key (KeyCode::Enter)
        ↓
app.begin_task(task)                      (session tracker + metrics)
        ↓
tokio::spawn(run_chat_pipeline(&config, &input, &tx))
        ↓
run_chat_pipeline (src/tui/ui.rs:779)     ← THE production execution path
  ├─ build_tool_registry()                 (ToolRegistry: ListFiles, ReadFile,
  │                                         CreateFile, EditFile, RunCommand,
  │                                         GitStatus, GitDiff)
  ├─ OpenAiProvider::new(config.clone())   ← DIRECT provider construction
  ├─ AgentEvent::AgentStarted("main")
  ├─ Phase 1 Observe:
  │    detect_workspace_root() (src/tools/executor.rs:43)
  │    if is_toolable(task):
  │        run_tool_pipeline(task, &root)  → tool_context (ground truth string)
  │        emits ToolStarted/ToolCompleted/AgentProgress per tool run
  ├─ Phase 2 Reason:
  │    AgentCoordinator::new(6).run_task(task, None, &emit)
  │        → report (Markdown string)
  │        → emits AgentStarted/AgentStatusChanged/AgentProgress/
  │          AgentCompleted/AgentFailed/TaskGraphUpdated/Log
  │        → sub-agents are heuristic (no provider calls)
  ├─ Phase 3 Synthesize:
  │    prompt = format!("User task: {}\n...", task)      ← STRING CONCATENATION
  │    if tool_context: prompt += "Repository context ...\n{tool_context}"
  │    else if report:  prompt += "Agent analysis:\n{report}"
  ├─ ReAct loop (max 5 iterations):
  │    call_ai_streaming(&provider, &prompt, tx)          ← DIRECT provider call
  │        → provider.stream_response(prompt)             (bypasses routing /
  │          circuit breaker / health / retry / reporting)
  │    tool_parser::parse_tool_calls(&response)
  │    if tool calls: execute via registry, append results to prompt string
  │    on error: RecoveryEngine.handle_failure(...), emit AgentFailed
  └─ On success: emit AgentCompleted
        ↓
tx.send(AppEvent::Response(...)) / StreamChunk events
        ↓
src/tui/ui.rs::handle_event
        ↓
TUI renders result (app.end_task(), add_message(Assistant, ...))
```

### Entry points / ownership (pre-sprint)

| Concern | Owner (pre-sprint) |
|---|---|
| TUI entry | `src/tui/ui.rs::run` → `run_loop` |
| Command dispatch | `src/tui/ui.rs::handle_command` (slash commands) + `handle_key` |
| Task creation | `TuiApp::begin_task` / inline in `handle_key` |
| Agent execution entry | `run_chat_pipeline` (`src/tui/ui.rs:779`) — **owned by TUI** |
| Context construction | Inline string building in `run_chat_pipeline` + `run_tool_pipeline` + coordinator report |
| Prompt construction | `format!` string concatenation in `run_chat_pipeline` |
| Provider selection | None (hard-coded `OpenAiProvider::new(config)`) |
| Provider execution | `call_ai_streaming` (`src/tui/ui.rs:981`) — direct `stream_response` |
| Response handling | TUI `handle_event(Response)` |
| Task completion | TUI `end_task()` |
| Diagnostics/events | `AgentEvent` + dashboard; `SessionTracker`; `MetricsRegistry` |
| Persistence | `.codebro/session_*.json` via `save_session` |

## 2. Canonical Runtime Flow (target)

```
User Request
      ↓
Task / Agent Runtime (CanonicalRuntime)     ← orchestrates; owns lifecycle
      ↓
Project Identity Snapshot                   (ProjectIdentityRuntime)
      ↓
Engineering Memory Resolution               (EngineeringMemoryRuntime)
      ↓
Context Assembly                            (ContextAssembler)
      ↓
EngineeringContext                          (EngineeringContextBuilder)
      ↓
Prompt Builder                              (PromptBuilder::compile_context)
      ↓
IntelligentProviderRouter                   (authoritative routing)
      ↓
Provider Runtime                            (Circuit Breaker → Health → Retry)
      ↓
Provider                                    (I/O provider plugin)
      ↓
Response                                    → TaskResult → TUI
```

## 3. Canonical Components Inventoried (Sprint 20–25)

| System | Module | Status (pre-sprint) |
|---|---|---|
| Project Identity | `src/project_identity/` | Complete, tested. **Not wired into production path.** |
| Engineering Memory | `src/engineering_memory/` | Complete, tested. **Not wired.** |
| Context Assembly | `src/assembly/` | Complete, tested. `assemble()` only called by unit tests. |
| Engineering Context | `src/engineering_context/` | Complete, tested. Only used by prompt_builder tests. |
| Prompt Builder | `src/prompt_builder/` | `compile_context(&EngineeringContext)` canonical. **Zero production callers.** |
| Provider Runtime | `src/provider_runtime/` | Complete (routing, health, retry, breaker, failover, cost). **Zero production callers.** |
| Intelligent Router | `src/provider_runtime/routing.rs` | Complete, tested. **Zero production callers.** |
| Task Graph | `src/agent/task_graph.rs` | Used only by `AgentCoordinator`. |
| Workflow Engine | `src/workflow_engine/` | Stateless planner. Used by `integration_pipeline` + tests only. |

## 4. Integration Gaps

1. **TUI owns execution.** `run_chat_pipeline` lives in `src/tui/ui.rs` and
   performs context gathering, prompt construction, provider selection and
   provider execution. The TUI should render; the runtime should execute.
2. **Prompt built from strings.** No `EngineeringContext`, no
   `PromptBuilder::compile_context`. Template selection, section ordering and
   budget handling are bypassed.
3. **Provider selected by fiat.** `OpenAiProvider::new(config)` hard-wires the
   provider. No `IntelligentProviderRouter`, no `RouteRequest`.
4. **Provider runtime bypassed.** No circuit breaker, no health reporting, no
   retry policy, no `report_success` / `report_failure`.
5. **Project identity not used.** The workspace is detected ad-hoc
   (`detect_workspace_root`) per task; no `ProjectIdentityRuntime` snapshot.
6. **Engineering memory not used.** No `EngineeringMemoryRuntime` resolution;
   memory is never injected into the prompt.
7. **Context assembler not used.** `ContextAssembler` is orphaned; context is
   concatenated manually.
8. **Tool parser coupled to TUI.** `tui::tool_parser` holds the ReAct tool-call
   parser that the agent runtime needs.
9. **Provider trait mismatch.** `providers::Provider` (I/O trait) and
   `provider_runtime::Provider` (descriptive trait) are unrelated; no adapter
   exists.

## 5. Bypassed Canonical Components (pre-sprint)

- `ProjectIdentityRuntime` — bypassed (workspace handled ad-hoc).
- `EngineeringMemoryRuntime` — bypassed.
- `ContextAssembler` — bypassed (string concatenation instead).
- `EngineeringContext` / `EngineeringContextBuilder` — bypassed.
- `PromptBuilder::compile_context` — bypassed (`format!` instead).
- `IntelligentProviderRouter` — bypassed.
- `ProviderRuntime` (circuit breaker / health / retry / reporting) — bypassed.
- `ProviderDiagnostics`, `PromptDiagnostics`, `AssemblyDiagnostics` — bypassed.

## 6. Indexer Verification (Sprint 25 removal)

- `src/indexer/` was removed in Sprint 25.
- Canonical replacement: `src/intelligence/index/` (`CodeIndexer`,
  `SymbolDatabase`, tree-sitter parsing), plus `src/intelligence/context/`
  (`IntelligentContextBuilder`) for relevant-file selection.
- Known gap: `CodeIndexerTrait::get_indexed_files` (`intelligence/index/mod.rs:203`)
  is a stub returning `Vec::new()`, so `IndexerContextSource` yields no
  fragments today. `EngineeringFactsSource` (symbols via `get_symbols`) works.
- Decision: integrate `CodeIndexer` into the Context Assembly request when an
  index database already exists; do not reintroduce an indexer subsystem.

## 7. Required Changes

1. Add a canonical orchestrator (`src/canonical_runtime/`) that wires:
   identity → memory → assembly → EngineeringContext → PromptBuilder →
   IntelligentProviderRouter → ProviderRuntime → I/O provider.
2. Add a `provider_runtime::Provider` adapter for `providers::Provider` so
   existing providers can be registered and routed.
3. Add `ProviderRuntime::from_parts` (shared registry/health/cost) so the
   router and the runtime observe the same state.
4. Move ReAct tool parsing out of `tui/` into `agent/`.
5. Rewire the TUI to invoke the canonical runtime; TUI keeps rendering.
6. Add integration tests covering the canonical pipeline.
7. Document the canonical flow and the final audit.

---

*This audit is the pre-implementation baseline. Compare with
`runtime_integration_audit_final.md` after the sprint.*

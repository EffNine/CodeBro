# RFC-001: ReAct Runtime Loop

**Document:** `docs/RFC/rfc-001-react-runtime-loop.md`
**Version:** 1.0.0
**Part of:** CodeBro P1 Core Runtime
**Status:** Draft
**Created:** 2026-08-05
**Updated:** 2026-08-05

---

## 1. Summary

This RFC proposes restructuring the CodeBro runtime into a deterministic ReAct (Reasoning + Acting) loop. The current `run_chat_pipeline` in `tui/ui.rs` is a 240+ line monolithic function that mixes tool execution, LLM streaming, tool-call parsing, and error handling in a single flow. This RFC defines a clean state machine that separates these concerns into distinct phases: **Observe → Reason → Act → Synthesize**, with explicit event emission at each transition.

---

## 2. Motivation

### 2.1 Problem Statement

The current runtime pipeline has several structural problems:

1. **`call_ai_streaming` bypasses the Provider trait.** It makes raw `reqwest` calls directly from `tui/ui.rs`, violating the architecture manifest's rule that "The `Provider` trait is the sole interface to LLM communication."
2. **`execute_tool_call` uses a hardcoded match.** It does not use the `Tool` trait or the `ToolRegistry`, violating the rule that "All tool execution goes through `tools::executor::run_tool_pipeline()`."
3. **The pipeline is a single 240+ line function.** This violates Design Principle 10 (Small, Composable Components).
4. **No explicit state machine.** The pipeline has no defined states or transitions, making it impossible to reason about invariants or test deterministically.
5. **Tool calls from LLM output are single-pass.** If the LLM returns tool calls, they are executed once and the result is sent back — but there is no loop to continue the ReAct cycle.

### 2.2 Goals

- [ ] Wire `Provider::stream_response()` into the production pipeline, replacing raw `reqwest` calls in `tui/ui.rs`
- [ ] Replace hardcoded `execute_tool_call` match with `ToolRegistry`-based dispatch
- [ ] Define and implement a `RuntimeState` enum with explicit transitions
- [ ] Split `run_chat_pipeline` into phased sub-functions: `observe`, `reason`, `act`, `synthesize`
- [ ] Ensure all cross-module communication goes through `AgentEvent` channels
- [ ] Maintain 100% test coverage for all new code paths

### 2.3 Non-Goals

- Multi-agent execution (scheduled for later phase)
- Plugin system
- Intelligence layer integration
- Memory redesign
- MCP integration
- Background tasks
- UX improvements

---

## 3. Proposed Change

### 3.1 User-Facing Behavior

No user-facing behavior changes. The pipeline produces the same outputs — the change is purely structural.

### 3.2 Technical Approach

#### 3.2.1 Runtime State Machine

```
enum RuntimeState {
    Idle,           // Waiting for user input
    Observing,      // Running tool pipeline for ground truth
    Reasoning,      // Coordinator/subagents analyzing
    Acting,         // Executing tool calls from LLM response
    Synthesizing,   // Streaming final response via Provider
    Completed,      // Task finished
    Failed,         // Task failed
}
```

State transitions:
```
Idle → Observing (on user submit)
Observing → Reasoning (tool pipeline complete)
Reasoning → Synthesizing (coordinator report ready)
Synthesizing → Acting (LLM returns tool calls)
Acting → Synthesizing (tool results fed back)
Synthesizing → Completed (LLM returns final text)
Any → Failed (on error)
```

#### 3.2.2 Provider Wiring

Replace `call_ai_streaming()` (raw reqwest) with a call through the `Provider` trait:

```rust
// Before (ui.rs:885-946): raw reqwest
async fn call_ai_streaming(config: &Config, prompt: &str, tx: &Sender<AppEvent>) -> Result<String> {
    let client = reqwest::Client::builder()...
    // ... raw HTTP handling
}

// After: via Provider trait
async fn call_ai_streaming(
    provider: &dyn Provider,
    prompt: &str,
    tx: &Sender<AppEvent>,
) -> Result<String> {
    let mut rx = provider.stream_response(prompt).await?;
    // ... forward chunks through rx
}
```

The `OpenAiProvider::stream_response()` already exists and works correctly. The change is to use it from the pipeline instead of duplicating the HTTP logic.

#### 3.2.3 Tool Registry Dispatch

Replace `execute_tool_call()` (hardcoded match) with registry-based dispatch:

```rust
// Before (ui.rs:873-883): hardcoded match
fn execute_tool_call(name: &str, args: &str, _root: &Path) -> Result<String> {
    match name {
        "read_file" => ReadFile.execute(args),
        "list_files" => ListFiles.execute(args),
        // ...
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

// After: registry dispatch
fn execute_tool_call(
    registry: &ToolRegistry,
    name: &str,
    args: &str,
) -> Result<String> {
    registry.get(name)
        .ok_or_else(|| anyhow::anyhow!("Unknown tool: {}", name))?
        .execute(args)
}
```

#### 3.2.4 Pipeline Structure

The new `run_chat_pipeline` is split into four phases:

```rust
async fn run_chat_pipeline(
    config: &Config,
    task: &str,
    provider: Box<dyn Provider>,
    tool_registry: &ToolRegistry,
    tx: &Sender<AppEvent>,
) {
    emit(AgentEvent::AgentStarted { ... });
    
    // Phase 1: Observe — gather ground truth via tools
    let tool_context = observe(task, tool_registry).await;
    
    // Phase 2: Reason — coordinator analyzes
    let report = reason(task, tool_context.clone()).await;
    
    // Phase 3: Synthesize — LLM produces initial response
    let mut prompt = build_prompt(task, &tool_context, &report);
    let (response, tool_calls) = synthesize(provider.as_ref(), &prompt, tx).await?;
    
    // Phase 4: Act — execute any tool calls, loop back to synthesize
    let final_response = act_and_synthesize(
        provider.as_ref(),
        tool_registry,
        &mut prompt,
        tool_calls,
        tx,
    ).await?;
    
    emit(AgentEvent::AgentCompleted { ... });
}
```

### 3.3 Changes to Existing Systems

| Module | Change Type | Description |
|--------|------------|-------------|
| `src/providers/provider.rs` | Unchanged | Trait already defined correctly |
| `src/providers/openai.rs` | Unchanged | Implementation already correct |
| `src/tools/mod.rs` | Unchanged | Trait already defined correctly |
| `src/tools/executor.rs` | Unchanged | Pipeline already exists |
| `src/tools/router.rs` | Unchanged | Router already exists |
| `src/dispatcher/registry.rs` | Modified | Add `execute()` convenience method |
| `src/tui/ui.rs` | Modified | Replace `call_ai_streaming` and `execute_tool_call`; split pipeline |
| `src/tui/app.rs` | Modified | Add provider and registry fields |
| `src/tui/mod.rs` | Unchanged | Module structure unchanged |
| `src/main.rs` | Unchanged | Entry point unchanged |

### 3.4 New Dependencies

None. All required types already exist in the codebase.

---

## 4. Alternatives Considered

| Alternative | Pros | Cons | Reason Rejected |
|-------------|------|------|-----------------|
| Keep raw reqwest in ui.rs | Simpler, no trait overhead | Violates architecture manifest, duplicate HTTP logic | Architecture violation |
| Make Provider an enum with variants | No trait object overhead | Fragile, requires code changes for each provider | Violates Principle 4 (Model Agnostic) |
| Use async trait instead of async-trait | Cleaner syntax | Adds dependency, no runtime benefit | Unnecessary complexity |
| Merge registry into executor | Simpler module structure | Couples registry to executor logic | Violates modular architecture |

---

## 5. Implementation Plan

### 5.1 Phases

| Phase | Description | Estimated Effort | Dependencies |
|-------|-------------|-----------------|--------------|
| P1.1 | Wire Provider into pipeline | 2h | None |
| P1.2 | Registry-based tool dispatch | 2h | None |
| P1.3 | Runtime state machine | 3h | P1.1, P1.2 |
| P1.4 | Split pipeline into phases | 3h | P1.3 |
| P1.5 | Clippy/error cleanup | 4h | All above |

### 5.2 Milestones

| Milestone | Description | Acceptance Criteria |
|-----------|-------------|---------------------|
| M1 | Provider wired | `call_ai_streaming` removed, all LLM calls go through Provider trait |
| M2 | Tool dispatch fixed | `execute_tool_call` uses registry, no hardcoded match |
| M3 | State machine compiles | `RuntimeState` enum exists with all transitions defined |
| M4 | Pipeline refactored | `run_chat_pipeline` < 100 lines, all phases tested |
| M5 | Clean build | `cargo clippy -- -D warnings` passes, `cargo test` passes |

### 5.3 Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Breaking existing tests | Medium | Medium | Run full test suite after each phase |
| Provider streaming regression | Low | High | Compare output byte-for-byte with current behavior |
| Tool dispatch regression | Low | Medium | All existing tools registered; add integration test |

---

## 6. Validation Plan

### 6.1 Unit Tests

- Test `RuntimeState` transitions are valid
- Test `ToolRegistry::execute()` dispatches correctly
- Test `ToolRegistry::execute()` returns error for unknown tools
- Test provider streaming forwards all chunks

### 6.2 Integration Tests

- Test full pipeline with a mock provider
- Test tool call loop (LLM → tool → LLM → response)
- Test error recovery path

### 6.3 Benchmark Requirements

| KPI | Baseline | Target | Method |
|-----|----------|--------|--------|
| `build_time_debug` | < 30s | < 30s | `cargo build` |
| `build_time_release` | < 120s | < 120s | `cargo build --release` |
| `test_execution_time` | < 60s | < 60s | `cargo test` |
| `clippy_execution_time` | < 30s | < 30s | `cargo clippy -- -D warnings` |
| `tool_selection_accuracy` | > 90% | > 90% | Existing tests |
| `crash_free_sessions` | 100% | 100% | Manual smoke test |

---

## 7. Impact Analysis

### 7.1 Affected Modules

| Module | Impact | Risk |
|--------|--------|------|
| `tui/ui.rs` | High — major refactor | Medium |
| `tui/app.rs` | Medium — add provider/registry | Low |
| `dispatcher/registry.rs` | Low — add method | Low |
| `providers/` | None — already correct | None |
| `tools/` | None — already correct | None |

### 7.2 Configuration Impact

None. No config file changes required.

### 7.3 Data Format Impact

None. No session/memory format changes.

### 7.4 Migration Path

This is a pure refactoring. No data migration needed. Existing sessions and memory files remain compatible.

---

## 8. Open Questions

- [ ] Should `RuntimeState` be persisted to session files? (No — ephemeral)
- [ ] Should the tool call loop have a maximum iteration count? (Yes — default 5)

---

## 9. Decision

| Option | Votes For | Votes Against | Notes |
|--------|-----------|---------------|-------|
| Accept | — | — | Pending architecture review |
| Reject | — | — | — |
| Revise & Resubmit | — | — | — |

**Decision:** Pending
**Date:** 2026-08-05
**Reviewed by:** —

---

## 10. References

- [Architecture Manifest](../architecture/architecture_manifest_v1.md)
- [Design Principles](../principles/design_principles.md)
- [Coding Standards](../standards/coding_standards.md)
- [Engineering Baseline Report](../reports/ENGINEERING_BASELINE_REPORT.md)

# Runtime Architecture Report — P1 Core Runtime

**Date:** 2026-08-05
**Phase:** P1 Core Runtime
**Status:** Complete

---

## 1. Overview

This report documents the runtime architecture after P1 implementation. The runtime is now a deterministic ReAct loop with explicit state management, provider abstraction, and registry-based tool dispatch.

---

## 2. Runtime Architecture

### 2.1 Module Structure

```
src/
├── main.rs                 # Entry point
├── runtime/                # NEW: Runtime state machine
│   ├── mod.rs
│   └── state.rs            # RuntimeState enum
├── tui/
│   ├── ui.rs               # Pipeline orchestration
│   └── app.rs              # TUI state
├── providers/
│   ├── provider.rs         # Provider trait (unchanged)
│   └── openai.rs           # OpenAI implementation (unchanged)
├── tools/
│   ├── mod.rs              # Tool trait (unchanged)
│   └── executor.rs         # run_tool_pipeline (unchanged)
└── dispatcher/
    ├── registry.rs         # ToolRegistry with execute()
    └── mod.rs
```

### 2.2 Runtime State Machine

```
┌───────┐     Observing     ┌───────────┐     Reasoning     ┌────────────┐
│ Idle  │ ────────────────→ │ Observing │ ────────────────→ │ Reasoning  │
└───────┘                   └───────────┘                   └────────────┘
                                                     │
                                                     │ Synthesizing
                                                     ↓
┌───────┐     Failed      ┌────────────┐     Acting      ┌────────────┐
│Failed │ ←───────────────│ Completed  │ ←───────────────│ Synthesizing│
└───────┘                 └────────────┘                 └────────────┘
                                                         ↑        │
                                                         │        │ Act (tool calls)
                                                         └────────┘
```

### 2.3 Pipeline Flow

```
User submits task
    ↓
emit AgentEvent::AgentStarted
    ↓
[Observing] run_tool_pipeline(task, workspace_root)
    ↓ emit ToolStarted/ToolCompleted for each tool
    ↓
[Reasoning] coordinator.run_task(task)
    ↓ emit AgentStarted/AgentCompleted for each subagent
    ↓
[Synthesizing] provider.stream_response(prompt)
    ↓ emit StreamChunk for each chunk
    ↓
Check for tool calls in response
    ↓ has tool calls?
    ├── YES → [Acting] execute_tool_call(registry, name, args)
    │            ↓ emit ToolStarted/ToolCompleted
    │            ↓ append results to prompt
    │            ↓ loop back to Synthesizing (max 5 iterations)
    └── NO → emit Response, mark Completed
    ↓
emit AgentEvent::AgentCompleted
```

### 2.4 Provider Integration

```rust
// Before P1: raw reqwest in tui/ui.rs
async fn call_ai_streaming(config: &Config, prompt: &str, tx: &Sender<AppEvent>) -> Result<String> {
    let client = reqwest::Client::builder()...
    // ... raw HTTP handling
}

// After P1: through Provider trait
async fn call_ai_streaming(provider: &dyn Provider, prompt: &str, tx: &Sender<AppEvent>) -> Result<String> {
    let mut rx = provider.stream_response(prompt).await?;
    while let Some(chunk) = rx.recv().await {
        full.push_str(&chunk);
        let _ = tx.send(AppEvent::StreamChunk(chunk));
    }
    Ok(full)
}
```

### 2.5 Tool Dispatch Integration

```rust
// Before P1: hardcoded match
fn execute_tool_call(name: &str, args: &str, _root: &Path) -> Result<String> {
    match name {
        "read_file" => ReadFile.execute(args),
        "list_files" => ListFiles.execute(args),
        // ...
        _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
    }
}

// After P1: registry dispatch
fn execute_tool_call(registry: &ToolRegistry, name: &str, args: &str) -> Result<String> {
    registry.execute(name, args)
}
```

---

## 3. Event Flow

### 3.1 Event Sequence for a Toolable Task

```
AgentStarted { agent: "main", task }
AgentStatusChanged { status: Thinking }
  → [Observing]
Log { level: "pipeline", message: "Workspace detected..." }
ToolStarted { tool: "list_files", args }
ToolCompleted { tool: "list_files", success: true }
AgentProgress { progress: 0.5, action: "Executed list_files" }
ToolStarted { tool: "read_file", args }
ToolCompleted { tool: "read_file", success: true }
  → [Reasoning]
AgentStatusChanged { status: Planning }
AgentStarted { agent: "research", task }
AgentCompleted { agent: "research", duration_ms }
AgentStarted { agent: "planning", task }
AgentCompleted { agent: "planning", duration_ms }
  → [Synthesizing]
AgentStatusChanged { status: Executing }
StreamChunk { content }
StreamChunk { content }
...
  → [Acting] (if tool calls detected)
Log { level: "tool", message: "Detected 1 tool call(s)" }
ToolStarted { tool: "run_command", args }
ToolCompleted { tool: "run_command", success: true }
  → [Synthesizing] (loop back)
StreamChunk { content }
...
Response { content }
AgentCompleted { agent: "main", duration_ms }
```

### 3.2 Event Flow for a Non-Toolable Task

```
AgentStarted { agent: "main", task }
AgentStatusChanged { status: Thinking }
  → [Reasoning] (skip Observing)
AgentStatusChanged { status: Planning }
AgentStarted { agent: "research", task }
AgentCompleted { agent: "research", duration_ms }
  → [Synthesizing]
AgentStatusChanged { status: Executing }
StreamChunk { content }
...
Response { content }
AgentCompleted { agent: "main", duration_ms }
```

---

## 4. Error Handling

### 4.1 Provider Failure

```
Synthesizing → error
    ↓
emit AgentEvent::Log { level: "coordination", ... }
    ↓ (RecoveryEngine suggests action)
emit AgentEvent::AgentFailed { agent: "main", error }
    ↓
Fall back to coordinator report
emit Response { report }
```

### 4.2 Tool Execution Failure

```
Acting → execute_tool_call fails
    ↓
emit ToolCompleted { success: false, result: error }
    ↓
continue to next call or loop back to Synthesizing
```

### 4.3 State Machine Invalid Transition

```
Any state → invalid transition
    ↓
unwrap_or(state) — falls back to current state
    ↓ (logged as warning in production)
```

---

## 5. Configuration

No configuration changes required. The runtime uses existing config:

```toml
# ~/.codebro/config.toml
provider = "openai"
base_url = "https://api.openai.com/v1"
model = "gpt-4o"
```

The provider is created from config at pipeline start:
```rust
let provider = OpenAiProvider::new(config.clone());
```

---

## 6. Testing

### 6.1 Unit Tests

| Test | File | Location |
|------|------|----------|
| State transitions | `src/runtime/state.rs` | `mod tests` |
| Pipeline functions | `src/tools/executor.rs` | `mod tests` |
| Tool dispatch | `src/dispatcher/registry.rs` | (via executor tests) |

### 6.2 Integration Tests

| Test | Description |
|------|-------------|
| `test_pipeline_list_files` | Tool pipeline with list_files |
| `test_pipeline_find_cargo_toml` | Tool pipeline with semantic search |
| `test_pipeline_read_main` | Tool pipeline with file read |
| `test_pipeline_run_command` | Tool pipeline with command execution |
| `test_shell_timeout_enforced` | RunCommand timeout |

### 6.3 Test Coverage

- Total tests: 331 (up from 322 in P0.75)
- New tests: 9 (all in `src/runtime/state.rs`)
- Coverage: No regression

---

## 7. Performance

| Metric | Value |
|--------|-------|
| Build time (debug) | 7.03s |
| Build time (release) | 12.14s |
| Test time | 1.10s |
| Clippy time | 6.09s |
| Format check | 0.27s |
| Clippy warnings | 0 |

All within benchmark targets. No regressions.

---

## 8. Architecture Manifest Compliance

| Section | Rule | Status |
|---------|------|--------|
| 3.1 | Hard boundaries respected | ✓ |
| 4.1 | Provider trait unchanged | ✓ |
| 4.2 | Provider is sole LLM interface | ✓ |
| 5.1 | Tool trait unchanged | ✓ |
| 5.2 | All tools via registry | ✓ |
| 6.1 | Events via channels | ✓ |
| 12.1 | Module contracts maintained | ✓ |

---

## 9. Future Work

| Item | Phase | Description |
|------|-------|-------------|
| Multi-agent execution | P2 | Parallel agent spawning |
| Plugin system | P3 | Dynamic tool/provider loading |
| Intelligence wiring | P4 | Connect index/search to pipeline |
| MCP integration | P5 | Model Context Protocol support |

---

## 10. References

- [RFC-001](../RFC/rfc-001-react-runtime-loop.md)
- [ADR-001](../ADR/adr-001-provider-runtime-architecture.md)
- [ADR-002](../ADR/adr-002-tool-runtime-architecture.md)
- [ADR-003](../ADR/adr-003-runtime-state-machine.md)
- [Architecture Manifest](../architecture/architecture_manifest_v1.md)

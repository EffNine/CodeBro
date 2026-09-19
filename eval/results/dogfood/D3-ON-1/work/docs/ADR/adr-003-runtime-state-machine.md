# ADR-003: Runtime State Machine

**Document:** `docs/ADR/adr-003-runtime-state-machine.md`
**Version:** 1.0.0
**Part of:** CodeBro P1 Core Runtime
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-001

---

## 1. Context

### 1.1 Background

The current runtime pipeline (`run_chat_pipeline` in `tui/ui.rs`) has no explicit state tracking. It is a single async function that flows linearly through tool execution, LLM synthesis, and optional tool-call loops. There is no way to:

1. Determine what phase the runtime is in at any point.
2. Validate that state transitions are correct.
3. Recover from errors at specific phases.
4. Test the pipeline deterministically.

This is a known violation of Design Principle 10 (Small, Composable Components) — the pipeline is 240+ lines and handles too many concerns.

### 1.2 Constraints

- No new dependencies.
- State transitions must be deterministic.
- The state machine must be internal to the runtime — no persistence required.
- All existing tests must continue to pass.

### 1.3 Stakeholders

- **TUI module**: Uses state machine to drive the pipeline
- **Agent module**: Emits events that may trigger state transitions
- **Tests**: Verify state transition validity

---

## 2. Decision

### 2.1 Decision Statement

Define a `RuntimeState` enum with explicit transitions and implement it as the driving logic for `run_chat_pipeline`. The state machine replaces the implicit linear flow with explicit phases.

### 2.2 Rationale

1. **Testability**: Each state transition can be tested independently.
2. **Debuggability**: The current state is always known and can be logged.
3. **Correctness**: Invalid transitions are caught at compile time (via the enum).
4. **Maintainability**: Adding a new phase only requires adding a state and transition.

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)**: State machine is a clear module boundary.
- **Principle 8 (Observable AI Actions)**: State transitions are events that can be logged.
- **Principle 9 (Performance Matters)**: State machine adds negligible overhead.
- **Principle 10 (Small, Composable Components)**: Splits monolithic pipeline into phased functions.

---

## 3. Consequences

### 3.1 Positive Consequences

- Explicit, testable state transitions.
- Clear separation of pipeline phases.
- Easier to add new phases (e.g., multi-agent, plugin hooks).
- Better error recovery (can retry from specific states).

### 3.2 Negative Consequences

- Slightly more code (state enum + transition logic).
- Need to thread state through pipeline functions.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Code size | +~50 lines for state machine | Offset by removing 240-line monolith |
| Complexity | New abstraction layer | Simple enum with sealed transitions |
| Performance | Negligible overhead | One enum check per transition |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `tui/ui.rs` | Add `RuntimeState`; refactor pipeline to use it |
| `tui/app.rs` | Track current runtime state in TuiApp |
| `agent/events.rs` | May add `RuntimeStateChanged` event variant |

### 3.5 Impact on Future Work

- P2 multi-agent: State machine can include multi-agent states.
- P3 plugin system: Plugin hooks can be inserted between states.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| No state machine | Keep current linear flow | Simpler | Untestable, unobservable | Fails Principle 10 |
| Status enum in TuiApp | Reuse AgentStatus | No new type | AgentStatus is for agents, not runtime | Semantic mismatch |
| Event-driven states | Derive state from events | Loose coupling | Harder to reason about validity | Less deterministic |

---

## 5. Implementation Notes

### 5.1 State Machine Definition

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeState {
    Idle,
    Observing,      // Tool pipeline gathering ground truth
    Reasoning,      // Coordinator/subagents analyzing
    Synthesizing,   // LLM streaming response
    Acting,         // Executing tool calls from response
    Completed,
    Failed,
}

impl RuntimeState {
    pub fn transitions_from(&self) -> &'static [RuntimeState] {
        match self {
            RuntimeState::Idle => &[RuntimeState::Observing],
            RuntimeState::Observing => &[RuntimeState::Reasoning],
            RuntimeState::Reasoning => &[RuntimeState::Synthesizing],
            RuntimeState::Synthesizing => &[RuntimeState::Acting, RuntimeState::Completed],
            RuntimeState::Acting => &[RuntimeState::Synthesizing],
            RuntimeState::Completed | RuntimeState::Failed => &[],
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, RuntimeState::Completed | RuntimeState::Failed)
    }
}
```

### 5.2 Transition Validation

```rust
impl RuntimeState {
    pub fn try_transition(self, next: RuntimeState) -> Result<RuntimeState, RuntimeError> {
        if self.transitions_from().contains(&next) {
            Ok(next)
        } else {
            Err(RuntimeError::InvalidTransition { from: self, to: next })
        }
    }
}
```

### 5.3 Pipeline Integration

```rust
async fn run_chat_pipeline(
    config: &Config,
    task: &str,
    tx: &Sender<AppEvent>,
) {
    let mut state = RuntimeState::Idle;
    
    // ... each phase updates state:
    state = state.try_transition(RuntimeState::Observing)?;
    let tool_context = observe(task).await?;
    
    state = state.try_transition(RuntimeState::Reasoning)?;
    let report = reason(task, &tool_context).await?;
    
    // ... etc
}
```

---

## 6. References

- [Architecture Manifest Section 6](../../architecture/architecture_manifest_v1.md#6-event-system)
- [Design Principle 10](../../principles/design_principles.md#principle-10-small-composable-components)
- [RFC-001](../../RFC/rfc-001-react-runtime-loop.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |

# ADR-001: Runtime Architecture

**ADR Number:** ADR-001
**Title:** Runtime Architecture
**Author:** CodeBro Engineering
**Status:** Proposed
**Created:** 2026-08-07
**Updated:** 2026-08-07
**Part of:** Design Summit v2
**Supersedes:** None
**Related:** ADR-002, ADR-003, ADR-004

---

## 1. Context

### 1.1 Background

CodeBro v1.0 established the Platform Foundation with a deterministic ReAct loop, provider abstraction, and plugin SDK. The runtime currently consists of:

- `src/runtime/state.rs` — Runtime state machine (Idle → Observing → Reasoning → Synthesizing → Acting → Completed/Failed)
- `src/agent/coordinator.rs` — Agent coordination
- `src/providers/provider.rs` — Provider trait
- `src/observability/` — Event bus, metrics, tracing
- `src/plugin_sdk/` — Plugin lifecycle

### 1.2 Problem

The v1.0 runtime lacks:

1. **AI orchestration** — No cost-aware routing, no failover, no budget control
2. **Memory management** — Basic JSON persistence, no tiers, no eviction policy
3. **Context assembly** — No intelligent context building, no window budgeting
4. **Provider management** — No health monitoring, no metrics, no discovery
5. **Agent orchestration** — Sequential only, no parallel execution, no inter-agent communication
6. **Communication fabric** — Ad-hoc event passing, no dead-letter handling

### 1.3 Constraints

- v1.0 Platform Foundation is frozen — no breaking changes
- All new components must be additive
- Trait abstractions must be preserved
- Observability must be built-in

### 1.4 Stakeholders

- **AI Runtime** — Orchestrates LLM calls
- **Memory Runtime** — Manages persistent knowledge
- **Context Runtime** — Assembles relevant context
- **Provider Runtime** — Manages LLM providers
- **Agent Runtime** — Orchestrates multi-agent execution
- **TUI** — Displays runtime state

---

## 2. Decision

### 2.1 Decision Statement

The CodeBro Runtime v2 adopts a **layered, trait-abstracted, event-driven architecture** with five new runtime subsystems (AI, Memory, Context, Provider, Agent) built on top of the frozen Platform Foundation.

### 2.2 Rationale

1. **Layered architecture** provides clear separation of concerns and enables independent testing
2. **Trait abstraction** preserves the v1.0 principle of interchangeability
3. **Event-driven communication** enables decoupling and observability
4. **Explicit state management** through RuntimeContext enables determinism and debugging
5. **Additive design** preserves backward compatibility

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)** — Each runtime subsystem is a distinct module
- **Principle 8 (Observable AI Actions)** — All runtime operations emit events
- **Principle 9 (Performance Matters)** — Components are designed for minimal overhead
- **Principle 10 (Small, Composable Components)** — Each subsystem is focused and composable

---

## 3. Architecture

### 3.1 Module Structure

```
src/runtime/
├── mod.rs                  # Runtime assembly
├── ai/                     # AI Runtime
│   ├── orchestrator.rs     # LLM call orchestration
│   ├── router.rs           # Cost-aware routing
│   ├── budget.rs           # Cost tracking and limits
│   ├── failover.rs         # Provider failover
│   └── streaming.rs        # Streaming response handling
├── memory/                 # Memory Runtime
│   ├── tiers.rs            # Tier definitions
│   ├── evictor.rs          # Eviction policy
│   ├── summarizer.rs       # Context summarization
│   └── persistence.rs      # JSON persistence
├── context/                # Context Runtime
│   ├── assembler.rs        # Context assembly
│   ├── budget.rs           # Context window budgeting
│   ├── prioritizer.rs      # Relevance prioritization
│   └── compressor.rs       # Context compression
├── provider/               # Provider Runtime
│   ├── discovery.rs        # Dynamic provider discovery
│   ├── health.rs           # Health monitoring
│   ├── metrics.rs          # Usage metrics
│   └── failover.rs         # Provider failover
├── agent/                  # Agent Runtime
│   ├── orchestrator.rs     # Multi-agent orchestration
│   ├── communication.rs    # Inter-agent messaging
│   ├── lifecycle.rs        # Agent lifecycle
│   └── resource.rs         # Resource management
└── lifecycle/              # Runtime Lifecycle
    ├── manager.rs          # Lifecycle state machine
    ├── startup.rs          # Startup sequence
    └── shutdown.rs         # Shutdown sequence
```

### 3.2 RuntimeContext

The central shared state container:

```rust
pub struct RuntimeContext {
    pub config: Arc<RuntimeConfig>,
    pub plugin_registry: PluginRegistry,
    pub hook_dispatcher: HookDispatcher,
    pub provider_runtime: ProviderRuntime,
    pub memory_runtime: MemoryRuntime,
    pub context_runtime: ContextRuntime,
    pub agent_runtime: AgentRuntime,
    pub event_bus: EventBus,
    pub diagnostics: RuntimeDiagnostics,
    pub security_auditor: SecurityAuditor,
}
```

### 3.3 Communication Model

```
Publisher → EventBus → Subscriber
     ↓
Channel (request/reply)
     ↓
DeadLetterStore (unreachable)
```

---

## 4. Consequences

### 4.1 Positive Consequences

- Clear separation between v1.0 foundation and v2.0 extensions
- Trait-based design enables future provider/agent swaps
- Event-driven architecture enables observability
- RuntimeContext provides single source of truth
- Deterministic lifecycle enables testing

### 4.2 Negative Consequences

- Increased complexity (5 new runtime subsystems)
- Additional indirection through traits
- RuntimeContext requires careful sharing

### 4.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Complexity | More modules to maintain | Clear boundaries, focused responsibilities |
| Indirection | Trait overhead | Negligible; bounds-checked critical paths |
| Sharing | Arc<Mutex> overhead | Cheap clone; lock contention minimized |
| Context size | Large RuntimeContext | Pass references, not clones |

### 4.4 Impact on v1.0

| Module | Impact |
|--------|--------|
| `src/runtime/state.rs` | No change; extended by lifecycle manager |
| `src/providers/provider.rs` | No change; wrapped by provider runtime |
| `src/agent/coordinator.rs` | No change; wrapped by agent runtime |
| `src/agent/memory.rs` | No change; migrated by memory runtime |
| `src/observability/` | No change; used by all runtimes |
| `src/plugin_sdk/` | No change; used for extension |

---

## 5. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| Monolithic runtime | Single runtime module | Simpler | Tight coupling | Violates modularity |
| No RuntimeContext | Spread state across components | Simpler types | No single source of truth | Debugging nightmare |
| Direct calls | Components call each other directly | Simpler | Tight coupling | Violates event-driven principle |
| Immutable runtime | All state immutable | Thread-safe | Copy overhead | Unnecessary for single-process |

---

## 6. Implementation Notes

### 6.1 Code Patterns

```rust
// Runtime components share context via Arc
pub struct AIOrchestrator {
    context: Arc<RuntimeContext>,
}

// Events flow through the event bus
context.event_bus.publish(RuntimeEvent::ProviderCallStarted { provider: "openai" });

// State transitions are explicit
state = state.try_transition(RuntimeState::Running)?;
```

### 6.2 Anti-Patterns

```rust
// NEVER: Bypass event bus
provider.call(prompt).await?;  // Silent call

// ALWAYS: Emit events
event_bus.publish(CallStarted);
let result = provider.call(prompt).await;
event_bus.publish(CallCompleted { result });
```

### 6.3 Migration Steps

1. Define `RuntimeContext` struct
2. Create `RuntimeManager` to own all runtimes
3. Implement AI Runtime with orchestrator, router, budget, failover
4. Implement Memory Runtime with tiers, eviction, persistence
5. Implement Context Runtime with assembler, budget, prioritizer, compressor
6. Extend Provider Runtime with discovery, health, metrics
7. Extend Agent Runtime with orchestrator, communication, lifecycle
8. Integrate with existing IntegrationPipeline
9. Add observability hooks to all new components
10. Write integration tests

---

## 7. References

- [Runtime Architecture](../summit/RuntimeArchitecture.md)
- [Runtime Layers](../summit/RuntimeLayers.md)
- [Runtime Principles](../summit/RuntimePrinciples.md)
- [ADR-003: Runtime State Machine](./adr-003-runtime-state-machine.md)
- [Runtime Architecture Report](../reports/RuntimeArchitectureReport.md)

---

## 8. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-07 | Created | CodeBro Engineering |

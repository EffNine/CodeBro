# CodeBro Runtime Principles

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-07
**Part of:** Design Summit v2
**Owner:** CodeBro Engineering

---

## 1. Existing Principles (v1.0 — Frozen)

These principles from the Platform Foundation remain in effect:

1. **Zero Configuration** — Everything works out of the box
2. **Developer First** — The developer is the user, not the machine
3. **Human in Control** — Every change requires approval
4. **Cost Transparency** — All costs are visible and auditable
5. **Progressive Discovery** — Features appear when relevant
6. **Observable AI** — All actions are logged and traceable
7. **Adaptive, Not Autonomous** — AI suggests, human decides
8. **Platform before Features** — Foundation before functionality
9. **Deterministic before AI** — Rule-based before probabilistic
10. **Everything from TUI** — All subsystems accessible from UI

---

## 2. Runtime Principles

### 2.1 Statelessness at the Core

**Statement:** The runtime core is stateless; all state is managed externally through the RuntimeContext.

**Rationale:** Statelessness enables horizontal scaling, deterministic testing, and fault recovery. State is held in explicit stores (MemoryRuntime, ProviderRegistry) rather than hidden in component internals.

**Implications:**
- Runtime components must be Clone (via Arc) for sharing
- No implicit state in runtime methods
- All state changes must be auditable

**Anti-Pattern:**
```rust
// NEVER: Hidden state in runtime component
pub struct AIOrchestrator {
    last_provider: String,  // Implicit state
    call_count: usize,       // Implicit state
}
```

**Pattern:**
```rust
// ALWAYS: State in explicit store
pub struct AIOrchestrator {
    context: Arc<RuntimeContext>,  // Explicit shared state
}
```

### 2.2 Explicit Transitions

**Statement:** All state transitions must be explicit, typed, and auditable.

**Rationale:** Explicit transitions enable debugging, recovery, and determinism. Implicit state changes are the source of most runtime bugs.

**Implications:**
- Use enum-based state machines
- Validate transitions before executing
- Log all transitions

**Anti-Pattern:**
```rust
// NEVER: Implicit state change
self.status = "running";  // No validation, no log
```

**Pattern:**
```rust
// ALWAYS: Explicit transition
state = state.try_transition(RuntimeState::Running)?;
emit(RuntimeEvent::StateTransition { from: old, to: RuntimeState::Running });
```

### 2.3 Failure as First-Class

**Statement:** Failure is a first-class concept with explicit handling, not an afterthought.

**Rationale:** AI systems are probabilistic; failures are expected. Treating failure as first-class enables graceful degradation and recovery.

**Implications:**
- Every async operation returns Result
- Failover is automatic, not manual
- Recovery policies are configurable
- Partial failures are distinguished from total failures

**Anti-Pattern:**
```rust
// NEVER: Panic on failure
provider.stream_response(prompt).await.unwrap();
```

**Pattern:**
```rust
// ALWAYS: Handle failure gracefully
match provider.stream_response(prompt).await {
    Ok(stream) => handle_stream(stream).await,
    Err(e) => failover_to_next(e).await,
}
```

### 2.4 Trait-Based Interchangeability

**Statement:** Components are interchangeable via trait abstraction; consumers depend only on traits.

**Rationale:** Trait-based design enables swap-in implementations, mocking for tests, and future extensibility.

**Implications:**
- All public interfaces are traits
- Concrete implementations are private to modules
- Tests use mock implementations

**Anti-Pattern:**
```rust
// NEVER: Depend on concrete type
fn process(provider: OpenAiProvider) { ... }
```

**Pattern:**
```rust
// ALWAYS: Depend on trait
fn process(provider: &dyn Provider) { ... }
```

### 2.5 Observability by Default

**Statement:** All runtime operations are observable without configuration.

**Rationale:** Unobservable systems are untrustworthy systems. Observability must be built-in, not bolted-on.

**Implications:**
- Every public method emits events
- Metrics are automatic, not opt-in
- Tracing spans are created automatically
- Diagnostics are always available

**Anti-Pattern:**
```rust
// NEVER: Silent operation
provider.call(prompt).await?;
```

**Pattern:**
```rust
// ALWAYS: Observable operation
let span = tracer.span("provider.call");
event_bus.publish(ProviderCallStarted { provider: id });
let result = provider.call(prompt).await;
event_bus.publish(ProviderCallCompleted { result: &result });
span.finish();
```

---

## 3. Provider Principles

### 3.1 Provider is a Trait

**Statement:** The Provider trait is the sole interface to LLM communication; no raw HTTP outside the provider module.

**Rationale:** Trait abstraction enables model-agnostic design and future provider additions.

**Implications:**
- All LLM calls go through Provider trait
- No reqwest calls in tui/, agent/, or runtime/
- Provider implementations are swappable

### 3.2 Routing is Configurable

**Statement:** Provider selection is driven by policy, not hardcoded.

**Implications:**
- Routing strategies are pluggable
- Cost, latency, and quality are routing factors
- Default is cost-optimal with quality floor

### 3.3 Failover is Automatic

**Statement:** Provider failures trigger automatic failover with no user intervention.

**Implications:**
- Health checks run continuously
- Failed providers are marked unhealthy
- Next available provider is selected transparently

### 3.4 Costs are Tracked Per-Request

**Statement:** Every LLM call tracks its cost; budgets are enforced globally.

**Implications:**
- Cost is calculated from provider metadata + usage
- Running totals are maintained
- Thresholds trigger warnings and stops

---

## 4. Memory Principles

### 4.1 Memory is Tiered

**Statement:** Memory is organized into tiers based on relevance and lifetime.

| Tier | Scope | Lifetime | Eviction |
|------|-------|----------|----------|
| Short-term | Session | Per-session | LRU, max 100 entries |
| Project | Project | Until change | Importance-weighted |
| Global | System | Indefinite | Confidence-based |

### 4.2 Persistence is Explicit

**Statement:** Memory is saved only through explicit save() calls; no implicit persistence.

**Implications:**
- Save is a first-class operation
- Save failures are logged but not fatal
- Atomic writes prevent corruption

### 4.3 Eviction is Deterministic

**Statement:** Memory eviction follows deterministic policies, not arbitrary deletion.

**Implications:**
- LRU for short-term
- Importance-weighted for project
- Confidence threshold for global

### 4.4 Privacy by Design

**Statement:** Sensitive data is never persisted without explicit consent.

**Implications:**
- Code content is not stored in memory
- Only metadata and patterns are stored
- User can purge all memory at any time

---

## 5. Agent Principles

### 5.1 Agents are Composable

**Statement:** Agents are small, focused components that can be composed into workflows.

**Implications:**
- Each agent has a single responsibility
- Agents communicate via message bus
- Agents can be chained or parallelized

### 5.2 Communication is Event-Driven

**Statement:** Agent communication uses events, not polling.

**Implications:**
- Agents publish messages to the bus
- Other agents subscribe to relevant channels
- No blocking waits between agents

### 5.3 Lifecycle is Managed

**Statement:** Agent lifecycle is managed by the runtime, not manual.

**Implications:**
- Runtime handles spawn, monitor, cleanup
- Agents declare their resource requirements
- Orphaned agents are detected and cleaned

### 5.4 Observability is Built-In

**Statement:** Every agent action is observable.

**Implications:**
- Agent events flow through the event bus
- Agent traces are preserved for debugging
- Agent performance is tracked

---

## 6. Extension Rules

### 6.1 Plugin-Only Extension

**Statement:** All extensions to core behavior must go through the Plugin SDK.

**Implications:**
- No modification to frozen modules
- Plugins declare capabilities in manifest
- Plugins are isolated via sandbox

### 6.2 Trait Extension Points

**Statement:** Extension points are defined as traits; implementations are pluggable.

**Implications:**
- New provider types implement Provider trait
- New agent types implement SubAgent trait
- New memory stores implement MemoryStore trait

### 6.3 Hook-Based Integration

**Statement:** Runtime integration happens via hooks, not direct calls.

**Implications:**
- Plugins register hooks for phases
- Hooks are dispatched in order
- Hooks can block, modify, or observe

### 6.4 Version Compatibility

**Statement:** Plugins declare required SDK version; runtime enforces compatibility.

**Implications:**
- Plugin manifest includes `required_sdk_version`
- Runtime checks version before loading
- Incompatible plugins are rejected

---

## 7. Security Rules

### 7.1 Least Privilege

**Statement:** Every component operates with the minimum permissions required.

**Implications:**
- Plugins declare required permissions
- Runtime enforces permission checks
- No implicit privilege escalation

### 7.2 Sandbox Enforcement

**Statement:** Untrusted code runs in a sandbox with restricted access.

**Implications:**
- Plugin code is isolated from core
- Filesystem access is scoped to workspace
- Network access requires explicit permission

### 7.3 Audit Trail

**Statement:** All state-changing operations are recorded in an immutable audit log.

**Implications:**
- Audit log is append-only
- Log entries include correlation ID
- Log is accessible via TUI

### 7.4 Anomaly Detection

**Statement:** Runtime monitors for anomalous behavior patterns.

**Implications:**
- Baseline behavior is learned over time
- Deviations trigger alerts
- Suspicious operations are logged

### 7.5 Budget Guards

**Statement:** Cost limits are hard constraints, not suggestions.

**Implications:**
- Daily, session, and per-task budgets
- Exceeding budget stops execution
- Overrides require approval and audit

---

## 8. Principle Conflict Resolution

When principles conflict, the following hierarchy applies:

1. **Human in Control** — Overrides all other principles
2. **Security** — Overrides performance and convenience
3. **Deterministic before AI** — Rule-based wins over probabilistic
4. **Cost Transparency** — Visibility wins over efficiency
5. **Observability** — Traceability wins over performance

---

## 9. Principle Compliance Checklist

| Principle | v1.0 | v2.0 Design | Gap |
|-----------|------|-------------|-----|
| Statelessness at Core | Partial | Full | RuntimeContext needed |
| Explicit Transitions | Full | Full | None |
| Failure as First-Class | Partial | Full | Failover layer needed |
| Trait-Based Interchangeability | Full | Full | None |
| Observability by Default | Full | Full | None |
| Provider is Trait | Full | Full | None |
| Routing is Configurable | Partial | Full | Router needed |
| Failover is Automatic | Partial | Full | Health monitor needed |
| Memory is Tiered | Full | Full | None |
| Agents are Composable | Partial | Full | Message bus needed |

---

*Runtime Principles v2 — Design Summit*

# Communication Review — Runtime Layer

**Version:** 1.0.0
**Status:** Audit Complete
**Date:** 2026-08-07
**Scope:** P10.0 Runtime Foundation, P10.1 AI Runtime, P10.2 Memory Runtime

---

## 1. Communication Architecture Overview

The approved architecture defines five communication mechanisms:

| Mechanism | Pattern | Implementation |
|-----------|---------|---------------|
| Runtime Events | Pub/Sub | `RuntimeEvent` enum + `EventBus` |
| Runtime Context | Shared snapshot | `RuntimeContext` (per-task, cloned) |
| Memory Resolution | Query/Response | `MemoryResolver::resolve()` |
| AI Runtime | Request/Response | `RuntimeRouter::route()` |
| Observability | Side-channel | `EventBus`, `Tracing`, `Metrics` |

---

## 2. Runtime Events

### 2.1 Event Enum Structure

**File:** `src/runtime/events.rs`

```rust
pub enum RuntimeEvent {
    PipelineStarted { task_id, correlation_id, user_request_summary },
    StateChange { from: RuntimeState, to: RuntimeState },
    ObserveComplete { tool_context_summary, duration_ms },
    ReasonComplete { report_summary, duration_ms },
    StreamChunk { chunk: String },
    SynthesizeComplete { response_summary, duration_ms, tool_calls_found },
    ToolExecuted { tool_name, args_summary, result_summary, success, duration_ms },
    ActComplete { loop_count, total_tool_calls },
    PipelineCompleted { duration_ms, tool_calls_total, response_length },
    PipelineFailed { error, duration_ms, state_at_failure },
    LifecycleEvent { from, to },
    DiagnosticsCollected { correlation_id, failure_count, recovery_count },
}
```

**Communication Model:** Fire-and-forget event emission to observers (TUI, diagnostics).

### 2.2 Event Flow

```
Pipeline Phase Complete
    ↓
emit RuntimeEvent::StateChange { from, to }
    ↓
observers (TUI, diagnostics) receive event
    ↓
event is cloned for each observer (no shared mutation)
```

### 2.3 Event Emission Points

| Event | Emitted By | Trigger |
|-------|-----------|---------|
| `PipelineStarted` | Pipeline orchestrator | Task begins |
| `StateChange` | State machine | Valid transition |
| `ObserveComplete` | Observe phase | Tool pipeline done |
| `ReasonComplete` | Reason phase | Coordinator done |
| `StreamChunk` | Provider stream | Each chunk received |
| `SynthesizeComplete` | Synthesis phase | Response complete |
| `ToolExecuted` | Tool execution | Each tool call |
| `ActComplete` | Act phase | Loop complete |
| `PipelineCompleted` | Pipeline orchestrator | Success |
| `PipelineFailed` | Pipeline orchestrator | Failure |
| `LifecycleEvent` | Lifecycle manager | State transition |
| `DiagnosticsCollected` | Diagnostics collector | Periodic flush |

**All events are Clone and Send. Thread-safe emission. PASS**

---

## 3. Runtime Context

### 3.1 Context Structure

**File:** `src/runtime/context.rs`

```rust
pub struct RuntimeContext {
    pub task_id: String,
    pub correlation_id: String,
    pub user_request: String,
    pub created_at: DateTime<Utc>,
    pub tool_context: Arc<Option<String>>,
    pub reason_report: Arc<Option<String>>,
    pub synthesized_response: Arc<String>,
    pub act_loop_count: u32,
    pub max_act_loops: u32,
    pub timeout_manager: TimeoutManager,
    pub health_monitor: HealthMonitor,
    pub resource_guard: ResourceGuard,
    pub shutdown_requested: bool,
}
```

### 3.2 Communication Pattern

| Pattern | Implementation | Status |
|---------|---------------|--------|
| Immutable after construction | `with_*` methods return new context | PASS |
| Cheap clone via Arc | `tool_context`, `reason_report`, `synthesized_response` are `Arc` | PASS |
| Per-task scope | New context per task via `RuntimeContext::new()` | PASS |
| Shared reliability components | `TimeoutManager`, `HealthMonitor`, `ResourceGuard` are Clone | PASS |

### 3.3 Context Factory

**File:** `src/runtime/traits.rs`

```rust
pub trait RuntimeContextFactory: Send + Sync {
    fn create_context(&self, request: &str) -> RuntimeContext;
}

pub struct DefaultContextFactory;
impl RuntimeContextFactory for DefaultContextFactory {
    fn create_context(&self, request: &str) -> RuntimeContext {
        RuntimeContext::new(request)
    }
}
```

**Factory pattern enables testability. PASS**

---

## 4. Memory Resolution

### 4.1 Resolution Interface

**File:** `src/memory_runtime/resolution.rs`

```rust
pub struct MemoryResolver {
    lifecycle: Arc<MemoryLifecycle>,
}

impl MemoryResolver {
    pub fn resolve(&self, query: &MemoryQuery) -> MemoryResolution {
        // Deterministic: Session → Project → Global
        // Returns first match or empty
    }

    pub fn resolve_with_policy(&self, query: &MemoryQuery, policy: &ConflictPolicy) -> MemoryResolution {
        // Collects all matches, applies policy
    }
}
```

### 4.2 Resolution Communication Model

| Aspect | Design | Status |
|--------|--------|--------|
| Input | `MemoryQuery` (value type) | PASS |
| Output | `MemoryResolution` (value type) | PASS |
| Determinism | Fixed tier order (Session > Project > Global) | PASS |
| Thread safety | `Arc<MemoryLifecycle>` with `RwLock` | PASS |
| No side effects | Read-only resolution | PASS |

### 4.3 Event Emission During Resolution

```rust
self.lifecycle.record_event(MemoryEvent::MemoryResolved {
    event_id: uuid::Uuid::new_v4().to_string(),
    query_key: query.key.clone(),
    tier: first_hit.tier,
    hit_count: hits.len(),
    timestamp: 0,
});
```

**Events are recorded but not emitted to external observers. This is an internal diagnostic. PASS**

---

## 5. AI Runtime Communication

### 5.1 Router Interface

**File:** `src/ai_runtime/router.rs`

```rust
pub struct RuntimeRouter {
    candidates: Arc<RwLock<Vec<ModelCandidate>>>,
    config: RoutingConfig,
    diagnostics: Arc<RwLock<RuntimeDiagnostics>>,
    request_history: Arc<RwLock<Vec<(ModelRequest, RoutingDecision)>>>,
}

impl RuntimeRouter {
    pub fn route(&self, request: &ModelRequest) -> AIRRuntimeResult<RoutingDecision> {
        // Scores all candidates
        // Returns best match
        // Records diagnostic event
        // Records in history
    }
}
```

### 5.2 Communication Patterns

| Pattern | Implementation | Status |
|---------|---------------|--------|
| Request/Response | `route(request) → RoutingDecision` | PASS |
| Event emission | `DiagnosticEvent::ModelSelected` | PASS |
| History tracking | `request_history` bounded to 1000 entries | PASS |
| Capability negotiation | `CapabilityNegotiation` embedded in decision | PASS |

### 5.3 Streaming Communication

**File:** `src/ai_runtime/stream.rs`

```rust
pub enum StreamEvent {
    Segment(StreamSegment),
    Complete { total_tokens, total_duration_ms },
    Error { error: String },
    Cancelled,
}

pub struct StreamPipeline {
    // Processes stream segments
    // Emits StreamEvent
}
```

**Stream pipeline is a consumer pattern — not a shared communication channel. PASS**

---

## 6. Observability Communication

### 6.1 EventBus

**File:** `src/observability/event_bus.rs`

```rust
pub struct EventBus {
    inner: Arc<Mutex<EventBusInner>>,
}

impl EventBus {
    pub fn subscribe(&self, observer: EventObserver) { ... }
    pub fn emit(&self, event: &Event) { ... }
    pub fn buffer(&self) -> Vec<Event> { ... }
}
```

**Communication model:** Pub/Sub with bounded buffer (10,000 events). Thread-safe via `Arc<Mutex<>>`. **PASS**

### 6.2 Diagnostics Communication

| Component | Communication | Status |
|-----------|--------------|--------|
| `RuntimeDiagnostics` (runtime) | In-process, Arc-shared | PASS |
| `RuntimeDiagnostics` (ai_runtime) | In-process, Arc-shared | PASS |
| `MemoryDiagnostics` | In-process, RwLock-shared | PASS |
| `EventBus` | Pub/Sub to observers | PASS |

### 6.3 Tracing Integration

**File:** `src/observability/tracing.rs`

```rust
pub struct TraceContext {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub correlation_id: CorrelationId,
}
```

**Tracing spans are created per-operation and correlated via `correlation_id`. PASS**

---

## 7. Cross-Runtime Communication

### 7.1 Current State

| Runtime | Communicates With | Mechanism |
|---------|------------------|-----------|
| `runtime/` | `reliability/` | Direct import (Layer 1) |
| `ai_runtime/` | None (isolated) | N/A — P10.3 integration pending |
| `memory_runtime/` | None (isolated) | N/A — P10.3 integration pending |

### 7.2 Planned P10.3 Communication

| Component | Will Communicate With | Mechanism |
|-----------|----------------------|-----------|
| `AIRRuntime` | `integration_pipeline/` | Direct call (Layer 4 internal) |
| `MemoryRuntime` | `agent/`, `integration_pipeline/` | Trait-based via `MemoryStore` |
| `RuntimeContext` | All runtimes | Passed as parameter (cloned) |
| `EventBus` | All runtimes | Pub/Sub (observers) |

**No circular communication paths. All flows are单向 (one-directional). PASS**

---

## 8. Interaction Model Verification

### 8.1 Event-Driven Compliance

| Requirement | Status | Evidence |
|-------------|--------|----------|
| Components communicate via events | PASS | `RuntimeEvent`, `MemoryEvent`, `DiagnosticEvent` |
| No direct coupling between runtimes | PASS | `ai_runtime` and `memory_runtime` are isolated |
| Observability is side-channel | PASS | `EventBus` is separate from business logic |
| Events are immutable once created | PASS | All event variants are `Clone`, no mutation |

### 8.2 Request/Reply Compliance

| Requirement | Status | Evidence |
|-------------|--------|----------|
| Synchronous calls return Result | PASS | `route() → AIRRuntimeResult<RoutingDecision>` |
| Async operations use Future | PASS | All async methods return `Pin<Box<dyn Future>>` |
| Timeouts are enforced | PASS | `TimeoutManager` in `RuntimeContext` |

### 8.3 Dead-Letter Handling

**Status:** Not yet implemented (planned for P10.2 communication extension).

| Component | Status |
|-----------|--------|
| `dead_letter.rs` (planned) | Not present |
| Current error handling | Returns `Result` to caller |
| Future: unprocessable events → dead-letter store | Planned for P10.2 |

**Observation: Dead-letter store is not yet present. This is acceptable for P10.0-P10.1; planned for P10.2.**

---

## 9. Communication Review Summary

| Mechanism | Implemented | Thread-Safe | Deterministic | Compliant |
|-----------|------------|-------------|---------------|-----------|
| Runtime Events | Yes | Yes (Clone) | Yes | PASS |
| Runtime Context | Yes | Yes (Arc) | Yes | PASS |
| Memory Resolution | Yes | Yes (RwLock) | Yes | PASS |
| AI Routing | Yes | Yes (RwLock) | Yes | PASS |
| Observability | Yes | Yes (Mutex) | Yes | PASS |
| Dead-Letter Store | No (planned) | N/A | N/A | OBSERVED |

**Overall Communication Review: COMPLIANT**

All communication mechanisms follow the approved event-driven, trait-abstracted model. The only gap is the dead-letter store, which is planned for a future phase.

# ADR-004: Reliability Layer Architecture

**Document:** `docs/ADR/adr-004-reliability-layer.md`
**Version:** 1.0.0
**Part of:** CodeBro P2 Reliability Layer
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-001

---

## 1. Context

### 1.1 Background

The P1 Core Runtime established a deterministic ReAct loop with explicit state management, provider abstraction, and registry-based tool dispatch. P1.5 validated all 386 tests pass with zero regressions. However, the runtime lacks systematic reliability mechanisms:

1. **No structured error classification** — errors are opaque strings, making recovery decisions heuristic.
2. **No centralized timeout management** — the shell tool has a timeout, but providers and tools have none.
3. **No health monitoring** — there is no way to track the health of providers, tools, or resources.
4. **No circuit breaker** — repeated provider/tool failures cause cascading failures with no cooldown.
5. **No resource guard** — memory, CPU, and shutdown are not protected.
6. **No diagnostics** — failures are not traced; there is no failure or recovery trace.
7. **No structured logging** — logs lack correlation IDs and consistent structure.

These gaps make the runtime fragile under adverse conditions (provider flakiness, tool timeouts, resource exhaustion).

### 1.2 Constraints

- No new dependencies (use existing `tracing`, `tokio`, `std`).
- No changes to the `Provider` or `Tool` traits.
- No changes to the `AgentEvent` enum (no new variants).
- No changes to the `RuntimeState` machine.
- No changes to the event flow between major subsystems.
- All existing 386 tests must continue to pass.
- The reliability layer is a new top-level module under `src/reliability/`.

### 1.3 Stakeholders

- **Runtime (tui/ui.rs)**: Needs reliability primitives to protect the pipeline.
- **Provider layer**: Needs timeout and circuit-breaker protection.
- **Tool layer**: Needs per-tool timeout and health tracking.
- **Tests**: Need deterministic reliability primitives for validation.
- **Diagnostics**: Need structured traces for post-mortem analysis.

---

## 2. Decision

### 2.1 Decision Statement

Create a new `reliability` module with seven focused components: error classification, timeout management, health monitoring, circuit breaking, resource guarding, diagnostics, and structured logging. Each component is independently testable and composable.

### 2.2 Rationale

1. **Separation of concerns**: Each reliability primitive has a single responsibility.
2. **Testability**: Components are pure Rust structs with no I/O, making them easy to test.
3. **Composability**: Components can be combined (e.g., circuit breaker with timeout).
4. **No regressions**: The reliability layer is additive; it wraps existing code without modifying it.
5. **Performance**: All components use zero-cost abstractions (no allocations in hot paths).

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)**: Clean module boundaries.
- **Principle 9 (Performance Matters)**: Zero-cost abstractions, no heap allocations in hot paths.
- **Principle 10 (Small, Composable Components)**: Each component is a focused, testable unit.
- **Principle 12 (Defensive Coding)**: Failures are classified, tracked, and recovered from.

---

## 3. Consequences

### 3.1 Positive Consequences

- Structured error classification enables informed recovery decisions.
- Centralized timeout management prevents hanging operations.
- Health monitoring provides visibility into system state.
- Circuit breaker prevents cascading failures.
- Resource guard prevents OOM and ensures safe shutdown.
- Diagnostics enable post-mortem analysis.
- Structured logging enables observability.

### 3.2 Negative Consequences

- Additional code (~800 lines).
- Additional dependencies on `tracing` (already present).
- Slight memory overhead for health state and traces.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Memory | Health state + traces add ~1-2 MB | Bounded buffers; LRU eviction |
| CPU | Circuit breaker checks add ~100ns/op | Lock-free atomic operations |
| Complexity | New module to maintain | Clear boundaries; comprehensive tests |
| Latency | Timeout wrapping adds ~1µs | Asynchronous; non-blocking |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `src/reliability/` | **NEW** — all reliability primitives |
| `src/main.rs` | Add `mod reliability` |
| `src/tui/ui.rs` | Wire reliability components into pipeline |
| `src/providers/openai.rs` | Add timeout wrapper |
| `src/tools/shell.rs` | Add timeout config via reliability |
| `src/tests/validation.rs` | Add P2 reliability tests |

### 3.5 Impact on Future Work

- P3 plugin system: Plugins can register reliability hooks.
- P4 intelligence wiring: Intelligence layer benefits from diagnostics.
- P5 MCP integration: MCP providers benefit from circuit breaker.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| Extend existing modules | Add reliability to agent/ or tui/ | Fewer modules | Violates separation of concerns | Fails Principle 7 |
| Use external crate | e.g., `backoff`, `tower`, `tracing-appender` | Mature, tested | New dependency | Constraint: no new deps |
| Event-driven reliability | React to AgentEvent for reliability | Loose coupling | Harder to reason about timing | Less deterministic |
| Single Reliability struct | One monolithic struct | Simple API | God object; hard to test | Fails Principle 10 |

---

## 5. Implementation Notes

### 5.1 Module Structure

```
src/reliability/
├── mod.rs                  # Module entry, re-exports
├── error.rs                # RuntimeError enum, classification
├── timeout.rs              # TimeoutManager, per-provider/tool timeout
├── health.rs               # HealthMonitor, health status tracking
├── circuit_breaker.rs      # CircuitBreaker, cooldown, recovery
├── resource_guard.rs       # ResourceGuard, memory limits, safe shutdown
├── diagnostics.rs          # Diagnostics, failure traces, recovery traces
└── logging.rs              # StructuredLogger, correlation IDs
```

### 5.2 Error Classification

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum RuntimeErrorCategory {
    ProviderTimeout,
    ProviderRateLimit,
    ProviderAuthFailure,
    ProviderNetworkError,
    ToolTimeout,
    ToolExecutionError,
    ToolPermissionDenied,
    SystemMemoryLimit,
    SystemShutdown,
    SystemCancellation,
    Unknown,
}

impl RuntimeErrorCategory {
    pub fn classify(message: &str) -> Self;
    pub fn is_retryable(&self) -> bool;
    pub fn escalation_level(&self) -> u32;
}
```

### 5.3 Timeout Manager

```rust
pub struct TimeoutManager {
    default_timeout_ms: u64,
    provider_timeouts: HashMap<String, u64>,
    tool_timeouts: HashMap<String, u64>,
}

impl TimeoutManager {
    pub fn get_timeout(&self, target: &str, kind: TimeoutKind) -> u64;
    pub fn set_provider_timeout(&mut self, provider: &str, timeout_ms: u64);
    pub fn set_tool_timeout(&mut self, tool: &str, timeout_ms: u64);
}

pub enum TimeoutKind {
    Provider,
    Tool,
    System,
}
```

### 5.4 Health Monitor

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

pub struct HealthMonitor {
    providers: HashMap<String, HealthEntry>,
    tools: HashMap<String, HealthEntry>,
    runtime: HealthEntry,
    resources: HealthEntry,
}

impl HealthMonitor {
    pub fn check_provider(&self, name: &str) -> HealthStatus;
    pub fn check_tool(&self, name: &str) -> HealthStatus;
    pub fn check_runtime(&self) -> HealthStatus;
    pub fn check_resources(&self) -> HealthStatus;
    pub fn record_failure(&mut self, target: &str, kind: HealthTarget);
    pub fn record_success(&mut self, target: &str, kind: HealthTarget);
}
```

### 5.5 Circuit Breaker

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum CircuitState {
    Closed,      // Normal operation
    Open,        // Failing, reject calls
    HalfOpen,    // Testing recovery
}

pub struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    success_count: u32,
    failure_threshold: u32,
    success_threshold: u32,
    cooldown_ms: u64,
    last_failure_time: Option<Instant>,
}

impl CircuitBreaker {
    pub fn new(failure_threshold: u32, cooldown_ms: u64) -> Self;
    pub fn can_execute(&self) -> bool;
    pub fn record_success(&mut self);
    pub fn record_failure(&mut self);
    pub fn state(&self) -> CircuitState;
}
```

### 5.6 Resource Guard

```rust
pub struct ResourceGuard {
    memory_limit_mb: usize,
    current_memory_mb: usize,
    max_operations: usize,
    operations_count: usize,
    shutdown_requested: bool,
}

impl ResourceGuard {
    pub fn check_memory(&self) -> bool;
    pub fn check_operations(&self) -> bool;
    pub fn request_shutdown(&mut self);
    pub fn should_shutdown(&self) -> bool;
}
```

### 5.7 Diagnostics

```rust
#[derive(Debug, Clone)]
pub struct FailureTrace {
    pub correlation_id: String,
    pub timestamp: String,
    pub category: RuntimeErrorCategory,
    pub message: String,
    pub recovery_action: Option<String>,
    pub recovered: bool,
}

#[derive(Debug, Clone)]
pub struct RecoveryTrace {
    pub correlation_id: String,
    pub timestamp: String,
    pub original_error: String,
    pub action_taken: String,
    pub success: bool,
    pub retry_count: u32,
}

pub struct Diagnostics {
    pub failure_traces: Vec<FailureTrace>,
    pub recovery_traces: Vec<RecoveryTrace>,
    pub correlation_id: String,
}
```

### 5.8 Structured Logging

```rust
pub struct StructuredLogger {
    correlation_id: String,
}

impl StructuredLogger {
    pub fn new(correlation_id: &str) -> Self;
    pub fn info(&self, message: &str);
    pub fn warn(&self, message: &str);
    pub fn error(&self, message: &str);
    pub fn trace(&self, message: &str);
}
```

---

## 6. References

- [Architecture Manifest Section 3](../../architecture/architecture_manifest_v1.md#3-module-boundaries)
- [Design Principle 12](../../principles/design_principles.md#principle-12-defensive-coding)
- [RFC-001](../../RFC/rfc-001-react-runtime-loop.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |

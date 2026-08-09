# Reliability Architecture Report

**Date:** 2026-08-05
**Phase:** P2 Reliability Layer
**Status:** Complete — superseded in part by ADR-012

> **Update (ADR-012):** `src/reliability/health.rs` and
> `src/reliability/circuit_breaker.rs` were removed in ADR-012 because they
> duplicated the canonical provider reliability implementation in
> `provider_runtime` (health, circuit breaker, retry, routing, failover).
> `reliability/` now contains only provider-agnostic generic infrastructure:
> `error.rs`, `timeout.rs`, `resource_guard.rs`, `diagnostics.rs`, `logging.rs`.
> Sections 3.3 (Health) and 3.4 (Circuit Breaker) below are historical.

---

## 1. Overview

The Reliability Layer provides systematic error handling, recovery, monitoring, and observability on top of the P1 Core Runtime. It is designed as an additive, non-invasive layer that wraps existing operations without modifying their interfaces.

---

## 2. Architecture

### 2.1 Module Structure

```
src/reliability/
├── mod.rs                  # Module entry, re-exports
├── error.rs                # RuntimeErrorCategory, classify_error()
├── timeout.rs              # TimeoutManager, TimeoutConfig
├── resource_guard.rs       # ResourceGuard, ResourceStatus
├── diagnostics.rs          # Diagnostics, FailureTrace, RecoveryTrace
└── logging.rs              # StructuredLogger, LogSink, LogEntry
```

### 2.2 Design Principles

1. **Zero new dependencies** — Uses only `std`, `serde`, `tracing` (already present).
2. **Thread-safe by design** — All shared state uses `Arc<Mutex<>>` or lock-free atomics.
3. **Composable** — Components can be combined (e.g., circuit breaker + timeout + diagnostics).
4. **Observable** — All failures and recoveries are traced with correlation IDs.
5. **Non-invasive** — No changes to `Provider`, `Tool`, `AgentEvent`, or `RuntimeState`.

### 2.3 Component Diagram

```
┌─────────────────────────────────────────────────────────────┐
│                    Reliability Layer                         │
│                                                             │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────┐  │
│  │   Error      │  │   Timeout    │  │    Health        │  │
│  │ Classification│  │   Manager    │  │    Monitor       │  │
│  └──────┬───────┘  └──────┬───────┘  └────────┬─────────┘  │
│         │                 │                    │            │
│  ┌──────▼───────┐  ┌──────▼───────┐  ┌────────▼─────────┐  │
│  │  Circuit     │  │  Resource    │  │   Diagnostics    │  │
│  │  Breaker     │  │   Guard      │  │   & Logging      │  │
│  └──────────────┘  └──────────────┘  └──────────────────┘  │
│                                                             │
└─────────────────────────────────────────────────────────────┘
           ↓              ↓              ↓              ↓
      Provider       Tool        Runtime       Resources
```

---

## 3. Component Details

### 3.1 Error Classification (`error.rs`)

**Purpose:** Convert opaque error strings into structured categories with metadata.

**Key Types:**
- `RuntimeErrorCategory`: 11 categories covering provider, tool, and system errors.
- `RuntimeError`: Structured error with category, source, and correlation ID.

**Classification Logic:**
- Keyword-based matching against common error patterns.
- Falls back to `Unknown` for unclassified errors.
- Each category has: `is_retryable()` and `escalation_level()`.

**Example:**
```rust
let category = classify_error("request timed out after 30s");
// → RuntimeErrorCategory::ProviderTimeout

let err = RuntimeError::new("timeout", category, "provider:openai");
// → RuntimeError { category, source: "provider:openai", correlation_id: "provider-openai-provider_timeout" }
```

### 3.2 Timeout Manager (`timeout.rs`)

**Purpose:** Centralized timeout management for providers, tools, and the system.

**Key Types:**
- `TimeoutManager`: Thread-safe timeout registry.
- `TimeoutConfig`: Per-target timeout configuration.
- `TimeoutKind`: Provider, Tool, or System.

**API:**
```rust
let tm = TimeoutManager::new();
tm.set_provider_timeout("openai", 30_000);
let timeout_ms = tm.start_timeout("req1", TimeoutKind::Provider, "openai");
let remaining = tm.remaining("req1").unwrap();
tm.remove("req1");
```

**Defaults:**
- Provider: 60s
- Tool: 60s
- System: 5min

### 3.3 Health Monitoring (`health.rs`)

**Purpose:** Track health of providers, tools, runtime, and resources.

**Key Types:**
- `HealthMonitor`: Thread-safe health tracker.
- `HealthStatus`: Unknown, Healthy, Degraded, Unhealthy.
- `HealthEntry`: Detailed health data per component.

**Degradation Thresholds:**
- Degraded: 2+ consecutive failures
- Unhealthy: 5+ consecutive failures
- Healthy: 3+ consecutive successes

**API:**
```rust
let hm = HealthMonitor::new();
hm.record_provider_failure("openai");
hm.record_provider_failure("openai");
assert_eq!(hm.check_provider("openai"), HealthStatus::Degraded);

hm.record_provider_success("openai");
hm.record_provider_success("openai");
hm.record_provider_success("openai");
assert_eq!(hm.check_provider("openai"), HealthStatus::Healthy);
```

### 3.4 Circuit Breaker (`circuit_breaker.rs`)

**Purpose:** Prevent cascading failures by opening the circuit after repeated failures.

**Key Types:**
- `CircuitBreaker`: Thread-safe circuit breaker.
- `CircuitState`: Closed, Open, HalfOpen.
- `CircuitBreakerConfig`: Failure threshold, success threshold, cooldown.

**State Machine:**
```
Closed ──(failures >= threshold)──> Open
  ^                                 │
  │        (successes >= threshold) │ (cooldown expired)
  └─────────────────────────────────┘
                  HalfOpen
```

**API:**
```rust
let cb = CircuitBreaker::with_config(CircuitBreakerConfig {
    failure_threshold: 5,
    success_threshold: 3,
    cooldown_ms: 30_000,
});
cb.record_failure();
if !cb.can_execute() {
    // Circuit is open, reject the request
}
```

### 3.5 Resource Guard (`resource_guard.rs`)

**Purpose:** Enforce memory and operation limits with graceful shutdown.

**Key Types:**
- `ResourceGuard`: Thread-safe resource monitor.
- `ResourceStatus`: OK, MemoryWarning, MemoryLimitExceeded, OperationLimitExceeded, ShutdownRequested.

**Defaults:**
- Memory limit: 512MB
- Operation limit: 10,000
- Warning threshold: 80%

**API:**
```rust
let guard = ResourceGuard::new();
guard.update_memory(400); // 400MB
assert_eq!(guard.status(), ResourceStatus::OK);

guard.update_memory(512);
assert_eq!(guard.status(), ResourceStatus::MemoryLimitExceeded);

guard.request_shutdown();
assert!(guard.should_shutdown());
```

### 3.6 Diagnostics (`diagnostics.rs`)

**Purpose:** Structured failure and recovery traces with correlation IDs.

**Key Types:**
- `Diagnostics`: Thread-safe trace collector.
- `FailureTrace`: Individual failure event.
- `RecoveryTrace`: Individual recovery event.

**Features:**
- Automatic correlation ID generation
- LRU eviction (max 1,000 traces)
- Category-based filtering
- Recovery tracking

**API:**
```rust
let diag = Diagnostics::new();
diag.record_failure(
    RuntimeErrorCategory::ProviderTimeout,
    "request timed out",
    "provider:openai",
    Some("retry"),
    false,
);
diag.record_recovery("timeout", "retry", true, 1);
let summary = diag.summary();
```

### 3.7 Structured Logging (`logging.rs`)

**Purpose:** Consistent logging with correlation IDs and pluggable sinks.

**Key Types:**
- `StructuredLogger`: Logger with correlation ID.
- `LogSink`: Trait for pluggable sinks.
- `MemoryLogSink`: In-memory sink for testing.
- `ConsoleLogSink`: stderr sink for production.

**Features:**
- Child loggers inherit correlation ID
- Pluggable sinks
- LRU log eviction

**API:**
```rust
let mut logger = StructuredLogger::new("corr-1", "pipeline");
let sink = MemoryLogSink::new(100);
logger.add_sink(Box::new(sink));

logger.info("task started");
logger.warn("provider slow");
logger.error("timeout");

let entries = sink.entries();
```

---

## 4. Integration Points

### 4.1 Provider Integration

The reliability layer wraps provider calls:
```rust
// Before: direct provider call
let response = provider.stream_response(prompt).await?;

// After: with timeout + circuit breaker + diagnostics
let timeout_ms = timeout_manager.get_provider_timeout(provider.name());
let cb = circuit_breakers.get(provider.name());
if !cb.can_execute() {
    return Err(RuntimeError::new("circuit open", RuntimeErrorCategory::ProviderNetworkError, "provider"));
}
let timeout_id = timeout_manager.start_timeout("req1", TimeoutKind::Provider, provider.name());
match provider.stream_response(prompt).await {
    Ok(response) => {
        cb.record_success();
        health.record_provider_success(provider.name());
        timeout_manager.remove("req1");
        Ok(response)
    }
    Err(e) => {
        cb.record_failure();
        health.record_provider_failure(provider.name());
        diagnostics.record_failure(...);
        timeout_manager.remove("req1");
        Err(e)
    }
}
```

### 4.2 Tool Integration

The reliability layer wraps tool calls:
```rust
let timeout_ms = timeout_manager.get_tool_timeout(tool.name());
// ... same pattern as provider
```

### 4.3 Pipeline Integration

The `run_chat_pipeline` function can be enhanced with reliability checks:
```rust
async fn run_chat_pipeline(config: &Config, task: &str, tx: &Sender<AppEvent>) {
    let diag = Diagnostics::new();
    let hm = HealthMonitor::new();
    let tm = TimeoutManager::new();
    let rg = ResourceGuard::new();
    
    // ... existing pipeline logic with reliability hooks
}
```

---

## 5. Performance Characteristics

| Component | Memory Overhead | CPU Overhead | Lock Type |
|-----------|----------------|--------------|-----------|
| Error Classification | None (pure function) | ~100ns per classification | None |
| Timeout Manager | ~100 bytes + 1 entry | ~1µs per operation | Mutex |
| Health Monitor | ~500 bytes + 1 entry | ~1µs per operation | Mutex |
| Circuit Breaker | ~100 bytes | ~100ns per operation | Mutex |
| Resource Guard | ~100 bytes | ~100ns per operation | Mutex |
| Diagnostics | ~200 bytes + 1 trace | ~1µs per trace | Mutex |
| Structured Logging | ~100 bytes + 1 entry | ~1µs per log | Arc<Vec> |

All components are lock-free for read-heavy paths and use fine-grained mutexes for write paths.

---

## 6. Future Work

| Item | Phase | Description |
|------|-------|-------------|
| Pipeline integration | P2.5 | Wire reliability components into `run_chat_pipeline` |
| Configurable policies | P3 | Allow users to configure thresholds via config file |
| Alerting | P3 | Emit alerts for critical failures |
| Persistence | P4 | Persist health and diagnostics to disk |
| Metrics export | P4 | Export health metrics to Prometheus |

---

## 7. References

- [ADR-004](../ADR/adr-004-reliability-layer.md)
- [Architecture Manifest](../architecture/architecture_manifest_v1.md)
- [Design Principle 12](../principles/design_principles.md#principle-12-defensive-coding)

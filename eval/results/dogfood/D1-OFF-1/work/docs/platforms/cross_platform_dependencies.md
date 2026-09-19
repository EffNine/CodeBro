# Cross-Platform Dependencies

**Date:** 2026-08-06
**Phase:** P4.5 Intelligence Platform Validation
**Version:** 1.0.0

---

## 1. Dependency Overview

This document defines the allowed and prohibited dependencies between CodeBro platforms.

### 1.1 Allowed Dependencies

| From | To | Purpose | Mechanism |
|------|----|---------|-----------|
| `reliability/` | `runtime/` | Error classification | Direct import |
| `tools/` | `runtime/` | State machine access | Direct import |
| `tools/` | `reliability/` | Diagnostics, circuit breaking | Direct import |
| `intelligence/` | `reliability/` | Diagnostics recording | `IntelligenceDiagnosticsTrait` |
| `agent/` | `tools/` | Tool execution | `ToolRegistry` |
| `agent/` | `intelligence/` | Context, reasoning | Trait-based reads |
| `tui/` | `intelligence/` | Diagnostics display | Direct import |

### 1.2 Prohibited Dependencies

| From | To | Reason |
|------|----|--------|
| `intelligence/` | `tools/` | Read-only boundary |
| `intelligence/` | `providers/` | No LLM calls |
| `intelligence/` | `agent/` | Circular dependency |
| `intelligence/` | `tui/` | Separation of concerns |
| `providers/` | `tools/` | Async/sync mismatch |
| `config/` | `agent/` | Load order |
| Any platform | `main.rs` | Entry point only |

---

## 2. Intelligence Platform Dependencies

### 2.1 Internal Dependencies

```
intelligence/
├── parser/        → tree-sitter crates, anyhow
├── index/         → parser, rusqlite, serde
├── graph/         → index
├── search/        → index
├── context/       → search, graph, index
├── reasoning/     → context, search, index
├── memory/        → config (path only), serde
├── lsp/           → serde
└── diagnostics/   → serde, chrono
```

### 2.2 External Dependencies (Allowed)

| Dependency | Platform | Purpose |
|------------|----------|---------|
| `reliability::Diagnostics` | Reliability | Health trace integration |

### 2.3 External Dependencies (Prohibited)

| Dependency | Reason |
|------------|--------|
| `tools::*` | Read-only boundary |
| `providers::*` | No LLM calls |
| `agent::*` | Circular dependency |
| `tui::*` | Separation of concerns |

---

## 3. Tool Platform Dependencies

### 3.1 Internal Dependencies

```
tools/
├── executor/      → registry, capabilities, diagnostics
├── registry/      → lifecycle, capabilities
├── filesystem/    → std::fs
├── shell/         → std::process, history
├── git/           → std::process
├── patch/         → std::fs
└── diagnostics/   → std::sync
```

### 3.2 External Dependencies (Allowed)

| Dependency | Platform | Purpose |
|------------|----------|---------|
| `runtime::RuntimeState` | Runtime | State-aware execution |
| `reliability::*` | Reliability | Diagnostics, circuit breaking |

### 3.3 External Dependencies (Prohibited)

| Dependency | Reason |
|------------|--------|
| `intelligence::*` | Tools are write operations; intelligence is read-only |
| `providers::*` | No LLM calls from tools |

---

## 4. Reliability Platform Dependencies

### 4.1 Internal Dependencies

```
reliability/
├── error/         → std::error
├── timeout/       → tokio::time
├── health/        → std::sync
├── circuit_breaker/ → std::sync
├── diagnostics/   → std::sync, serde
├── logging/       → tracing
└── resource_guard/ → std::sync
```

### 4.2 External Dependencies (Allowed)

| Dependency | Platform | Purpose |
|------------|----------|---------|
| `runtime::RuntimeState` | Runtime | State-aware error classification |

### 4.3 External Dependencies (Prohibited)

| Dependency | Reason |
|------------|--------|
| `tools::*` | Reliability observes, doesn't execute |
| `intelligence::*` | No code understanding needed |

---

## 5. Runtime Platform Dependencies

### 5.1 Internal Dependencies

```
runtime/
└── state/         → std (no external deps)
```

### 5.2 External Dependencies (Allowed)

None. Runtime is the foundation.

### 5.3 External Dependencies (Prohibited)

None. Runtime depends on nothing.

---

## 6. Agent Platform Dependencies

### 6.1 Internal Dependencies

```
agent/
├── coordinator/   → tools, intelligence (future), events
├── subagent/      → tools, memory
├── memory/        → std
├── events/        → std::sync
└── ...
```

### 6.2 External Dependencies (Allowed)

| Dependency | Platform | Purpose |
|------------|----------|---------|
| `tools::*` | Tool | Tool execution |
| `intelligence::*` | Intelligence | Context, reasoning (read-only) |
| `reliability::*` | Reliability | Diagnostics, recovery |

### 6.3 External Dependencies (Prohibited)

| Dependency | Reason |
|------------|--------|
| `providers::*` | Providers are called through TUI pipeline |
| `tui::*` | TUI is the caller, not the callee |

---

## 7. TUI Platform Dependencies

### 7.1 Internal Dependencies

```
tui/
├── app/           → agent, metrics
├── ui/            → app, tools, agent events
├── dashboard/     → agent events, metrics
├── markdown/      → pulldown-cmark
└── ...
```

### 7.2 External Dependencies (Allowed)

| Dependency | Platform | Purpose |
|------------|----------|---------|
| `agent::*` | Agent | Agent event handling |
| `intelligence::*` | Intelligence | Diagnostics display |
| `metrics::*` | Metrics | Cost/token display |
| `providers::*` | Providers | Model selection |

### 7.3 External Dependencies (Prohibited)

| Dependency | Reason |
|------------|--------|
| `tools::*` (direct) | Tools run through agent pipeline |
| `config::*` (direct) | Config loaded at startup |

---

## 8. Dependency Verification

### 8.1 Compile-Time Checks

The following compile-time assertions verify dependency direction:

```rust
// These should compile (allowed):
use crate::intelligence::diagnostics::IntelligenceDiagnostics;
use crate::reliability::Diagnostics;

// These should NOT compile (prohibited):
// use crate::tools::Tool;          // intelligence → tools
// use crate::providers::Provider;  // intelligence → providers
// use crate::agent::Agent;         // intelligence → agent
```

### 8.2 Test Verification

The `test_no_tool_dependencies_p45` and `test_no_provider_dependencies_p45` tests verify that the intelligence platform does not import from prohibited modules.

---

## 9. Future Dependency Changes

Any change to the dependency matrix requires:

1. **ADR** if adding a new cross-platform dependency
2. **RFC** if changing an existing dependency direction
3. **Update this document** after approval

---

## 10. Version History

| Version | Date | Changes |
|---------|------|---------|
| 1.0.0 | 2026-08-06 | Initial cross-platform dependency document |

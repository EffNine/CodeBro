# CodeBro Runtime Layers

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-07
**Part of:** Design Summit v2
**Owner:** CodeBro Engineering

---

## 1. Layer Architecture

The CodeBro Runtime v2 is organized into six layered tiers, each with明确的 responsibilities and bounded interfaces.

```
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 6: TUI / CLI (Presentation)                                  │
│  · User input/output                                                 │
│  · Dashboard rendering                                               │
│  · Slash command parsing                                             │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 5: Integration Pipeline (Decision)                           │
│  · Intent classification                                             │
│  · Recommendation generation                                         │
│  · Workflow planning                                                 │
│  · Validation and approval preview                                   │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 4: Runtime Core (Orchestration)                              │
│  · Runtime state machine                                             │
│  · AI Runtime orchestration                                          │
│  · Agent Runtime coordination                                        │
│  · Context assembly                                                  │
│  · Lifecycle management                                              │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 3: Service Registry (Discovery)                              │
│  · Provider registry                                                 │
│  · Tool registry                                                     │
│  · Plugin registry                                                   │
│  · Capability advertisement                                          │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 2: Foundation Engines (Execution)                            │
│  · Provider implementations (OpenAI, etc.)                          │
│  · Tool implementations (filesystem, shell, git, patch)             │
│  · Agent implementations (research, planning, coding, test, review) │
│  · Memory implementations (tiers, eviction)                         │
└─────────────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────────────┐
│  Layer 1: Cross-Cutting Concerns (Support)                          │
│  · Observability (events, metrics, tracing, logging)                │
│  · Reliability (errors, timeouts, circuit breakers, health)         │
│  · Security (permissions, sandbox, audit, anomaly detection)        │
│  · Plugin SDK (lifecycle, hooks, capabilities)                      │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 2. Layer Specifications

### 2.1 Layer 1: Cross-Cutting Concerns

**Purpose:** Provide infrastructure services to all layers.

**Modules:**
- `src/observability/` — Event bus, metrics, tracing, logging
- `src/reliability/` — Error classification, timeouts, circuit breakers, health monitoring
- `src/security/` — Permissions, sandboxing, audit logging (NEW)
- `src/plugin_sdk/` — Plugin lifecycle, hooks, capabilities

**Dependencies:** None (foundation layer)

**Dependency Rules:**
- May be imported by any layer
- May NOT import from higher layers
- Must not contain business logic

---

### 2.2 Layer 2: Foundation Engines

**Purpose:** Implement concrete services used by the runtime.

**Modules:**
- `src/providers/` — Provider trait and implementations
- `src/tools/` — Tool implementations (filesystem, shell, git, patch)
- `src/agent/` — Agent traits and implementations
- `src/agent/memory.rs` — Memory implementation
- `src/indexer/` — Code indexing
- `src/intelligence/` — Code intelligence (parser, index, graph, search)

**Dependencies:** Layer 1 only

**Dependency Rules:**
- May import from Layer 1
- May NOT import from Layers 3-6
- Must implement traits defined in Layer 3 contracts

---

### 2.3 Layer 3: Service Registry

**Purpose:** Discover, register, and manage services.

**Modules:**
- `src/dispatcher/` — Tool registry and dispatch
- `src/provider_manager/` — Provider discovery and management (NEW)
- `src/plugin_sdk/registry.rs` — Plugin registry
- `src/capability_discovery/` — Capability advertisement (NEW)

**Dependencies:** Layer 1, Layer 2

**Dependency Rules:**
- May import from Layer 1 and Layer 2
- May NOT import from Layers 4-6
- Must use traits, not concrete types

---

### 2.4 Layer 4: Runtime Core

**Purpose:** Orchestrate the main execution pipeline.

**Modules:**
- `src/runtime/` — Runtime state machine
- `src/runtime/ai/` — AI Runtime (NEW)
- `src/runtime/memory/` — Memory Runtime (NEW)
- `src/runtime/context/` — Context Runtime (NEW)
- `src/runtime/agent/` — Agent Runtime (NEW)
- `src/runtime/lifecycle/` — Runtime lifecycle (NEW)
- `src/communication/` — Runtime communication (NEW)
- `src/intent_engine/` — Intent classification
- `src/workflow_engine/` — Workflow planning
- `src/preference_engine/` — Preference management
- `src/adaptive_validation/` — Adaptive validation (NEW)
- `src/integration_pipeline/` — Integration pipeline

**Dependencies:** Layer 1, Layer 2, Layer 3

**Dependency Rules:**
- May import from Layers 1-3
- May NOT import from Layer 5 or 6
- Must not directly call Layer 2 implementations (use traits)

---

### 2.5 Layer 5: Integration Pipeline

**Purpose:** Wire decision engines into a deterministic pipeline.

**Modules:**
- `src/integration_pipeline/` — Pipeline orchestration
- `src/intent_engine/` — Intent classification (also Layer 4)
- `src/recommendation_engine/` — Recommendation generation (NEW)
- `src/workflow_engine/` — Workflow planning (also Layer 4)
- `src/adaptive_validation/` — Validation (also Layer 4)

**Dependencies:** Layer 1, Layer 2, Layer 3, Layer 4

**Dependency Rules:**
- May import from Layers 1-4
- May NOT import from Layer 6
- Must be stateless (no persistent state)

---

### 2.6 Layer 6: TUI / CLI (Presentation)

**Purpose:** User interface and command-line interaction.

**Modules:**
- `src/tui/` — Terminal UI
- `src/cli/` — Command-line interface
- `src/session/` — Session management
- `src/onboarding/` — Onboarding flow

**Dependencies:** All layers

**Dependency Rules:**
- May import from any layer
- Must not modify Layer 1-5 state directly (use events)
- Must be display-only for core logic

---

## 3. Cross-Layer Communication

### 3.1 Event Flow

```
Layer 6 (TUI)
    │ emit UserEvent
    ▼
Layer 5 (Pipeline)
    │ emit PipelineEvent
    ▼
Layer 4 (Runtime Core)
    │ emit RuntimeEvent
    ▼
Layer 3 (Registry)
    │ emit RegistryEvent
    ▼
Layer 2 (Engines)
    │ execute operation
    ▼
Layer 1 (Cross-Cutting)
    │ observe, log, metric
```

### 3.2 Data Flow

| Direction | Mechanism | Example |
|-----------|-----------|---------|
| Top-down | Function calls | Pipeline calls Runtime |
| Bottom-up | Events | Engine emits RuntimeEvent |
| Horizontal | Shared registry | Runtime reads from ProviderRegistry |
| Side-channel | Observability | All layers emit to EventBus |

---

## 4. Layer Isolation Rules

### 4.1 Forbidden Dependencies

| From Layer | Cannot Import |
|------------|---------------|
| Layer 1 | Layers 2-6 |
| Layer 2 | Layers 3-6 |
| Layer 3 | Layers 4-6 |
| Layer 4 | Layers 5-6 |
| Layer 5 | Layer 6 |

### 4.2 Allowed Dependencies

| From Layer | Can Import |
|------------|------------|
| Layer 1 | (none) |
| Layer 2 | Layer 1 |
| Layer 3 | Layers 1, 2 |
| Layer 4 | Layers 1, 2, 3 |
| Layer 5 | Layers 1, 2, 3, 4 |
| Layer 6 | Layers 1, 2, 3, 4, 5 |

---

## 5. New Layer Additions for v2

### 5.1 Communication Layer (NEW — between Layer 3 and 4)

```
src/communication/
├── mod.rs              # Module assembly
├── event_bus.rs        # Pub/sub event bus
├── channels.rs         # Request/reply channels
├── dead_letter.rs      # Dead-letter store
└── ordering.rs         # Message ordering guarantees
```

**Purpose:** Provide a centralized communication fabric for runtime components.

**Dependencies:** Layer 1 (observability)

**Exported Traits:**
```rust
pub trait EventBus: Send + Sync {
    fn subscribe(&self, event_type: EventType, handler: Box<dyn EventHandler>);
    fn publish(&self, event: RuntimeEvent);
}

pub trait MessageChannel: Send + Sync {
    fn send(&self, message: RuntimeMessage) -> Result<MsgId>;
    fn recv(&self, id: &MsgId) -> Result<RuntimeMessage>;
}
```

### 5.2 Security Layer Extension (NEW — Layer 1 enhancement)

```
src/security/
├── mod.rs              # Module assembly
├── permissions.rs      # Permission checking
├── sandbox.rs          # Code sandboxing
├── audit.rs            # Audit logging
└── anomaly.rs          # Anomaly detection
```

**Purpose:** Enforce security policies across all runtime components.

**Dependencies:** Layer 1 existing (observability, reliability)

**Exported Traits:**
```rust
pub trait SecurityAuditor: Send + Sync {
    fn check_permission(&self, actor: &str, domain: SecurityDomain, action: &str) -> Result<PermissionDecision>;
    fn audit(&self, event: AuditEvent);
    fn detect_anomaly(&self, pattern: &str) -> Option<Anomaly>;
}
```

---

## 6. Module-to-Layer Mapping

| Module | Layer | Status |
|--------|-------|--------|
| `src/observability/` | 1 | Frozen |
| `src/reliability/` | 1 | Frozen |
| `src/plugin_sdk/` | 1 | Frozen |
| `src/security/` | 1 | NEW |
| `src/communication/` | 1.5 | NEW |
| `src/providers/` | 2 | Frozen |
| `src/tools/` | 2 | Frozen |
| `src/agent/` | 2 | Frozen |
| `src/indexer/` | 2 | Frozen |
| `src/intelligence/` | 2 | Frozen |
| `src/dispatcher/` | 3 | Frozen |
| `src/provider_manager/` | 3 | Frozen |
| `src/plugin_sdk/registry.rs` | 3 | Frozen |
| `src/capability_discovery/` | 3 | NEW |
| `src/runtime/` | 4 | Frozen + NEW |
| `src/intent_engine/` | 4 | Frozen |
| `src/workflow_engine/` | 4 | Frozen |
| `src/preference_engine/` | 4 | Frozen |
| `src/adaptive_validation/` | 4 | NEW |
| `src/integration_pipeline/` | 4 | Frozen |
| `src/integration_pipeline/` | 5 | Frozen |
| `src/recommendation_engine/` | 5 | NEW |
| `src/tui/` | 6 | Frozen |
| `src/cli/` | 6 | Frozen |
| `src/session/` | 6 | Frozen |
| `src/onboarding/` | 6 | Frozen |

---

## 7. Layer Validation

### 7.1 Compile-Time Checks

```toml
# Cargo.toml — enforce layer dependencies
[dependencies]
# Layer 1: No dependencies on project code
# Layer 2: Depends on layer_1
# Layer 3: Depends on layer_1, layer_2
# ...
```

### 7.2 Static Analysis Rules

```rust
// Layer enforcement via module visibility
mod layer1 { /* public */ }
mod layer2 { /* pub(crate) */ }
mod layer3 { /* pub(crate) */ }
// etc.
```

### 7.3 Test Isolation

| Test Type | Scope | Can Access |
|-----------|-------|------------|
| Unit tests | Single module | Module internals |
| Integration tests | Layer boundary | Downward deps only |
| End-to-end tests | Full stack | All layers |

---

*Runtime Layers v2 — Design Summit*

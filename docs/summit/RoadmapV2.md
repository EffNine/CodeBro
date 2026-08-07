# CodeBro Roadmap v2.x

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-07
**Part of:** Design Summit v2
**Owner:** CodeBro Engineering

---

## 1. Overview

This roadmap defines the v2.x implementation path for the Runtime Architecture designed in Design Summit v2. The roadmap is divided into phases, each building on the previous one.

---

## 2. Phase Breakdown

### Phase P10.0: Runtime Foundation

**Objective:** Implement the core AI and Memory runtimes.

**Scope:**
- `src/runtime/ai/` — AI orchestrator, router, budget, failover
- `src/runtime/memory/` — Memory tiers, eviction, summarization
- `src/runtime/lifecycle/` — Runtime startup and shutdown
- `src/communication/` — Event bus, channels (foundation)

**Out of Scope:**
- Context Runtime
- Provider Runtime (extends existing)
- Agent Runtime (extends existing)

**Estimated Effort:** 14–21 days

**Exit Criteria:**
- [ ] AI orchestrator selects provider based on routing policy
- [ ] Cost tracking is accurate per-request
- [ ] Budget limits are enforced
- [ ] Provider failover triggers automatically on failure
- [ ] Memory tiers persist and load correctly
- [ ] Eviction policy removes entries deterministically
- [ ] Runtime starts and stops cleanly
- [ ] All P0–P9 tests pass

---

### Phase P10.1: Context and Provider

**Objective:** Implement Context Runtime and extend Provider Runtime.

**Scope:**
- `src/runtime/context/` — Context assembler, budget, prioritizer, compressor
- `src/runtime/provider/` — Provider discovery, health, metrics, failover
- `src/capability_discovery/` — Capability advertisement

**Out of Scope:**
- Agent Runtime extension
- Communication layer extension

**Estimated Effort:** 14–21 days

**Exit Criteria:**
- [ ] Context assembler builds relevant context from memory
- [ ] Context budget prevents exceeding window limits
- [ ] Context compressor reduces context when over budget
- [ ] Provider health is monitored continuously
- [ ] Provider metrics track usage and cost
- [ ] Provider discovery finds plugin-provided providers
- [ ] No P10.0 regressions

---

### Phase P10.2: Agent Orchestration

**Objective:** Implement Agent Runtime and extend Communication layer.

**Scope:**
- `src/runtime/agent/` — Agent orchestrator, communication, lifecycle, resource
- `src/communication/` — Dead-letter store, ordering guarantees
- `src/security/` — Permission checks, audit logging, anomaly detection

**Out of Scope:**
- New agent implementations
- TUI changes

**Estimated Effort:** 14–21 days

**Exit Criteria:**
- [ ] Agent runtime spawns and manages agents
- [ ] Inter-agent messaging works via message bus
- [ ] Agent lifecycle states are tracked
- [ ] Resource limits prevent agent overload
- [ ] Dead-letter store captures unprocessable messages
- [ ] Security auditor enforces permission policies
- [ ] Audit log records all state changes
- [ ] No P10.0–P10.1 regressions

---

### Phase P10.3: Integration and Polish

**Objective:** Full runtime integration, testing, and documentation.

**Scope:**
- End-to-end integration testing
- Performance optimization
- Documentation updates
- Benchmark comparison
- Regression testing

**Out of Scope:**
- New features
- TUI changes
- Plugin development

**Estimated Effort:** 10–14 days

**Exit Criteria:**
- [ ] Full runtime pipeline works end-to-end
- [ ] Performance meets targets (startup < 200ms, memory < 100MB)
- [ ] Documentation is complete
- [ ] All tests pass (unit, integration, e2e)
- [ ] No P0–P9 regressions
- [ ] Benchmarks show improvement or parity

---

## 3. Dependency Graph

```
P10.0 (Runtime Foundation)
    │
    ├──→ P10.1 (Context & Provider)
    │       │
    │       └──→ P10.2 (Agent Orchestration)
    │               │
    │               └──→ P10.3 (Integration & Polish)
    │
    └──→ P10.1 (Context & Provider)
            └──→ P10.2 (Agent Orchestration)
                    └──→ P10.3 (Integration & Polish)
```

---

## 4. Module Implementation Order

### P10.0 Order
1. `src/runtime/ai/orchestrator.rs` — AI orchestrator
2. `src/runtime/ai/router.rs` — Cost-aware router
3. `src/runtime/ai/budget.rs` — Budget tracking
4. `src/runtime/ai/failover.rs` — Failover logic
5. `src/runtime/memory/tiers.rs` — Memory tier definitions
6. `src/runtime/memory/evictor.rs` — Eviction policy
7. `src/runtime/memory/persistence.rs` — JSON persistence
8. `src/runtime/lifecycle/manager.rs` — Lifecycle manager
9. `src/runtime/lifecycle/startup.rs` — Startup sequence
10. `src/communication/event_bus.rs` — Event bus foundation
11. `src/communication/channels.rs` — Channel foundation
12. `src/runtime/mod.rs` — Module assembly

### P10.1 Order
1. `src/runtime/context/assembler.rs` — Context assembler
2. `src/runtime/context/budget.rs` — Context budget
3. `src/runtime/context/prioritizer.rs` — Prioritization
4. `src/runtime/context/compressor.rs` — Compression
5. `src/runtime/provider/discovery.rs` — Provider discovery
6. `src/runtime/provider/health.rs` — Health monitoring
7. `src/runtime/provider/metrics.rs` — Usage metrics
8. `src/runtime/provider/failover.rs` — Provider failover
9. `src/capability_discovery/mod.rs` — Capability advertisement
10. `src/runtime/context/mod.rs` — Module assembly
11. `src/runtime/provider/mod.rs` — Module assembly

### P10.2 Order
1. `src/runtime/agent/orchestrator.rs` — Agent orchestrator
2. `src/runtime/agent/communication.rs` — Agent messaging
3. `src/runtime/agent/lifecycle.rs` — Agent lifecycle
4. `src/runtime/agent/resource.rs` — Resource management
5. `src/communication/dead_letter.rs` — Dead-letter store
6. `src/communication/ordering.rs` — Ordering guarantees
7. `src/security/permissions.rs` — Permission checking
8. `src/security/audit.rs` — Audit logging
9. `src/security/anomaly.rs` — Anomaly detection
10. `src/runtime/agent/mod.rs` — Module assembly
11. `src/communication/mod.rs` — Module assembly
12. `src/security/mod.rs` — Module assembly

### P10.3 Order
1. Integration tests
2. Performance benchmarks
3. Documentation
4. Regression testing
5. Final validation

---

## 5. Validation Gates

| Gate | Requirements |
|------|-------------|
| P10.0 → P10.1 | All P10.0 exit criteria met; no high-severity bugs |
| P10.1 → P10.2 | All P10.1 exit criteria met; P10.0 tests still pass |
| P10.2 → P10.3 | All P10.2 exit criteria met; P10.0–P10.1 tests still pass |
| P10.3 → Release | All P10.3 exit criteria met; full regression suite passes |

---

## 6. Risk Mitigation

| Risk | Mitigation |
|------|------------|
| Context assembly performance | Lazy evaluation, caching |
| Memory bloat | Strict eviction, size limits |
| Provider failover flapping | Hysteresis, cooldown periods |
| Agent communication overhead | Batching, backpressure |
| Security audit log growth | Rotation, compression |

---

## 7. Migration Strategy

### 7.1 From v1.0 to v2.0

| v1.0 Component | v2.0 Equivalent | Migration |
|----------------|-----------------|-----------|
| `src/runtime/state.rs` | `src/runtime/lifecycle/` | Extend, don't replace |
| `src/providers/provider.rs` | `src/runtime/provider/` | Wrap, don't replace |
| `src/agent/memory.rs` | `src/runtime/memory/` | Migrate data format |
| `src/agent/coordinator.rs` | `src/runtime/agent/` | Wrap, don't replace |
| `src/observability/` | `src/observability/` | No change |
| `src/plugin_sdk/` | `src/plugin_sdk/` | No change |

### 7.2 Data Migration

```
~/.codebro/memory.json (v1.0)
    ↓ migrate
~/.codebro/memory/short_term.json (v2.0)
~/.codebro/memory/project/{project_id}.json (v2.0)
~/.codebro/memory/global.json (v2.0)
```

### 7.3 API Compatibility

- v1.0 public APIs remain unchanged
- v2.0 adds new APIs alongside existing ones
- Deprecation warnings for obsolete paths
- Full backward compatibility for 1 release cycle

---

## 8. Success Metrics

| Metric | Target | Measurement |
|--------|--------|-------------|
| Startup time impact | < 200ms | Benchmark suite |
| Memory impact | < 100MB | RSS measurement |
| Provider failover time | < 5s | Failure injection test |
| Cost accuracy | < 5% error | Comparison with provider billing |
| Context relevance | > 0.8 score | Human evaluation |
| Test coverage | > 80% | Coverage report |
| Regression count | 0 | Test suite |

---

## 9. References

- [Runtime Architecture](./RuntimeArchitecture.md)
- [Runtime Layers](./RuntimeLayers.md)
- [Runtime Principles](./RuntimePrinciples.md)
- [Architecture Vision v2](./ArchitectureVisionV2.md)
- [Design Summit v2 Charter](./DesignSummitV2.md)

---

*Roadmap v2.x — Design Summit*

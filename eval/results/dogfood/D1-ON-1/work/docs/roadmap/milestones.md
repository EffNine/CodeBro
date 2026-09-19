# CodeBro Milestones

**Document:** `docs/roadmap/milestones.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## Overview

This document tracks the key milestones for CodeBro development. Milestones are significant achievements that mark progress toward the stable release. Each milestone corresponds to one or more phases.

---

## Milestone Tracker

| Milestone | Phase(s) | Status | Target Date | Actual Date | Notes |
|-----------|----------|--------|-------------|-------------|-------|
| M0: Baseline Established | P0, P0.5 | Complete | — | — | Architecture documented, benchmarks recorded |
| M0.5: Engineering Foundation | P0.75 | In Progress | — | — | Governance documents being created |
| M1: Core Runtime Stable | P1, P1.5 | Pending | — | — | Agent loop works, sessions persist |
| M2: Reliable by Default | P2, P2.5 | Pending | — | — | Safety gates, recovery, crash resistance |
| M3: Tools Execute Well | P3, P3.5 | Pending | — | — | Multi-tool, streaming, parallel |
| M4: Code-Aware | P4, P4.5 | Pending | — | — | Intelligence layer in production |
| M5: Pleasant to Use | P5, P5.5 | Pending | — | — | UX foundation complete |
| M6: Agent Team | P6, P6.5 | Pending | — | — | Multi-agent, parallel, stress-tested |
| M7: Release Ready | P7, P7.5 | Pending | — | — | Hardened, documented, security-reviewed |
| M8: v0.1.0 Shipped | P8 | Pending | — | — | First stable release |

---

## Milestone Details

### M0: Baseline Established

**Corresponding Phases:** P0, P0.5

**Description:** The codebase is fully understood, the architecture is documented, and measurable baselines are established for all KPIs.

**Acceptance Criteria:**
- [ ] Architecture summary document is complete
- [ ] Technical debt inventory is complete
- [ ] Baseline benchmarks are recorded for all KPIs
- [ ] Architecture constraints are documented
- [ ] ADR and RFC registries are initialized

**Deliverables:**
- `docs/reports/phase-P0-repository-audit.md`
- `docs/reports/phase-P0.5-architecture-freeze.md`
- `docs/ADR/` registry initialized
- `docs/RFC/` registry initialized

---

### M0.5: Engineering Foundation

**Corresponding Phases:** P0.75

**Description:** The engineering governance foundation is established. All standards, benchmarks, CI pipeline, and project documentation are in place. No runtime code is written — this is purely process and documentation.

**Acceptance Criteria:**
- [ ] Architecture manifest is complete and frozen
- [ ] Design principles are documented
- [ ] Engineering philosophy is documented
- [ ] Definition of Ready and Definition of Done are defined
- [ ] Coding standards are documented
- [ ] Benchmark baselines are defined (targets set, to be measured)
- [ ] CI/CD baseline is documented
- [ ] Decision log has at least 5 entries
- [ ] Project dashboard is initialized
- [ ] All documents cross-reference correctly

**Deliverables:**
- `docs/architecture/architecture_manifest_v1.md`
- `docs/principles/design_principles.md`
- `docs/philosophy/engineering_philosophy.md`
- `docs/standards/definition_of_ready.md`
- `docs/standards/definition_of_done.md`
- `docs/standards/coding_standards.md`
- `docs/benchmark/baseline.md`
- `docs/ci/ci_baseline.md`
- `docs/history/decision_log.md`
- `docs/dashboard/status.md`
- `docs/reports/phase-P0.75-engineering-baseline.md`

---

### M1: Core Runtime Stable

**Corresponding Phases:** P1, P1.5

**Description:** CodeBro can reliably process user requests through an iterative agent loop, with proper session management and LLM streaming.

**Acceptance Criteria:**
- [ ] Agent loop executes at least 3 tool calls per complex task
- [ ] Session auto-resumes on restart
- [ ] Startup time < 500ms
- [ ] All baseline tests pass
- [ ] LLM streaming updates UI progressively

**Deliverables:**
- `docs/reports/phase-P1-core-runtime.md`
- `docs/reports/phase-P1.5-runtime-validation.md`

**KPI Targets:**
- startup_time: < 500ms
- response_latency (TTFT): < 3000ms
- crash_free_sessions: 100%

---

### M2: Reliable by Default

**Corresponding Phases:** P2, P2.5

**Description:** CodeBro is safe to use by default — file writes require approval, provider failures are recoverable, and sessions survive crashes.

**Acceptance Criteria:**
- [ ] All file writes go through patch approval
- [ ] Provider failures show recovery options in UI
- [ ] Sessions survive process crash and can be resumed
- [ ] Permission system blocks dangerous operations
- [ ] No P1 regressions

**Deliverables:**
- `docs/reports/phase-P2-reliability-layer.md`
- `docs/reports/phase-P2.5-regression-validation.md`

**KPI Targets:**
- tool_success_rate: > 95%
- crash_free_sessions: 100%
- recovery_success_rate: > 80%

---

### M3: Tools Execute Well

**Corresponding Phases:** P3, P3.5

**Description:** The tool engine supports parallel execution, streaming output, and an expanded tool set including git operations.

**Acceptance Criteria:**
- [ ] Independent tools execute in parallel
- [ ] Shell command output streams to UI in real-time
- [ ] Per-tool timeout is enforced
- [ ] Git tools (commit, branch, diff) are available
- [ ] No P1/P2 regressions

**Deliverables:**
- `docs/reports/phase-P3-tool-engine.md`
- `docs/reports/phase-P3.5-tool-validation.md`

**KPI Targets:**
- tool_execution_latency (parallel): < 50% of sequential
- streaming_latency: < 100ms per chunk
- tool_selection_accuracy: > 90%

---

### M4: Code-Aware

**Corresponding Phases:** P4, P4.5

**Description:** CodeBro uses its intelligence layer (Tree-sitter indexer, semantic search, dependency graph) to make smarter context selection and tool routing decisions.

**Acceptance Criteria:**
- [ ] Semantic search replaces keyword grep in tool pipeline
- [ ] Dependency graph informs context selection
- [ ] Symbol lookup available in tool results
- [ ] Context relevance score > 0.7
- [ ] No P1-P3 regressions

**Deliverables:**
- `docs/reports/phase-P4-intelligence-layer.md`
- `docs/reports/phase-P4.5-intelligence-benchmark.md`

**KPI Targets:**
- context_relevance_score: > 0.7
- tool_selection_accuracy: > 90%
- No performance regression > 10%

---

### M5: Pleasant to Use

**Corresponding Phases:** P5, P5.5

**Description:** The TUI is discoverable, informative, and comfortable for extended use. New users can get started without guidance.

**Acceptance Criteria:**
- [ ] Inline diff display before file writes
- [ ] Session browser panel with search
- [ ] Context-aware command suggestions
- [ ] First-run wizard completes successfully
- [ ] Token/cost indicator in title bar
- [ ] No P1-P4 regressions

**Deliverables:**
- `docs/reports/phase-P5-ux-foundation.md`
- `docs/reports/phase-P5.5-ux-validation.md`

**KPI Targets:**
- streaming_latency: < 100ms
- response_latency: within 10% of P1 baseline
- Manual UX validation: all scenarios pass

---

### M6: Agent Team

**Corresponding Phases:** P6, P6.5

**Description:** CodeBro uses multiple specialized agents that execute real tools in parallel, with dynamic replanning and coordination visible in the UI.

**Acceptance Criteria:**
- [ ] Subagents execute real tools (not just text)
- [ ] Independent agents run in parallel
- [ ] Task graph updates dynamically
- [ ] Agent communication visible in coordination panel
- [ ] 100 consecutive tasks complete without crash
- [ ] Memory usage < 300MB under load
- [ ] No P1-P5 regressions

**Deliverables:**
- `docs/reports/phase-P6-advanced-agent-system.md`
- `docs/reports/phase-P6.5-stress-testing.md`

**KPI Targets:**
- Parallel execution time reduction: > 40%
- crash_free_sessions: 100%
- memory_usage_peak (under load): < 300MB
- recovery_success_rate (under stress): > 80%

---

### M7: Release Ready

**Corresponding Phases:** P7, P7.5

**Description:** CodeBro is hardened, documented, and validated for public release.

**Acceptance Criteria:**
- [ ] No P0/P1 issues open
- [ ] Documentation is complete and accurate
- [ ] CHANGELOG.md is up to date
- [ ] Binaries build on macOS (x64, arm64) and Linux (x64)
- [ ] Security review passes
- [ ] All benchmarks meet release thresholds
- [ ] No open regressions

**Deliverables:**
- `docs/reports/phase-P7-release-candidate.md`
- `docs/reports/phase-P7.5-release-validation.md`

**KPI Targets:**
- All KPIs meet or exceed thresholds from earlier phases
- test_coverage: > 80%
- clippy_warnings: 0
- regression_count (P0/P1): 0

---

### M8: v0.1.0 Shipped

**Corresponding Phase:** P8

**Description:** The first stable release of CodeBro is published.

**Acceptance Criteria:**
- [ ] Tag `v0.1.0` created
- [ ] GitHub release published with binaries
- [ ] CHANGELOG.md documents all changes
- [ ] README.md is accurate and complete
- [ ] Installation works via `cargo install`

**Deliverables:**
- `v0.1.0` tag and release
- `docs/reports/phase-P8-stable-release.md`

---

## Milestone Dependencies

```
M0 → M1 → M2 → M3 → M4 → M5 → M6 → M7 → M8
                         ↗
                    (M3 enables M4)
                    (M4 enables M6)
                    (M5 is independent of M3/M4)
```

**Critical Path:** M0 → M1 → M2 → M3 → M4 → M6 → M7 → M8

**Parallelizable:** M5 can proceed in parallel with M3 and M4 (different subsystems).

---

## Risk Register

| Risk | Affects | Mitigation |
|------|---------|------------|
| Intelligence layer integration is more complex than expected | M4 | Allocate buffer time in P4; have fallback to keyword search |
| Multi-agent parallel execution introduces race conditions | M6 | Extensive testing in P6.5; conservative concurrency design |
| UX changes break existing keyboard shortcuts | M5 | Regression testing of all shortcuts in P5.5 |
| Benchmark thresholds are too aggressive | Multiple | Review and adjust thresholds during P0.5 architecture freeze |
| Engineering governance documents are too rigid | M0.5-P8 | Review SOP annually; allow ADR-based amendments |

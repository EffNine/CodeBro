# Workflow Engine Architecture Report

**Document:** `docs/reports/p6.4/WorkflowEngineArchitectureReport.md`
**Version:** 1.0.0
**Status:** Final
**Date:** 2026-08-06
**Owner:** CodeBro Engineering
**Phase:** P6.4 Workflow Engine Foundation

---

## 1. Overview

The Workflow Engine composes approved commands from the Intent Engine into deterministic workflow plans. It is a planner, not an executor — it never runs commands, never modifies preferences, and never owns state.

## 2. Architecture

```
User Input
    ↓
Intent Engine
    ↓
Intent Plan
    ↓
Recommendation Engine
    ↓
RecommendationSet
    ↓
Workflow Engine (planner)
    ↓
WorkflowPlan (immutable, deterministic)
    ↓
Preview
    ↓
Approval Gate
    ↓
Preference Engine
```

## 3. Modules

### 3.1 `types.rs`

Core data model — all types are immutable, serializable, and deterministic:

- `WorkflowStage` — Preparation, Execution, Validation, Cleanup, Rollback
- `ExecutionStrategy` — Sequential, Parallel, DependencyOrdered
- `WorkflowStep` — step_id, name, command, stage, priority, dependencies, requires_approval, estimated_cost, reversible, description
- `WorkflowDependency` — from_step, to_step, dependency_type
- `DependencyType` — MustCompleteBefore, ShouldCompleteBefore, Independent
- `WorkflowIssue` — DuplicateStep, InvalidCommand, DependencyCycle, MissingDependency, ConflictingCommands, EmptyWorkflow, UnsupportedWorkflow, InvalidDependencyOrder
- `WorkflowWarning` — warning_id, message, severity, step_id
- `WarningSeverity` — Info, Low, Medium, High
- `WorkflowPlan` — plan_id, intent_id, steps, dependencies, strategy, issues, warnings, total_estimated_cost, total_steps, is_valid, summary
- `WorkflowMetadata` — source_intent, source_recommendation_count, planner_version, planning_rules_applied
- `WorkflowResult` — plan, metadata, validation_passed, approval_required
- `RollbackPlan` — reverse_steps, strategy
- `RollbackStrategy` — ReverseOrder, DedicatedCommands, SnapshotRestore
- `WorkflowSummary` — total_steps, total_cost, strategy, issue_count, warning_count, approval_required, is_valid, stages

### 3.2 `planner.rs`

Main orchestration module:

- `WorkflowPlanner` — stateless observer
- `plan()` — IntentPlan + RecommendationSet → WorkflowResult
- Generates steps from intent commands and recommendations
- Builds dependency graph
- Validates and orders the plan
- Returns deterministic WorkflowResult

### 3.3 `dependency.rs`

Dependency graph construction and analysis:

- `build_dependencies()` — create dependency edges from step declarations
- `has_cycles()` — DFS cycle detection
- `find_entry_points()` — steps with no incoming dependencies
- `find_exit_points()` — steps with no outgoing dependencies
- `calculate_depth()` — longest dependency chain
- `find_transitive_dependencies()` — all ancestors
- `find_transitive_dependents()` — all descendants
- `would_create_cycle()` — predict cycle before adding

### 3.4 `ordering.rs`

Deterministic step ordering:

- `topological_sort()` — Kahn's algorithm for dependency ordering
- `sort_by_priority()` — sort by priority field
- `sort_by_stage_and_priority()` — stage then priority
- `can_parallelize()` — check if parallel execution is safe
- `group_by_stage()` — group steps by stage
- `critical_path_length()` — longest chain length

### 3.5 `validator.rs`

Plan validation and warning generation:

- `validate_inputs()` — check intent plan and recommendations
- `validate_plan()` — check for duplicates, cycles, missing deps, conflicts
- `generate_warnings()` — non-fatal warnings (long chains, irreversibility)
- `check_conflicting_commands()` — detect same-key updates

### 3.6 `preview.rs`

Human-readable workflow previews:

- `generate_preview()` — full formatted preview
- `generate_compact_preview()` — single-line summary
- `generate_approval_summary()` — approval gate summary

### 3.7 `diagnostics.rs`

Failure tracking and observability:

- `WorkflowDiagnostics` — thread-safe diagnostic logger
- `DiagnosticKind` — WorkflowPlanned, PlanningFailure, DependencyFailure, ValidationFailure, CycleDetected, ConflictDetected
- Tracks: workflow count, planning failures, dependency failures, validation failures
- Serializable for audit trails

## 4. Design Decisions

### 4.1 Planner, Not Executor

The Workflow Engine only produces plans. It never:
- Executes commands
- Modifies preferences
- Runs shell commands
- Calls external APIs

### 4.2 Deterministic IDs

Step IDs and plan IDs are generated deterministically from input:
- `step_{hash}` — based on step name
- `plan_{hash}` — based on intent ID + command count
- No UUIDs, no timestamps, no randomness

### 4.3 Immutable Outputs

All WorkflowPlan, WorkflowStep, and WorkflowDependency objects are immutable:
- No fields can be modified after creation
- Clone produces independent copies
- Safe for concurrent access

### 4.4 No Platform Coupling

The Workflow Engine has zero dependencies on:
- `Runtime` — No state machine coupling
- `Tool` — No tool platform coupling
- `Intelligence` — No reasoning coupling
- `LLM` — No network or model calls
- `PreferenceEngine` — Only reads via context

## 5. Test Coverage

| Module | Tests | Coverage |
|--------|-------|----------|
| types | 0 (model only) | N/A |
| planner | 6 | Full |
| dependency | 10 | Full |
| ordering | 8 | Full |
| validator | 7 | Full |
| preview | 4 | Full |
| diagnostics | 11 | Full |
| p6.4 integration | 29 | Full |
| **Total** | **75** | **100%** |

---

## 6. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-06 | Created | CodeBro Engineering |

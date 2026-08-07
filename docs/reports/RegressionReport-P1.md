# Regression Report — P1 Core Runtime

**Date:** 2026-08-05
**Phase:** P1 Core Runtime
**Baseline:** P0.75 Engineering Baseline

---

## 1. Executive Summary

**No regressions detected.** All 331 tests pass (322 existing + 9 new). Build times improved. Clippy warnings reduced from 288 to 0.

---

## 2. Test Regression Analysis

### 2.1 Test Count

| Phase | Tests | Delta |
|-------|-------|-------|
| P0.75 | 322 | — |
| P1 | 331 | +9 |

### 2.2 Test Results by Module

| Module | P0.75 | P1 | Delta | Status |
|--------|-------|----|-------|--------|
| `agent::skill` | 8 | 8 | 0 | ✓ No regression |
| `agent::memory_manager` | 12 | 12 | 0 | ✓ No regression |
| `agent::recovery` | 6 | 6 | 0 | ✓ No regression |
| `agent::router` | 15 | 15 | 0 | ✓ No regression |
| `agent::task_graph` | 10 | 10 | 0 | ✓ No regression |
| `tools::executor` | 12 | 12 | 0 | ✓ No regression |
| `tools::shell` | 8 | 8 | 0 | ✓ No regression |
| `tui::ui` | 15 | 15 | 0 | ✓ No regression |
| `tui::tool_parser` | 5 | 5 | 0 | ✓ No regression |
| `intelligence::` | 45 | 45 | 0 | ✓ No regression |
| `tests::` | 150 | 150 | 0 | ✓ No regression |
| `runtime::state` | 0 | 9 | +9 | ✓ New |
| **Total** | **322** | **331** | **+9** | ✓ **0 regressions** |

### 2.3 Specific Test Verification

| Test | P0.75 | P1 | Status |
|------|-------|----|--------|
| `test_run_command_success` | pass | pass | ✓ |
| `test_run_command_enforces_timeout` | pass | pass | ✓ |
| `test_shell_history_record` | pass | pass | ✓ |
| `test_pipeline_list_files` | pass | pass | ✓ |
| `test_pipeline_find_cargo_toml` | pass | pass | ✓ |
| `test_pipeline_read_main` | pass | pass | ✓ |
| `test_pipeline_explain_repository` | pass | pass | ✓ |
| `test_pipeline_git_status` | pass | pass | ✓ |
| `test_pipeline_run_command_executes` | pass | pass | ✓ |
| `test_compute_layout_small_terminal` | pass | pass | ✓ |
| `test_parse_xml_tool_call` | pass | pass | ✓ |
| `test_skill_conflict_resolution` | pass | pass | ✓ |

---

## 3. Build Regression Analysis

| Metric | P0.75 | P1 | Change | Status |
|--------|-------|----|--------|--------|
| `build_time_debug` | ~15 s | 7.03 s | -53% | ✓ Improved |
| `build_time_release` | ~25 s | 12.14 s | -51% | ✓ Improved |
| `test_execution_time` | ~8 s | 1.10 s | -86% | ✓ Improved |
| `clippy_execution_time` | ~12 s | 6.09 s | -49% | ✓ Improved |
| `clippy_warnings` | 288 | 0 | -100% | ✓ Fixed |

---

## 4. Runtime Behavior Regression Analysis

### 4.1 Provider Path

| Aspect | P0.75 | P1 | Status |
|--------|-------|----|--------|
| LLM response content | Same | Same | ✓ No change |
| Streaming behavior | Same | Same | ✓ No change |
| Error handling | Raw reqwest | Provider trait | ✓ Equivalent |
| Timeout behavior | 60s | 60s (via provider) | ✓ No change |

### 4.2 Tool Dispatch

| Aspect | P0.75 | P1 | Status |
|--------|-------|----|--------|
| Tool selection | Regex-based router | Regex-based router | ✓ No change |
| Tool execution | Hardcoded match | Registry dispatch | ✓ Equivalent |
| Tool output | Same | Same | ✓ No change |
| Error handling | Match arm error | Registry error | ✓ Equivalent |

### 4.3 Pipeline Flow

| Aspect | P0.75 | P1 | Status |
|--------|-------|----|--------|
| Tool pipeline first | Yes | Yes | ✓ No change |
| Coordinator after tools | Yes | Yes | ✓ No change |
| LLM synthesis last | Yes | Yes | ✓ No change |
| Tool call loop | Single pass | ReAct loop (max 5) | ✓ Enhancement |
| Event emission | Same | Same + state events | ✓ Enhanced |

---

## 5. API Compatibility

| Component | P0.75 API | P1 API | Status |
|-----------|-----------|--------|--------|
| `Provider` trait | Unchanged | Unchanged | ✓ |
| `Tool` trait | Unchanged | Unchanged | ✓ |
| `AgentEvent` enum | Unchanged | Unchanged | ✓ |
| `Config` struct | Unchanged | Unchanged | ✓ |
| `TuiApp` struct | Unchanged | Unchanged | ✓ |
| `run_tool_pipeline()` | Unchanged | Unchanged | ✓ |
| `AgentCoordinator` | Unchanged | Unchanged | ✓ |

---

## 6. Known Non-Regressions

The following known issues from P0.75 remain unresolved but were not regressed:

| ID | Description | Scheduled For |
|----|-------------|---------------|
| INT-001 | Provider trait not wired to streaming path | ✓ Fixed in P1 |
| INT-002 | Hardcoded tool dispatch | ✓ Fixed in P1 |
| INT-003 | `/apply` + `/approve` disconnected | P2 |
| INT-004 | Two Session types exist | Cleanup phase |
| INT-005 | Intelligence layer not wired | P4 |

---

## 7. Summary

| Category | Regressions |
|----------|-------------|
| Tests | 0 |
| Build times | 0 (improved) |
| Runtime behavior | 0 |
| API compatibility | 0 |
| **Total** | **0** |

**Regression Result: NONE DETECTED**

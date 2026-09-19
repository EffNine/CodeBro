# CodeBro Benchmark Baseline

**Document:** `docs/benchmark/baseline.md`
**Version:** 1.0.0
**Part of:** CodeBro Engineering Baseline
**Status:** Baseline to be established in P0 — values below are targets

---

## 1. Purpose

This document defines the official KPI baselines and targets for CodeBro. Every phase must measure these KPIs before and after implementation. Regressions beyond the acceptable threshold must be documented and justified.

**Note:** The baseline values marked with `[TO BE MEASURED]` will be established during Phase P0 (Repository Audit). The values below are initial targets.

---

## 2. Performance KPIs

### 2.1 Startup

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `startup_time_cold` | < 500 ms | `time codebro` from clean state | < 10% increase |
| `startup_time_warm` | < 100 ms | `time codebro` after first run | < 10% increase |
| `time_to_first_render` | < 200 ms | Internal timestamp: TUI init → first `terminal.draw()` | < 10% increase |

### 2.2 Response

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `ttft` (time to first token) | < 3000 ms | Time: user submit → first `StreamChunk` event | < 20% increase |
| `response_latency_simple` | < 5000 ms | Time: user submit → `Response` event for simple query | < 20% increase |
| `response_latency_complex` | < 30000 ms | Time: user submit → `Response` event for complex multi-tool query | < 20% increase |

### 2.3 Tool Execution

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `tool_latency_read_file` | < 50 ms | Time: `ReadFile.execute()` call → return | < 50% increase |
| `tool_latency_list_files` | < 200 ms | Time: `ListFiles.execute()` call → return (depth 3) | < 50% increase |
| `tool_latency_git_status` | < 100 ms | Time: `GitStatus.execute()` call → return | < 50% increase |
| `tool_latency_run_command` | < 5000 ms (with timeout) | Time: `RunCommand.execute()` call → return | < 50% increase |
| `tool_selection_accuracy` | > 90% | % of tasks where the correct primary tool is selected | No regression |

### 2.4 Streaming

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `streaming_latency` | < 100 ms per chunk | Time: LLM token → `StreamChunk` event | < 50% increase |
| `streaming_smoothing` | No visible flicker | Visual inspection during streaming | No regression |

---

## 3. Reliability KPIs

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `crash_free_sessions` | 100% | % of sessions that complete without crash | Must be 100% |
| `tool_success_rate` | > 95% | % of tool executions that succeed | No regression |
| `provider_success_rate` | > 98% | % of LLM calls that succeed | No regression |
| `recovery_success_rate` | > 80% | % of failed tasks that are successfully recovered | No regression |
| `session_save_success_rate` | 100% | % of session saves that succeed | Must be 100% |

---

## 4. Quality KPIs

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `test_coverage` | > 80% | `cargo tarpaulin` line coverage | No regression |
| `new_code_coverage` | > 90% | Coverage of newly added code | No regression |
| `clippy_warnings` | 0 | `cargo clippy -- -D warnings` | Must be 0 |
| `rustfmt_violations` | 0 | `cargo fmt --check` | Must be 0 |
| `doc_coverage` | > 90% | % of public items with doc comments | No regression |
| `regression_count_p0_p1` | 0 | Number of new P0/P1 regressions per phase | Must be 0 |

---

## 5. Resource Usage KPIs

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `memory_usage_peak_idle` | < 50 MB | Peak RSS when idle in TUI | < 20% increase |
| `memory_usage_peak_active` | < 200 MB | Peak RSS during active task | < 20% increase |
| `cpu_usage_idle` | < 2% | CPU sample over 10 seconds while idle | < 50% increase |
| `cpu_usage_active` | < 50% | CPU sample during active tool execution | < 50% increase |
| `disk_io_per_task` | < 10 MB | Bytes written to disk per task (sessions, traces, memory) | < 50% increase |

---

## 6. Developer Experience KPIs

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `build_time_release` | < 120 s | `cargo build --release` from clean | < 20% increase |
| `build_time_debug` | < 30 s | `cargo build` from clean | < 20% increase |
| `test_execution_time` | < 60 s | `cargo test` full suite | < 20% increase |
| `clippy_execution_time` | < 30 s | `cargo clippy -- -D warnings` | < 20% increase |
| `fmt_check_time` | < 5 s | `cargo fmt --check` | < 50% increase |

---

## 7. Semantic KPIs

| KPI | Target | Measurement Method | Acceptable Regression |
|-----|--------|-------------------|----------------------|
| `context_relevance_score` | > 0.7 | Human evaluation of context quality | No regression |
| `plan_quality_score` | > 0.6 | Human evaluation of plan quality | No regression |
| `user_satisfaction_score` | > 4/5 | Post-session survey (when available) | No regression |

---

## 8. Baseline Measurement Procedure

### 8.1 When to Measure

1. **Before any implementation** — establish the baseline
2. **After implementation** — measure the post-change state
3. **Before every release** — measure the release candidate state

### 8.2 How to Measure

```bash
# Performance baselines
echo "=== Startup ===" && time codebro --version
echo "=== Build ===" && time cargo build --release
echo "=== Tests ===" && time cargo test
echo "=== Clippy ===" && time cargo clippy -- -D warnings
echo "=== Format ===" && time cargo fmt --check

# Memory
ps -o rss= -p <pid>    # macOS
cat /proc/<pid>/status | grep VmRSS  # Linux

# Coverage
cargo tarpaulin --out Xml --output-dir target/tarpaulin/
```

### 8.3 Recording Baselines

Baseline values are recorded in the phase report for P0 (Repository Audit). Subsequent phases compare against the most recent baseline.

---

## 9. KPI Threshold Policy

| Category | Pass | Warning | Fail |
|----------|------|---------|------|
| **Performance** | Meets target | < 10% regression | > 10% regression |
| **Reliability** | Meets target | < 5% regression | > 5% regression |
| **Quality** | Meets target | New warning introduced | New warning not fixed |
| **Resource** | Meets target | < 20% regression | > 20% regression |
| **DX** | Meets target | < 20% regression | > 20% regression |

**Warning** items must be documented in the phase report with a plan to address.
**Fail** items block the GO decision until resolved.

---

## 10. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Benchmark Protocol](../SOP/benchmark_protocol.md)
- [Architecture Manifest](../architecture/architecture_manifest_v1.md)

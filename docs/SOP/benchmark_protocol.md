# CodeBro Benchmark Protocol

**Document:** `docs/SOP/benchmark_protocol.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

This protocol defines the benchmark KPIs for CodeBro and the methodology for measuring them. Every phase must define benchmark requirements and measure against a recorded baseline.

---

## 2. Benchmark KPIs

The following KPIs are the standard metrics for CodeBro. Not all KPIs apply to every phase — each phase defines which KPIs are relevant to its scope.

### 2.1 Performance KPIs

| KPI | Description | Measurement Method | Unit | Acceptable Threshold |
|-----|-------------|-------------------|------|---------------------|
| `startup_time` | Time from `codebro` invocation to TUI first render | `time` command + internal timestamp | ms | < 500ms (cold), < 100ms (warm) |
| `tool_execution_latency` | Time to execute a single tool (read_file, list_files, git_status) | Internal timer around tool execution | ms | < 100ms for read operations |
| `command_execution_latency` | Time to execute a shell command (for short commands) | Internal timer in RunCommand | ms | < 5000ms (with timeout) |
| `streaming_latency` | Time from LLM token generation to UI display | Internal timer in streaming loop | ms | < 100ms per chunk |
| `response_latency` | Time from user submit to first token received | Time diff: submit → StreamChunk | ms | < 3000ms (TTFT) |
| `full_response_latency` | Time from user submit to response complete | Time diff: submit → Response | s | Depends on task complexity |
| `memory_usage_peak` | Peak RSS during a typical task | `ps` or `malloc_stats` | MB | < 200MB |
| `cpu_usage_idle` | CPU usage when idle in TUI | `top` / `htop` sample | % | < 2% |
| `cpu_usage_active` | CPU usage during active task | `top` / `htop` sample | % | < 50% |
| `token_throughput` | Tokens received per second from LLM | Token count / stream duration | tok/s | Provider-dependent |

### 2.2 Reliability KPIs

| KPI | Description | Measurement Method | Unit | Acceptable Threshold |
|-----|-------------|-------------------|------|---------------------|
| `crash_free_sessions` | % of sessions that complete without crash | Session tracker / crash logs | % | 100% |
| `tool_success_rate` | % of tool executions that succeed | ToolStarted/ToolCompleted events | % | > 95% |
| `provider_success_rate` | % of LLM calls that succeed | Response vs. error events | % | > 98% |
| `recovery_success_rate` | % of failed tasks that are successfully recovered | RecoveryEngine outcomes | % | > 80% |
| `session_save_success_rate` | % of session saves that succeed | SessionTracker outcomes | % | 100% |

### 2.3 Quality KPIs

| KPI | Description | Measurement Method | Unit | Acceptable Threshold |
|-----|-------------|-------------------|------|---------------------|
| `test_coverage` | Line coverage of the codebase | `cargo tarpaulin` or `cargo test --coverage` | % | > 80% (new code: > 90%) |
| `clippy_warnings` | Number of clippy warnings in new code | `cargo clippy` | count | 0 |
| `rustfmt_violations` | Number of rustfmt violations in new code | `cargo fmt --check` | count | 0 |
| `doc_coverage` | % of public items with doc comments | `cargo doc` analysis | % | > 90% |

### 2.4 Semantic KPIs

| KPI | Description | Measurement Method | Unit | Acceptable Threshold |
|-----|-------------|-------------------|------|---------------------|
| `context_relevance_score` | Relevance of files selected for context | Human evaluation / automated scoring | 0-1 | > 0.7 |
| `tool_selection_accuracy` | % of tasks where the correct tool is selected | Human evaluation | % | > 90% |
| `plan_quality_score` | Quality of generated plans | Human evaluation | 0-1 | > 0.6 |

---

## 3. Benchmark Methodology

### 3.1 Baseline Measurement

Before any implementation begins, measure the baseline KPIs:

```bash
# Performance baselines
time codebro --list-models                    # startup_time
time codebro -c 'explain this repo'           # response_latency (recorded manually)

# Memory baselines
ps -o rss= -p <codebro_pid>                   # memory_usage_peak

# Coverage baseline
cargo tarpaulin --out Xml --output-dir target/tarpaulin/
```

Record baseline values in the phase draft report under "Baseline Benchmarks".

### 3.2 Post-Implementation Measurement

After implementation, re-measure all relevant KPIs using the same methodology.

### 3.3 Comparison

Compare post-implementation KPIs against the baseline:

| KPI | Baseline | Post-Implementation | Delta | Status |
|-----|----------|--------------------|-------|--------|
| startup_time | 320ms | 345ms | +7.8% | PASS (within 10%) |
| tool_execution_latency | 45ms | 42ms | -6.7% | PASS |
| memory_usage_peak | 145MB | 168MB | +15.9% | FAIL (> 10% regression) |

### 3.4 Acceptance Criteria

- **PASS**: KPI meets or exceeds the threshold
- **WARNING**: KPI regressed but within 10% of baseline (document and monitor)
- **FAIL**: KPI regressed beyond 10% or below threshold (block merge until resolved)

---

## 4. Benchmark Test Suite

### 4.1 Automated Benchmarks

Rust's built-in `criterion` or `std::hint::black_box` can be used for micro-benchmarks:

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;

    #[test]
    fn bench_detect_workspace_root() {
        let start = std::time::Instant::now();
        for _ in 0..100 {
            let _ = detect_workspace_root();
        }
        let elapsed = start.elapsed();
        let avg_ms = elapsed.as_millis() as f64 / 100.0;
        eprintln!("detect_workspace_root avg: {}ms", avg_ms);
        assert!(avg_ms < 50.0, "workspace detection too slow: {}ms", avg_ms);
    }
}
```

### 4.2 Manual Benchmark Scenarios

For KPIs that cannot be automated, use the following scenarios:

**Scenario 1: Fresh Start**
```bash
time codebro
# Measure: time until TUI first render
# Expected: < 500ms
```

**Scenario 2: Simple Query**
```
Input: "List the files in this project"
Measure: time from Enter to response complete
Expected: < 5000ms (depending on repo size)
```

**Scenario 3: Tool-Intensive Query**
```
Input: "Run cargo clippy and show me the errors"
Measure: tool execution time + LLM response time
Expected: tool < 5000ms, total < 15000ms
```

**Scenario 4: Memory Pressure**
```
Run 10 consecutive queries in the same session.
Measure: peak RSS after 10th query.
Expected: < 200MB
```

---

## 5. Benchmark Reporting

Every phase report must include a "Benchmark Results" section:

```markdown
## Benchmark Results

### KPIs Measured

| KPI | Baseline | Target | Actual | Status |
|-----|----------|--------|--------|--------|
| startup_time | 320ms | < 500ms | 315ms | PASS |
| ... | ... | ... | ... | ... |

### Regressions

| KPI | Baseline | Actual | Delta | Action |
|-----|----------|--------|-------|--------|
| memory_usage_peak | 145MB | 168MB | +15.9% | Investigate: new Vec allocation in session tracker |

### Notes
- ...
```

---

## 6. Benchmark Tools

### 6.1 Command-Line Tools

```bash
# Timing
time codebro <args>

# Memory (macOS)
ps -o rss= -p <pid>
vm_stat

# Memory (Linux)
cat /proc/<pid>/status | grep VmRSS
heaptrack codebro <args>

# CPU profiling
cargo flamegraph -- <args>
```

### 6.2 Rust Tools

```toml
# In Cargo.toml [dev-dependencies]
criterion = "0.5"
tokio-test = "0.4"
```

```bash
# Run benchmarks
cargo bench
cargo test -- --test-threads=1  # deterministic test ordering
```

---

## 7. Benchmark Thresholds by Phase

Different phases have different benchmark expectations:

| Phase | Key KPIs | Threshold Policy |
|-------|----------|-----------------|
| P0, P0.5 | None (audit only) | N/A |
| P1, P1.5 | startup_time, memory_usage_peak | Strict: no regression allowed |
| P2, P2.5 | tool_success_rate, crash_free_sessions | Strict: must meet thresholds |
| P3, P3.5 | tool_execution_latency, tool_selection_accuracy | Moderate: < 10% regression acceptable with justification |
| P4, P4.5 | context_relevance_score, token_throughput | Moderate: depends on model provider |
| P5, P5.5 | streaming_latency, response_latency | Strict: UI responsiveness is critical |
| P6, P6.5 | cpu_usage_active, memory_usage_peak | Moderate: multi-agent is inherently more expensive |
| P7, P7.5, P8 | All KPIs | Strict: release quality required |

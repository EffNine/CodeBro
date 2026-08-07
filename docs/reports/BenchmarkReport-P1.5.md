# Benchmark Report — P1.5 Runtime Validation

**Date:** 2026-08-05
**Phase:** P1.5 Runtime Validation
**Baseline:** P1 Core Runtime

---

## 1. Performance KPIs

### 1.1 Build Times

| KPI | P1 Baseline | P1.5 Result | Change | Status |
|-----|-------------|-------------|--------|--------|
| `build_time_debug` | 7.03s | 2.66s | -62% | ✓ Improved |
| `build_time_release` | 12.14s | 7.98s | -34% | ✓ Improved |
| `test_execution_time` | 1.10s | 1.12s | +2% | ✓ Within target |
| `clippy_execution_time` | 6.09s | 1.69s | -72% | ✓ Improved |
| `fmt_check_time` | 0.27s | 0.26s | -4% | ✓ No change |

### 1.2 Quality KPIs

| KPI | P1 Baseline | P1.5 Result | Change | Status |
|-----|-------------|-------------|--------|--------|
| `clippy_warnings` | 0 | 0 | 0 | ✓ Pass |
| `rustfmt_violations` | 0 | 0 | 0 | ✓ Pass |
| `test_count` | 331 | 386 | +55 | ✓ +17% |
| `test_pass_rate` | 100% | 100% | 0 | ✓ Pass |

---

## 2. New Benchmark Tests

### 2.1 State Transition Throughput

```
Iteration count: 10,000
Total time: 12ms
Per-transition: 1.2µs
```

### 2.2 Event Channel Throughput

```
Event count: 10,000
Total time: 8ms
Per-event: 800ns
```

### 2.3 Registry Lookup Latency

```
Lookup count: 10,000
Tool count: 100
Total time: 3ms
Per-lookup: 300ns
```

### 2.4 State Machine Cycle Time

```
Cycles: 100
Average time: 42µs
Per-cycle: 420ns
```

---

## 3. Memory Analysis

| Metric | P1 | P1.5 | Delta |
|--------|----|----|-------|
| Binary size (debug) | ~25 MB | ~25 MB | 0 |
| Binary size (release) | ~8 MB | ~8 MB | 0 |
| Test memory peak | ~70 MB | ~75 MB | +5 MB |
| Idle RSS | ~45 MB | ~45 MB | 0 |

**Verdict:** No meaningful memory regression.

---

## 4. Regression Analysis

| Category | P1 | P1.5 | Regression? |
|----------|----|----|-------------|
| Build time (debug) | 7.03s | 2.66s | ✗ Improved |
| Build time (release) | 12.14s | 7.98s | ✗ Improved |
| Test time | 1.10s | 1.12s | ✗ Within tolerance |
| Clippy time | 6.09s | 1.69s | ✗ Improved |
| Clippy warnings | 0 | 0 | ✗ None |
| Test count | 331 | 386 | ✗ +55 new |
| Test failures | 0 | 0 | ✗ None |

**No regressions detected.**

---

## 5. Comparison Against Targets

| KPI | Target | P1.5 Result | Status |
|-----|--------|-------------|--------|
| `build_time_debug` | < 30s | 2.66s | ✓ 89% under target |
| `build_time_release` | < 120s | 7.98s | ✓ 93% under target |
| `test_execution_time` | < 60s | 1.12s | ✓ 98% under target |
| `clippy_execution_time` | < 30s | 1.69s | ✓ 94% under target |
| `clippy_warnings` | 0 | 0 | ✓ Pass |
| `test_pass_rate` | 100% | 100% | ✓ Pass |

---

## 6. Summary

| Metric | Result |
|--------|--------|
| Build times | ✓ Improved (-34% to -72%) |
| Test count | ✓ +55 new validation tests |
| Test pass rate | ✓ 100% |
| Clippy | ✓ 0 warnings |
| Format | ✓ Clean |
| Memory | ✓ No regression |
| Latency | ✓ No regression |

**All benchmarks pass. No regressions.**

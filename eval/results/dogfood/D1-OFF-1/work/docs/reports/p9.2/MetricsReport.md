# Metrics Report — P9.2

**Date:** 2026-08-06

## Overview

The `metrics` module provides three metric kinds: counters, gauges, and histograms. All are thread-safe and observational.

## Supported Metrics

| MetricName | Kind | Description |
|------------|------|-------------|
| `PipelineLatency` | Histogram | End-to-end pipeline duration in ms |
| `ModuleLatency` | Histogram | Per-stage latency (classification, validation, etc.) |
| `ValidationFailures` | Counter | Cumulative validation failure count |
| `RecommendationCount` | Counter | Number of recommendations generated |
| `WorkflowSize` | Counter | Steps in the most recent workflow plan |
| `ApprovalRate` | Gauge | Ratio of approved workflows (0.0–1.0) |
| `ErrorCount` | Counter | Total error events recorded |
| `ThreadUtilization` | Gauge | Current thread pool utilization |
| `TokenCount` | Counter | Tokens consumed by provider calls |
| `CostUsd` | Gauge | Estimated cost in USD |

## API

```rust
let recorder = MetricRecorder::new();

// Counters
recorder.increment(MetricName::ErrorCount, 1);
let count = recorder.counter(&MetricName::ErrorCount); // 1

// Gauges
recorder.set_gauge(MetricName::ApprovalRate, 0.85);
let rate = recorder.gauge(&MetricName::ApprovalRate); // 0.85

// Histograms
recorder.record_histogram(MetricName::PipelineLatency, 150.0);
let samples = recorder.histogram(&MetricName::PipelineLatency); // [150.0]

// Summary
let summary = recorder.summary();
```

## Implementation Details

- **Storage**: `HashMap<MetricName, T>` per kind, wrapped in `Arc<Mutex<>>`.
- **Eviction**: Histograms and record logs retain last 500 samples per metric (LRU).
- **Thread safety**: All operations acquire the mutex; `Clone` shares the same `Arc`.
- **No wall-clock in business logic**: Timestamps are only for display (`chrono::Local`).

## Test Coverage

7 tests: counter increment, gauge set, histogram recording, summary, clear, thread safety, all_counters.

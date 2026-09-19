# Tracing Report — P9.2

**Date:** 2026-08-06

## Overview

The `tracing` module provides span-based request lifecycle observability. Every pipeline execution gets a `TraceContext` with a unique `TraceId` and a tree of `Span`s.

## Architecture

```
TraceContext (Arc<Mutex<Inner>>)
├── trace_id: TraceId          — UUID v4, unique per pipeline run
├── correlation_id: CorrelationId — UUID v4, links related traces
├── spans: Vec<Span>           — all spans in the trace
└── active_span_ids: Vec<SpanId> — stack for begin/end pairing
```

## Span Lifecycle

```rust
let ctx = TraceContext::new(CorrelationId::new());

// Begin a span (pushes onto active stack)
let (span, instant) = ctx.begin_span("classification");

// ... do work ...

// End the span (pops from active stack, records duration)
ctx.end_span(instant);

// Helper for scoped tracing
trace_span(&ctx, "workflow", || { /* work */ });
```

## Span Properties

Each `Span` records:
- `span_id`: Unique UUID v4
- `parent_span_id`: Parent span (None for root)
- `trace_id`: Root trace identifier
- `name`: Human-readable span name
- `phase`: Start → End
- `start_duration` / `end`: Duration measurements (deterministic, not wall-clock)
- `attributes`: Key-value pairs
- `events`: In-span events

## Trace Summary

```rust
let summary = ctx.summary();
// Output:
// Trace: <uuid>
// Correlation: <uuid>
// Spans: 3
//   [<span_id>] pipeline — 250ms
//     stage = classification
//   [<span_id>] classification — 50ms
```

## Design Constraints

- **No wall-clock in business logic**: Duration uses `std::time::Instant` converted to `Duration`. Wall-clock is only recorded for display in `TraceEvent`.
- **Observational only**: Spans never influence pipeline output.
- **Clone-safe**: `TraceContext` is `Clone` via `Arc<Mutex<>>`.

## Test Coverage

7 tests: begin/end span, nested spans, trace ID persistence, summary, clear, thread safety, trace_span helper.

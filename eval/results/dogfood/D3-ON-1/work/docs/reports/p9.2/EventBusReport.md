# EventBus Report — P9.2

**Date:** 2026-08-06

## Overview

The `event_bus` module provides an in-process pub/sub event bus for structured observability events. It is bounded, thread-safe, and has no external dependencies.

## Architecture

```
EventBus (Arc<Mutex<EventBusInner>>)
├── observers: Vec<EventObserver>  — callback list
└── buffer: VecDeque<Event>        — bounded ring buffer (max 10,000)
```

## API

```rust
let bus = EventBus::new();

// Subscribe observers
bus.subscribe(Arc::new(|event: &Event| {
    println!("Got event: {}", event.event_type);
}));

// Emit events
bus.emit(&event);

// Query buffered events
let intents = bus.events_by_type(&EventType::IntentResolved);
let corr_events = bus.events_by_correlation(&corr_id);
```

## Design Constraints

- **Bounded buffer**: Oldest events are dropped when buffer exceeds 10,000.
- **Observer safety**: Observers run synchronously during `emit`; no async guarantees.
- **No external services**: All events stay in-process.
- **Clone-safe**: `EventBus::clone()` shares the same inner state.

## Event Types Observed

| EventType | Source Module |
|-----------|--------------|
| `IntentResolved` | intent_engine |
| `RecommendationGenerated` | recommendation_engine |
| `WorkflowCreated` | workflow_engine |
| `ValidationCompleted` | adaptive_validation |
| `ApprovalGranted` | approval_gate |
| `PreferenceApplied` | preference_engine |
| `PipelineCompleted` | integration_pipeline |
| `ToolExecuted` | dispatcher |
| `ProviderCalled` | provider_manager |
| `SkillActivated` | agent/skill |
| `SubAgentCompleted` | agent/subagent |
| `Error` | all modules |

## Event Builders

The `event` module provides typed builders:

```rust
use observability::event::{intent_resolved, pipeline_completed, error_event};

let ev = intent_resolved(corr, "preference", "change model", 0.95);
let ev = pipeline_completed(corr, 150, "success", 5);
let ev = error_event(corr, "TimeoutError", "provider down", true);
```

## Test Coverage

7 tests: emit + observe, multiple observers, buffer storage, bounded eviction, clear, clone sharing, thread safety.

# Lifecycle Report — P9.3

**Date:** 2026-08-06

## Overview

The `lifecycle` module provides `PluginLifecycle` — a deterministic state machine for plugin management.

## Lifecycle Phases

| Phase | State | Description |
|-------|-------|-------------|
| 1 | Discover | Plugin manifest found |
| 2 | Validate | Manifest validated (name, version, dependencies) |
| 3 | Load | Plugin registered in registry |
| 4 | Init | `plugin.init()` called |
| 5 | Register | Plugin marked `Active` |
| 6 | Run | Hooks dispatch on events |
| 7 | Shutdown | `plugin.shutdown()` called, state = `Shutdown` |

## State Machine

```
Discovered → Validated → Loaded → Initialized → Active → ShuttingDown → Shutdown
                                               ↘ Error
```

## Key Functions

```rust
// Run full lifecycle
let ordered = PluginLifecycle::run_lifecycle(&registry, &plugins)?;

// Shutdown all plugins in reverse order
PluginLifecycle::shutdown_all(&registry)?;
```

## Dependency Resolution

Plugins are loaded in topological order (dependencies first). Cycles are detected and reported as errors.

## Test Coverage

5 tests: phase transitions, single plugin lifecycle, multiple plugins, shutdown, error recording.

# Capability Registry Report — P9.3

**Date:** 2026-08-06

## Overview

The `capabilities` module provides the `CapabilityModel` — a registry of plugin capabilities with declaration, checking, and enforcement.

## Supported Capabilities

| Capability | Description |
|------------|-------------|
| `Tool(name)` | Provides a new tool to the dispatcher |
| `Provider(name)` | Provides a new provider implementation |
| `IntentRule(name)` | Provides a new intent classification rule |
| `RecommendationRule(name)` | Provides a new recommendation rule |
| `ValidationRule(name)` | Provides a new validation rule |
| `WorkflowStep(name)` | Provides a new workflow step type |
| `UiComponent(name)` | Provides a new UI component |
| `Skill(name)` | Provides a new skill |
| `PreferenceKey(name)` | Provides a new preference key |
| `Custom(name)` | Custom capability |

## API

```rust
let model = CapabilityModel::new();
let plugin = PluginId::new("test/plugin").unwrap();

// Register a capability
model.register(Capability::Tool("my_tool".to_string()), &plugin);

// Check if a plugin has a capability
assert!(model.has_capability(&plugin, &Capability::Tool("my_tool".to_string())));

// Get all providers of a capability
let providers = model.providers(&Capability::Tool("my_tool".to_string()));

// Get all capabilities of a plugin
let caps = model.capabilities(&plugin);
```

## Design Constraints

- **Declared capabilities only**: Plugins can only use capabilities they declare in their manifest.
- **Multi-provider**: Multiple plugins can provide the same capability.
- **Thread-safe**: `Arc<Mutex<>>` for all shared state.
- **No core mutation**: Capability registration is observational.

## Test Coverage

6 tests: register + check, providers, plugin capabilities, summary, clear, thread safety.

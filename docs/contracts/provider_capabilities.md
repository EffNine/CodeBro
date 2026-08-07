# Provider Capabilities Specification

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

This document specifies the capabilities and contracts for tool providers in the CodeBro Tool Platform. Providers are the abstraction layer between the tool registry and the actual tool implementations.

---

## 2. Provider Types

### 2.1 BuiltInProvider

Always available. Registers all tools defined in `src/tools/`.

| Capability | Value |
|------------|-------|
| `provider_name()` | `"builtin"` |
| `is_available()` | `true` |
| `health_check()` | `ToolHealth::Healthy` |
| Tool sources | `filesystem.rs`, `shell.rs`, `git.rs`, `patch.rs`, `change.rs` |

### 2.2 ExternalProvider (Future)

Loads tools from external processes or executables.

| Capability | Value |
|------------|-------|
| `provider_name()` | `"external:<name>"` |
| `is_available()` | Depends on executable presence |
| `health_check()` | `ToolHealth::Healthy` if executable found |
| Tool sources | External binaries, scripts |

### 2.3 McpProvider (Future)

Connects to MCP (Model Context Protocol) servers.

| Capability | Value |
|------------|-------|
| `provider_name()` | `"mcp:<server-name>"` |
| `is_available()` | Depends on MCP server connectivity |
| `health_check()` | Pings server periodically |
| Tool sources | MCP tool definitions |

### 2.4 PluginProvider (Future)

Loads tools from `.codebro-plugin` files.

| Capability | Value |
|------------|-------|
| `provider_name()` | `"plugin:<plugin-name>"` |
| `is_available()` | Depends on plugin being installed and valid |
| `health_check()` | Verifies plugin signature and version |
| Tool sources | Plugin shared libraries |

---

## 3. Provider Interface

```rust
pub trait ToolProvider: Send + Sync {
    /// Unique name for this provider.
    fn provider_name(&self) -> &str;

    /// Semantic version.
    fn provider_version(&self) -> &str { "1.0.0" }

    /// Whether this provider is currently available.
    fn is_available(&self) -> bool;

    /// Discover tool definitions offered by this provider.
    fn discover_tools(&self) -> Vec<ToolDefinition>;

    /// Register tools from this provider into the registry.
    fn register_tools(&self, registry: &mut ToolRegistry) -> Result<()>;

    /// Health check for this provider.
    fn health_check(&self) -> ToolHealth;

    /// Human-readable description.
    fn description(&self) -> &str { "Unknown provider" }
}
```

---

## 4. Provider Registry

The `ProviderRegistry` manages all registered providers:

| Method | Description |
|--------|-------------|
| `add_provider(provider)` | Register a new provider |
| `get_provider(name)` | Lookup provider by name |
| `register_all(registry)` | Register all available providers |
| `health_status()` | Get health of all providers |
| `providers()` | Get all provider instances |

---

## 5. Provider Capabilities Matrix

| Provider | Name | Available | Tools | Health Check | Dynamic |
|----------|------|-----------|-------|--------------|---------|
| BuiltIn | `builtin` | Always | Filesystem, Shell, Git, Patch | N/A | No |
| External | `external:*` | Conditional | External tools | Executable check | Yes |
| MCP | `mcp:*` | Conditional | MCP tools | Server ping | Yes |
| Plugin | `plugin:*` | Conditional | Plugin tools | Signature verify | Yes |

---

## 6. Provider Lifecycle

```
Uninitialized → Available → Unavailable
                         → Healthy → Degraded
```

---

## 7. Extensibility

To add a new provider type:

1. Implement the `ToolProvider` trait
2. Register it with `ProviderRegistry::add_provider()`
3. The registry will automatically discover and register its tools

---

## 8. Future Providers

| Planned Provider | Status | Description |
|-----------------|--------|-------------|
| `McpProvider` | RFC Only | MCP server integration |
| `PluginProvider` | RFC Only | Plugin system |
| `ExternalProvider` | RFC Only | External executables |
| `RemoteProvider` | RFC Only | Remote tool servers |

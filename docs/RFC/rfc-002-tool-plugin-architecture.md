# RFC-002: Tool Plugin Architecture

**Document:** `docs/RFC/rfc-002-tool-plugin-architecture.md`
**Version:** 1.0.0
**Part of:** CodeBro P3 Tool Platform
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related ADRs:** ADR-005, ADR-006, ADR-007

---

## 1. Abstract

This RFC defines the architectural foundation for a plugin-based tool system in CodeBro. It establishes the abstractions, interfaces, and registration patterns that will enable built-in tools, external tools, MCP integration, and third-party plugins to coexist within a single unified tool platform.

**Note:** This RFC prepares the architecture. Implementation of actual plugin loading and MCP integration is deferred to future phases.

---

## 2. Motivation

### 2.1 Current Limitations

The current tool system (`src/tools/`) supports only built-in tools with a flat registration model. This limits:

1. **Extensibility**: Adding new tools requires code changes
2. **Isolation**: Tools share the same process and cannot be sandboxed
3. **Lifecycle**: No mechanism for dynamic tool loading/unloading
4. **Distribution**: No way to package and share tool collections

### 2.2 Goals

- Define a provider abstraction that can represent built-in, external, MCP, and plugin tools
- Establish a discovery mechanism for finding available tools
- Prepare the registry for future plugin loading without breaking changes
- Define clear interfaces that plugin authors will implement

### 2.3 Non-Goals

- Implement actual plugin loading (deferred to P4)
- Implement MCP protocol (deferred to P4)
- Define a package format (deferred to P4)
- Implement a plugin marketplace (deferred to P5)

---

## 3. Architecture

### 3.1 Provider Model

```rust
pub trait ToolProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn provider_version(&self) -> &str { "1.0.0" }
    fn is_available(&self) -> bool;
    fn discover_tools(&self) -> Vec<ToolDefinition>;
    fn register_tools(&self, registry: &mut ToolRegistry) -> Result<()>;
    fn health_check(&self) -> ToolHealth;
}
```

### 3.2 Tool Definition

```rust
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub metadata: ToolMetadata,
    pub factory: Box<dyn Fn() -> Arc<dyn Tool> + Send + Sync>,
}
```

### 3.3 Discovery System

```rust
pub struct ToolDiscovery {
    providers: Vec<Arc<dyn ToolProvider>>,
}

impl ToolDiscovery {
    pub fn discover(&self) -> Vec<DiscoveredTool> { ... }
    pub fn add_provider(&mut self, provider: Arc<dyn ToolProvider>) { ... }
}
```

### 3.4 Provider Types

| Provider | Status | Implementation |
|----------|--------|----------------|
| `BuiltInProvider` | Implemented | Registers all built-in tools |
| `ExternalProvider` | Architecture only | Future: loads tools from external processes |
| `McpProvider` | Architecture only | Future: connects to MCP servers |
| `PluginProvider` | Architecture only | Future: loads .codebro-plugin files |

---

## 4. Registry Integration

The `ToolRegistry` is enhanced to support provider-based registration:

```rust
impl ToolRegistry {
    pub fn register_provider(&mut self, provider: Arc<dyn ToolProvider>) -> Result<()> {
        if provider.is_available() {
            provider.register_tools(self)?;
        }
        Ok(())
    }

    pub fn register_builtin(&mut self) {
        let provider = BuiltInProvider::new();
        self.register_provider(Arc::new(provider)).unwrap();
    }
}
```

---

## 5. Implementation Plan

### Phase 1: Architecture Foundation (P3 - Current)

- [x] Define `ToolProvider` trait
- [x] Define `ToolDefinition` struct
- [x] Implement `BuiltInProvider` for existing tools
- [x] Create `ToolDiscovery` system
- [x] Update `ToolRegistry` with provider support
- [x] Write tests for all new abstractions

### Phase 2: External Tools (P4)

- [ ] Define external tool protocol
- [ ] Implement `ExternalProvider`
- [ ] Add sandboxing for external tools
- [ ] Implement tool signing and verification

### Phase 3: MCP Integration (P4)

- [ ] Implement `McpProvider`
- [ ] Add MCP server discovery
- [ ] Implement MCP tool adaptation layer

### Phase 4: Plugin System (P5)

- [ ] Define plugin format
- [ ] Implement plugin loader
- [ ] Add plugin versioning and conflict resolution
- [ ] Implement plugin sandboxing

---

## 6. Open Questions

1. **Q**: Should providers be hot-reloadable?
   **A**: Yes, but deferred to P4. Current design supports it via the provider trait.

2. **Q**: How should tool name collisions be resolved?
   **A**: Last provider wins. Future: namespace by provider.

3. **Q**: Should providers have dependencies?
   **A**: Not in P3. Future: dependency graph resolution.

---

## 7. References

- [ADR-005: Tool Capability Model](../ADR/adr-005-tool-capability-model.md)
- [ADR-006: Tool Lifecycle Management](../ADR/adr-006-tool-lifecycle-management.md)
- [ADR-007: Tool Hook System](../ADR/adr-007-tool-hook-system.md)

---

## 8. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |

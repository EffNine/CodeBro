# ADR-005: Tool Capability Model

**Document:** `docs/ADR/adr-005-tool-capability-model.md`
**Version:** 1.0.0
**Part of:** CodeBro P3 Tool Platform
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-002

---

## 1. Context

### 1.1 Background

The current `Tool` trait (`src/tools/mod.rs`) is a minimal interface with three methods:
`name()`, `description()`, and `execute()`. This works for simple tools but lacks the
richness needed for a scalable tool platform supporting built-in tools, external tools,
MCP integration, and plugin tools.

Key gaps identified:
- No typed capability model (read, write, execute, network, etc.)
- No metadata beyond name/description
- No permission hook system
- No streaming support for long-running or large-output tools
- No rollback tracking for mutating tools
- No lifecycle management (enabled/disabled/deprecated)
- No diagnostic reporting (health, performance, error rates)

### 1.2 Constraints

- Must remain compatible with the existing `Tool` trait (no breaking changes)
- No new dependencies
- Must support async/await for future streaming
- Must be thread-safe (`Send + Sync`)
- Must support zero-cost abstraction for simple tools

### 1.3 Stakeholders

- **TUI module**: Needs tool metadata for display
- **Agent module**: Needs capability info for routing decisions
- **Dispatcher module**: Needs lifecycle and permission hooks
- **Security**: Needs capability model for permission enforcement
- **Future MCP/Plugin**: Needs provider abstraction

---

## 2. Decision

### 2.1 Decision Statement

Introduce a layered tool architecture with:

1. **Capability Model**: Typed flags describing what a tool can do
2. **Tool Metadata**: Rich, serializable metadata for each tool
3. **Tool Lifecycle**: State machine for tool registration, enablement, deprecation
4. **Hook System**: Pre-execution permission hooks and post-execution rollback hooks
5. **Streaming Support**: Async output streaming via channels
6. **Diagnostics**: Per-tool health, performance, and error tracking
7. **Provider Abstraction**: Future-proof layer for MCP and plugin providers

### 2.2 Rationale

| Concern | Approach | Why |
|---------|----------|-----|
| Capabilities | `ToolCapabilities` bitflags struct | Compile-time safety, zero overhead |
| Metadata | `ToolMetadata` struct with serde | Serializable, queryable, displayable |
| Lifecycle | `ToolLifecycleState` enum | Explicit state transitions, audit trail |
| Hooks | Trait-based hook interfaces | Extensible, no framework coupling |
| Streaming | `AsyncTool` trait + channel-based | Non-blocking, memory-safe, future-ready |
| Diagnostics | `ToolDiagnostics` struct | Observable, queryable, no overhead when unused |
| Provider | `ToolProvider` trait | Abstraction layer for future MCP/plugins |

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)**: Each subsystem is independently testable
- **Principle 8 (Observable AI Actions)**: Diagnostics and lifecycle enable observability
- **Principle 9 (Reliability)**: Rollback hooks and circuit breakers enhance reliability
- **Principle 11 (Security)**: Capability model enables fine-grained permission enforcement

---

## 3. Architecture

### 3.1 Capability Model

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ToolCapabilities {
    pub reads_files: bool,
    pub writes_files: bool,
    pub executes_commands: bool,
    pub accesses_network: bool,
    pub accesses_environment: bool,
    pub modifies_state: bool,
    pub requires_confirmation: bool,
    pub streams_output: bool,
}
```

### 3.2 Tool Metadata

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetadata {
    pub name: String,
    pub description: String,
    pub version: String,
    pub capabilities: ToolCapabilities,
    pub provider: String,
    pub deprecated: bool,
    pub deprecation_note: Option<String>,
    pub tags: Vec<String>,
    pub examples: Vec<String>,
}
```

### 3.3 Tool Lifecycle

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolLifecycleState {
    Unregistered,
    Registered,
    Enabled,
    Disabled,
    Deprecating,
    Removed,
}
```

### 3.4 Hook System

```rust
pub trait PermissionHook: Send + Sync {
    fn check(&self, context: &ToolContext) -> PermissionDecision;
}

pub trait RollbackHook: Send + Sync {
    fn before_execute(&self, context: &mut ToolContext) -> Result<()>;
    fn after_execute(&self, context: &ToolContext, result: &ToolResult) -> Result<()>;
}
```

### 3.5 Streaming Support

```rust
pub trait AsyncTool: Send + Sync {
    fn name(&self) -> &str;
    fn execute_stream(
        &self,
        args: &str,
        context: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamResult>> + Send>>;
}
```

### 3.6 Provider Abstraction

```rust
pub trait ToolProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn register_tools(&self, registry: &mut ToolRegistry) -> Result<()>;
    fn is_available(&self) -> bool;
}
```

---

## 4. Module Structure

```
src/tools/
  mod.rs              - Re-exports, core trait, new types
  capabilities.rs     - ToolCapabilities bitflags
  metadata.rs         - ToolMetadata, ToolDefinition
  lifecycle.rs        - ToolLifecycleState, LifecycleManager
  context.rs          - ToolContext, ExecutionContext
  hooks.rs            - PermissionHook, RollbackHook traits
  streaming.rs        - AsyncTool trait, StreamResult
  diagnostics.rs      - ToolDiagnostics, DiagnosticReport
  discovery.rs        - ToolDiscovery, DiscoveryResult
  provider.rs         - ToolProvider trait, ProviderRegistry
  registry.rs         - Enhanced ToolRegistry
  executor.rs         - Enhanced ToolExecutor with hooks
```

---

## 5. Migration Path

### 5.1 Backward Compatibility

The existing `Tool` trait is preserved. New types are additive:

```rust
// Existing - unchanged
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute(&self, args: &str) -> Result<String>;
}

// New - additive
impl Tool for ExistingTool {
    // No changes needed
}

// New trait for streaming tools
pub trait AsyncTool: Send + Sync {
    // For tools that need streaming
}
```

### 5.2 Registry Enhancement

The `ToolRegistry` gains new methods without breaking existing ones:

```rust
impl ToolRegistry {
    // Existing
    pub fn register(self, tool: Arc<dyn Tool>) -> Self { ... }
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> { ... }

    // New
    pub fn register_with_metadata(self, tool: Arc<dyn Tool>, metadata: ToolMetadata) -> Self { ... }
    pub fn get_metadata(&self, name: &str) -> Option<&ToolMetadata> { ... }
    pub fn get_capabilities(&self, name: &str) -> ToolCapabilities { ... }
    pub fn enable(&mut self, name: &str) -> Result<()> { ... }
    pub fn disable(&mut self, name: &str) -> Result<()> { ... }
    pub fn set_hooks(&mut self, name: &str, permission: Box<dyn PermissionHook>, rollback: Box<dyn RollbackHook>) { ... }
}
```

### 5.3 Implementation Steps

1. Create new modules with types and traits
2. Enhance `ToolRegistry` with metadata, lifecycle, hooks
3. Create `ToolExecutor` that wires registry + hooks + diagnostics
4. Update existing tests to cover new functionality
5. Verify all existing tests pass

---

## 6. Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Complexity | More types and traits | Clear separation of concerns; each module is small |
| Performance | Metadata lookup overhead | Cached lookups; metadata is cheap to copy |
| Memory | Per-tool diagnostic state | Only allocated for tools that need it |
| Learning curve | More concepts for new tools | Clear documentation and examples |
| API surface | More public types | Only essential types are re-exported |

---

## 7. Open Questions

1. **Q**: Should `ToolCapabilities` use `bitflags!` macro or manual impl?
   **A**: Manual impl for zero dependency on `bitflags` crate.

2. **Q**: Should streaming be optional or required?
   **A**: Optional. `Tool` trait remains sync for simple tools. `AsyncTool` for streaming.

3. **Q**: How do hooks interact with the existing `PermissionManager`?
   **A**: Hooks are tool-level; `PermissionManager` is agent-level. Hooks delegate to `PermissionManager` when needed.

---

## 8. References

- [ADR-001: Provider Runtime Architecture](adr-001-provider-runtime-architecture.md)
- [ADR-002: Tool Runtime Architecture](adr-002-tool-runtime-architecture.md)
- [ADR-004: Reliability Layer](adr-004-reliability-layer.md)
- [RFC-001: ReAct Runtime Loop](../../RFC/rfc-001-react-runtime-loop.md)

---

## 9. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |

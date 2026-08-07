# Architecture Report: P3 Tool Platform

**Date:** 2026-08-05
**Phase:** P3 - Tool Platform
**Status:** Complete

---

## 1. Architecture Overview

The P3 Tool Platform introduces a layered architecture for tool management in CodeBro. The architecture separates concerns into nine independent subsystems that compose together through well-defined interfaces.

```
┌─────────────────────────────────────────────────────────────────────┐
│                         ToolRegistry                                │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                │
│  │  Metadata   │  │   Lifecycle │  │    Hooks    │                │
│  └─────────────┘  └─────────────┘  └─────────────┘                │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                │
│  │ Diagnostics │  │  Provider   │  │  Discovery  │                │
│  └─────────────┘  └─────────────┘  └─────────────┘                │
└─────────────────────────────────────────────────────────────────────┘
                              │
        ┌─────────────────────┼─────────────────────┐
        ▼                     ▼                     ▼
┌───────────────┐   ┌───────────────┐   ┌───────────────┐
│  BuiltIn      │   │  MCP          │   │  Plugin       │
│  Provider     │   │  Provider     │   │  Provider     │
│  (implemented)│   │  (future)     │   │  (future)     │
└───────────────┘   └───────────────┘   └───────────────┘
```

---

## 2. Component Diagrams

### 2.1 Tool Registry Architecture

```
┌──────────────────────────────────────────────────────────────────────┐
│                        ToolRegistry                                  │
│                                                                      │
│  ┌────────────────────────────────────────────────────────────────┐ │
│  │                     tools: HashMap<String, Arc<dyn Tool>>      │ │
│  │                     metadata: HashMap<String, ToolMetadata>    │ │
│  │                     lifecycle: LifecycleManager                │ │
│  │                     hooks: HookManager                         │ │
│  │                     diagnostics: DiagnosticCollector           │ │
│  │                     providers: ProviderRegistry                │ │
│  └────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐                 │
│  │  register() │  │  execute()  │  │  enable()   │                 │
│  │  disable()  │  │  get_meta() │  │  deprecate()│                 │
│  │  list()     │  │  has_tool() │  │  set_hooks()│                 │
│  └─────────────┘  └─────────────┘  └─────────────┘                 │
└──────────────────────────────────────────────────────────────────────┘
```

### 2.2 Provider Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                      ToolProvider Trait                             │
│                                                                     │
│  fn provider_name() -> &str                                         │
│  fn is_available() -> bool                                          │
│  fn discover_tools() -> Vec<ToolDefinition>                         │
│  fn register_tools(registry) -> Result<()>                          │
│  fn health_check() -> ToolHealth                                    │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
           │                    │                    │
           ▼                    ▼                    ▼
┌─────────────────┐  ┌─────────────────┐  ┌─────────────────┐
│ BuiltInProvider │  │  McpProvider    │  │ PluginProvider  │
│ (always avail)  │  │  (network dep)  │  │ (file dep)      │
└─────────────────┘  └─────────────────┘  └─────────────────┘
```

### 2.3 Hook Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                         HookManager                                 │
│                                                                     │
│  global_permission: Option<Box<dyn PermissionHook>>                 │
│  global_rollback:   Option<Box<dyn RollbackHook>>                   │
│  per_tool_hooks:    HashMap<String, ToolHooks>                      │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
           │
           ▼
┌─────────────────────────────────────────────────────────────────────┐
│                        ToolHooks (per-tool)                         │
│                                                                     │
│  permission: Option<Box<dyn PermissionHook>>                        │
│  rollback:   Option<Box<dyn RollbackHook>>                          │
│                                                                     │
└─────────────────────────────────────────────────────────────────────┘
```

---

## 3. Data Flow Diagrams

### 3.1 Tool Registration Flow

```
BuiltInProvider
    │
    │ register_tools(registry)
    ▼
ToolRegistry
    │
    ├──► tools.insert(name, Arc<dyn Tool>)
    ├──► metadata.insert(name, ToolMetadata)
    ├──► lifecycle.register(name)  [Unregistered → Registered → Enabled]
    └──► diagnostics.get_or_create(name)
```

### 3.2 Tool Execution Flow

```
Caller
    │ execute(name, args)
    ▼
ToolRegistry
    │
    ├──► Check tool exists (error if not)
    ├──► Check lifecycle state (error if not active)
    ├──► Check permission hooks (error if denied)
    ├──► Run before-execute hooks
    ├──► Spawn blocking task: tool.execute(args)
    ├──► Record diagnostics (success/failure)
    ├──► Update metadata (usage counts)
    ├──► Run after-execute hooks
    └──► Return ToolResult
```

### 3.3 Discovery Flow

```
ToolDiscovery
    │
    ├──► For each provider:
    │       ├──► provider.is_available()?
    │       │       ├──► Yes: add to discovered
    │       │       │       └──► provider.discover_tools()
    │       │       └──► No: add to unavailable
    │       └──► Return DiscoveryResult
    └──► tools: Vec<DiscoveredTool>
```

---

## 4. State Machines

### 4.1 Tool Lifecycle State Machine

```
                    ┌──────────────┐
                    │  Unregistered │
                    └──────┬───────┘
                           │ register()
                           ▼
                    ┌──────────────┐
              ┌────│   Registered  │────┐
              │     └──────┬───────┘    │
              │            │            │
              │     enable()     disable()
              │            │            │
              │            ▼            │
              │     ┌──────────────┐    │
              │     │    Enabled   │────┘
              │     └──────┬───────┘
              │            │ deprecate()
              │            ▼
              │     ┌──────────────┐
              │     │  Deprecating │────┐
              │     └──────┬───────┘    │
              │            │ remove()   │
              │            ▼            │
              │     ┌──────────────┐    │
              └─────│   Removed    │    │
                    └──────────────┘    │
                                         │
                    ┌────────────────────┘
                    │ enable()
                    └────────────────────┘
```

### 4.2 Permission Decision Flow

```
ToolContext
    │
    ▼
HookManager.check_permission()
    │
    ├──► Get per-tool hooks
    │       ├──► Has permission hook? → hook.check(context)
    │       └──► No → CapabilityPermissionHook.check(context)
    │
    ├──► Capability-based default:
    │       ├──► Read-only → AutoAllow
    │       ├──► High-risk → RequireConfirmation
    │       └──► Otherwise → AutoAllow
    │
    └──► Return PermissionDecision
            ├──► Allowed → Execute
            ├──► Ask → Block (return error)
            └──► Denied → Block (return error)
```

---

## 5. Type System Design

### 5.1 Core Types

```rust
// Capability model
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

// Rich metadata
pub struct ToolMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub capabilities: ToolCapabilities,
    pub category: ToolCategory,
    pub provider: String,
    pub deprecated: bool,
    pub deprecation_note: Option<String>,
    pub tags: Vec<String>,
    pub examples: Vec<String>,
    pub usage_count: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub avg_execution_ms: f64,
    pub last_used: Option<String>,
}

// Execution context
pub struct ToolContext {
    pub execution_id: ExecutionId,
    pub session_id: Option<String>,
    pub workspace_root: Option<PathBuf>,
    pub working_directory: Option<PathBuf>,
    pub tool_name: String,
    pub tool_capabilities: ToolCapabilities,
    pub args: String,
    pub requires_confirmation: bool,
    pub correlation_id: String,
}

// Tool result
pub struct ToolResult {
    pub context: ToolContext,
    pub success: bool,
    pub output: String,
    pub duration_ms: f64,
    pub exit_code: Option<i32>,
    pub trace: Option<String>,
}
```

### 5.2 Trait Hierarchy

```rust
// Core tool trait (unchanged)
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute(&self, args: &str) -> anyhow::Result<String>;
}

// Streaming tool trait (new)
pub trait AsyncTool: Send + Sync {
    fn name(&self) -> &str;
    fn execute_stream(
        &self,
        args: &str,
        context: &ToolContext,
    ) -> Pin<Box<dyn Future<Output = Result<StreamResult>> + Send>>;
}

// Provider trait (new)
pub trait ToolProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn is_available(&self) -> bool;
    fn discover_tools(&self) -> Vec<ToolDefinition>;
    fn register_tools(&self, registry: &mut ToolRegistry) -> Result<()>;
    fn health_check(&self) -> ToolHealth;
}

// Hook traits (new)
pub trait PermissionHook: Send + Sync {
    fn check(&self, context: &ToolContext) -> PermissionDecision;
}

pub trait RollbackHook: Send + Sync {
    fn before_execute(&self, context: &mut ToolContext) -> Result<()>;
    fn after_execute(&self, context: &ToolContext, result: &ToolResult) -> Result<()>;
}
```

---

## 6. Module Dependencies

```
src/tools/
  mod.rs ──────────────────────────────────────────────┐
  capabilities.rs ◄────────────────────────────────────┤
  metadata.rs      ◄───── capabilities.rs             │
  lifecycle.rs                ────────────────────────┤
  context.rs     ◄───── capabilities.rs              │
  hooks.rs       ◄───── context.rs, capabilities.rs  │
  streaming.rs   ◄───── context.rs                   │
  diagnostics.rs              ────────────────────────┤
  discovery.rs   ◄───── metadata.rs, provider.rs     │
  provider.rs    ◄───── diagnostics.rs               │
                                                    │
src/dispatcher/                                    │
  mod.rs ────────────────────────────────────────────┘
  registry.rs ◄──── tools/* (all modules)
```

---

## 7. Error Handling Strategy

| Error Type | Source | Handling |
|------------|--------|----------|
| Unknown tool | `dispatch_tool()` | `Err("Unknown tool: {name}")` |
| Lifecycle inactive | `execute_with_context()` | `Err("Tool is not active")` |
| Permission denied | `check_permission()` | `Err("Tool requires confirmation")` |
| Hook failure | `before_execute()` | Propagated as `Err` |
| Tool panic | `spawn_blocking()` | Wrapped as `Err("Tool execution panic")` |
| Tool execution error | `dispatch_tool()` | Wrapped in `ToolResult::failure()` |

---

## 8. Concurrency Model

- **Registry**: Mutable access via `&mut self` for execution
- **Diagnostics**: `std::sync::Mutex` for thread-safe counters
- **Tool execution**: `tokio::task::spawn_blocking` to avoid blocking async runtime
- **Hooks**: Synchronous, called within the blocking task

---

## 9. Extensibility Points

| Extension | How to Add |
|-----------|------------|
| New capability flag | Add field to `ToolCapabilities`, update `is_read_only()`, `is_high_risk()` |
| New lifecycle state | Add variant to `ToolLifecycleState`, add transition to `VALID_TRANSITIONS` |
| New hook type | Implement trait, add to `HookManager` |
| New provider type | Implement `ToolProvider` trait, register with `ProviderRegistry` |
| New diagnostic metric | Add field to `ToolDiagnostics`, update `record_success()` / `record_failure()` |

---

## 10. Migration Guide

### From P2 to P3

```rust
// P2: Simple registry
let registry = ToolRegistry::new()
    .register(Arc::new(ListFiles))
    .register(Arc::new(ReadFile));
let result = registry.execute("read_file", "main.rs").unwrap();

// P3: Enhanced registry (same API, more features)
let mut registry = ToolRegistry::new()
    .register(Arc::new(ListFiles))
    .register(Arc::new(ReadFile));
let result = registry.execute("read_file", "main.rs").await.unwrap();
// Optional: use enhanced features
let meta = registry.get_metadata("read_file").unwrap();
let diags = registry.get_diagnostics("read_file").await.unwrap();
```

---

## 11. Conclusion

The P3 Tool Platform architecture provides a robust, extensible foundation for tool management. The design emphasizes:

1. **Separation of concerns**: Each subsystem has a single responsibility
2. **Backward compatibility**: Existing code continues to work
3. **Future readiness**: Provider abstraction enables MCP and plugins
4. **Observability**: Diagnostics and lifecycle tracking enable debugging
5. **Safety**: Capability model drives permission enforcement

**Recommendation:** GO for P3.5 review.

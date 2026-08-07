# Tool Contract

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Tool Trait Contract

All tools must implement the `Tool` trait:

```rust
pub trait Tool: Send + Sync {
    /// Unique name used for dispatch and lookup.
    fn name(&self) -> &str;

    /// Human-readable description shown in TUI and prompts.
    fn description(&self) -> &str;

    /// Execute the tool with the given arguments.
    /// Returns Ok(output) on success, Err(error) on failure.
    fn execute(&self, args: &str) -> anyhow::Result<String>;
}
```

### Guarantees

1. **Name Uniqueness**: Each registered tool must have a unique name. Duplicate registration overwrites the previous entry.
2. **Thread Safety**: Tools must be `Send + Sync` to allow concurrent execution.
3. **Idempotency**: Tools should not have side effects beyond their stated purpose.
4. **Output Limits**: Tools should cap output to prevent memory exhaustion (see `MAX_TOOL_OUTPUT`).

---

## 2. AsyncTool Trait Contract

For tools that produce streaming output:

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

### Guarantees

1. **Chunk Ordering**: Chunks must be emitted in order.
2. **Final Chunk**: The stream must emit a chunk with `is_final: true` when complete.
3. **Error Handling**: Errors must be propagated through the `Result`, not by panicking.

---

## 3. ToolProvider Trait Contract

All providers must implement:

```rust
pub trait ToolProvider: Send + Sync {
    fn provider_name(&self) -> &str;
    fn provider_version(&self) -> &str { "1.0.0" }
    fn is_available(&self) -> bool;
    fn discover_tools(&self) -> Vec<ToolDefinition>;
    fn register_tools(&self, registry: &mut ToolRegistry) -> Result<()>;
    fn health_check(&self) -> ToolHealth;
    fn description(&self) -> &str { "Unknown provider" }
}
```

### Guarantees

1. **Availability**: `is_available()` must be fast and deterministic.
2. **Registration Safety**: `register_tools()` must not panic.
3. **Health Checks**: `health_check()` must reflect actual provider status.

---

## 4. Hook Contracts

### PermissionHook

```rust
pub trait PermissionHook: Send + Sync {
    fn check(&self, context: &ToolContext) -> PermissionDecision;
}
```

### RollbackHook

```rust
pub trait RollbackHook: Send + Sync {
    fn before_execute(&self, context: &mut ToolContext) -> Result<()>;
    fn after_execute(&self, context: &ToolContext, result: &ToolResult) -> Result<()>;
}
```

---

## 5. Lifecycle Contract

Tools progress through these states:

```
Unregistered → Registered → Enabled
                         → Disabled → Enabled
                         → Deprecating → Removed
```

Invalid transitions return `LifecycleError`.

---

## 6. Metadata Contract

```rust
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
```

---

## 7. Capability Contract

```rust
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

---

## 8. Context Contract

```rust
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
```

---

## 9. Diagnostics Contract

```rust
pub struct ToolDiagnostics {
    pub tool_name: String,
    pub total_executions: u64,
    pub success_count: u64,
    pub failure_count: u64,
    pub total_duration_ms: f64,
    pub avg_duration_ms: f64,
    pub min_duration_ms: f64,
    pub max_duration_ms: f64,
    pub error_rate: f64,
    pub health: ToolHealth,
    pub recent_traces: Vec<ExecutionTrace>,
    pub last_error: Option<String>,
    pub last_execution: Option<String>,
}
```

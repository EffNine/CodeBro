# ADR-002: Tool Runtime Architecture

**Document:** `docs/ADR/adr-002-tool-runtime-architecture.md`
**Version:** 1.0.0
**Part of:** CodeBro P1 Core Runtime
**Status:** Proposed
**Created:** 2026-08-05
**Updated:** 2026-08-05
**Related RFC:** RFC-001

---

## 1. Context

### 1.1 Background

The `Tool` trait is defined in `src/tools/mod.rs` and all tool implementations (`ListFiles`, `ReadFile`, `CreateFile`, `EditFile`, `RunCommand`, `GitStatus`, `GitDiff`) implement it correctly. The `ToolRegistry` in `src/dispatcher/registry.rs` provides name-based lookup.

However, `tui/ui.rs::execute_tool_call()` (lines 873–883) ignores both the trait and the registry, using a hardcoded `match` statement instead. This violates:

- Architecture Manifest Section 5.2, Rule 1: "All tool execution goes through `tools::executor::run_tool_pipeline()`. Direct tool calls from `tui/` or `agent/` are prohibited except for the legacy `execute_tool_call()` in `tui/ui.rs` (deprecated path, must be removed)."
- Design Principle 7 (Modular Architecture): "Tools are registered by name, not hardcoded in match statements."

### 1.2 Constraints

- The `Tool` trait signature is frozen (Section 5.1 of Architecture Manifest).
- The `ToolRegistry` already exists and is used by `SmartToolRouter`.
- No new dependencies.
- All existing tests must continue to pass.

### 1.3 Stakeholders

- **TUI module**: Must use registry instead of hardcoded match
- **Dispatcher module**: May need a convenience `execute()` method
- **Tools module**: No changes needed

---

## 2. Decision

### 2.1 Decision Statement

Replace the hardcoded `execute_tool_call` match statement with registry-based dispatch through `ToolRegistry::get()`. The `ToolRegistry` gains a convenience `execute()` method.

### 2.2 Rationale

1. **Architecture compliance**: The manifest explicitly requires registry-based dispatch.
2. **Extensibility**: New tools register themselves; no match statement changes needed.
3. **Single source of truth**: Tool registration happens in one place (the registry).
4. **Testability**: Tests can inject a registry with mock tools.

### 2.3 Principles Applied

- **Principle 7 (Modular Architecture)**: Modules communicate through well-defined contracts.
- **Principle 10 (Small, Composable Components)**: Tool dispatch is a small, focused operation.
- **Principle 8 (Observable AI Actions)**: Registry-based dispatch makes tool calls traceable.

---

## 3. Consequences

### 3.1 Positive Consequences

- Architecture manifest compliance achieved.
- Adding a new tool only requires registration — no match statement changes.
- Tool dispatch is now testable with mock registries.
- Consistent with how `SmartToolRouter` already uses the registry.

### 3.2 Negative Consequences

- `execute_tool_call` now depends on `ToolRegistry` being passed in.
- The pipeline needs to create or receive a registry instance.

### 3.3 Trade-offs

| Aspect | Trade-off | Mitigation |
|--------|-----------|------------|
| Function signature | `execute_tool_call` gains a registry parameter | Registry is created once and reused |
| Error type | Still returns `anyhow::Result` | Consistent with existing pattern |

### 3.4 Impact on Architecture

| Module | Impact |
|--------|--------|
| `dispatcher/registry.rs` | Add `execute(name, args)` convenience method |
| `tui/ui.rs` | Replace hardcoded match with registry dispatch |
| `tools/` | No changes |

### 3.5 Impact on Future Work

- P2 multi-agent: Subagents can use the same registry pattern.
- Plugin system (future): New tools auto-register into the same registry.

---

## 4. Alternatives Considered

| Alternative | Description | Pros | Cons | Why Rejected |
|-------------|-------------|------|------|--------------|
| Keep hardcoded match | Leave as-is | No changes | Violates architecture manifest | Manifest violation |
| Make tool dispatch async | Use `async fn execute` | Consistent with provider | Trait is sync; would require redesign | Trait freeze |
| Inline registry in executor | Move registry into executor | Simpler module structure | Couples registry to executor | Violates modularity |

---

## 5. Implementation Notes

### 5.1 Code Patterns

```rust
// dispatcher/registry.rs — new convenience method
impl ToolRegistry {
    pub fn execute(&self, name: &str, args: &str) -> Result<String> {
        self.get(name)
            .map(|t| t.execute(args))
            .unwrap_or(Err(CodeBroError::Tool(format!("Unknown tool: {}", name))))
    }
}

// tui/ui.rs — replaced execute_tool_call
fn execute_tool_call(
    registry: &ToolRegistry,
    name: &str,
    args: &str,
) -> Result<String> {
    registry.execute(name, args)
}
```

### 5.2 Anti-Patterns

```rust
// NEVER do this:
match name {
    "read_file" => ReadFile.execute(args),
    "list_files" => ListFiles.execute(args),
    _ => Err(anyhow::anyhow!("Unknown tool: {}", name)),
}

// ALWAYS use the registry:
registry.get(name).ok_or(...)?.execute(args)
```

### 5.3 Migration Steps

1. Add `execute()` method to `ToolRegistry`
2. Replace `execute_tool_call` in `tui/ui.rs`
3. Update call sites to pass the registry
4. Run tests

---

## 6. References

- [Architecture Manifest Section 5](../../architecture/architecture_manifest_v1.md#5-tool-abstraction)
- [Design Principle 7](../../principles/design_principles.md#principle-7-modular-architecture)
- [RFC-001](../../RFC/rfc-001-react-runtime-loop.md)

---

## 7. History

| Date | Change | Author |
|------|--------|--------|
| 2026-08-05 | Created | CodeBro Engineering |

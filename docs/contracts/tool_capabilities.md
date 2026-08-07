# Tool Capabilities Specification

**Version:** 1.0.0
**Status:** Proposed
**Date:** 2026-08-05
**Owner:** CodeBro Engineering

---

## 1. Overview

The Tool Capability Model provides a typed, composable system for describing what each tool can do. Capabilities drive permission decisions, router behavior, and diagnostic classification.

---

## 2. Capability Flags

| Flag | Type | Description | Default |
|------|------|-------------|---------|
| `reads_files` | bool | Can read files from the filesystem | `false` |
| `writes_files` | bool | Can write/create/modify files | `false` |
| `executes_commands` | bool | Can execute shell commands or processes | `false` |
| `accesses_network` | bool | Can make network requests | `false` |
| `accesses_environment` | bool | Can read/modify environment variables | `false` |
| `modifies_state` | bool | Can modify program or system state | `false` |
| `requires_confirmation` | bool | Requires explicit user confirmation | `false` |
| `streams_output` | bool | Supports streaming output | `false` |

---

## 3. Derived Properties

### 3.1 Read-Only Check

A tool is read-only when it has NO write, execute, state, or environment capabilities:

```rust
impl ToolCapabilities {
    pub fn is_read_only(&self) -> bool {
        !self.writes_files
            && !self.executes_commands
            && !self.modifies_state
            && !self.accesses_environment
    }
}
```

### 3.2 High-Risk Check

A tool is high-risk when:
- It requires confirmation, OR
- It both executes commands AND writes files, OR
- It accesses network AND modifies state

### 3.3 Mutating Check

A tool is mutating when it writes files, executes commands, or modifies state.

---

## 4. Permission Policy

Based on capabilities, each tool gets a default permission policy:

| Condition | Policy |
|-----------|--------|
| Read-only, no confirmation required | `AutoAllow` |
| High-risk or confirmation required | `RequireConfirmation` |
| Explicitly blocked | `Blocked` |
| Delegated to external system | `External` |

---

## 5. Tool Categories

Categories are derived from capability combinations:

| Category | Capabilities | Examples |
|----------|--------------|----------|
| `Informational` | reads_files only | `ListFiles`, `ReadFile`, `GitStatus` |
| `Mutating` | reads_files + writes_files | `CreateFile`, `EditFile` |
| `Executable` | executes_commands only | `RunCommand` |
| `Network` | accesses_network only | (future MCP tools) |
| `Stateful` | modifies_state | (future database tools) |
| `Composite` | multiple capabilities | (complex tools) |
| `Unknown` | no matching pattern | (default) |

---

## 6. Capability Operations

| Operation | Description |
|-----------|-------------|
| `is_subset_of(other)` | Check if this is a subset of another |
| `union(other)` | Combine two capability sets |
| `intersection(other)` | Find common capabilities |
| `format()` | Human-readable string (e.g., "read, write, execute") |
| `permission_policy()` | Derive default permission policy |

---

## 7. Built-in Tool Capabilities

### 7.1 ListFiles
```rust
ToolCapabilities {
    reads_files: true,
    ..Default::default()
}
```

### 7.2 ReadFile
```rust
ToolCapabilities {
    reads_files: true,
    ..Default::default()
}
```

### 7.3 CreateFile
```rust
ToolCapabilities {
    reads_files: true,
    writes_files: true,
    requires_confirmation: false,
    ..Default::default()
}
```

### 7.4 EditFile
```rust
ToolCapabilities {
    reads_files: true,
    writes_files: true,
    requires_confirmation: false,
    ..Default::default()
}
```

### 7.5 RunCommand
```rust
ToolCapabilities {
    executes_commands: true,
    streams_output: true,
    requires_confirmation: false,
    ..Default::default()
}
```

### 7.6 GitStatus
```rust
ToolCapabilities {
    executes_commands: true,
    ..Default::default()
}
```

### 7.7 GitDiff
```rust
ToolCapabilities {
    executes_commands: true,
    ..Default::default()
}
```

---

## 8. Extensibility

New capabilities can be added by:
1. Adding a new field to `ToolCapabilities`
2. Updating `is_read_only()`, `is_high_risk()`, `is_mutating()` as needed
3. Updating `from_capabilities()` in `ToolCategory`
4. Updating existing tool registrations

---

## 9. Security Implications

| Capability | Risk Level | Default Policy |
|------------|------------|----------------|
| reads_files | Low | AutoAllow |
| writes_files | Medium | AutoAllow (with audit) |
| executes_commands | High | RequireConfirmation |
| accesses_network | Medium | AutoAllow (with proxy) |
| accesses_environment | High | RequireConfirmation |
| modifies_state | High | RequireConfirmation |
| streams_output | Low | AutoAllow |

# CodeBro Coding Standards

**Document:** `docs/standards/coding_standards.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

These standards define the coding conventions for all CodeBro source code. They ensure consistency, readability, and maintainability across the codebase. All new code must comply with these standards. Existing code should be brought into compliance during natural development cycles.

---

## 2. Module Organization

### 2.1 File Structure

Each module follows this structure:

```rust
// mod.rs
//! Module-level documentation explaining responsibility.
pub mod sub1;
pub mod sub2;

pub use sub1::PublicType;
pub use sub2::OtherPublicType;
```

```rust
// sub1.rs
//! Sub-module documentation.

use crate::error::Result;

/// Public type documentation.
pub struct PublicType {
    // fields
}

impl PublicType {
    /// Method documentation.
    pub fn new() -> Self {
        Self { /* ... */ }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_example() {
        assert!(true);
    }
}
```

### 2.2 Module Responsibilities

| Module | Responsibility | Cannot |
|--------|---------------|--------|
| `cli/` | Parse CLI arguments, resolve config | Access TUI or agent state |
| `config/` | Load, validate, persist configuration | Depend on agent or TUI |
| `session/` | Persist and retrieve session data | Depend on agent logic |
| `metrics/` | Track and report metrics | Depend on agent logic |
| `tui/` | Render UI, handle user input | Execute tools directly |
| `agent/` | Orchestrate agent behavior | Render UI directly |
| `tools/` | Execute tools, manage tool state | Call LLM providers |
| `providers/` | Communicate with LLM providers | Execute tools |
| `intelligence/` | Analyze code, build context | Execute tools or modify files |
| `context/` | Build LLM context from project data | Execute tools |
| `prompt/` | Assemble LLM prompts | Execute tools |

### 2.3 Prohibited Patterns

- **No global mutable state.** Use `Arc<Mutex<T>>` or channels for shared state.
- **No direct `println!` in production code.** Use the `tracing` crate.
- **No `unwrap()` on user-provided data.** Use `?` or explicit error handling.
- **No `tokio::block_on()` in async context.** Use `await` or `spawn_blocking`.
- **No raw `reqwest` calls outside `providers/`.** Use the `Provider` trait.
- **No tool execution outside `tools/`.** Use the `Tool` trait.

---

## 3. Rust Style Conventions

### 3.1 Formatting

- Run `cargo fmt` before every commit
- Line length: 100 characters (soft limit)
- Use trailing commas in multi-line structs and enums
- Group `use` statements: stdlib, then external crates, then internal `crate::`

```rust
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
};

use anyhow::Result;
use tokio::sync::Mutex;

use crate::agent::events::AgentEvent;
use crate::config::Config;
```

### 3.2 Linting

- Run `cargo clippy -- -D warnings` before every commit
- No allowed lints in new code (existing allowed lints must be justified in an ADR)
- Fix clippy suggestions unless they reduce readability

### 3.3 Error Handling

- Use `thiserror` for error types:
  ```rust
  #[derive(Error, Debug)]
  pub enum CodeBroError {
      #[error("IO error: {0}")]
      Io(#[from] std::io::Error),
  }
  ```
- Use `anyhow::Result` at public boundaries:
  ```rust
  pub fn run() -> Result<()> {
      // ...
  }
  ```
- Use `?` for error propagation, never `unwrap()` on fallible operations
- Use `with_context()` when the error type changes:
  ```rust
  fs::read_to_string(&path)
      .with_context(|| format!("Failed to read config: {}", path.display()))?;
  ```

---

## 4. Naming Conventions

### 4.1 Modules

```
snake_case_with_underscores
src/agent/coordinator.rs
src/tools/executor.rs
```

### 4.2 Types (Structs, Enums, Traits)

```
PascalCase
pub struct AgentCoordinator { ... }
pub enum AgentStatus { ... }
pub trait Provider: Send + Sync { ... }
```

### 4.3 Functions and Methods

```
snake_case
pub fn run_tool_pipeline(...) -> Result<...> { ... }
pub fn handle_event(...) -> bool { ... }
```

### 4.4 Constants

```
UPPER_SNAKE_CASE
const DEFAULT_TIMEOUT_SECS: u64 = 300;
const MAX_TOOL_OUTPUT: usize = 32_768;
```

### 4.5 Variables

```
snake_case, descriptive
let tool_name = "read_file";
let mut pipeline_results = Vec::new();
```

### 4.6 Test Functions

```
test_<function_name>_<scenario>_<expected_behavior>
#[test]
fn test_run_command_success() { ... }

#[test]
fn test_compute_layout_small_terminal() { ... }

#[test]
fn test_truncate_long_with_ellipsis() { ... }
```

### 4.7 Type Parameters

```
Single uppercase letter, descriptive when needed
T: Clone + Serialize
F: Fn(AgentEvent) + Send + Sync + 'static
```

---

## 5. Async Guidelines

### 5.1 Runtime

- Use `tokio` for all async operations
- The entry point uses `#[tokio::main]`
- Long-running blocking operations use `tokio::task::spawn_blocking`

### 5.2 Spawning

- Use `tokio::spawn` for fire-and-forget tasks
- Use `tokio::select!` for cooperative cancellation
- Always handle cancellation cleanly (no leaked file handles, connections)

```rust
// Correct: fire-and-forget with cleanup
tokio::spawn(async move {
    match some_async_operation().await {
        Ok(result) => { /* handle */ }
        Err(e) => { /* log and emit error event */ }
    }
});
```

### 5.3 Channels

- Use `std::sync::mpsc` for UI ↔ worker communication (existing pattern)
- Use `tokio::sync::mpsc` for async worker ↔ worker communication
- Never block the async runtime on channel operations

### 5.4 Shared State

- Use `Arc<Mutex<T>>` for shared mutable state
- Prefer message passing over shared memory
- Minimize lock hold time

---

## 6. Logging

### 6.1 Tracing

- Use the `tracing` crate for all logging
- Never use `println!` or `eprintln!` in production code
- Use appropriate log levels:

| Level | Use Case |
|-------|----------|
| `trace` | Detailed diagnostic information |
| `debug` | Developer-oriented diagnostic information |
| `info` | Significant progress events |
| `warn` | Potentially harmful situations |
| `error` | Error conditions |

### 6.2 Log Format

```rust
// Good
tracing::info!(
    tool = %tool_name,
    args = %args,
    "Tool executed"
);

// Good
tracing::warn!(
    provider = %config.provider,
    model = %config.model,
    "Provider response slow"
);

// Good
tracing::error!(
    agent = %agent_name,
    error = %error,
    "Agent failed"
);
```

### 6.3 Do Not Log

- API keys or secrets
- Full tool output (use `tracing::debug!` with truncation)
- User conversation content (use event system instead)

---

## 7. Testing Expectations

### 7.1 Unit Tests

- Every public function must have at least one test
- Tests must be deterministic (no randomness, no timing dependencies)
- Tests must not depend on external services (mock instead)
- Tests must not depend on file system state (use `tempfile`)

### 7.2 Test Organization

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_example() {
        // Arrange
        let input = "test input";

        // Act
        let result = some_function(input);

        // Assert
        assert_eq!(result, expected);
    }
}
```

### 7.3 Integration Tests

- Live in `src/tests.rs` or `tests/` directory
- Test cross-module behavior
- Use temporary directories for file I/O
- Mock the LLM provider for LLM-dependent tests

### 7.4 Test Coverage

| Area | Minimum Coverage |
|------|-----------------|
| New code | 80% line coverage |
| Modified existing code | Coverage must not decrease |
| Tool execution paths | 100% coverage |
| Provider communication | 100% coverage |
| Session persistence | 100% coverage |

---

## 8. Documentation Expectations

### 8.1 Doc Comments

```rust
/// A brief one-line description.
///
/// Longer description explaining what the type does,
/// why it exists, and how it is used.
///
/// # Examples
///
/// ```
/// let value = MyType::new();
/// assert!(value.is_valid());
/// ```
pub struct MyType {
    // ...
}
```

### 8.2 Module Docs

```rust
//! The `agent` module contains the agent orchestration system.
//!
//! # Overview
//!
//! The agent system coordinates user requests through a pipeline:
//! 1. Tool execution for ground-truth context
//! 2. Subagent analysis for planning
//! 3. LLM synthesis for the final response
//!
//! # Architecture
//!
//! See [Architecture Manifest](../../architecture/architecture_manifest_v1.md).
```

### 8.3 In-Line Comments

- Explain **why**, not **what**
- Avoid comments that restate the code
- Use `// TODO(<author>): <description>` for known issues
- Use `// FIXME(<author>): <description>` for known bugs

### 8.4 What Not to Document

- Don't document private implementation details in public APIs
- Don't document obvious behavior ("returns true if the value is true")
- Don't document TODOs in doc comments (use `// TODO:` instead)

---

## 9. Git Commit Standards

### 9.1 Message Format

```
<type>(<scope>): <description>

[Optional body]

Refs: RFC-XXX ADR-XXX
```

### 9.2 Types

| Type | Use Case |
|------|----------|
| `feat` | New feature or phase work |
| `fix` | Bug fix |
| `refactor` | Code restructuring without behavior change |
| `docs` | Documentation only |
| `test` | Test additions or modifications |
| `chore` | Build, config, tooling |
| `perf` | Performance improvement |
| `revert` | Revert a previous commit |

### 9.3 Examples

```
feat(tui): add inline diff display for pending changes
Refs: RFC-003 ADR-007

fix(tools): cap shell output before storing in session
Refs: ADR-003

refactor(agent): split run_chat_pipeline into phases
Refs: ADR-015
```

### 9.4 Commit Size

- Each commit should represent a single logical unit
- Each commit should compile and pass all existing tests
- Do not mix feature implementation with unrelated refactoring

---

## 10. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Architecture Manifest](../architecture/architecture_manifest_v1.md)
- [Development Protocol](../SOP/development_protocol.md)

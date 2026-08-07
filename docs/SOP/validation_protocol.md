# CodeBro Validation Protocol

**Document:** `docs/SOP/validation_protocol.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

This protocol defines the validation requirements for every phase of CodeBro development. Validation proves that a change works correctly, meets its requirements, and does not break existing functionality.

---

## 2. Validation Hierarchy

Validation occurs at four levels, each with increasing scope:

```
Level 1: Unit Validation    — Individual functions and methods
Level 2: Integration Validation — Module interactions
Level 3: System Validation  — End-to-end behavior
Level 4: Manual Validation  — Human-verified exploration
```

All four levels must pass before a phase can receive a GO decision.

---

## 3. Level 1: Unit Validation

### 3.1 Requirements

- Every new public function must have at least one unit test
- Every new public method on a struct must have at least one unit test
- Edge cases must be tested: empty input, error paths, boundary values
- Tests must be deterministic (no randomness, no timing dependencies)

### 3.2 Test Naming Convention

```rust
#[test]
fn test_<function_name>_<scenario>_<expected_behavior>() {
    ...
}
```

**Examples:**
```rust
#[test]
fn test_compute_layout_small_terminal() {
    ...
}

#[test]
fn test_run_command_success() {
    ...
}

#[test]
fn test_truncate_long_with_ellipsis() {
    ...
}
```

### 3.3 Test Categories

| Category | Purpose | Example |
|----------|---------|---------|
| `happy_path` | Verify normal operation | `test_run_command_success` |
| `error_handling` | Verify error paths | `test_read_file_not_found` |
| `boundary` | Verify edge values | `test_truncate_zero_width` |
| `empty_input` | Verify empty/none handling | `test_pipeline_empty_task` |
| `concurrent` | Verify thread safety | `test_event_bus_concurrent_send` |

### 3.4 Coverage Requirements

- New code: minimum 80% line coverage
- Existing modified code: coverage must not decrease
- Critical paths (tool execution, provider communication, session persistence): 100% coverage required

---

## 4. Level 2: Integration Validation

### 4.1 Requirements

- Test that modules interact correctly when composed
- Test the full pipeline from input to output
- Test error propagation across module boundaries
- Use temporary directories for file I/O tests (never test against the real workspace)

### 4.2 Integration Test Locations

- Inline integration tests in `src/tests.rs`
- Module-level `#[cfg(test)]` modules that test cross-module behavior
- Dedicated `tests/` directory for complex multi-module scenarios

### 4.3 Integration Test Scenarios

Every integration test must verify:

1. **Input → Processing → Output** flow
2. **Error propagation** across module boundaries
3. **State persistence** (files, databases, memory)
4. **Cleanup** (no leaked resources, temp files, or state)

---

## 5. Level 3: System Validation

### 5.1 Requirements

- End-to-end validation of user-facing workflows
- Validate the TUI renders correctly under all conditions
- Validate session persistence across restarts
- Validate tool execution produces correct results

### 5.2 System Validation Scenarios

| Scenario | Validation Method |
|----------|-------------------|
| Fresh start with no config | Manual: verify config wizard / auto-detection |
| Normal task execution | Manual: verify tool pipeline + LLM response |
| Multi-turn conversation | Manual: verify session persistence and context |
| Provider failure | Manual: verify error handling and recovery |
| Long-running command | Manual: verify timeout and output capping |
| Large repository | Manual: verify indexing and search performance |

### 5.3 System Validation Automation

Where possible, system validations should be automated:

```rust
// Example: automated system validation
#[tokio::test]
async fn test_full_pipeline_with_mock_provider() {
    let config = MockConfig::new();
    let mut app = TuiApp::new_with_config(config).unwrap();
    
    // Simulate user input
    app.input = "explain this repo".to_string();
    
    // Drive the pipeline
    // ... (mock the provider response)
    
    // Assert final state
    assert!(app.messages.iter().any(|m| m.role == MessageRole::Assistant));
}
```

---

## 6. Level 4: Manual Validation

### 6.1 Requirements

Certain validations can only be performed by human interaction. These are documented in the phase's validation requirements and checked manually.

### 6.2 Manual Validation Checklist

- [ ] TUI renders correctly at minimum terminal size (80x24)
- [ ] TUI renders correctly at maximum terminal size (200x60)
- [ ] Keyboard shortcuts respond correctly
- [ ] Command palette filters and selects correctly
- [ ] Session save/load round-trips correctly
- [ ] Tool output is correctly displayed (truncated, redacted)
- [ ] Streaming response updates the UI progressively
- [ ] Error messages are clear and actionable
- [ ] No visual glitches during panel toggling
- [ ] Mouse scroll works in the conversation area

### 6.3 Manual Validation Recording

Manual validations are recorded in the phase report with:
- The scenario tested
- The observed behavior
- Whether it matched the expected behavior
- Any deviations noted

---

## 7. Validation Gate

### 7.1 Go/No-Go Criteria

A phase passes validation when:

1. **All unit tests pass** — `cargo test` exits with code 0
2. **All integration tests pass** — no failures in cross-module tests
3. **All system validations pass** — automated and manual
4. **No new clippy warnings** — `cargo clippy -- -D warnings` exits with code 0
5. **No rustfmt violations** — `cargo fmt --check` exits with code 0
6. **Test coverage does not decrease** — compared to baseline
7. **Manual validation checklist is complete** — all items checked

### 7.2 Failure Handling

If validation fails:

1. **Unit test failure**: Fix the code or the test. Re-run until clean.
2. **Integration test failure**: Investigate the module interaction. Document the root cause.
3. **System validation failure**: This is a blocking issue. Do not proceed until resolved.
4. **Manual validation failure**: Document the deviation. Assess severity. If P0/P1, block merge.

### 7.3 Known Issues

If a validation issue is known but not blocked:

1. Document the issue in the phase report under "Known Issues"
2. Classify the severity (P0-P3)
3. Create a tracking item for follow-up
4. The phase can receive a HOLD (not REJECT) if the issue is P2/P3 and documented

---

## 8. Validation Artifacts

Every phase produces the following validation artifacts:

| Artifact | Location | Description |
|----------|----------|-------------|
| Test files | `src/`, `tests/` | All unit and integration tests |
| Validation log | `docs/reports/phase-<N>-<name>-validation.md` | Record of all validation checks |
| Coverage report | `target/doc/codebro/` | Generated by `cargo test --doc` |
| Manual validation checklist | In phase report | Completed manual checklist |

---

## 9. Regression Prevention

### 9.1 Regression Test Requirement

Every bug fix MUST include a regression test that reproduces the original failure. The regression test must:

1. Fail before the fix is applied
2. Pass after the fix is applied
3. Be stable across runs (no flakiness)

### 9.2 Regression Suite

The full regression suite is run before every merge:

```bash
cargo test --all-targets
cargo test --doc
cargo clippy -- -D warnings
cargo fmt --check
```

### 9.3 Regression Tracking

Regressions are tracked in the phase report under "Regression Results". Any regression from the baseline must be:

1. Documented with severity
2. Analyzed for root cause
3. Either fixed or documented as an accepted risk with a follow-up item

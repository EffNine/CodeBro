# CodeBro Standard Operating Procedures (SOP) v1.0

**Version:** 1.0.0
**Effective Date:** 2026-01-01
**Maintained By:** CodeBro Engineering
**Classification:** Internal — Development Governance

---

## 1. Purpose

These Standard Operating Procedures establish the engineering discipline required to evolve CodeBro from a working prototype into a production-quality terminal coding agent. The SOP prioritizes architecture stability, validation rigor, maintainability, and human-gated progress over velocity.

The core principle: **every change must be understandable, verifiable, and reversible.**

---

## 2. Scope

This SOP applies to:

- All source code under `src/`
- All configuration under `config/` and `~/.codebro/`
- All documentation under `docs/`
- All test code under `tests/` and `#[cfg(test)]` modules
- All build artifacts and release process

It governs:

- Feature development
- Bug fixes
- Refactoring
- Dependency updates
- Architecture changes
- Release preparation

---

## 3. Development Lifecycle

Every phase of development must follow this sequential pipeline. No stage may be skipped.

```
┌─────────────────────┐
│  1. Repository      │
│     Analysis        │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  2. Architecture    │
│     Review          │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  3. Planning        │
│     (RFC + ADR)     │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  4. Human Approval  │
│     Gate            │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  5. Implementation  │
│     (with tests)    │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  6. Validation      │
│     (unit + integ)  │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  7. Regression      │
│     Testing         │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  8. Benchmark       │
│     (KPI check)     │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│  9. Phase Report    │
│     (docs/reports/) │
└─────────┬───────────┘
          ▼
┌─────────────────────┐
│ 10. Architecture    │
│     Review (again)  │
└─────────┬───────────┘
          ▼
     ┌──────┐
  ┌──┴──┬──┴──┐
  ▼     ▼     ▼
 GO   HOLD  REJECT
  └──────┘
       │
       ▼ (GO only)
  Next Phase
```

### Stage Descriptions

**1. Repository Analysis**
- Inventory all files, modules, and dependencies that will be touched
- Identify existing tests covering the area
- Document current behavior (baseline)
- Identify known issues in the area

**2. Architecture Review**
- Confirm the proposed change aligns with existing architecture
- Identify which modules are affected
- Assess whether new modules are needed
- Check for conflicts with existing or planned work

**3. Planning**
- Write an RFC (Request for Comments) for any non-trivial change
- Write an ADR (Architecture Decision Record) for any architectural decision
- Define explicit entry/exit criteria
- Define measurable validation and benchmark requirements

**4. Human Approval Gate**
- RFC and ADR must be reviewed before implementation begins
- No code changes without approved documentation
- Reviewers must include at least one person who did not write the RFC

**5. Implementation**
- Implement only what the RFC/ADR specifies
- Write tests alongside production code (test-first preferred)
- No partial features committed to main
- Each commit must compile and pass existing tests

**6. Validation**
- All new unit tests must pass
- All affected integration tests must pass
- Manual verification against the validation requirements from planning
- Benchmark against baseline from repository analysis

**7. Regression Testing**
- Full test suite must pass (`cargo test`)
- No pre-existing test may be broken by the change
- If a pre-existing test breaks, it must be investigated and documented before proceeding

**8. Benchmark**
- Measure KPIs defined in the benchmark protocol
- Compare against baseline from repository analysis
- Document any regressions
- KPIs must meet or exceed thresholds defined in the phase spec

**9. Phase Report**
- Write a structured report in `docs/reports/`
- Include all results, decisions, and outstanding issues
- Archive the report for future reference

**10. Architecture Review (Post-Implementation)**
- Re-confirm architecture integrity after all changes are in
- Verify no architectural drift occurred during implementation
- Check that all ADRs are up to date
- Confirm the codebase is in a clean, documented state

**GO / HOLD / REJECT Decision**
- **GO**: All criteria met. Proceed to next phase.
- **HOLD**: Partial progress. Document what remains and schedule a follow-up review.
- **REJECT**: Fundamental issue. Document the blocker and return to planning.

---

## 4. Architecture Rules

These rules are non-negotiable. Violations require explicit architecture review approval before proceeding.

### Rule 1: No Feature Without Architecture Approval

Before any feature implementation begins, the change must be documented in an RFC and the architectural impact assessed in an ADR. The RFC must describe the user-facing behavior; the ADR must describe the technical approach.

**Exception:** Trivial bug fixes (single-line changes that do not affect architecture) may skip RFC/ADR but still require a commit message describing the fix.

### Rule 2: No Breaking Architectural Changes During Implementation

Once an ADR is approved, the implementation must follow it. If the implementation reveals that the ADR is inadequate, the ADR must be updated and re-approved before proceeding. Do not silently diverge from the approved design.

### Rule 3: RFC Required for Major Features

An RFC is required for any change that:
- Introduces a new module or top-level directory
- Changes the signature of a public trait or struct
- Adds a new dependency
- Changes the event flow between major subsystems
- Affects more than 3 existing modules
- Changes user-visible behavior (TUI, CLI commands, configuration)

RFCs are not required for:
- Bug fixes that restore intended behavior
- Documentation updates
- Test additions or improvements
- Dependency version bumps that are backward compatible

### Rule 4: ADR Required for Architectural Decisions

An ADR is required for any decision that:
- Chooses between multiple technical approaches
- Defines a new pattern or convention
- Sets a threshold, limit, or constant that future work will depend on
- Modifies an existing architectural constraint

### Rule 5: No Merge Without Validation

A pull request may not be merged until:
- All unit tests pass (`cargo test`)
- All integration tests pass
- Benchmark KPIs meet the thresholds defined in the phase spec
- The phase report is written and archived

### Rule 6: No Merge Without Regression Tests

Every bug fix must include a regression test that reproduces the original failure. Every new feature must include tests that cover the new behavior. No merge is allowed if it reduces test coverage of an existing module.

### Rule 7: No Phase Without Measurable Benchmarks

Every phase must define quantitative KPIs before implementation begins. Benchmarks are measured at both the baseline (pre-implementation) and post-implementation stages. Phases that cannot define measurable benchmarks must be re-scoped.

---

## 5. Branching Strategy

```
main                  ← Always green. Only accepts merged, validated phases.
├── feature/rfc-XXX   ← Individual feature branches
├── fix/YYY           ← Bug fix branches
├── refactor/ZZZ      ← Refactoring branches
└── phase/P0          ← Phase branches (temporary, merged then deleted)
```

### Branch Naming
- Feature branches: `feature/rfc-<number>-<short-description>`
- Fix branches: `fix/<issue-number>-<short-description>`
- Refactor branches: `refactor/<short-description>`
- Phase branches: `phase/P<phase-number>`

### Merge Requirements
- Branch must be up to date with `main`
- All CI checks must pass
- At least one approval from a reviewer who did not write the branch
- All phase criteria (entry/exit) must be satisfied
- Phase report must be written

---

## 6. Code Standards

### 6.1 Style

- Follow `rustfmt` defaults (run `cargo fmt --check` before every commit)
- Follow `clippy` defaults with no allowed lint warnings in new code
- Use `thiserror` for error types, `anyhow` for failure context
- Prefer descriptive error types over `Box<dyn Error>`

### 6.2 Naming

- Modules: lowercase with underscores (`tool_pipeline`, `agent_coordinator`)
- Types: PascalCase (`ToolPipeline`, `AgentCoordinator`)
- Functions: snake_case (`run_tool_pipeline`, `handle_agent_event`)
- Constants: UPPER_SNAKE_CASE (`DEFAULT_TIMEOUT_SECS`, `MAX_TOOL_OUTPUT`)
- Tests: `test_<function_name>_<scenario>` (`test_run_command_success`, `test_compute_layout_small_terminal`)

### 6.3 Documentation

- Public items must have doc comments (`///`)
- Doc comments must explain what, not how
- Modules must have a module-level doc comment explaining the module's responsibility
- New types must have an example in their doc comment when non-obvious

### 6.4 Testing

- Every new public function must have at least one test
- Edge cases must be tested (empty input, error paths, boundary values)
- Integration tests go in `src/tests.rs` or dedicated `tests/` files
- Tests must be deterministic (no randomness, no network unless mocked)

### 6.5 Git Commits

- Commit messages follow conventional commits: `<type>(<scope>): <description>`
- Types: `feat`, `fix`, `refactor`, `docs`, `test`, `chore`, `perf`, `revert`
- Scope references the module: `agent`, `tui`, `tools`, `providers`, `config`, `session`
- Each commit must compile and pass existing tests

---

## 7. Configuration Standards

### 7.1 Configuration Loading

- Config is loaded once at startup from `~/.codebro/config.toml`
- Environment variables override config file values
- No config changes at runtime (restart required)
- Config changes must be backward compatible

### 7.2 Secrets

- API keys must never be logged, printed, or stored in plain text in session files
- Use the existing secret redaction in `tools/shell.rs` as the model
- Add new redaction patterns to `redact_secrets()` when new secret types are introduced

### 7.3 File Paths

- Config files: `~/.codebro/`
- Session data: `~/.codebro/sessions/`
- Memory: `~/.codebro/memory.json`
- Traces: `~/.codebro/traces/`
- Workspace data: `<project>/.codebro/`
- All paths must be validated before use (no path traversal)

---

## 8. Dependency Standards

### 8.1 Adding Dependencies

- Justify every new dependency in the RFC
- Prefer existing dependencies over new ones
- Avoid dependencies that pull in large transitive trees
- Run `cargo audit` after every dependency addition
- Pin dependency versions in `Cargo.toml`

### 8.2 Updating Dependencies

- Major version bumps require architecture review
- Check for breaking changes in changelogs
- Update one dependency at a time
- Run full test suite after each update

---

## 9. Error Handling Standards

### 9.1 Propagation

- Use `?` operator for error propagation
- Use `anyhow::Result` at public boundaries (TUI, CLI)
- Use `CodeBroError` for internal error typing
- Never unwrap user-provided input

### 9.2 Error Context

- Every `?` should carry context when the error type changes
- Use `with_context()` from `anyhow` for user-facing errors
- Internal errors should preserve the original error chain

### 9.3 User-Facing Errors

- Errors shown in the TUI must be actionable (tell the user what to do)
- Never expose raw stack traces in the UI
- Provider errors must include the model and base URL for debugging

---

## 10. Concurrency Standards

### 10.1 Async Patterns

- Use `tokio` for all async operations
- Prefer `tokio::spawn` for fire-and-forget tasks
- Use `mpsc` channels for UI ↔ worker communication (existing pattern)
- Never block the async runtime with synchronous I/O

### 10.2 Shared State

- Use `Arc<Mutex<T>>` for shared mutable state across tasks
- Prefer message passing over shared memory where possible
- No shared mutable state between the TUI event loop and spawned tasks without synchronization

### 10.3 Cancellation

- All spawned tasks must be cancellable
- Use `tokio::select!` for cooperative cancellation
- Resource cleanup must happen on cancellation (no leaked file handles, connections)

---

## 11. Security Standards

### 11.1 Input Validation

- All user input (CLI args, TUI input, tool arguments) must be validated
- Path inputs must be resolved against the workspace root and checked for traversal
- Shell commands must be passed through `sh -c` with proper escaping

### 11.2 Output Safety

- Tool output is capped at `MAX_TOOL_OUTPUT` (32KB) before entering the UI or context
- API keys and tokens are redacted from all output paths
- No user data is sent to external services except the configured LLM provider

### 11.3 File System Safety

- File write operations must go through the patch/change system with approval
- File read operations must be scoped to the workspace root
- No recursive directory traversal beyond configured depth limits

---

## 12. Review Standards

### 12.1 What Reviewers Check

- Does the implementation match the RFC/ADR?
- Are all tests passing?
- Are there any architectural drifts from the approved design?
- Is the code readable and maintainable?
- Are there any security concerns?
- Are there any performance regressions?

### 12.2 Reviewer Qualifications

- At least one reviewer must have read the RFC and ADR
- At least one reviewer must be familiar with the affected module
- The author of the change cannot be the sole reviewer

---

## 13. Amendment Process

This SOP may be amended by:

1. Filing an RFC against this document (title: `RFC: Amend SOP v<N>`)
2. Describing the proposed change and rationale
3. Receiving architecture review approval
4. Updating the document with the new version number and date

All amendments are tracked in `docs/RFC/` and `docs/ADR/`.

---

## 14. References

- [Development Protocol](./development_protocol.md)
- [Validation Protocol](./validation_protocol.md)
- [Benchmark Protocol](./benchmark_protocol.md)
- [Release Protocol](./release_protocol.md)
- [Regression Protocol](./regression_protocol.md)
- [RFC Template](../RFC/template.md)
- [ADR Template](../ADR/template.md)
- [Phase Report Template](../reports/phase_report_template.md)
- [Roadmap](../roadmap/roadmap.md)

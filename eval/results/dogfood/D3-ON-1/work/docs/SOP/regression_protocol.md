# CodeBro Regression Protocol

**Document:** `docs/SOP/regression_protocol.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

This protocol defines how regressions are detected, tracked, and resolved in CodeBro. A regression is any change that causes previously-working functionality to break or degrade.

---

## 2. Regression Categories

| Category | Description | Example | Severity |
|----------|-------------|---------|----------|
| **Functional** | A feature no longer produces correct output | `read_file` returns empty string for existing files | P0 |
| **Performance** | A KPI degrades beyond acceptable threshold | Startup time increases from 300ms to 800ms | P1 |
| **Visual** | TUI rendering is broken or degraded | Panel overlap, truncated text, missing spinner | P2 |
| **Data** | Persisted data is corrupted or lost | Session files fail to load after format change | P0 |
| **Compatibility** | Existing config or data no longer works | Old `memory.json` fails to parse with new code | P1 |
| **Security** | A previously-secure behavior becomes insecure | API key exposed in tool output | P0 |

---

## 3. Regression Detection

### 3.1 Automated Detection

Every merge triggers automated regression detection:

```bash
# Full test suite
cargo test --all-targets

# Doc tests
cargo test --doc

# Clippy (new warnings = regression in code quality)
cargo clippy -- -D warnings

# Format (violations = regression in style)
cargo fmt --check
```

### 3.2 Baseline Comparison

Benchmarks are compared against the recorded baseline. Any KPI that regresses beyond the threshold defined in `benchmark_protocol.md` is flagged as a regression.

### 3.3 Manual Regression Detection

During manual validation, testers should actively look for:
- Existing features that no longer work
- Visual glitches in the TUI
- Keyboard shortcut conflicts
- Session save/load issues
- Tool execution issues

---

## 4. Regression Tracking

### 4.1 Regression Report Format

Every regression is documented in `docs/reports/regressions/<phase>-<issue-id>.md`:

```markdown
# Regression: <short description>

## Metadata
- **Phase:** P<N>
- **Detected:** YYYY-MM-DD
- **Severity:** P0 / P1 / P2 / P3
- **Category:** Functional / Performance / Visual / Data / Compatibility / Security
- **Status:** Open / Investigating / Fixed / Accepted

## Description
<What broke and how it was detected>

## Reproduction Steps
1. ...
2. ...
3. ...

## Expected Behavior
<What should have happened>

## Actual Behavior
<What actually happened>

## Root Cause
<Why it happened>

## Fix
<How it was fixed, or plan to fix>

## Regression Test
<The test that prevents this from recurring>
```

### 4.2 Regression Registry

All regressions are tracked in a central registry: `docs/reports/regressions/README.md`

```markdown
# Regression Registry

| ID | Phase | Severity | Category | Status | Fixed In |
|----|-------|----------|----------|--------|----------|
| REG-001 | P2 | P1 | Functional | Fixed | v0.6.2 |
| REG-002 | P3 | P2 | Visual | Open | — |
```

---

## 5. Regression Response Procedure

### 5.1 P0 Regressions (Critical)

**Immediate action required.**

1. **Detect**: Automated test failure or manual report
2. **Triage**: Confirm the regression, assess scope
3. **Communicate**: Notify all team members
4. **Fix**: Create a fix branch from `main`
5. **Validate**: Full test suite + manual validation
6. **Merge**: Priority merge (bypasses normal queue)
7. **Release**: Patch release if the broken version was already released
8. **Document**: Full regression report

### 5.2 P1 Regressions (Major)

**Fix within the current development cycle.**

1. **Detect**: Automated or manual
2. **Triage**: Document the impact
3. **Schedule**: Add to the current phase's backlog
4. **Fix**: Implement and validate
5. **Document**: Full regression report

### 5.3 P2 Regressions (Minor)

**Fix before the next release.**

1. **Detect**
2. **Triage**: Document with lower urgency
3. **Schedule**: Add to backlog
4. **Fix**: When resources allow
5. **Document**: Abbreviated regression report

### 5.4 P3 Regressions (Trivial)

**Track and fix when convenient.**

1. **Detect**
2. **Triage**: Log in the regression registry
3. **Schedule**: Backlog, low priority
4. **Fix**: When convenient
5. **Document**: Abbreviated regression report

---

## 6. Regression Prevention

### 6.1 Test-Driven Prevention

- Every bug fix includes a regression test
- Regression tests are added to the permanent test suite
- No regression test is ever removed

### 6.2 Architecture Prevention

- ADRs document why certain patterns exist (preventing accidental breakage)
- Module boundaries are clearly defined
- Public APIs are stable and documented

### 6.3 Process Prevention

- Every phase includes regression testing
- Baseline benchmarks are recorded and compared
- Code review checks for regression risk

---

## 7. Regression Test Requirements

### 7.1 Regression Test Checklist

A regression test must:

- [ ] Reproduce the exact failure condition
- [ ] Fail before the fix and pass after
- [ ] Be stable across runs (no flakiness)
- [ ] Have a clear, descriptive name
- [ ] Be placed in the appropriate test module
- [ ] Not depend on external services (mock instead)
- [ ] Not depend on timing (use assertions, not sleeps)

### 7.2 Regression Test Naming

```rust
#[test]
fn test_<feature>_<regression_scenario>() {
    // Regression: <description from report>
    ...
}
```

**Examples:**
```rust
#[test]
fn test_session_load_corrupted_json_falls_back_to_default() {
    // Regression: REG-001 - SessionLoader panicked on malformed JSON
    ...
}

#[test]
fn test_tool_output_truncation_does_not_panic_on_empty_output() {
    // Regression: REG-005 - Empty tool output caused panic in cap_output()
    ...
}
```

---

## 8. Regression Metrics

Track these metrics in every phase report:

| Metric | Definition | Target |
|--------|-----------|--------|
| `regression_count` | Number of new regressions introduced in the phase | 0 P0/P1, < 3 P2/P3 |
| `regression_fix_time` | Time from detection to fix for P0 regressions | < 24 hours |
| `regression_open_count` | Number of open (unfixed) regressions | Decreasing over time |
| `regression_reopen_count` | Number of regressions that re-occurred after fix | 0 |

---

## 9. Regression Review

At the end of every phase, conduct a regression review:

1. List all regressions detected during the phase
2. Classify by severity
3. Confirm fix status for each
4. Verify regression tests exist for each fixed regression
5. Identify patterns (are certain modules producing more regressions?)
6. Update the regression registry

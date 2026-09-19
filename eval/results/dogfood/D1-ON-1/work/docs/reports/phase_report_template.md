# Phase Report Template

**Document:** `docs/reports/phase_report_template.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

# Phase Report: P<N> — <Phase Name>

| Field | Value |
|-------|-------|
| **Phase** | P<N> — <Phase Name> |
| **Status** | GO / HOLD / REJECT |
| **Author** | <Name> |
| **Start Date** | YYYY-MM-DD |
| **End Date** | YYYY-MM-DD |
| **RFC** | RFC-XXX (if applicable) |
| **ADR** | ADR-XXX (if applicable) |
| **Branch** | `phase/P<N>` |
| **Merge Commit** | `<sha>` (if merged) |

---

## Executive Summary

<A concise summary of what was accomplished, the key results, and the GO/HOLD/REJECT recommendation. 3-5 sentences maximum.>

---

## Completed Work

### Features Implemented

| Feature | Description | Status |
|---------|-------------|--------|
| <feature 1> | <what it does> | Complete / Partial / Skipped |
| <feature 2> | <what it does> | Complete / Partial / Skipped |

### Code Changes

| Module | Files Changed | Lines Added | Lines Removed |
|--------|--------------|-------------|---------------|
| <module> | <n> | <n> | <n> |
| **Total** | **<n>** | **<n>** | **<n>** |

### Tests Added

| Test Module | Tests Added | Coverage Delta |
|-------------|-------------|---------------|
| <module> | <n> | +<n>% |
| **Total** | **<n>** | **+<n>%** |

---

## Architecture Changes

### New Modules

| Module | Purpose | RFC/ADR |
|--------|---------|---------|
| `<path>` | <purpose> | RFC-XXX / ADR-XXX |

### Modified Modules

| Module | Change | Rationale |
|--------|--------|-----------|
| `<module>` | <what changed> | <why> |

### Architectural Drift

<Did the implementation deviate from the approved ADR? If yes, document the deviation and rationale. If no, state "None — implementation matched approved ADR.">

---

## Validation Results

### Unit Tests

```
cargo test --all-targets
Result: <n> passed, <n> failed, <n> ignored
```

### Integration Tests

```
cargo test --test '*'
Result: <n> passed, <n> failed
```

### Clippy

```
cargo clippy -- -D warnings
Result: <n> warnings, <n> errors
```

### Format

```
cargo fmt --check
Result: <clean / violations found>
```

### Manual Validation

| Scenario | Expected | Actual | Pass? |
|----------|----------|--------|-------|
| <scenario 1> | <expected> | <actual> | Yes / No |
| <scenario 2> | <expected> | <actual> | Yes / No |

---

## Benchmark Results

### KPIs Measured

| KPI | Baseline | Target | Actual | Delta | Status |
|-----|----------|--------|--------|-------|--------|
| <kpi 1> | <value> | <target> | <value> | <delta> | PASS / WARNING / FAIL |
| <kpi 2> | <value> | <target> | <value> | <delta> | PASS / WARNING / FAIL |

### Regressions

| KPI | Baseline | Actual | Delta | Action Required |
|-----|----------|--------|-------|----------------|
| <kpi> | <value> | <value> | <delta> | <investigate / accept / fix> |

### Notes

<any notes about benchmark methodology or anomalies>

---

## Regression Results

### New Regressions

| ID | Severity | Category | Description | Status |
|----|----------|----------|-------------|--------|
| REG-XXX | P<n> | <category> | <description> | Open / Fixed |

### Existing Regressions Addressed

| ID | Description | Fix Phase |
|----|-------------|-----------|
| REG-XXX | <description> | P<N> |

---

## Known Issues

<Issues discovered during this phase that are not blocking but need follow-up>

| Issue | Severity | Description | Follow-up |
|-------|----------|-------------|-----------|
| <issue> | P<n> | <description> | RFC-XXX / ADR-XXX / Backlog |

---

## Technical Debt

<New debt introduced by this phase>

| Debt | Location | Description | Recommended Action |
|------|----------|-------------|-------------------|
| <debt> | `<path>` | <description> | <action> |

---

## Risk Assessment

<Current risks associated with the phase output>

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| <risk> | <L/M/H> | <L/M/H> | <mitigation> |

---

## Recommendations

### For Next Phase

<What should the next phase focus on? Any prerequisites?>

### For Architecture Review

<Any concerns or observations for the architecture review?>

---

## GO / HOLD / REJECT Decision

| Option | Decision | Rationale |
|--------|----------|-----------|
| GO | The phase is complete and meets all criteria. | <reasoning> |
| HOLD | The phase is partially complete. | <what remains, what is blocking> |
| REJECT | The phase has fundamental issues. | <what is wrong, what needs to change> |

**Decision:** <GO / HOLD / REJECT>
**Date:** YYYY-MM-DD
**Reviewed by:** <names>

---

## Appendices

### A. Detailed Test Output

<paste or reference full test output>

### B. Benchmark Methodology

<describe how benchmarks were measured>

### C. Manual Validation Log

<log of manual validation sessions>

### D. Related Documents

- RFC-XXX: <title>
- ADR-XXX: <title>
- Previous phase report: P<N-1>

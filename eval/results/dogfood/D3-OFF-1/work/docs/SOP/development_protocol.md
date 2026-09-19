# CodeBro Development Protocol

**Document:** `docs/SOP/development_protocol.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

This protocol defines the step-by-step process for developing any phase, feature, or fix in CodeBro. It operationalizes the lifecycle described in `codebro_sop_v1.md`.

---

## 2. Phase Definition

Every development effort is scoped as a **Phase**. A phase is the smallest unit of work that produces a verifiable, valuable increment.

### Phase Naming Convention

Phases are named with a prefix and number:

```
P0    Repository Audit
P0.5  Architecture Freeze
P1    Core Runtime
P1.5  Runtime Validation
P2    Reliability Layer
P2.5  Regression Validation
P3    Tool Engine
P3.5  Tool Validation
P4    Intelligence Layer
P4.5  Intelligence Benchmark
P5    UX Foundation
P5.5  UX Validation
P6    Advanced Agent System
P6.5  Stress Testing
P7    Release Candidate
P7.5  Release Validation
P8    Stable Release
```

The `.5` phases are validation/regression phases that follow their paired implementation phase. They contain no new implementation — only verification.

---

## 3. Phase Anatomy

Every phase MUST define the following fields before implementation begins.

### 3.1 Objective

A single paragraph describing what the phase achieves and why it matters.

**Format:**
```
## Objective

[What the phase builds and the problem it solves]
```

### 3.2 Scope

A bulleted list of what is IN scope and what is OUT of scope.

**Format:**
```
## Scope

### In Scope
- ...

### Out of Scope
- ...
```

### 3.3 Deliverables

A checklist of tangible outputs.

**Format:**
```
## Deliverables

- [ ] <artifact 1>
- [ ] <artifact 2>
- ...
```

Deliverables include:
- Source code changes
- Tests
- Documentation (RFC, ADR, phase report)
- Benchmark results
- No regression in existing KPIs

### 3.4 Entry Criteria

Conditions that must be satisfied before the phase begins.

**Format:**
```
## Entry Criteria

- [ ] Previous phase exit criteria are satisfied
- [ ] RFC is approved
- [ ] ADR is approved
- [ ] Baseline benchmarks are recorded
- [ ] No outstanding P0/P1 issues in the affected modules
```

### 3.5 Exit Criteria

Conditions that must be satisfied before the phase can be considered complete.

**Format:**
```
## Exit Criteria

- [ ] All deliverables are complete
- [ ] All unit tests pass (`cargo test`)
- [ ] All integration tests pass
- [ ] Benchmark KPIs meet or exceed thresholds
- [ ] Phase report is written and archived
- [ ] Architecture review confirms no drift
- [ ] GO decision recorded
```

### 3.6 Validation Requirements

Specific, testable conditions that prove the phase achieved its objective.

**Format:**
```
## Validation Requirements

1. <condition 1> — verified by <test/method>
2. <condition 2> — verified by <test/method>
...
```

### 3.7 Benchmark Requirements

Quantitative KPIs the phase must meet.

**Format:**
```
## Benchmark Requirements

| KPI | Baseline | Target | Measurement Method |
|-----|----------|--------|-------------------|
| ... | ... | ... | ... |
```

See [Benchmark Protocol](./benchmark_protocol.md) for measurement methodology.

### 3.8 Risks

Known risks and mitigation strategies.

**Format:**
```
## Risks

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| ... | ... | ... | ... |
```

### 3.9 Dependencies

Other phases, modules, or external systems this phase depends on.

**Format:**
```
## Dependencies

- Phase P<N>: <description>
- Module: <module name>
- External: <dependency>
```

### 3.10 GO Conditions

The explicit conditions under which the phase receives a GO decision.

**Format:**
```
## GO Conditions

The phase receives a GO decision when:
1. All exit criteria are satisfied
2. Benchmark KPIs meet targets
3. No new architectural debt is introduced
4. Phase report is reviewed and accepted
```

### 3.11 HOLD Conditions

The explicit conditions under which the phase is placed on hold.

**Format:**
```
## HOLD Conditions

The phase is placed on HOLD when:
1. A critical bug is discovered in existing functionality
2. Benchmark KPIs regress beyond acceptable threshold
3. Architecture review identifies a fundamental design issue
4. A dependency is blocked on an unresolved external issue
```

---

## 4. Phase Implementation Workflow

### 4.1 Pre-Implementation

```
1. Read the approved RFC and ADR
2. Run baseline benchmarks: `cargo bench` (or manual timing)
3. Run full test suite: `cargo test` — confirm all pass
4. Record baseline KPIs in the phase draft report
5. Create a feature branch: `git checkout -b phase/P<N>`
```

### 4.2 Implementation

```
6. Implement changes in small, compile-worthy commits
7. Write tests alongside production code
8. Run tests after each logical unit: `cargo test -- <module>`
9. Run formatter: `cargo fmt --check`
10. Run clippy: `cargo clippy -- -D warnings`
11. Never commit a breaking change without updating dependent code first
```

### 4.3 Post-Implementation

```
12. Run full test suite: `cargo test`
13. Run full benchmark suite
14. Compare post-implementation KPIs against baseline
15. Document any KPI regressions with analysis
16. Write the phase report in `docs/reports/`
17. Request architecture review
18. Submit for merge review
```

---

## 5. Branch Protection

The `main` branch is protected. The following rules apply:

- No force pushes
- No direct commits
- Pull requests require at least one approval
- All checks must pass before merge
- Branch must be up to date with `main` at time of merge
- Phase report must be attached to the merge PR

---

## 6. Commit Discipline

### 6.1 Commit Message Format

```
<type>(<scope>): <description>

[Optional body]

Refs: <RFC-number> <ADR-number>
```

**Types:**
- `feat` — new feature or phase work
- `fix` — bug fix
- `refactor` — code restructuring without behavior change
- `docs` — documentation only
- `test` — test additions or modifications
- `chore` — build, config, tooling
- `perf` — performance improvement
- `revert` — revert a previous commit

**Examples:**
```
feat(tui): add inline diff display for pending changes
Refs: RFC-003 ADR-007

feat(agent): wire intelligence layer into tool pipeline
Refs: RFC-005 ADR-012

fix(tools): cap shell output before storing in session
Refs: ADR-003
```

### 6.2 Commit Size

- Each commit should represent a single logical unit of change
- A commit should compile and pass all existing tests
- Do not mix feature implementation with unrelated refactoring in one commit

---

## 7. Code Review Checklist

Reviewers must verify:

- [ ] Implementation matches the approved RFC/ADR
- [ ] All new code has tests
- [ ] All existing tests still pass
- [ ] No new clippy warnings
- [ ] No new rustfmt violations
- [ ] Error handling is consistent with existing patterns
- [ ] No secrets or sensitive data in output
- [ ] Benchmarks meet or exceed thresholds
- [ ] Phase report is complete and accurate
- [ ] No architectural drift from the approved design

---

## 8. Abort Conditions

Development on a phase must be aborted (and the branch reverted) when:

1. A critical security vulnerability is discovered in the change
2. Benchmark KPIs regress by more than 2x the acceptable threshold
3. The implementation introduces an unresolvable architectural conflict
4. A blocker in a dependent phase is not resolved within the sprint

Abort procedure:
1. Document the abort reason in the phase report
2. Revert the branch
3. Schedule a follow-up architecture review
4. Do not merge

---

## 9. Phase Report Template

All phases produce a report using the template at `docs/reports/phase_report_template.md`. The report is archived in `docs/reports/phase-<N>-<name>.md`.

---

## 10. Cross-Phase Coordination

When a phase depends on another:

1. The dependent phase must document the dependency explicitly
2. The producing phase must stabilize its API before the dependent phase begins
3. If the producing phase is delayed, the dependent phase enters HOLD
4. Interface contracts between phases are documented in ADRs

---

## 11. Emergency Changes

Security fixes and critical bug patches may bypass the normal phase workflow under these conditions:

1. The issue is classified as P0 (security vulnerability or data loss)
2. A post-mortem RFC is filed within 24 hours of the fix
3. The fix is reviewed by at least one other engineer
4. Regression tests are added before merge
5. A phase report is written within 48 hours

Emergency changes are logged separately in `docs/reports/emergency/`.

# Definition of Done

**Document:** `docs/standards/definition_of_done.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

The Definition of Done (DoD) is a checklist that must be satisfied before any phase or feature is considered complete. It exists to prevent partial implementations from being merged and to ensure that every deliverable is testable, documented, and validated.

**A phase is not complete until ALL items below are satisfied.**

---

## 2. Implementation

- [ ] **All planned features are implemented** according to the RFC/ADR
- [ ] **No unplanned features were added** without a corresponding ADR
- [ ] **No dead code was introduced** — every new function is used or tested
- [ ] **The code compiles** with `cargo build --release`
- [ ] **No new clippy warnings** — `cargo clippy -- -D warnings` passes
- [ ] **No rustfmt violations** — `cargo fmt --check` passes

---

## 3. Validation

- [ ] **Unit tests pass** — `cargo test` exits with code 0
- [ ] **Integration tests pass** — all `#[tokio::test]` and `#[test]` in `src/tests.rs` pass
- [ ] **Doc tests pass** — `cargo test --doc` exits with code 0
- [ ] **All new public items have doc comments**
- [ ] **Manual validation scenarios pass** — all scenarios in the RFC/ADR are verified
- [ ] **No new P0/P1 bugs are introduced**

---

## 4. Regression

- [ ] **All existing tests still pass** — no pre-existing test was broken
- [ ] **Regression tests exist for all bug fixes** — each fix has a test that reproduces the original failure
- [ ] **No benchmark KPI regresses beyond threshold** — see [Benchmark Protocol](../SOP/benchmark_protocol.md)
- [ ] **No architectural drift** — the implementation matches the approved ADR

---

## 5. Benchmarks

- [ ] **Baseline KPIs were recorded** before implementation
- [ ] **Post-implementation KPIs were recorded** using the same methodology
- [ ] **KPI comparison is documented** in the phase report
- [ ] **All KPIs meet or exceed targets** (or regressions are documented and accepted)
- [ ] **Benchmark methodology is described** in the phase report

---

## 6. Documentation

- [ ] **Phase report is written** using the template at `docs/reports/phase_report_template.md`
- [ ] **Phase report is archived** in `docs/reports/`
- [ ] **README is updated** if user-visible behavior changed
- [ ] **Architecture Manifest is updated** if module boundaries changed (via ADR)
- [ ] **Changelog is updated** if this is a release phase
- [ ] **Decision log is updated** with any new engineering decisions

---

## 7. Review

- [ ] **Code review completed** — at least one reviewer who did not write the code
- [ ] **Architecture review completed** — post-implementation architecture check
- [ ] **GO/HOLD/REJECT decision is recorded** in the phase report
- [ ] **Branch is merged** to `main` (or phase branch is complete)
- [ ] **Merge commit references the RFC and ADR**

---

## 8. Done Checklist Template

```markdown
## Definition of Done — <Phase/Feature Name>

### Implementation
- [ ] All features implemented
- [ ] No unplanned features added
- [ ] No dead code introduced
- [ ] Compiles: cargo build --release
- [ ] Clippy clean: cargo clippy -- -D warnings
- [ ] Format clean: cargo fmt --check

### Validation
- [ ] Unit tests: cargo test
- [ ] Integration tests: cargo test --test '*'
- [ ] Doc tests: cargo test --doc
- [ ] Public items documented
- [ ] Manual scenarios verified
- [ ] No new P0/P1 bugs

### Regression
- [ ] All existing tests pass
- [ ] Regression tests added for bug fixes
- [ ] No KPI regression beyond threshold
- [ ] No architectural drift

### Benchmarks
- [ ] Baseline recorded
- [ ] Post-implementation recorded
- [ ] Comparison documented
- [ ] All KPIs meet targets
- [ ] Methodology described

### Documentation
- [ ] Phase report written
- [ ] Phase report archived
- [ ] README updated (if needed)
- [ ] Architecture Manifest updated (if needed)
- [ ] Changelog updated (if release phase)
- [ ] Decision log updated

### Review
- [ ] Code review completed
- [ ] Architecture review completed
- [ ] GO/HOLD/REJECT recorded
- [ ] Branch merged
- [ ] Merge references RFC/ADR
```

---

## 9. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Development Protocol](../SOP/development_protocol.md)
- [Validation Protocol](../SOP/validation_protocol.md)
- [Benchmark Protocol](../SOP/benchmark_protocol.md)
- [Regression Protocol](../SOP/regression_protocol.md)

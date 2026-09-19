# CodeBro CI/CD Baseline

**Document:** `docs/ci/ci_baseline.md`
**Version:** 1.0.0
**Part of:** CodeBro Engineering Baseline

---

## 1. Purpose

This document defines the Continuous Integration and Continuous Deployment baseline for CodeBro. Every merge to `main` must pass all CI checks. No implementation should bypass CI.

---

## 2. CI Pipeline Stages

```
┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────┐    ┌──────────────┐
│  FORMAT      │ →  │  CLIPPY      │ →  │  TEST        │ →  │  BENCHMARK   │ →  │  DOCS        │
│  CHECK       │    │  CHECK       │    │  SUITE       │    │  CHECK       │    │  CHECK       │
└──────────────┘    └──────────────┘    └──────────────┘    └──────────────┘    └──────────────┘
```

All stages must pass before a merge is allowed.

---

## 3. Stage Details

### 3.1 Format Check

```bash
cargo fmt --check
```

- **Failure condition:** Any file violates rustfmt rules
- **Action:** Developer runs `cargo fmt` locally, commits the formatted code
- **No warnings allowed:** Format must be 100% clean

### 3.2 Clippy Check

```bash
cargo clippy -- -D warnings
```

- **Failure condition:** Any clippy warning is emitted
- **Action:** Developer fixes the warning or adds a justified `#[allow(...)]` with ADR reference
- **No allowed lints in new code:** Existing allowed lints require ADR justification

### 3.3 Test Suite

```bash
cargo test --all-targets
cargo test --doc
```

- **Failure condition:** Any test fails
- **Action:** Developer fixes the failing test or the code causing the failure
- **Coverage check:** New code must meet minimum coverage thresholds (see [Benchmark Baseline](../benchmark/baseline.md))
- **No flaky tests:** Tests must be deterministic; flaky tests are removed or fixed

### 3.4 Benchmark Check

```bash
# Micro-benchmarks (if criterion is configured)
cargo bench

# Manual benchmark comparison
# Compare post-implementation KPIs against baseline in docs/benchmark/baseline.md
```

- **Failure condition:** Any KPI regresses beyond the acceptable threshold
- **Action:** Developer investigates the regression, fixes if possible, or documents as accepted risk with ADR
- **Baseline comparison:** Post-implementation KPIs are compared against the most recent baseline

### 3.5 Documentation Check

```bash
# Check that all public items have doc comments
cargo doc --no-deps

# Check README links
# Verify that linked documents exist
```

- **Failure condition:** Missing doc comments on public items, broken links
- **Action:** Developer adds documentation or fixes links

---

## 4. CI Environment

### 4.1 CI Platform

- **Primary:** GitHub Actions (`.github/workflows/` to be created)
- **Fallback:** Any CI platform that can run `cargo` commands

### 4.2 Rust Toolchain

- **Version:** Stable (latest at time of phase start)
- **Components:** `rustfmt`, `clippy`
- **Target:** `x86_64-unknown-linux-gnu` (CI), `x86_64-apple-darwin` + `aarch64-apple-darwin` (release)

### 4.3 Cache

- **Cargo registry cache:** Enabled
- **Target directory cache:** Enabled
- **Cache key:** `cargo-{hash of Cargo.lock}`

---

## 5. Branch Protection Rules

### 5.1 Required Checks

The following checks are required on all branches that merge to `main`:

| Check | Description |
|-------|-------------|
| `format-check` | `cargo fmt --check` passes |
| `clippy-check` | `cargo clippy -- -D warnings` passes |
| `test-suite` | `cargo test --all-targets` passes |
| `doc-test` | `cargo test --doc` passes |
| `benchmark-check` | KPIs meet thresholds (for phase branches) |
| `doc-check` | Documentation is complete |

### 5.2 Pull Request Requirements

- At least **1 approval** from a reviewer who did not write the code
- All required checks must pass
- Branch must be up to date with `main`
- Phase report must be attached (for phase branches)
- No merge commits allowed (rebase or squash merge only)

---

## 6. Release CI

### 6.1 Release Trigger

Release CI runs when a tag matching `v*` is pushed.

### 6.2 Release Steps

```bash
# 1. Build release binaries
cargo build --release

# 2. Run full test suite
cargo test --all-targets
cargo test --doc

# 3. Run clippy
cargo clippy -- -D warnings

# 4. Run fmt check
cargo fmt --check

# 5. Generate documentation
cargo doc --no-deps

# 6. Package binaries (if cross-compilation is configured)
#    macOS arm64, macOS x64, Linux x64
```

### 6.3 Release Artifacts

| Artifact | Platform | Method |
|----------|----------|--------|
| Source tag | — | `git tag -a v<M>.<m>.<p>` |
| macOS arm64 binary | `aarch64-apple-darwin` | `cargo build --release --target` |
| macOS x64 binary | `x86_64-apple-darwin` | `cargo build --release --target` |
| Linux x64 binary | `x86_64-unknown-linux-gnu` | `cargo build --release --target` |

---

## 7. Security Checks

### 7.1 Dependency Audit

```bash
cargo audit
```

- **Failure condition:** Any high or critical vulnerability is found
- **Action:** Update the vulnerable dependency or document the risk with ADR

### 7.2 Secret Scan

- **Failure condition:** Any secret (API key, token, password) is found in committed code
- **Action:** Rotate the secret, remove it from the commit, document the incident

### 7.3 Binary Scan (Release Only)

- **Failure condition:** Any known vulnerability in compiled dependencies
- **Action:** Update vulnerable dependencies before release

---

## 8. CI Matrix

| OS | Rust Version | Check |
|----|-------------|-------|
| Ubuntu Latest | Stable | format, clippy, test, doc |
| macOS Latest | Stable | format, clippy, test, doc |
| Ubuntu Latest | Beta | format, clippy, test (best-effort) |
| Ubuntu Latest | Nightly | format, clippy, test (best-effort) |

---

## 9. Offline Development

When CI is not available (offline development), developers must run the full CI suite locally before submitting a PR:

```bash
# Run all CI checks locally
cargo fmt --check && \
cargo clippy -- -D warnings && \
cargo test --all-targets && \
cargo test --doc && \
cargo audit
```

All checks must pass before opening a PR.

---

## 10. References

- [SOP v1.0](../SOP/codebro_sop_v1.md)
- [Benchmark Baseline](../benchmark/baseline.md)
- [Coding Standards](../standards/coding_standards.md)

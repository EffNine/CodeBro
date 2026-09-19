# CodeBro Release Protocol

**Document:** `docs/SOP/release_protocol.md`
**Version:** 1.0.0
**Part of:** CodeBro SOP v1.0

---

## 1. Purpose

This protocol defines the release process for CodeBro. It ensures that every release is validated, documented, and reproducible.

---

## 2. Versioning Scheme

CodeBro uses [Semantic Versioning](https://semver.org/) with the format `MAJOR.MINOR.PATCH`:

- **MAJOR**: Breaking changes to the architecture, configuration format, or stored data format
- **MINOR**: New features that are backward compatible
- **PATCH**: Bug fixes that are backward compatible

### 2.1 Breaking Change Criteria

A MAJOR version bump is required when:
- The `Tool` trait signature changes
- The `AgentEvent` enum gains or loses variants
- The config file format changes incompatibly
- The session file format changes incompatibly
- The memory JSON structure changes incompatibly
- Public module boundaries change

A MINOR version bump is required when:
- A new public API is added
- A new tool is added
- A new TUI panel is added
- A new configuration option is added

A PATCH version bump is required when:
- A bug is fixed
- Performance is improved
- Documentation is updated
- Tests are added

---

## 3. Release Branch Strategy

```
main                  ← Latest stable release
├── release/v0.7.0    ← Release preparation branch
├── release/v0.6.3    ← Patch release branch (if needed)
└── develop           ← Next development cycle
```

### 3.1 Release Branch Creation

When P8 (Stable Release) begins:

1. Create `release/v<M>.<m>.<p>` from `main`
2. Only allow: version bumps, changelog updates, documentation fixes
3. No new features on release branches
4. No breaking changes on release branches

### 3.2 Patch Releases

If a critical bug is found in a released version:

1. Create a branch from the released tag: `git checkout -b release/v<M>.<m>.<p+1> v<M>.<m>.<p>`
2. Apply only the bug fix
3. Run full validation
4. Release as `v<M>.<m>.<p+1>`

---

## 4. Release Checklist

Before any release, the following checklist must be complete:

### 4.1 Pre-Release

- [ ] All P0-P7 phases are complete with GO decisions
- [ ] P7.5 (Release Validation) is complete with GO decision
- [ ] All known issues are documented in the release notes
- [ ] No P0/P1 known issues remain open
- [ ] Benchmark KPIs meet release thresholds
- [ ] Changelog is updated
- [ ] README is updated with current features
- [ ] Config format is documented

### 4.2 Build Verification

```bash
# Clean build
cargo clean
cargo build --release

# Test
cargo test --all-targets

# Doc test
cargo test --doc

# Clippy
cargo clippy -- -D warnings

#Fmt
cargo fmt --check

# Audit
cargo audit

# Benchmarks
cargo bench
```

### 4.3 Distribution Verification

- [ ] Binary builds on macOS (x86_64, aarch64)
- [ ] Binary builds on Linux (x86_64)
- [ ] `cargo install --path .` works from clean checkout
- [ ] `codebro --version` reports correct version
- [ ] `codebro --help` displays correct information
- [ ] First-run config flow works without pre-existing config
- [ ] Session persistence works across restarts

---

## 5. Release Artifacts

Each release produces:

| Artifact | Location | Description |
|----------|----------|-------------|
| Source tag | GitHub tag `v<M>.<m>.<p>` | Point-in-time source snapshot |
| Release notes | `CHANGELOG.md` + GitHub release | Human-readable change summary |
| Binary (macOS arm64) | GitHub release asset | `codebro-aarch64-apple-darwin` |
| Binary (macOS x64) | GitHub release asset | `codebro-x86_64-apple-darwin` |
| Binary (Linux x64) | GitHub release asset | `codebro-x86_64-unknown-linux-gnu` |
| Documentation | GitHub Pages / README | API and usage documentation |

---

## 6. Release Process

### 6.1 Step-by-Step

```
1. Verify all phases are complete (P0-P7)
2. Complete P7.5 Release Validation
3. Bump version in Cargo.toml
4. Update CHANGELOG.md
5. Update README.md if features changed
6. Create release branch: git checkout -b release/v<M>.<m>.<p>
7. Run full validation on release branch
8. Create tag: git tag -a v<M>.<m>.<p> -m "Release v<M>.<m>.<p>"
9. Push tag: git push origin v<M>.<m>.<p>
10. Build binaries (if applicable)
11. Create GitHub release with notes
12. Merge release branch to main
13. Delete release branch
14. Create develop branch for next cycle
15. Bump version in Cargo.toml to next dev version
```

### 6.2 Post-Release

- [ ] Monitor issue tracker for 48 hours post-release
- [ ] Respond to any reported bugs within 24 hours
- [ ] Document any post-release fixes as patch releases
- [ ] Archive the phase report for the released version

---

## 7. Release Tiers

| Tier | Description | Requirements |
|------|-------------|-------------|
| **Alpha** | Internal testing only | No public release. Development snapshots. |
| **Beta** | Community testing | All P0-P6 complete. P6.5 stress testing complete. Known issues documented. |
| **Release Candidate** | Pre-final | All phases complete. P7.5 validation complete. No P0/P1 issues. |
| **Stable** | Production ready | Full release checklist complete. All KPIs meet thresholds. |

---

## 8. Rollback Procedure

If a release is found to be defective:

1. **Minor defect (P2/P3)**: Issue a patch release (`v<M>.<m>.<p+1>`)
2. **Major defect (P0/P1)**: 
   - Announce the rollback publicly
   - Create a hotfix branch from the previous good tag
   - Apply the fix
   - Release the patched version
   - Document the rollback in CHANGELOG.md

---

## 9. Changelog Format

```markdown
## [v0.7.0] - YYYY-MM-DD

### Added
- ...

### Changed
- ...

### Fixed
- ...

### Deprecated
- ...

### Removed
- ...

### Security
- ...

### Known Issues
- ...
```

Use [Keep a Changelog](https://keepachangelog.com/) format.

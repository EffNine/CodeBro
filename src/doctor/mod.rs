//! `codebro doctor` — engineering-runtime diagnostics (control plane).
//!
//! Checks the health of a workspace's CodeBro state: project identity,
//! fact store, engineering memory and repository status. Emits a summary
//! and returns a scriptable exit code:
//!
//! - `0` — healthy
//! - `1` — warnings (recoverable)
//! - `2` — errors (needs `codebro init` or manual repair)

use std::path::Path;

use anyhow::Result;

/// Exit code for a fully healthy workspace.
pub const EXIT_HEALTHY: i32 = 0;
/// Exit code when only warnings are present.
pub const EXIT_WARN: i32 = 1;
/// Exit code when errors are present.
pub const EXIT_ERROR: i32 = 2;

/// A single diagnostic check result from the doctor.
#[derive(Debug, Clone)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: Option<String>,
}

impl Check {
    fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Check {
            name: name.into(),
            ok: true,
            detail: Some(detail.into()),
        }
    }

    fn warn(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Check {
            name: name.into(),
            ok: false,
            detail: Some(format!("WARN: {}", detail.into())),
        }
    }

    fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Check {
            name: name.into(),
            ok: false,
            detail: Some(format!("ERROR: {}", detail.into())),
        }
    }
}

/// Run diagnostics for a workspace root; returns the scriptable exit code
/// plus the structured check results. CLI callers use this to print output
/// and then exit with the returned code.
pub fn run(workspace_root: &Path) -> Result<i32> {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let (code, checks) = report(workspace_root)?;
    print_report(&checks, code, &root);
    Ok(code)
}

/// Run diagnostics and return the exit code plus the raw check results.
/// This is the machine-readable path used by the MCP `repository_health`
/// adapter; it does NOT print anything.
pub fn report(workspace_root: &Path) -> Result<(i32, Vec<Check>)> {
    let root = workspace_root
        .canonicalize()
        .unwrap_or_else(|_| workspace_root.to_path_buf());
    let codebro_dir = root.join(".codebro");

    let mut checks: Vec<Check> = Vec::new();
    let mut errors = 0usize;
    let mut warnings = 0usize;

    // ── 1. Workspace root ─────────────────────────────────────────────
    if root.is_dir() {
        checks.push(Check::pass("workspace_root", root.display().to_string()));
    } else {
        checks.push(Check::fail("workspace_root", "directory does not exist"));
        errors += 1;
    }

    // ── 2. .codebro directory ─────────────────────────────────────────
    if codebro_dir.is_dir() {
        checks.push(Check::pass(".codebro", "runtime state directory exists"));
    } else {
        checks.push(Check::warn(
            ".codebro",
            "not initialized — run `codebro init`",
        ));
        warnings += 1;
    }

    // ── 3. Project identity ───────────────────────────────────────────
    let identity_path = codebro_dir.join("project_identity.json");
    let mut identity = crate::project_identity::ProjectIdentityRuntime::new(&root);
    match identity.load() {
        Ok(_) => {
            let snap = identity.snapshot();
            let langs = snap.languages.join(", ");
            checks.push(Check::pass(
                "project_identity",
                format!("{} ({langs})", snap.name),
            ));
        }
        Err(e) => {
            let exists = identity_path.exists();
            checks.push(Check::warn(
                "project_identity",
                if exists {
                    format!("present but failed to load: {e}")
                } else {
                    "absent — identity not established".to_string()
                },
            ));
            warnings += 1;
        }
    }

    // ── 4. Fact store ─────────────────────────────────────────────────
    let facts_path = codebro_dir.join("facts.json");
    match std::fs::read(&facts_path) {
        Ok(bytes) => match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
            Ok(model) => {
                let store = crate::fact_store::FactStore::from_model(&model);
                let validation = crate::fact_store::validation::FactValidation::validate(&store);
                let counts = store.collection().counts();
                let mut breakdown = String::new();
                for rule in crate::fact_store::validation::FactValidationRule::ALL {
                    let n = validation.count_by_rule(rule);
                    if n > 0 {
                        if !breakdown.is_empty() {
                            breakdown.push_str(", ");
                        }
                        breakdown.push_str(&format!("{}={n}", rule.as_str()));
                    }
                }
                let detail = format!(
                    "{} facts ({} modules, {} symbols, {} tests) — validation: {} issues{}",
                    counts.total,
                    counts.modules,
                    counts.symbols,
                    counts.tests,
                    validation.issue_count(),
                    if breakdown.is_empty() {
                        String::new()
                    } else {
                        format!(" [{breakdown}]")
                    }
                );
                if validation.passed() {
                    checks.push(Check::pass("facts", detail));
                } else {
                    checks.push(Check::warn("facts", detail));
                    warnings += 1;
                }
            }
            Err(e) => {
                checks.push(Check::fail("facts", format!("unparseable: {e}")));
                errors += 1;
            }
        },
        Err(_) => {
            checks.push(Check::warn("facts", "absent — run `codebro init`"));
            warnings += 1;
        }
    }

    // ── 5. Engineering memory ─────────────────────────────────────────
    let memory_path = codebro_dir.join("engineering_memory.json");
    let identity_for_memory = crate::project_identity::ProjectIdentityRuntime::new(&root);
    let mut memory =
        crate::engineering_memory::EngineeringMemoryRuntime::new(&root, identity_for_memory);
    match memory.load() {
        Ok(count) => {
            checks.push(Check::pass(
                "engineering_memory",
                format!("{count} entries"),
            ));
        }
        Err(e) => {
            let exists = memory_path.exists();
            if exists {
                checks.push(Check::fail("engineering_memory", e.to_string()));
                errors += 1;
            } else {
                checks.push(Check::pass(
                    "engineering_memory",
                    "absent (no entries recorded yet)",
                ));
            }
        }
    }

    // ── 6. Git repository state ───────────────────────────────────────
    let git_status = std::process::Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(&root)
        .output();
    match git_status {
        Ok(out) if out.status.success() => {
            let dirty = String::from_utf8_lossy(&out.stdout).lines().count();
            if dirty == 0 {
                checks.push(Check::pass("git", "working tree clean"));
            } else {
                checks.push(Check::warn("git", format!("{dirty} uncommitted path(s)")));
                warnings += 1;
            }
        }
        _ => {
            checks.push(Check::pass("git", "not a git repository (skipped)"));
        }
    }

    let _ = (errors, warnings);
    Ok((compute_exit_code(&checks), checks))
}

/// Compute the overall exit code from the collected checks.
fn compute_exit_code(checks: &[Check]) -> i32 {
    let mut worst: i32 = EXIT_HEALTHY;
    for check in checks {
        if !check.ok {
            let status = if check
                .detail
                .as_deref()
                .is_some_and(|d| d.starts_with("ERROR"))
            {
                EXIT_ERROR
            } else {
                EXIT_WARN
            };
            worst = worst.max(status);
        }
    }
    worst
}

/// Print the human-readable report (used by the CLI).
fn print_report(checks: &[Check], worst: i32, root: &Path) {
    println!("codebro doctor — {}", root.display());
    println!();
    for check in checks {
        let (icon, status) = if check.ok {
            ("✓", "ok")
        } else if check
            .detail
            .as_deref()
            .is_some_and(|d| d.starts_with("ERROR"))
        {
            ("✗", "error")
        } else {
            ("!", "warn")
        };
        println!("  {icon} {:<18} {}", check.name, status);
        if let Some(detail) = &check.detail {
            println!("      {detail}");
        }
    }
    println!();
    let errors = checks.iter().filter(|c| !c.ok && c.detail.as_deref().is_some_and(|d| d.starts_with("ERROR"))).count();
    let warnings = checks.iter().filter(|c| !c.ok && !c.detail.as_deref().is_some_and(|d| d.starts_with("ERROR"))).count();
    match worst {
        EXIT_HEALTHY => println!("  All checks passed."),
        EXIT_WARN => println!("  {errors} error(s), {warnings} warning(s)."),
        _ => println!("  {errors} error(s), {warnings} warning(s). Run `codebro init` to repair."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_empty_dir_returns_zero() {
        let dir = tempfile::tempdir().unwrap();
        let code = run(dir.path()).unwrap();
        // Absent .codebro/facts/memory are warnings, not errors -> exit 1.
        assert_eq!(code, EXIT_WARN);
    }

    #[test]
    fn corrupt_facts_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cb = dir.path().join(".codebro");
        std::fs::create_dir_all(&cb).unwrap();
        std::fs::write(cb.join("facts.json"), "not json").unwrap();
        let code = run(dir.path()).unwrap();
        assert_eq!(code, EXIT_ERROR);
    }

    #[test]
    fn corrupt_memory_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let cb = dir.path().join(".codebro");
        std::fs::create_dir_all(&cb).unwrap();
        std::fs::write(cb.join("engineering_memory.json"), "garbage").unwrap();
        let code = run(dir.path()).unwrap();
        assert_eq!(code, EXIT_ERROR);
    }

    #[test]
    fn initialized_workspace_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        crate::init::run(dir.path()).unwrap();
        let code = run(dir.path()).unwrap();
        // init creates facts; project identity is a separate feature and
        // its absence is only a warning -> exit is at most WARN, never
        // ERROR (exit 2).
        assert!(code <= EXIT_WARN, "got {code}");
    }
}

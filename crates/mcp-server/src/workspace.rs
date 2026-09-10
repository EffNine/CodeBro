//! Workspace-root resolution for the CodeBro MCP runtime.
//!
//! A single CodeBro server process serves exactly one workspace root by
//! default. Every state file (`.codebro/facts.json`,
//! `engineering_memory.json`, `project_identity.json`,
//! `execution_evidence.json`) and every guarded mutation
//! ([`ChangeEngine`](crate::coding::change_engine::ChangeEngine)) is
//! bound to that root.
//!
//! # Precedence
//!
//! 1. Explicit `--root <path>` CLI argument (highest priority).
//! 2. `CODEBRO_WORKSPACE_ROOT` environment variable.
//! 3. The process current working directory (previous default, kept for
//!    backward compatibility).
//!
//! The resolved path is always canonicalized (symlinks resolved) and verified
//! to be an existing directory. Canonicalization matters because the
//! ChangeEngine compares canonical paths when enforcing its workspace
//! boundary: serving `/tmp/link-to-repo` and `/tmp/real-repo` must resolve
//! to the same root so identity, facts, memory, and mutation guards agree.
//!
//! # Additional authorized roots (P8 security boundary closure)
//!
//! The operator may authorize additional workspace roots at launch:
//!
//! - repeatable `--allow-root <path>` CLI flags on `codebro serve`, and/or
//! - the `CODEBRO_ALLOW_ROOTS` environment variable (path-list separated
//!   by `:` on unix, `;` on Windows).
//!
//! These extend the authorized set the
//! [`WorkspaceRegistry`](crate::workspace_registry::WorkspaceRegistry)
//! enforces: a per-call `workspace_root` tool argument must canonicalize
//! to exactly one authorized root. Default single-root deployments change
//! nothing — the server root alone is authorized, exactly as before.
//!
//! # What this module deliberately does NOT do
//!
//! - **No automatic git-root walk-up.** If OpenCode opens `<repo>/subdir`,
//!   auto-climbing to `<repo>` would silently widen the ChangeEngine write
//!   boundary beyond what the operator configured. Explicit configuration
//!   (arg or env) is required to serve a different root.
//! - **No per-call authorization widening.** A `workspace_root` tool
//!   argument selects among operator-authorized roots (discovery); it can
//!   never add one. Multi-root service requires operator authorization
//!   at launch (`--allow-root` / `CODEBRO_ALLOW_ROOTS`), and letting a
//!   per-call argument name arbitrary roots would turn the single-root
//!   ChangeEngine boundary into a per-call variable (P8 audit F3 — now
//!   closed by the registry's authorization gate).
//! - **No weakening of the ChangeEngine boundary.** This module only decides
//!   *which* directories are roots; [`resolve_path`](crate::coding::change_engine)
//!   still denies `..` traversal, outside-root absolutes, and symlink escape
//!   at prepare time and again at apply time.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::path::PathBuf;

/// Environment variable naming an explicit workspace root.
///
/// Honoured by [`resolve_workspace_root`] when no `--root` argument is given.
/// This is the recommended way to scope a globally-registered MCP server
/// (e.g. OpenCode's `~/.config/opencode/opencode.jsonc`) to a specific
/// repository without editing the launch command per project:
///
/// ```json
/// { "mcp": { "codebro": {
///     "type": "local",
///     "command": ["codebro", "serve"],
///     "environment": { "CODEBRO_WORKSPACE_ROOT": "/path/to/repo" }
/// } } }
/// ```
pub const WORKSPACE_ROOT_ENV_VAR: &str = "CODEBRO_WORKSPACE_ROOT";

/// Environment variable authorizing additional workspace roots at launch
/// (P8 security boundary closure). Path-list separated by `:` on unix,
/// `;` on Windows. Each entry must canonicalize to an existing directory;
/// entries that do not are skipped with a stderr warning (the server keeps
/// running with the roots that do resolve).
pub const ALLOW_ROOTS_ENV_VAR: &str = "CODEBRO_ALLOW_ROOTS";

/// Path-list separator for `CODEBRO_ALLOW_ROOTS` entries.
#[cfg(windows)]
const ALLOW_ROOTS_SEP: char = ';';
#[cfg(not(windows))]
const ALLOW_ROOTS_SEP: char = ':';

/// Resolve the workspace root for this server process.
///
/// Precedence: explicit argument > `CODEBRO_WORKSPACE_ROOT` env var >
/// process current directory. The result is canonicalized and verified
/// to be an existing directory; otherwise an error is returned and the caller
/// (CLI `serve`/`init`/`doctor`/…) fails closed instead of silently serving
/// an empty workspace.
pub fn resolve_workspace_root(explicit: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let raw = match explicit {
        Some(p) => p,
        None => match std::env::var(WORKSPACE_ROOT_ENV_VAR) {
            Ok(v) if !v.trim().is_empty() => PathBuf::from(v.trim()),
            _ => std::env::current_dir()?,
        },
    };
    let canonical = raw
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("workspace root '{}' is not usable: {e}", raw.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!(
            "workspace root '{}' is not a directory",
            canonical.display()
        );
    }
    Ok(canonical)
}

/// Resolve the additional operator-authorized workspace roots for this
/// server process (P8 security boundary closure).
///
/// Sources (merged, de-duplicated against the default root):
/// - `allow_root_args`: every `--allow-root <path>` CLI flag, in order.
/// - `CODEBRO_ALLOW_ROOTS` env var: path-list entries.
///
/// Each entry must canonicalize to an existing directory. Invalid entries
/// are skipped with a warning on stderr (never stdout — protocol purity)
/// so one bad path does not take the server down; the operator sees it
/// immediately in the launch log. A path that canonicalizes to the default
/// root is a harmless duplicate.
pub fn resolve_additional_roots(allow_root_args: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = allow_root_args.to_vec();
    if let Ok(list) = std::env::var(ALLOW_ROOTS_ENV_VAR) {
        for entry in list.split(ALLOW_ROOTS_SEP) {
            let trimmed = entry.trim();
            if !trimmed.is_empty() {
                candidates.push(PathBuf::from(trimmed));
            }
        }
    }

    let mut resolved: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        match candidate.canonicalize() {
            Ok(canon) if canon.is_dir() => {
                if !resolved.contains(&canon) {
                    resolved.push(canon);
                }
            }
            Ok(canon) => {
                eprintln!(
                    "skipping --allow-root '{}': canonicalized path is not a directory",
                    canon.display()
                );
            }
            Err(e) => {
                eprintln!("skipping --allow-root '{}': {e}", candidate.display());
            }
        }
    }
    resolved
}

/// Where the resolved root came from (for diagnostics and startup logs).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceRootSource {
    /// `--root <path>` CLI argument.
    ExplicitArg,
    /// `CODEBRO_WORKSPACE_ROOT` environment variable.
    Environment,
    /// Process current working directory (default).
    CurrentDir,
}

impl std::fmt::Display for WorkspaceRootSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WorkspaceRootSource::ExplicitArg => write!(f, "--root argument"),
            WorkspaceRootSource::Environment => write!(f, "{WORKSPACE_ROOT_ENV_VAR}"),
            WorkspaceRootSource::CurrentDir => write!(f, "current directory"),
        }
    }
}

/// Resolve like [`resolve_workspace_root`] and also report which source won.
/// Used for startup logging so a mis-scoped server is visible immediately.
pub fn resolve_with_source(
    explicit: Option<PathBuf>,
) -> anyhow::Result<(PathBuf, WorkspaceRootSource)> {
    if let Some(p) = explicit {
        let canonical = p
            .canonicalize()
            .map_err(|e| anyhow::anyhow!("workspace root '{}' is not usable: {e}", p.display()))?;
        if !canonical.is_dir() {
            anyhow::bail!(
                "workspace root '{}' is not a directory",
                canonical.display()
            );
        }
        return Ok((canonical, WorkspaceRootSource::ExplicitArg));
    }
    if let Ok(v) = std::env::var(WORKSPACE_ROOT_ENV_VAR) {
        if !v.trim().is_empty() {
            let raw = PathBuf::from(v.trim());
            let canonical = raw.canonicalize().map_err(|e| {
                anyhow::anyhow!("workspace root '{}' is not usable: {e}", raw.display())
            })?;
            if !canonical.is_dir() {
                anyhow::bail!(
                    "workspace root '{}' is not a directory",
                    canonical.display()
                );
            }
            return Ok((canonical, WorkspaceRootSource::Environment));
        }
    }
    let cwd = std::env::current_dir()?;
    let canonical = cwd
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("workspace root '{}' is not usable: {e}", cwd.display()))?;
    if !canonical.is_dir() {
        anyhow::bail!(
            "workspace root '{}' is not a directory",
            canonical.display()
        );
    }
    Ok((canonical, WorkspaceRootSource::CurrentDir))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn explicit_root_wins_and_is_canonicalized() {
        let dir = temp_root();
        let out = resolve_workspace_root(Some(dir.path().to_path_buf())).unwrap();
        assert_eq!(out, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn nonexistent_explicit_root_fails_closed() {
        let dir = temp_root();
        let missing = dir.path().join("does-not-exist");
        assert!(resolve_workspace_root(Some(missing)).is_err());
    }

    #[test]
    fn file_as_root_fails_closed() {
        let dir = temp_root();
        let file = dir.path().join("file.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(resolve_workspace_root(Some(file)).is_err());
    }

    #[test]
    fn nested_path_is_used_verbatim_without_walk_up() {
        // A subdirectory is a legitimate (if facts-less) workspace root on
        // its own. We must NOT silently climb to the enclosing repository:
        // that would widen the mutation boundary without operator consent.
        let dir = temp_root();
        let sub = dir.path().join("subdir");
        std::fs::create_dir_all(&sub).unwrap();
        let out = resolve_workspace_root(Some(sub.clone())).unwrap();
        assert_eq!(out, sub.canonicalize().unwrap());
        assert_ne!(out, dir.path().canonicalize().unwrap());
    }

    #[test]
    fn symlinked_root_resolves_to_canonical_target() {
        let dir = temp_root();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = dir.path().join("link");
            symlink(&real, &link).unwrap();
            let via_link = resolve_workspace_root(Some(link)).unwrap();
            let via_real = resolve_workspace_root(Some(real.clone())).unwrap();
            assert_eq!(via_link, via_real);
            assert_eq!(via_link, real.canonicalize().unwrap());
        }
    }

    #[test]
    fn source_reports_explicit_arg() {
        let dir = temp_root();
        let (root, source) = resolve_with_source(Some(dir.path().to_path_buf())).unwrap();
        assert_eq!(source, WorkspaceRootSource::ExplicitArg);
        assert_eq!(root, dir.path().canonicalize().unwrap());
    }

    // ── P8 root authorization: additional-roots resolution ─────────────

    #[test]
    fn additional_roots_resolve_flags_and_env() {
        let home = tempfile::tempdir().unwrap();
        let a = home.path().join("a");
        let b = home.path().join("b");
        let c = home.path().join("c");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::create_dir_all(&c).unwrap();
        let a_canon = a.canonicalize().unwrap();
        let b_canon = b.canonicalize().unwrap();
        let c_canon = c.canonicalize().unwrap();

        // Flag-only.
        let via_flags = resolve_additional_roots(&[a]);
        assert_eq!(via_flags, vec![a_canon.clone()]);

        // Env-only (path list).
        std::env::set_var(
            ALLOW_ROOTS_ENV_VAR,
            format!("{}:{}", b.display(), c.display()),
        );
        let via_env = resolve_additional_roots(&[]);
        assert_eq!(via_env, vec![b_canon.clone(), c_canon.clone()]);

        // Flags + env merge, de-duplicated, canonicalized (symlinks
        // collapse to their targets).
        std::env::set_var(ALLOW_ROOTS_ENV_VAR, b.display().to_string());
        let merged = resolve_additional_roots(&[home.path().join("a"), home.path().join("b")]);
        assert_eq!(merged, vec![a_canon.clone(), b_canon.clone()]);

        std::env::remove_var(ALLOW_ROOTS_ENV_VAR);
    }

    #[test]
    fn additional_roots_skip_invalid_entries_without_failing() {
        let home = tempfile::tempdir().unwrap();
        let good = home.path().join("good");
        std::fs::create_dir_all(&good).unwrap();
        let file_entry = home.path().join("file.txt");
        std::fs::write(&file_entry, "x").unwrap();

        std::env::set_var(
            ALLOW_ROOTS_ENV_VAR,
            format!(
                "{}:{}:{}",
                good.display(),
                file_entry.display(),
                home.path().join("missing").display()
            ),
        );
        let resolved = resolve_additional_roots(&[]);
        assert_eq!(
            resolved,
            vec![good.canonicalize().unwrap()],
            "invalid entries are skipped; valid ones survive"
        );
        std::env::remove_var(ALLOW_ROOTS_ENV_VAR);
    }
}

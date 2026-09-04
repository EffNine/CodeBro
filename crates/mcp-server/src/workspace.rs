//! Workspace-root resolution for the CodeBro MCP runtime.
//!
//! A single CodeBro server process serves exactly one workspace root. Every
//! state file (`.codebro/facts.json`, `engineering_memory.json`,
//! `project_identity.json`, `execution_evidence.json`) and every guarded
//! mutation ([`ChangeEngine`](crate::coding::change_engine::ChangeEngine))
//! is bound to that root.
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
//! # What this module deliberately does NOT do
//!
//! - **No automatic git-root walk-up.** If OpenCode opens `<repo>/subdir`,
//!   auto-climbing to `<repo>` would silently widen the ChangeEngine write
//!   boundary beyond what the operator configured. Explicit configuration
//!   (arg or env) is required to serve a different root.
//! - **No per-tool workspace override.** Letting individual tool calls name
//!   an arbitrary root would turn the single-root ChangeEngine boundary into
//!   a per-call variable and allow writes anywhere the server process can
//!   reach. Multi-root service requires a separate server process per root.
//! - **No weakening of the ChangeEngine boundary.** This module only decides
//!   *which* directory is the root; [`resolve_path`](crate::coding::change_engine)
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

/// Resolve the workspace root for this server process.
///
/// Precedence: explicit argument > `CODEBRO_WORKSPACE_ROOT` env var >
/// process current directory. The result is canonicalized and verified to
/// be an existing directory; otherwise an error is returned and the caller
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
}

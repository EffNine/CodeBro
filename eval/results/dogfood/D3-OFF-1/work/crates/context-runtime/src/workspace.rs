//! Workspace identity canonicalization for the context store.
//!
//! The user-context database is user-level (`~/.codebro/state.db`) with
//! per-workspace rows keyed by `workspace_root`. Without canonicalization,
//! `/repo`, `/repo/`, `/repo/./sub/..`, and a symlinked alias of the same
//! directory become separate namespaces and project fingerprint/intent
//! silently split-brains.
//!
//! The MCP layer already resolves every `workspace_root` argument through
//! the filesystem (`WorkspaceRegistry::resolve` → `canonicalize`), so the
//! store normalization below is a defensive second layer for direct store
//! callers. It is deliberately lightweight: pure lexical normalization plus
//! a best-effort symlink resolution that only touches the filesystem when
//! the path exists. Callers holding an already-canonical root pay one
//! `symlink_metadata` probe at most.

use std::path::{Path, PathBuf};

/// Canonicalize a workspace root string into its stable storage key.
///
/// Steps (deterministic, no network, no writes):
/// 1. Trim surrounding whitespace.
/// 2. Lexically normalize separators and `.` / `..` segments (no
///    filesystem access; unresolvable `..` above a relative root is kept).
/// 3. When the result exists on disk, resolve symlinks via
///    [`std::fs::canonicalize`]; otherwise keep the lexical result.
///
/// Lexically identical workspaces always map to the same key even when the
/// path does not exist (yet); resolvable symlinks collapse once the target
/// exists. Case is preserved (Linux-first; no case folding).
pub fn canonical_workspace_key(raw: &str) -> String {
    let trimmed = raw.trim();
    let lexical = lexical_normalize(trimmed);
    if lexical.is_empty() {
        return String::new();
    }
    let path = Path::new(&lexical);
    // One cheap existence probe; only then pay for full canonicalization.
    if path.exists() {
        if let Ok(canonical) = path.canonicalize() {
            return canonical.display().to_string();
        }
    }
    lexical
}

/// Pure lexical path normalization: collapse duplicate separators, drop
/// `.`, resolve `..` against the preceding segment, drop trailing slashes
/// (except the filesystem root itself).
fn lexical_normalize(raw: &str) -> String {
    if raw.is_empty() {
        return String::new();
    }
    // Normalize Windows-style separators so mixed inputs still collapse.
    let slashed = raw.replace('\\', "/");
    let absolute = slashed.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in slashed.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() && !absolute {
                    // Relative `..` above the root cannot resolve lexically.
                    parts.push("..");
                }
            }
            other => parts.push(other),
        }
    }
    let mut out = parts.join("/");
    if absolute {
        out.insert(0, '/');
    }
    if out.is_empty() {
        out.push('.');
    }
    out
}

/// Canonicalize an optional workspace root in place (store write/query
/// helper). `None` stays `None` (global scope).
pub fn canonicalize_opt(root: Option<String>) -> Option<String> {
    root.map(|r| canonical_workspace_key(&r))
}

/// Join a canonical key back into a path for filesystem use.
pub fn key_to_path(key: &str) -> PathBuf {
    PathBuf::from(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_slashes_dots_and_dotdots_collapse() {
        // None of these fixtures need to exist: lexical equivalence holds
        // without filesystem access.
        let base = "/nonexistent-codebro-canonical-probe/repo";
        let variants = [
            base.to_string(),
            format!("{base}/"),
            format!("{base}//"),
            format!("{base}/./"),
            format!("{base}/sub/.."),
            format!("{base}//sub/../"),
        ];
        let mut keys: Vec<String> = variants
            .iter()
            .map(|v| canonical_workspace_key(v))
            .collect();
        keys.dedup();
        assert_eq!(keys, vec![base.to_string()], "variants: {variants:?}");
    }

    #[test]
    fn whitespace_is_trimmed() {
        assert_eq!(
            canonical_workspace_key("  /tmp/probe  "),
            canonical_workspace_key("/tmp/probe")
        );
    }

    #[test]
    fn resolvable_symlinks_collapse_to_the_target() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        std::fs::create_dir_all(&real).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = dir.path().join("link");
            symlink(&real, &link).unwrap();
            assert_eq!(
                canonical_workspace_key(link.to_str().unwrap()),
                canonical_workspace_key(real.to_str().unwrap()),
                "symlink and target must share one namespace"
            );
        }
    }

    #[test]
    fn missing_paths_still_normalize_lexically() {
        // A workspace that does not exist yet (init has not run) must still
        // get a stable key — never an error, never an empty string.
        let key = canonical_workspace_key("/definitely/not/here/../here/");
        assert_eq!(key, "/definitely/not/here");
    }

    #[test]
    fn empty_stays_empty_for_validation_to_reject() {
        assert_eq!(canonical_workspace_key(""), "");
        assert_eq!(canonical_workspace_key("   "), "");
        assert_eq!(canonicalize_opt(None), None);
    }
}

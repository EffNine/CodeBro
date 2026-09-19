//! Repository state capture — shared runtime primitive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::hash::Hasher;

/// Size gate for untracked-file content hashing in [`RepoState::capture`].
/// Mirrors the indexer's `MAX_SOURCE_FILE_BYTES`: files above this
/// contribute path+size only, so dataset-heavy worktrees stay cheap while
/// remaining deterministic (the skip decision depends on size alone).
pub const MAX_UNTRACKED_FILE_BYTES: u64 = 512 * 1024;

/// Repository state at the time of capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoState {
    /// Current HEAD commit SHA (full or short). Empty if not a git repo.
    pub commit_sha: String,
    /// Whether the working tree has uncommitted changes.
    pub working_tree_dirty: bool,
    /// Deterministic hash of relevant working-tree state.
    /// For a clean repo this is the commit SHA; for dirty it includes diff.
    pub working_tree_hash: String,
}

impl RepoState {
    /// Capture repository state from the workspace root.
    /// Returns None if not a git repository or git is unavailable.
    ///
    /// The `.codebro/` directory is CodeBro-derived output (facts, caches,
    /// journals) and is excluded from every signal via the `':!.codebro'`
    /// pathspec: indexing must never invalidate its own revision. Untracked
    /// file *contents* (bounded by [`MAX_UNTRACKED_FILE_BYTES`]) feed the
    /// hash so editing an untracked source file invalidates freshness;
    /// oversized files contribute path+size only (deterministic skip).
    pub fn capture(workspace_root: &PathBuf) -> Option<Self> {
        let output = std::process::Command::new("git")
            .current_dir(workspace_root)
            .args(["rev-parse", "--verify", "HEAD"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let commit_sha = String::from_utf8_lossy(&output.stdout).trim().to_string();

        // Check dirty: uncommitted changes or untracked files, excluding
        // CodeBro-derived output (see above).
        let dirty_output = std::process::Command::new("git")
            .current_dir(workspace_root)
            .args(["status", "--porcelain", "--", ":!.codebro"])
            .output()
            .ok()?;
        let working_tree_dirty = !dirty_output.status.success()
            || !String::from_utf8_lossy(&dirty_output.stdout)
                .trim()
                .is_empty();

        // Compute a deterministic hash of working-tree state.
        // Strategy: hash(sorted tracked files + diff + untracked names and
        // contents, all excluding `.codebro/`).
        let working_tree_hash = {
            let mut parts: Vec<Vec<u8>> = Vec::new();

            // Tracked files list (sorted)
            let ls_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["ls-files", "--", ":!.codebro"])
                .output()
                .ok();
            if let Some(ref ls) = ls_output {
                if ls.status.success() {
                    parts.push(ls.stdout.clone());
                }
            }

            // Uncommitted changes
            let diff_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["diff", "HEAD", "--", ":!.codebro"])
                .output()
                .ok();
            if let Some(ref diff) = diff_output {
                if diff.status.success() {
                    parts.push(diff.stdout.clone());
                }
            }

            // Untracked files: names AND bounded contents. Names alone miss
            // content edits to never-added files (false-Fresh); contents
            // alone would miss deletions, so both feed the hash.
            let untracked_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args([
                    "ls-files",
                    "--others",
                    "--exclude-standard",
                    "--",
                    ":!.codebro",
                ])
                .output()
                .ok();
            if let Some(ref ut) = untracked_output {
                if ut.status.success() {
                    let text = String::from_utf8_lossy(&ut.stdout);
                    let mut names: Vec<&str> = text.lines().collect();
                    names.sort();
                    let mut untracked_blob: Vec<u8> = Vec::new();
                    for name in names {
                        untracked_blob.extend_from_slice(name.as_bytes());
                        untracked_blob.push(0);
                        // Bounded content hash: oversized files contribute
                        // path+size only (same 512 KiB gate as the indexer).
                        let abs = workspace_root.join(name);
                        match std::fs::metadata(&abs) {
                            Ok(meta) if meta.len() <= MAX_UNTRACKED_FILE_BYTES => {
                                match std::fs::read(&abs) {
                                    Ok(bytes) => {
                                        use sha2::Digest as _;
                                        let mut h = sha2::Sha256::new();
                                        h.update(&bytes);
                                        untracked_blob.extend_from_slice(
                                            format!("{:x}", h.finalize()).as_bytes(),
                                        );
                                    }
                                    Err(_) => {
                                        untracked_blob.extend_from_slice(b"unreadable");
                                    }
                                }
                            }
                            Ok(meta) => {
                                untracked_blob.extend_from_slice(
                                    format!("oversized:{}", meta.len()).as_bytes(),
                                );
                            }
                            Err(_) => {
                                untracked_blob.extend_from_slice(b"missing");
                            }
                        }
                        untracked_blob.push(0);
                    }
                    parts.push(untracked_blob);
                }
            }

            // Sort each part for determinism
            for part in &mut parts {
                part.sort();
            }

            // Concatenate and hash
            let mut hasher = sha2::Sha256::new();
            for part in &parts {
                sha2::Digest::update(&mut hasher, part);
            }
            if parts.is_empty() {
                // Fallback: just hash the commit SHA
                sha2::Digest::update(&mut hasher, commit_sha.as_bytes());
            }
            format!("{:x}", hasher.finalize())
        };

        Some(RepoState {
            commit_sha,
            working_tree_dirty,
            working_tree_hash,
        })
    }
}

/// Repository identity: what project is being tested.
///
/// P6 strengthens this from a bare path hash into a deterministic
/// engineering identity.
///
/// Identity combines canonical root with VCS identity where available,
/// plus project structure. Filesystem paths alone are never the identity.
/// The canonical root is resolved (symlinks, `.`/`..` lexically
/// normalised) and combined with the strongest available VCS signal.
///
/// Workspace isolation: identities are always derived from a canonical
/// workspace root; two different canonical roots never share an identity,
/// and identity comparison never crosses workspace boundaries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIdentity {
    /// Deterministic project identifier derived from canonical root +
    /// VCS identity (stable across restarts, unique per repository).
    pub project_id: String,
    /// Canonical absolute workspace root path (symlinks resolved).
    pub root: String,
    /// Detected repository type: "cargo", "go", "npm", "python", "unknown".
    pub repository_type: String,
    /// Canonical workspace root (symlink-resolved, lexically clean).
    /// Equals `root` when resolution succeeds; kept separate so callers
    /// can distinguish raw vs canonical inputs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_root: Option<String>,
    /// Git remote URL (`origin` preferred) where available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
    /// HEAD commit SHA where available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}

impl RepoIdentity {
    /// Derive from a workspace root. Uses project_identity runtime where available.
    ///
    /// P6: canonicalises the root (symlink-resolved where possible),
    /// captures VCS identity (git remote + HEAD) where available, and
    /// derives a stable `project_id` from canonical-root + remote (not
    /// the raw input path). Backwards compatible: `root` remains the
    /// canonical path string, `repository_type` keeps its vocabulary
    /// (plus `python`), and new fields are `None` when unavailable.
    pub fn from_workspace(workspace_root: &Path) -> Self {
        let canonical = canonical_root_of(workspace_root);
        let repository_type = if workspace_root.join("Cargo.toml").exists()
            || Path::new(&canonical).join("Cargo.toml").exists()
        {
            "cargo".to_string()
        } else if workspace_root.join("go.mod").exists()
            || Path::new(&canonical).join("go.mod").exists()
        {
            "go".to_string()
        } else if workspace_root.join("package.json").exists()
            || Path::new(&canonical).join("package.json").exists()
        {
            "npm".to_string()
        } else if workspace_root.join("pyproject.toml").exists()
            || workspace_root.join("setup.py").exists()
            || workspace_root.join("requirements.txt").exists()
            || Path::new(&canonical).join("pyproject.toml").exists()
        {
            "python".to_string()
        } else {
            "unknown".to_string()
        };
        let git_remote = git_remote_of(Path::new(&canonical));
        let commit_sha = git_head_of(Path::new(&canonical));
        // project_id: stable hash of canonical-root + remote. The remote
        // binds clones of the same repository; the canonical root keeps
        // non-VCS workspaces distinct. Deterministic across restarts.
        let project_id = {
            use sha2::Digest;
            let mut hasher = sha2::Sha256::new();
            hasher.update(canonical.as_bytes());
            hasher.update([0u8]);
            if let Some(remote) = git_remote.as_deref() {
                hasher.update(remote.as_bytes());
            }
            let digest = hasher.finalize();
            format!("{:x}", digest)[..16].to_string()
        };
        RepoIdentity {
            project_id,
            root: canonical.clone(),
            repository_type,
            canonical_root: Some(canonical),
            git_remote,
            commit_sha,
        }
    }

    /// Canonical workspace root for this identity (always `Some` for
    /// identities built via [`RepoIdentity::from_workspace`]).
    pub fn canonical(&self) -> &str {
        self.canonical_root.as_deref().unwrap_or(&self.root)
    }

    /// True when both identities denote the same repository workspace.
    /// Comparison is on canonical roots only — never on raw paths —
    /// so workspace isolation holds even with symlinked inputs.
    pub fn same_workspace(&self, other: &RepoIdentity) -> bool {
        self.canonical() == other.canonical()
    }
}

/// Lexically normalise + symlink-resolve a workspace root. Never fails:
/// falls back to the raw path string when canonicalisation is impossible
/// (missing directory in tests, permission errors).
fn canonical_root_of(path: &Path) -> String {
    // Resolve symlinks where the target exists; otherwise lexically clean
    // `.`/`..` and trailing slashes without touching the filesystem.
    // The lexical pass mirrors `canonical_workspace_key` in
    // context-runtime (absolute roots stay absolute, `..` above `/`
    // saturates at `/`) so identities agree with storage keys even for
    // paths that do not exist (yet).
    if let Ok(resolved) = std::fs::canonicalize(path) {
        return resolved.to_string_lossy().to_string();
    }
    let raw = path.to_string_lossy().replace('\\', "/");
    let absolute = raw.starts_with('/');
    let mut parts: Vec<&str> = Vec::new();
    for seg in raw.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() && !absolute {
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
        path.to_string_lossy().to_string()
    } else {
        out
    }
}

/// Best-effort `origin` (else first) git remote URL. `None` outside git repos.
fn git_remote_of(canonical_root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .current_dir(canonical_root)
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            return Some(url);
        }
    }
    // Fallback: first remote when `origin` is absent.
    let output = std::process::Command::new("git")
        .current_dir(canonical_root)
        .args(["remote", "-v"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_first_remote_url(&String::from_utf8_lossy(&output.stdout))
}

/// First remote URL from `git remote -v` output. Pure over the text so
/// blank or malformed lines are skipped (never abort the whole parse).
fn parse_first_remote_url(text: &str) -> Option<String> {
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        // Blank/malformed lines are skipped line-locally: they must never
        // abort the scan for a later well-formed remote entry.
        let Some(_name) = parts.next() else {
            continue;
        };
        let Some(url) = parts.next() else {
            continue;
        };
        if url.is_empty() {
            continue;
        }
        return Some(url.to_string());
    }
    None
}

/// Best-effort HEAD SHA. `None` outside git repos.
fn git_head_of(canonical_root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .current_dir(canonical_root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

#[cfg(test)]
mod p6_identity_tests {
    use super::*;

    #[test]
    fn canonical_root_is_stable_across_dotdot_inputs() {
        let dir = tempfile::tempdir().unwrap();
        let a = RepoIdentity::from_workspace(dir.path());
        let via_dotdot = dir
            .path()
            .join(".")
            .join("..")
            .join(dir.path().file_name().expect("tempdir has a file name"));
        let b = RepoIdentity::from_workspace(&via_dotdot);
        assert_eq!(a.canonical(), b.canonical());
        assert_eq!(a.project_id, b.project_id);
        assert!(a.same_workspace(&b));
    }

    #[test]
    fn different_roots_have_different_identities() {
        let a_dir = tempfile::tempdir().unwrap();
        let b_dir = tempfile::tempdir().unwrap();
        let a = RepoIdentity::from_workspace(a_dir.path());
        let b = RepoIdentity::from_workspace(b_dir.path());
        assert_ne!(a.project_id, b.project_id);
        assert!(!a.same_workspace(&b));
    }

    #[test]
    fn identity_is_stable_across_calls() {
        let dir = tempfile::tempdir().unwrap();
        let a = RepoIdentity::from_workspace(dir.path());
        let b = RepoIdentity::from_workspace(dir.path());
        assert_eq!(a.project_id, b.project_id);
        assert_eq!(a.root, b.root);
    }

    #[test]
    fn legacy_fields_remain_populated() {
        let dir = tempfile::tempdir().unwrap();
        let id = RepoIdentity::from_workspace(dir.path());
        assert!(!id.project_id.is_empty());
        assert!(!id.root.is_empty());
        assert!(!id.repository_type.is_empty());
    }

    #[test]
    fn lexical_fallback_keeps_absolute_roots() {
        // Missing paths: `..` segments resolve lexically, absolute roots
        // stay absolute, and equivalent spellings share one identity.
        let a = RepoIdentity::from_workspace(Path::new("/definitely-missing-audit-probe/sub/../c"));
        assert_eq!(a.canonical(), "/definitely-missing-audit-probe/c");
        let b = RepoIdentity::from_workspace(Path::new("/definitely-missing-audit-probe/c"));
        assert_eq!(a.canonical(), b.canonical());
        assert_eq!(a.project_id, b.project_id);
        assert!(a.same_workspace(&b));
        // `..` above `/` saturates at the root instead of producing a
        // relative path (the old fallback returned `"c"` here).
        let c = RepoIdentity::from_workspace(Path::new("/b/../../c"));
        assert_eq!(c.canonical(), "/c");
    }

    #[test]
    fn remote_parse_skips_blank_and_malformed_lines() {
        // Blank lines and name-only lines are skipped, not fatal.
        let out =
            parse_first_remote_url("\n   \norigin\nupstream\thttps://example.com/b.git (fetch)\n");
        assert_eq!(out.as_deref(), Some("https://example.com/b.git"));
        let out = parse_first_remote_url("\n\norigin\thttps://example.com/a.git (fetch)\n");
        assert_eq!(out.as_deref(), Some("https://example.com/a.git"));
        let out = parse_first_remote_url("");
        assert_eq!(out, None);
        let out = parse_first_remote_url("   \n\t\n");
        assert_eq!(out, None);
    }

    fn git_repo_with_commit() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["config", "user.email", "t@t"],
            vec!["config", "user.name", "t"],
        ] {
            std::process::Command::new("git")
                .args(&args)
                .current_dir(dir.path())
                .output()
                .unwrap();
        }
        std::fs::write(dir.path().join("tracked.txt"), "v1\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "tracked.txt"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-qm", "init"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        dir
    }

    #[test]
    fn untracked_content_edit_changes_working_tree_hash() {
        let dir = git_repo_with_commit();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("helper.txt"), "one\n").unwrap();
        let before = RepoState::capture(&root).expect("git repo must capture");
        std::fs::write(root.join("helper.txt"), "two\n").unwrap();
        let after = RepoState::capture(&root).expect("git repo must capture");
        assert_ne!(
            before.working_tree_hash, after.working_tree_hash,
            "editing untracked contents must invalidate the revision"
        );
    }

    #[test]
    fn codebro_derived_output_does_not_invalidate_revision() {
        let dir = git_repo_with_commit();
        let root = dir.path().to_path_buf();
        let before = RepoState::capture(&root).expect("git repo must capture");
        // Simulate an index run writing derived output (untracked, and
        // possibly un-ignored in fresh repos): the revision must not move.
        std::fs::create_dir_all(root.join(".codebro")).unwrap();
        std::fs::write(root.join(".codebro/facts.json"), "{\"v\":1}").unwrap();
        std::fs::write(root.join(".codebro/cache.bin"), vec![0u8; 1024]).unwrap();
        let after = RepoState::capture(&root).expect("git repo must capture");
        assert_eq!(
            before.working_tree_hash, after.working_tree_hash,
            "derived .codebro/ output must not invalidate the revision"
        );
        assert_eq!(before.working_tree_dirty, after.working_tree_dirty);
    }

    #[test]
    fn capture_is_deterministic_across_calls() {
        let dir = git_repo_with_commit();
        let root = dir.path().to_path_buf();
        std::fs::write(root.join("notes.txt"), "scratch\n").unwrap();
        let a = RepoState::capture(&root).expect("capture");
        let b = RepoState::capture(&root).expect("capture");
        assert_eq!(a.working_tree_hash, b.working_tree_hash);
    }
}

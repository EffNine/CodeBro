//! Repository state capture — shared runtime primitive.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::hash::Hasher;

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

        // Check dirty: uncommitted changes or untracked files.
        let dirty_output = std::process::Command::new("git")
            .current_dir(workspace_root)
            .args(["status", "--porcelain"])
            .output()
            .ok()?;
        let working_tree_dirty = !dirty_output.status.success()
            || !String::from_utf8_lossy(&dirty_output.stdout)
                .trim()
                .is_empty();

        // Compute a deterministic hash of working-tree state.
        // Strategy: hash(sorted tracked files + diff + untracked).
        let working_tree_hash = {
            let mut parts: Vec<Vec<u8>> = Vec::new();

            // Tracked files list (sorted)
            let ls_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["ls-files"])
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
                .args(["diff", "HEAD"])
                .output()
                .ok();
            if let Some(ref diff) = diff_output {
                if diff.status.success() {
                    parts.push(diff.stdout.clone());
                }
            }

            // Untracked files
            let untracked_output = std::process::Command::new("git")
                .current_dir(workspace_root)
                .args(["ls-files", "--others", "--exclude-standard"])
                .output()
                .ok();
            if let Some(ref ut) = untracked_output {
                if ut.status.success() {
                    parts.push(ut.stdout.clone());
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoIdentity {
    /// Deterministic project identifier derived from workspace root + manifest.
    pub project_id: String,
    /// Absolute workspace root path.
    pub root: String,
    /// Detected repository type: "cargo", "go", "npm", "unknown".
    pub repository_type: String,
}

impl RepoIdentity {
    /// Derive from a workspace root. Uses project_identity runtime where available.
    pub fn from_workspace(workspace_root: &Path) -> Self {
        let root = workspace_root.to_string_lossy().to_string();
        let repository_type = if workspace_root.join("Cargo.toml").exists() {
            "cargo".to_string()
        } else if workspace_root.join("go.mod").exists() {
            "go".to_string()
        } else if workspace_root.join("package.json").exists() {
            "npm".to_string()
        } else {
            "unknown".to_string()
        };
        // project_id: hash of root for deterministic short identifier
        let project_id = {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            std::hash::Hash::hash(&root, &mut hasher);
            format!("{:x}", hasher.finish())
        };
        RepoIdentity {
            project_id,
            root,
            repository_type,
        }
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Repository Discovery (P10.4).
//!
//! Observes whether a workspace is a revision-control repository and
//! captures lightweight facts about it (branch, remotes, submodules).
//! This is **observation only** — it does not run git, does not stage or
//! commit, and does not analyse diffs. Git *implementation* belongs to
//! the git layer, not the Workspace Runtime.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::workspace_runtime::context::WorkspaceRoot;

/// The flavour of version control observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VcsKind {
    Git,
    Mercurial,
    Svn,
    None,
}

impl Default for VcsKind {
    fn default() -> Self {
        VcsKind::None
    }
}

/// Facts observed about a repository (read-only).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RepositoryFacts {
    pub kind: VcsKind,
    pub root: Option<PathBuf>,
    /// Current branch (from `.git/HEAD`), when discoverable.
    pub head: Option<String>,
    /// Remote names → urls observed in `.git/config`.
    pub remotes: Vec<(String, String)>,
    /// Whether there are nested submodules.
    pub has_submodules: bool,
}

impl RepositoryFacts {
    pub fn is_git(&self) -> bool {
        self.kind == VcsKind::Git
    }
    pub fn head(&self) -> Option<&str> {
        self.head.as_deref()
    }
}

/// Lightweight repository observer.
pub struct RepositoryDetector;

impl RepositoryDetector {
    /// Detect repository facts for a workspace root.
    ///
    /// Reads only the top-level `.git` directory metadata — never runs a
    /// git subprocess, so it stays cheap and safe.
    pub fn detect(root: &WorkspaceRoot) -> RepositoryFacts {
        let git_dir = root.join(".git");
        if git_dir.is_dir() || git_dir.is_file() {
            Self::git_facts(root, &git_dir)
        } else if root.join(".hg").is_dir() {
            RepositoryFacts {
                kind: VcsKind::Mercurial,
                root: Some(root.0.clone()),
                ..Default::default()
            }
        } else if root.join(".svn").is_dir() {
            RepositoryFacts {
                kind: VcsKind::Svn,
                root: Some(root.0.clone()),
                ..Default::default()
            }
        } else {
            RepositoryFacts {
                kind: VcsKind::None,
                ..Default::default()
            }
        }
    }

    fn git_facts(root: &WorkspaceRoot, git_dir: &Path) -> RepositoryFacts {
        let mut facts = RepositoryFacts {
            kind: VcsKind::Git,
            root: Some(root.0.clone()),
            ..Default::default()
        };

        // Branch: `.git/HEAD` is typically "ref: refs/heads/<branch>".
        let head_path = git_dir.join("HEAD");
        if let Ok(content) = std::fs::read_to_string(&head_path) {
            facts.head = parse_head(&content);
        }

        // Remotes + submodules from `.git/config`.
        let config_path = git_dir.join("config");
        if let Ok(content) = std::fs::read_to_string(&config_path) {
            facts.remotes = parse_remotes(&content);
            facts.has_submodules = content.contains("[submodule");
        }

        facts
    }
}

/// Parse `ref: refs/heads/main` into `main`.
fn parse_head(content: &str) -> Option<String> {
    let raw = content.trim();
    if let Some(rest) = raw.strip_prefix("ref: refs/heads/") {
        Some(rest.trim().to_string())
    } else {
        // Detached HEAD: a 40-hex sha.
        None
    }
}

/// Parse remote url pairs from a git config text.
fn parse_remotes(content: &str) -> Vec<(String, String)> {
    let mut remotes = Vec::new();
    let mut current: Option<String> = None;
    for line in content.lines() {
        let line = line.trim();
        if line.starts_with("[remote ") {
            let name = line
                .trim_start_matches("[remote \"")
                .trim_end_matches("\"]")
                .trim()
                .to_string();
            current = Some(name);
        } else if let Some(name) = &current {
            if let Some(rest) = line.strip_prefix("url = ") {
                remotes.push((name.clone(), rest.to_string()));
            }
        }
    }
    remotes
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Workspace Metadata (P10.4).
//!
//! Aggregates the lightweight facts the Workspace Runtime has observed:
//! discovery results, repository facts, environment facts, and file-count
//! from the latest snapshot. Metadata is immutable once built.
//!
//! Building metadata is **lazy** — it is a pure aggregation over the
//! runtime's cached observations, so it never touches the disk by itself.

use serde::{Deserialize, Serialize};

use crate::workspace_runtime::context::WorkspaceRoot;
use crate::workspace_runtime::discovery::DiscoveryReport;
use crate::workspace_runtime::environment::EnvironmentProfile;
use crate::workspace_runtime::repository::RepositoryFacts;

/// Immutable metadata describing a workspace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceMetadata {
    pub root: WorkspaceRoot,
    pub language: Option<String>,
    pub has_git: bool,
    pub branch: Option<String>,
    pub remote_url: Option<String>,
    pub build_tool: Option<String>,
    pub package_manager: Option<String>,
    pub os: String,
    pub toolchains: Vec<String>,
    pub file_count: usize,
    /// Number of snapshots captured in this runtime session.
    pub snapshot_count: usize,
    /// Whether the runtime has captured at least one snapshot.
    pub has_snapshot: bool,
}

/// Serialisable view of the detected OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Environment {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Environment::MacOs => "macos",
            Environment::Linux => "linux",
            Environment::Windows => "windows",
            Environment::Other => "other",
        };
        write!(f, "{s}")
    }
}

impl WorkspaceMetadata {
    /// A minimal metadata record with nothing observed yet.
    pub fn empty(root: WorkspaceRoot) -> Self {
        WorkspaceMetadata {
            root,
            language: None,
            has_git: false,
            branch: None,
            remote_url: None,
            build_tool: None,
            package_manager: None,
            os: Environment::Other.to_string(),
            toolchains: Vec::new(),
            file_count: 0,
            snapshot_count: 0,
            has_snapshot: false,
        }
    }

    /// Fold discovery + repository + environment observations into metadata.
    pub fn build(
        root: WorkspaceRoot,
        discovery: &DiscoveryReport,
        repo: &RepositoryFacts,
        env: &EnvironmentProfile,
        file_count: usize,
        snapshot_count: usize,
        has_snapshot: bool,
    ) -> Self {
        let remote_url = repo
            .remotes
            .iter()
            .find(|r| r.0 == "origin")
            .or_else(|| repo.remotes.first())
            .map(|r| r.1.clone());

        WorkspaceMetadata {
            root,
            language: discovery.language.clone(),
            has_git: repo.is_git(),
            branch: repo.head.clone(),
            remote_url,
            build_tool: discovery.build_tool().map(|t| t.to_string()),
            package_manager: discovery.package_tool().map(|t| t.to_string()),
            os: match env.os {
                crate::workspace_runtime::environment::Os::MacOs => Environment::MacOs,
                crate::workspace_runtime::environment::Os::Linux => Environment::Linux,
                crate::workspace_runtime::environment::Os::Windows => Environment::Windows,
                _ => Environment::Other,
            }
            .to_string(),
            toolchains: env.available_tools.clone(),
            file_count,
            snapshot_count,
            has_snapshot,
        }
    }
}

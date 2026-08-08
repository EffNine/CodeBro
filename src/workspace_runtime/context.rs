#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Workspace Context & Core Types (P10.4 - Workspace Intelligence Runtime).
//!
//! The Workspace Runtime understands the developer workspace WITHOUT
//! performing full-project indexing. It is lightweight, incremental and
//! lazy. This module defines the shared, immutable value types that other
//! runtime modules operate on.
//!
//! # Design Rules
//!
//! - **Immutable snapshots**: every value here is cheaply cloneable and is
//!   shared through `Arc` produced by the snapshot manager.
//! - **Thread safety**: every value in this module is `Send + Sync`.
//! - **Lazy by default**: nothing here builds or indexes the project until
//!   explicitly requested.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

/// A lightweight newtype wrapper over a workspace root directory.
///
/// It is intentionally opaque: consumers hold a `WorkspaceRoot` rather than
/// a raw path so the runtime can enforce that all traversal happens through
/// the runtime's own filesystem abstraction.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkspaceRoot(pub PathBuf);

impl WorkspaceRoot {
    /// Construct a new workspace root from a path.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        WorkspaceRoot(path.into())
    }

    /// The underlying path.
    pub fn as_path(&self) -> &std::path::Path {
        &self.0
    }

    /// The leaf directory name (e.g. the project folder name).
    pub fn name(&self) -> String {
        self.0
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.0.to_string_lossy().to_string())
    }

    /// Join a relative segment onto the root.
    pub fn join(&self, segment: impl AsRef<std::path::Path>) -> PathBuf {
        self.0.join(segment)
    }

    /// Whether the root currently exists on disk.
    pub fn exists(&self) -> bool {
        self.0.exists()
    }
}

impl Default for WorkspaceRoot {
    fn default() -> Self {
        WorkspaceRoot::new(".")
    }
}

impl fmt::Display for WorkspaceRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl From<PathBuf> for WorkspaceRoot {
    fn from(p: PathBuf) -> Self {
        WorkspaceRoot::new(p)
    }
}

/// A workspace context bundles a root with lazily-discovered metadata.
///
/// The context does not own any indexes. It is the handle consumers pass
/// into the runtime's discovery and snapshot services.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceContext {
    /// The workspace root directory.
    pub root: WorkspaceRoot,
    /// Maximum traversal depth for listing operations. `None` = unlimited.
    pub max_depth: Option<usize>,
    /// Directory segments excluded from any traversal (e.g. `.git`).
    pub exclusion_globs: Vec<String>,
}

impl WorkspaceContext {
    /// Construct a new, empty workspace context for a root.
    pub fn new(root: impl Into<WorkspaceRoot>) -> Self {
        WorkspaceContext {
            root: root.into(),
            max_depth: None,
            exclusion_globs: vec![".git".to_string(), "target".to_string()],
        }
    }

    /// Limit listing depth.
    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = Some(depth);
        self
    }

    /// Add an exclusion glob segment.
    pub fn with_exclusion(mut self, glob: impl Into<String>) -> Self {
        self.exclusion_globs.push(glob.into());
        self
    }

    /// True when a relative path's first component is excluded.
    pub fn is_excluded(&self, rel: &std::path::Path) -> bool {
        let first = rel
            .components()
            .next()
            .map(|c| c.as_os_str().to_string_lossy().to_string());
        match first {
            Some(seg) => self.exclusion_globs.contains(&seg),
            None => false,
        }
    }
}

/// Error raised by the Workspace Runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum WorkspaceRuntimeError {
    /// The workspace root does not exist or is not a directory.
    InvalidRoot(String),
    /// A filesystem operation failed.
    Io(String),
    /// The requested snapshot does not exist.
    SnapshotNotFound(String),
    /// The runtime requires a snapshot that has not yet been taken.
    NoSnapshot,
    /// The filesystem abstraction rejected a path (e.g. non-UTF8, unsafe).
    UnsupportedPath(String),
    /// The environment could not be detected within budget.
    EnvironmentDetectionFailed(String),
    /// A generic runtime error.
    Generic(String),
}

impl fmt::Display for WorkspaceRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspaceRuntimeError::InvalidRoot(p) => write!(f, "Invalid workspace root: {p}"),
            WorkspaceRuntimeError::Io(msg) => write!(f, "Filesystem IO error: {msg}"),
            WorkspaceRuntimeError::SnapshotNotFound(id) => {
                write!(f, "Snapshot not found: {id}")
            }
            WorkspaceRuntimeError::NoSnapshot => write!(f, "No snapshot has been captured"),
            WorkspaceRuntimeError::UnsupportedPath(p) => {
                write!(f, "Unsupported path: {p}")
            }
            WorkspaceRuntimeError::EnvironmentDetectionFailed(msg) => {
                write!(f, "Environment detection failed: {msg}")
            }
            WorkspaceRuntimeError::Generic(msg) => write!(f, "Workspace runtime error: {msg}"),
        }
    }
}

impl std::error::Error for WorkspaceRuntimeError {}

pub type WorkspaceRuntimeResult<T> = Result<T, WorkspaceRuntimeError>;

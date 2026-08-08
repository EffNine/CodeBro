#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Workspace Intelligence Runtime (P10.4).
//!
//! The Workspace Runtime understands the developer workspace **without**
//! performing full-project indexing. It is lightweight, incremental and
//! lazy.
//!
//! # Ownership
//!
//! The runtime owns:
//! - Workspace discovery
//! - Repository discovery
//! - Filesystem abstraction
//! - Workspace snapshot
//! - Incremental file watching abstraction
//! - Build system discovery
//! - Package manager discovery
//! - Environment detection
//! - Workspace diagnostics
//! - Workspace metadata
//!
//! The runtime does **not** own: AI logic, memory, provider logic, git
//! implementation, LSP analysis, engineering graphs, or agent
//! orchestration.
//!
//! # Performance Contract
//!
//! - Cold startup < 300 ms
//! - Idle CPU < 1%
//! - Idle memory < 64 MB
//! - Workspace discovery < 100 ms (small projects)
//! - No eager indexing — everything expensive is lazy

pub mod context;
pub mod diagnostics;
pub mod discovery;
pub mod environment;
pub mod filesystem;
pub mod metadata;
pub mod repository;
pub mod snapshot;
pub mod watcher;

#[cfg(test)]
mod tests;

pub use context::{WorkspaceContext, WorkspaceRoot, WorkspaceRuntimeError, WorkspaceRuntimeResult};
pub use diagnostics::{DiagnosticsSummary, OpRecord, WorkspaceDiagnostics};
pub use discovery::{
    BuildSystemInfo, DiscoveryEngine, DiscoveryReport, PackageManagerInfo, ToolKind,
};
pub use environment::{Arch, EnvironmentDetector, EnvironmentProfile, Os};
pub use filesystem::{EntryInfo, EntryKind, FileSystem, Listing, LocalFileSystem};
pub use metadata::{Environment, WorkspaceMetadata};
pub use repository::{RepositoryDetector, RepositoryFacts, VcsKind};
pub use snapshot::{
    compute_diff, Change, ChangeKind, FileKind, SnapshotDiff, SnapshotEntry, SnapshotManager,
    WorkspaceSnapshot,
};
pub use watcher::{FileWatcher, WatchBatch, WatchEvent};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Coordinates all Workspace Runtime services through a single observation
/// facade. Built lazily; capture/discovery happen only on explicit calls.
pub struct WorkspaceRuntime {
    context: WorkspaceContext,
    fs: Arc<dyn FileSystem>,
    snapshots: SnapshotManager,
    watcher: FileWatcher,
    diagnostics: WorkspaceDiagnostics,
    discovery: std::sync::RwLock<Option<DiscoveryReport>>,
    environment: std::sync::RwLock<Option<EnvironmentProfile>>,
    repository: std::sync::RwLock<Option<RepositoryFacts>>,
    metadata: std::sync::RwLock<Option<WorkspaceMetadata>>,
}

impl WorkspaceRuntime {
    /// Construct a new runtime for a workspace root. Performs no filesystem
    /// work — discovery and snapshot capture are fully lazy.
    pub fn new(root: impl Into<PathBuf>, fs: Arc<dyn FileSystem>) -> Self {
        let context = WorkspaceContext::new(root.into());
        let snapshot_manager = SnapshotManager::new();
        let watcher = FileWatcher::new(context.clone(), fs.clone());
        WorkspaceRuntime {
            context,
            fs,
            snapshots: snapshot_manager,
            watcher,
            diagnostics: WorkspaceDiagnostics::new(),
            discovery: std::sync::RwLock::new(None),
            environment: std::sync::RwLock::new(None),
            repository: std::sync::RwLock::new(None),
            metadata: std::sync::RwLock::new(None),
        }
    }

    /// Construct a runtime with an explicitly configured context.
    pub fn with_context(context: WorkspaceContext, fs: Arc<dyn FileSystem>) -> Self {
        let watcher = FileWatcher::new(context.clone(), fs.clone());
        WorkspaceRuntime {
            context,
            fs,
            snapshots: SnapshotManager::new(),
            watcher,
            diagnostics: WorkspaceDiagnostics::new(),
            discovery: std::sync::RwLock::new(None),
            environment: std::sync::RwLock::new(None),
            repository: std::sync::RwLock::new(None),
            metadata: std::sync::RwLock::new(None),
        }
    }

    // ── Accessors ─────────────────────────────────────────────────────────

    pub fn context(&self) -> &WorkspaceContext {
        &self.context
    }

    pub fn snapshots(&self) -> &SnapshotManager {
        &self.snapshots
    }

    pub fn watcher(&self) -> &FileWatcher {
        &self.watcher
    }

    pub fn diagnostics(&self) -> &WorkspaceDiagnostics {
        &self.diagnostics
    }

    pub fn filesystem(&self) -> &dyn FileSystem {
        self.fs.as_ref()
    }

    // ── Lazy observation operations ───────────────────────────────────────

    /// Run static discovery (build system, package manager). Cheap and
    /// shallow. Results are cached for later access.
    pub fn discover(&self) -> WorkspaceRuntimeResult<DiscoveryReport> {
        let start = Instant::now();
        let report = DiscoveryEngine::discover(&self.context.root);

        // Environment detection is also cheap and cached here.
        let env = EnvironmentDetector::detect();
        let repo = RepositoryDetector::detect(&self.context.root);

        self.discovery
            .write()
            .unwrap()
            .clone_from(&Some(report.clone()));
        *self.environment.write().unwrap() = Some(env);
        *self.repository.write().unwrap() = Some(repo);

        self.diagnostics
            .record_discovery(start.elapsed().as_millis() as u64);
        Ok(report)
    }

    /// Ensure discovery has run at least once (idempotent, cheap).
    pub fn ensure_discovered(&self) {
        if self.discovery.read().unwrap().is_none() {
            let _ = self.discover();
        }
    }

    /// Capture a lazy workspace snapshot named `id`. Returns the immutable
    /// snapshot. This is the only place expensive traversal happens.
    pub fn snapshot(&self, id: impl Into<String>) -> WorkspaceRuntimeResult<WorkspaceSnapshot> {
        let id = id.into();
        let start = Instant::now();
        let listing = self
            .fs
            .list(&self.context, 100_000)
            .map_err(|e| WorkspaceRuntimeError::Io(format!("{e:?}")))?;
        let entries = listing
            .entries
            .into_iter()
            .map(|e| (e.rel_path.clone(), e.size, e.modified_ms, e.kind))
            .collect();
        let snapshot = self.snapshots.capture(id, &self.context.root, entries)?;
        self.watcher.set_baseline(snapshot.clone());
        self.diagnostics
            .record_snapshot(start.elapsed().as_millis() as u64);
        Ok(snapshot)
    }

    /// Compute a diff between two snapshot ids.
    pub fn diff(&self, from: &str, to: &str) -> WorkspaceRuntimeResult<SnapshotDiff> {
        let diff = self.snapshots.diff(from, to)?;
        self.diagnostics.record_diff();
        Ok(diff)
    }

    /// Incrementally scan for changes since the last poll. Lazily rescans.
    pub fn poll(&self, max_entries: usize) -> WorkspaceRuntimeResult<WatchBatch> {
        let start = Instant::now();
        let batch = self.watcher.poll("poll", max_entries)?;
        self.diagnostics
            .record_poll(start.elapsed().as_millis() as u64);
        Ok(batch)
    }

    /// Current workspace metadata, folded lazily from cached observations.
    /// Runs discovery first if it hasn't run.
    pub fn metadata(&self) -> WorkspaceMetadata {
        self.ensure_discovered();

        let discovery = self
            .discovery
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| DiscoveryReport {
                root: self.context.root.clone(),
                ..Default::default()
            });
        let repo = (*self.repository.read().unwrap())
            .clone()
            .unwrap_or_default();
        let env = (*self.environment.read().unwrap())
            .clone()
            .unwrap_or_default();

        let has_snapshot = !self.snapshots.is_empty();
        let file_count = self
            .snapshots
            .latest_id()
            .and_then(|id| self.snapshots.get(&id))
            .map(|s| s.file_count)
            .unwrap_or(0);

        WorkspaceMetadata::build(
            self.context.root.clone(),
            &discovery,
            &repo,
            &env,
            file_count,
            self.snapshots.len(),
            has_snapshot,
        )
    }

    /// A single diagnostics summary.
    pub fn summary(&self) -> DiagnosticsSummary {
        self.diagnostics.summary()
    }

    /// Convenience: name of the workspace root folder.
    pub fn name(&self) -> String {
        self.context.root.name()
    }
}

impl Default for WorkspaceRuntime {
    fn default() -> Self {
        WorkspaceRuntime::new(PathBuf::from("."), Arc::new(LocalFileSystem::new()))
    }
}

impl std::fmt::Debug for WorkspaceRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkspaceRuntime")
            .field("root", &self.context.root)
            .field("snapshot_count", &self.snapshots.len())
            .finish()
    }
}

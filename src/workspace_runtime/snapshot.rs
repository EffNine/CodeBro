#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Workspace Snapshot (P10.4).
//!
//! A snapshot is an immutable, point-in-time observation of the workspace
//! file tree (paths, sizes, mtimes, kinds). Snapshots are built **lazily**
//! on request and stored in the runtime; nothing is indexed eagerly.
//!
//! The snapshot layer also computes incremental diffs between two
//! snapshots so that higher layers can react to changes without re-walking
//! the whole tree.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::workspace_runtime::context::{
    WorkspaceRoot, WorkspaceRuntimeError, WorkspaceRuntimeResult,
};
use crate::workspace_runtime::filesystem::{EntryInfo, FileSystem};

/// A single file-tree entry in a snapshot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SnapshotEntry {
    pub rel_path: PathBuf,
    pub size: u64,
    pub modified_ms: Option<u64>,
    pub kind: FileKind,
}

/// The kind of snapshot entry.
pub use crate::workspace_runtime::filesystem::EntryKind as FileKind;

/// An immutable snapshot of the workspace file tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceSnapshot {
    pub id: String,
    pub root: WorkspaceRoot,
    pub captured_at_ms: u64,
    pub entries: Vec<SnapshotEntry>,
    /// Number of files counted (entries, for quick access).
    pub file_count: usize,
}

/// Change kind between two snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChangeKind {
    Created,
    Modified,
    Deleted,
}

/// A single delta between snapshots.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Change {
    pub rel_path: PathBuf,
    pub kind: ChangeKind,
    pub from: Option<SnapshotEntry>,
    pub to: Option<SnapshotEntry>,
}

/// The diff computed between two snapshots.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct SnapshotDiff {
    pub changes: Vec<Change>,
    pub created: usize,
    pub modified: usize,
    pub deleted: usize,
}

impl SnapshotDiff {
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
    pub fn count(&self) -> usize {
        self.changes.len()
    }
}

/// Builds and stores snapshots of a workspace.
///
/// Thread-safe via interior mutability. Snapshot capture is lazy — it
/// happens only when `capture` is called; constructing the manager does no
/// filesystem work.
pub struct SnapshotManager {
    snapshots: std::sync::RwLock<HashMap<String, WorkspaceSnapshot>>,
    order: std::sync::RwLock<Vec<String>>,
}

impl Default for SnapshotManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SnapshotManager {
    pub fn new() -> Self {
        SnapshotManager {
            snapshots: std::sync::RwLock::new(HashMap::new()),
            order: std::sync::RwLock::new(Vec::new()),
        }
    }

    /// Capture a snapshot by walking `root` through the filesystem
    /// abstraction. Lazy: performs work now, but only on explicit request.
    pub fn capture(
        &self,
        id: impl Into<String>,
        root: &WorkspaceRoot,
        entries: Vec<(PathBuf, u64, Option<u64>, FileKind)>,
    ) -> WorkspaceRuntimeResult<WorkspaceSnapshot> {
        let id = id.into();
        let mut entries = entries;
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let file_count = entries.iter().filter(|e| e.3 == FileKind::File).count();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let snapshot = WorkspaceSnapshot {
            id: id.clone(),
            root: root.clone(),
            captured_at_ms: now,
            entries: entries
                .into_iter()
                .map(|(p, s, m, k)| SnapshotEntry {
                    rel_path: p,
                    size: s,
                    modified_ms: m,
                    kind: k,
                })
                .collect(),
            file_count,
        };
        {
            let mut store = self.snapshots.write().unwrap();
            store.insert(id.clone(), snapshot.clone());
        }
        {
            let mut order = self.order.write().unwrap();
            if !order.contains(&id) {
                order.push(id);
            }
        }
        Ok(snapshot)
    }

    /// Retrieve a stored snapshot.
    pub fn get(&self, id: &str) -> Option<WorkspaceSnapshot> {
        self.snapshots.read().unwrap().get(id).cloned()
    }

    /// The most recently captured snapshot id and its index, if any.
    pub fn latest_id(&self) -> Option<String> {
        self.order.read().unwrap().last().cloned()
    }

    pub fn len(&self) -> usize {
        self.order.read().unwrap().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Diff two snapshots by id.
    pub fn diff(&self, from: &str, to: &str) -> WorkspaceRuntimeResult<SnapshotDiff> {
        let a = self
            .get(from)
            .ok_or_else(|| WorkspaceRuntimeError::SnapshotNotFound(from.to_string()))?;
        let b = self
            .get(to)
            .ok_or_else(|| WorkspaceRuntimeError::SnapshotNotFound(to.to_string()))?;
        Ok(compute_diff(&a, &b))
    }

    /// Diff the most recent snapshot against empty (itself being "all created").
    pub fn diff_from_empty(&self, id: &str) -> WorkspaceRuntimeResult<SnapshotDiff> {
        let snap = self
            .get(id)
            .ok_or_else(|| WorkspaceRuntimeError::SnapshotNotFound(id.to_string()))?;
        let empty = WorkspaceSnapshot {
            id: "empty".into(),
            root: snap.root.clone(),
            captured_at_ms: 0,
            entries: Vec::new(),
            file_count: 0,
        };
        Ok(compute_diff(&empty, &snap))
    }
}

/// Compute a diff between two snapshots (pure, deterministic).
pub fn compute_diff(from: &WorkspaceSnapshot, to: &WorkspaceSnapshot) -> SnapshotDiff {
    let from_map: HashMap<&Path, &SnapshotEntry> = from
        .entries
        .iter()
        .map(|e| (e.rel_path.as_path(), e))
        .collect();
    let to_map: HashMap<&Path, &SnapshotEntry> = to
        .entries
        .iter()
        .map(|e| (e.rel_path.as_path(), e))
        .collect();

    let mut diff = SnapshotDiff::default();

    // Existing files: created or modified.
    for entry in &to.entries {
        match from_map.get(entry.rel_path.as_path()) {
            None => {
                diff.created += 1;
                diff.changes.push(Change {
                    rel_path: entry.rel_path.clone(),
                    kind: ChangeKind::Created,
                    from: None,
                    to: Some(entry.clone()),
                });
            }
            Some(old) => {
                if old.size != entry.size
                    || (old.modified_ms != entry.modified_ms)
                    || old.kind != entry.kind
                {
                    diff.modified += 1;
                    diff.changes.push(Change {
                        rel_path: entry.rel_path.clone(),
                        kind: ChangeKind::Modified,
                        from: Some((*old).clone()),
                        to: Some(entry.clone()),
                    });
                }
            }
        }
    }

    // Removed files.
    for (path, old) in &from_map {
        if !to_map.contains_key(path) {
            diff.deleted += 1;
            diff.changes.push(Change {
                rel_path: (*path).to_path_buf(),
                kind: ChangeKind::Deleted,
                from: Some((*old).clone()),
                to: None,
            });
        }
    }

    // Deterministic ordering by path.
    diff.changes.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    diff
}

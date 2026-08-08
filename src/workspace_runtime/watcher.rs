#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Incremental File Watching Abstraction (P10.4).
//!
//! A lightweight, lazy watcher that detects changes to the workspace over
//! time by comparing the latest snapshot with a freshly observed listing.
//! It does **not** hold OS-level watches or a hot reactor loop — it is an
//! abstraction that reports what changed between the last observed point
//! and now, using the same filesystem layer and diff logic as the runtime.
//!
//! Polling is opt-in (`poll`), so idle CPU stays ~0.

use serde::{Deserialize, Serialize};
use std::sync::{Arc, RwLock};

use crate::workspace_runtime::context::WorkspaceContext;
use crate::workspace_runtime::filesystem::{EntryKind, FileSystem};
use crate::workspace_runtime::snapshot::{
    compute_diff, Change, ChangeKind, SnapshotDiff, WorkspaceSnapshot,
};

/// What changed between two observed points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WatchEvent {
    pub change: Change,
}

impl WatchEvent {
    pub fn path(&self) -> &std::path::Path {
        &self.change.rel_path
    }
    pub fn kind(&self) -> ChangeKind {
        self.change.kind
    }
}

/// Summarises a batch of watch events.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct WatchBatch {
    pub events: Vec<WatchEvent>,
}

impl WatchBatch {
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }
    pub fn count(&self) -> usize {
        self.events.len()
    }
}

/// Lazy, incremental file watcher.
///
/// Holds the last captured snapshot. `poll` re-scans lazily (on demand) and
/// returns the diff. Thread-safe.
pub struct FileWatcher {
    ctx: WorkspaceContext,
    fs: Arc<dyn FileSystem>,
    baseline: RwLock<Option<WorkspaceSnapshot>>,
    generation: RwLock<u64>,
}

impl FileWatcher {
    pub fn new(ctx: WorkspaceContext, fs: Arc<dyn FileSystem>) -> Self {
        FileWatcher {
            ctx,
            fs,
            baseline: RwLock::new(None),
            generation: RwLock::new(0),
        }
    }

    /// Set the baseline snapshot to diff against. No scanning performed.
    pub fn set_baseline(&self, snapshot: WorkspaceSnapshot) {
        *self.baseline.write().unwrap() = Some(snapshot);
    }

    /// The current baseline id, if any.
    pub fn baseline_id(&self) -> Option<String> {
        self.baseline
            .read()
            .ok()
            .and_then(|g| g.as_ref().map(|s| s.id.clone()))
    }

    /// Lazily rescan the workspace and diff against the baseline. If no
    /// baseline is set, everything observed is reported as created.
    ///
    /// Performs real filesystem work only when called.
    pub fn poll(
        &self,
        snapshot_id: impl Into<String>,
        max_entries: usize,
    ) -> crate::workspace_runtime::context::WorkspaceRuntimeResult<WatchBatch> {
        // Shallow scan through the abstraction.
        let listing = self.fs.list(&self.ctx, max_entries).map_err(|e| {
            crate::workspace_runtime::context::WorkspaceRuntimeError::Io(format!("{e:?}"))
        })?;

        let file_count = listing
            .entries
            .iter()
            .filter(|e| e.kind == EntryKind::File)
            .count();

        let snapshot = WorkspaceSnapshot {
            id: snapshot_id.into(),
            root: self.ctx.root.clone(),
            captured_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            entries: listing
                .entries
                .into_iter()
                .map(|e| crate::workspace_runtime::snapshot::SnapshotEntry {
                    rel_path: e.rel_path.clone(),
                    size: e.size,
                    modified_ms: e.modified_ms,
                    kind: e.kind,
                })
                .collect(),
            file_count,
        };

        let diff = {
            let read_ref = self.baseline.read().unwrap();
            match &*read_ref {
                Some(base) => compute_diff(base, &snapshot),
                None => snapshot_empty(&snapshot),
            }
        };

        // Advance generation and store the new baseline.
        let events = diff
            .changes
            .into_iter()
            .map(|change| WatchEvent { change })
            .collect();
        let batch = WatchBatch { events };
        *self.baseline.write().unwrap() = Some(snapshot);
        let next = self.generation.read().unwrap().saturating_add(1);
        *self.generation.write().unwrap() = next;
        Ok(batch)
    }

    pub fn generation(&self) -> u64 {
        *self.generation.read().unwrap()
    }
}

/// Build a diff that treats every entry as newly-created.
fn snapshot_empty(snapshot: &WorkspaceSnapshot) -> SnapshotDiff {
    let mut diff = SnapshotDiff::default();
    for entry in &snapshot.entries {
        diff.created += 1;
        diff.changes.push(Change {
            rel_path: entry.rel_path.clone(),
            kind: ChangeKind::Created,
            from: None,
            to: Some(entry.clone()),
        });
    }
    diff
}

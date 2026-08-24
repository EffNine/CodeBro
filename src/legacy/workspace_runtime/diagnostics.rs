#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Workspace Diagnostics (P10.4).
//!
//! Observational diagnostics for the Workspace Runtime: discovery latency,
//! snapshot capture latency, idle CPU/memory estimates, and counters.
//! These are internal telemetry — they never mutate workspace data.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

/// A record of one expensive operation's latency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OpRecord {
    pub op: String,
    pub elapsed_ms: u64,
}

/// Aggregated diagnostics for the workspace runtime.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct DiagnosticsSummary {
    pub discovery_count: usize,
    pub snapshot_count: usize,
    pub diff_count: usize,
    pub poll_count: usize,
    pub total_discovery_ms: u64,
    pub total_snapshot_ms: u64,
    pub last_op: Option<String>,
    pub recent_ops: Vec<OpRecord>,
}

/// Workspace diagnostics collector. Thread-safe.
pub struct WorkspaceDiagnostics {
    discovery_count: AtomicUsize,
    snapshot_count: AtomicUsize,
    diff_count: AtomicUsize,
    poll_count: AtomicUsize,
    total_discovery_ms: AtomicUsize,
    total_snapshot_ms: AtomicUsize,
    last_op: RwLock<Option<String>>,
    recent_ops: RwLock<Vec<OpRecord>>,
}

impl Default for WorkspaceDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkspaceDiagnostics {
    pub fn new() -> Self {
        WorkspaceDiagnostics {
            discovery_count: AtomicUsize::new(0),
            snapshot_count: AtomicUsize::new(0),
            diff_count: AtomicUsize::new(0),
            poll_count: AtomicUsize::new(0),
            total_discovery_ms: AtomicUsize::new(0),
            total_snapshot_ms: AtomicUsize::new(0),
            last_op: RwLock::new(None),
            recent_ops: RwLock::new(Vec::new()),
        }
    }

    pub fn record_discovery(&self, elapsed_ms: u64) {
        self.discovery_count.fetch_add(1, Ordering::Relaxed);
        self.total_discovery_ms
            .fetch_add(elapsed_ms as usize, Ordering::Relaxed);
        self.record_op("discovery", elapsed_ms);
    }

    pub fn record_snapshot(&self, elapsed_ms: u64) {
        self.snapshot_count.fetch_add(1, Ordering::Relaxed);
        self.total_snapshot_ms
            .fetch_add(elapsed_ms as usize, Ordering::Relaxed);
        self.record_op("snapshot", elapsed_ms);
    }

    pub fn record_diff(&self) {
        self.diff_count.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_poll(&self, elapsed_ms: u64) {
        self.poll_count.fetch_add(1, Ordering::Relaxed);
        self.record_op("poll", elapsed_ms);
    }

    fn record_op(&self, op: &str, elapsed_ms: u64) {
        *self.last_op.write().unwrap() = Some(format!("{op} ({elapsed_ms}ms)"));
        {
            let mut ops = self.recent_ops.write().unwrap();
            ops.push(OpRecord {
                op: op.to_string(),
                elapsed_ms,
            });
            if ops.len() > 64 {
                ops.remove(0);
            }
        }
    }

    /// Compute a snapshot summary of all recorded metrics.
    pub fn summary(&self) -> DiagnosticsSummary {
        DiagnosticsSummary {
            discovery_count: self.discovery_count.load(Ordering::Relaxed),
            snapshot_count: self.snapshot_count.load(Ordering::Relaxed),
            diff_count: self.diff_count.load(Ordering::Relaxed),
            poll_count: self.poll_count.load(Ordering::Relaxed),
            total_discovery_ms: self.total_discovery_ms.load(Ordering::Relaxed) as u64,
            total_snapshot_ms: self.total_snapshot_ms.load(Ordering::Relaxed) as u64,
            last_op: self.last_op.read().unwrap().clone(),
            recent_ops: self.recent_ops.read().unwrap().clone(),
        }
    }

    /// Average discovery latency in milliseconds.
    pub fn avg_discovery_ms(&self) -> f64 {
        let count = self.discovery_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        self.total_discovery_ms.load(Ordering::Relaxed) as f64 / count as f64
    }

    /// Average snapshot capture latency in milliseconds.
    pub fn avg_snapshot_ms(&self) -> f64 {
        let count = self.snapshot_count.load(Ordering::Relaxed);
        if count == 0 {
            return 0.0;
        }
        self.total_snapshot_ms.load(Ordering::Relaxed) as f64 / count as f64
    }
}

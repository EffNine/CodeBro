//! Transactional multi-file mutation.
//!
//! Layers the single-file prepare/apply seam into an all-or-nothing
//! transaction:
//!
//! 1. `prepare_changes` — validates every request against CURRENT content
//!    (workspace boundary, no blind overwrite, unambiguous match, no
//!    duplicate targets). Read-only.
//! 2. [`PreparedTransaction::preview`] — combined diff preview.
//! 3. `apply_transaction` — re-validates staleness across the WHOLE set
//!    (conflict detection), then applies sequentially; any failure rolls
//!    back every already-applied change from its preparation-time snapshot.
//!
//! Atomicity model: POSIX offers no true multi-file atomic commit, so the
//! engine guarantees **all-or-nothing by rollback** — either every change
//! lands, or the workspace is restored to its exact pre-transaction state.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers

use std::path::PathBuf;

use crate::coding::change_engine::{ChangeEngine, PreparedChange};

/// One requested change inside a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionRequest {
    /// Target path, relative to the workspace root (or absolute inside it).
    pub path: String,
    /// Exact existing text to replace; empty to create the file.
    pub old: String,
    /// Replacement text / full new-file content.
    pub new: String,
}

/// A validated, all-or-nothing set of prepared changes.
#[derive(Debug, Clone)]
pub struct PreparedTransaction {
    pub changes: Vec<PreparedChange>,
}

impl PreparedTransaction {
    /// Combined per-file diff preview for review before applying.
    pub fn preview(&self) -> String {
        let mut out = String::new();
        for c in &self.changes {
            out.push_str(&format!(
                "── {} ({}) ──\n",
                c.path.display(),
                if c.created { "create" } else { "modify" }
            ));
            out.push_str(&c.preview);
            if !out.ends_with('\n') {
                out.push('\n');
            }
        }
        out
    }

    pub fn len(&self) -> usize {
        self.changes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// The outcome of an applied transaction.
#[derive(Debug, Clone, PartialEq)]
pub struct TransactionReport {
    /// Files written (absolute paths), in apply order.
    pub applied: Vec<PathBuf>,
    /// Files created by this transaction.
    pub created: Vec<PathBuf>,
    /// Files rolled back after a mid-transaction failure (empty when the
    /// whole transaction succeeded).
    pub rolled_back: Vec<PathBuf>,
    /// The error that triggered rollback, if any.
    pub failure: Option<String>,
}

impl TransactionReport {
    pub fn success(&self) -> bool {
        self.failure.is_none()
    }
}

impl ChangeEngine {
    /// Validate and prepare a multi-file transaction against CURRENT
    /// filesystem state. Read-only. Rejects the whole set if ANY request is
    /// invalid, or if two requests target the same file (ordering within a
    /// transaction would be ambiguous otherwise).
    pub fn prepare_changes(
        &self,
        requests: &[TransactionRequest],
    ) -> crate::error::Result<PreparedTransaction> {
        if requests.is_empty() {
            return Err(crate::error::CodeBroError::Patch(
                "empty transaction: at least one change is required".to_string(),
            ));
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for r in requests {
            if !seen.insert(r.path.clone()) {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "duplicate target in transaction: '{}' appears more than once",
                    r.path
                )));
            }
        }
        let mut changes = Vec::with_capacity(requests.len());
        for r in requests {
            let prepared = self.prepare(&r.path, &r.old, &r.new)?;
            changes.push(prepared);
        }
        Ok(PreparedTransaction { changes })
    }

    /// Apply a prepared transaction atomically-by-rollback:
    ///
    /// - **Conflict pass** — every file must still match its preparation
    ///   snapshot before anything is written; one stale file aborts the
    ///   whole transaction with zero mutations.
    /// - **Apply pass** — sequential writes through the single-file seam;
    ///   the first failure stops the pass and restores every previously
    ///   applied file from its snapshot (created files are deleted).
    pub fn apply_transaction(
        &self,
        tx: &PreparedTransaction,
    ) -> crate::error::Result<TransactionReport> {
        // Conflict/staleness pass — no writes yet.
        for c in &tx.changes {
            let current = if c.created {
                if c.path.exists() {
                    return Err(crate::error::CodeBroError::Patch(format!(
                        "transaction conflict: '{}' was created since preparation",
                        c.path.display()
                    )));
                }
                continue;
            } else {
                match std::fs::read_to_string(&c.path) {
                    Ok(text) => text,
                    Err(e) => {
                        return Err(crate::error::CodeBroError::Patch(format!(
                            "transaction conflict: '{}' unreadable: {e}",
                            c.path.display()
                        )))
                    }
                }
            };
            if current != c.backup {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "transaction conflict: '{}' changed since preparation",
                    c.path.display()
                )));
            }
        }

        // Apply pass with rollback.
        let mut applied: Vec<PathBuf> = Vec::new();
        let mut created: Vec<PathBuf> = Vec::new();
        let mut rolled_back: Vec<PathBuf> = Vec::new();
        for c in &tx.changes {
            match self.apply(c) {
                Ok(_) => {
                    applied.push(c.path.clone());
                    if c.created {
                        created.push(c.path.clone());
                    }
                }
                Err(e) => {
                    // Roll back in reverse apply order.
                    rolled_back = self.rollback_changes(tx, &applied);
                    return Err(crate::error::CodeBroError::Patch(format!(
                        "transaction failed at '{}': {e}; rolled back {} file(s)",
                        c.path.display(),
                        rolled_back.len()
                    )));
                }
            }
        }

        Ok(TransactionReport {
            applied,
            created,
            rolled_back,
            failure: None,
        })
    }

    /// Restore every file in `applied` (in reverse order) to its
    /// preparation-time snapshot: modified files are rewritten with their
    /// original bytes; created files are deleted. Returns the paths that
    /// were rolled back.
    ///
    /// Public because it is the reusable core of mid-transaction rollback;
    /// also exercised directly by tests since inducing real write failures
    /// is environment-dependent (root ignores permission bits).
    pub fn rollback_changes(&self, tx: &PreparedTransaction, applied: &[PathBuf]) -> Vec<PathBuf> {
        let mut rolled_back = Vec::with_capacity(applied.len());
        for done in applied.iter().rev() {
            let Some(original) = tx.changes.iter().find(|c| &c.path == done) else {
                continue;
            };
            if original.created {
                let _ = std::fs::remove_file(done);
            } else if let Err(restore_err) =
                codebro_core::persistence::write_atomic(done, original.backup.as_bytes())
            {
                tracing::error!("rollback of {} failed: {restore_err}", done.display());
            }
            rolled_back.push(done.clone());
        }
        rolled_back
    }
}

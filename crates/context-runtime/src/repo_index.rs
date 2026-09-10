//! P6 repository index metadata runtime: derived freshness bookkeeping.
//!
//! The `repo_indexes` table (schema v7) holds *derived* per-workspace
//! index state: canonical root, repository identity JSON, lifecycle
//! status, timestamps, revision, and counts. It never stores file
//! contents, symbols, or edges — those live in `.codebro/facts.json`.
//!
//! ```text
//! reindex/init completes
//!   └── upsert_index(READY, counts, revision)  (one row per workspace)
//! repository changes
//!   └── computed STALE at read time (diff vs facts.json digests)
//! ```
//!
//! Invariants:
//! - Workspace isolation: every read/write canonicalises the root and
//!   scopes by exact match. Cross-workspace reads are refused by construction.
//! - No fabrication: absent rows read as UNKNOWN, never READY.
//! - Bounded: identity JSON is capped (4 KiB) and redacted before storage.
//! - Request-driven: no background writes; callers upsert explicitly.

#![allow(dead_code, unused_imports)]

use rusqlite::{params, OptionalExtension};

use crate::store::{ContextError, ContextStore};
use crate::workspace::canonical_workspace_key;

/// Lifecycle status for the persisted index row. Mirrors the indexer
/// `IndexStatus` vocabulary without depending on that crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoIndexStatus {
    Unknown,
    Discovering,
    Indexing,
    Ready,
    Stale,
    Failed,
}

impl RepoIndexStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RepoIndexStatus::Unknown => "UNKNOWN",
            RepoIndexStatus::Discovering => "DISCOVERING",
            RepoIndexStatus::Indexing => "INDEXING",
            RepoIndexStatus::Ready => "READY",
            RepoIndexStatus::Stale => "STALE",
            RepoIndexStatus::Failed => "FAILED",
        }
    }

    pub fn parse(s: &str) -> Option<RepoIndexStatus> {
        match s {
            "UNKNOWN" => Some(RepoIndexStatus::Unknown),
            "DISCOVERING" => Some(RepoIndexStatus::Discovering),
            "INDEXING" => Some(RepoIndexStatus::Indexing),
            "READY" => Some(RepoIndexStatus::Ready),
            "STALE" => Some(RepoIndexStatus::Stale),
            "FAILED" => Some(RepoIndexStatus::Failed),
            _ => None,
        }
    }
}

/// Derived index bookkeeping for one workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoIndexRecord {
    pub workspace_root: String,
    /// Redacted, bounded repository identity JSON (`{}` when unknown).
    pub repository_identity: String,
    pub index_status: RepoIndexStatus,
    pub indexed_at: u64,
    pub repository_revision: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
    pub stale_count: usize,
    pub updated_at: u64,
}

impl RepoIndexRecord {
    pub fn unknown_for(workspace_root: &str, now: u64) -> Self {
        RepoIndexRecord {
            workspace_root: workspace_root.to_string(),
            repository_identity: "{}".to_string(),
            index_status: RepoIndexStatus::Unknown,
            indexed_at: 0,
            repository_revision: "unknown".to_string(),
            file_count: 0,
            symbol_count: 0,
            edge_count: 0,
            stale_count: 0,
            updated_at: now,
        }
    }
}

/// Input for [`ContextStore::upsert_repo_index`]. All counts are derived
/// from the freshly written `facts.json`; callers must not invent them.
#[derive(Debug, Clone)]
pub struct RepoIndexUpsert {
    pub repository_identity: String,
    pub index_status: RepoIndexStatus,
    pub indexed_at: u64,
    pub repository_revision: String,
    pub file_count: usize,
    pub symbol_count: usize,
    pub edge_count: usize,
    pub stale_count: usize,
}

/// Max bytes for the stored identity JSON (bounded response discipline).
pub const MAX_IDENTITY_JSON_BYTES: usize = 4096;

fn bound_identity_json(raw: &str) -> String {
    // Secret-redact via the single central authority (same patterns as
    // history/tasks redaction: key=value secrets, bearer/PAT shapes, and
    // URL-embedded credentials such as git remote user:pass@host), then
    // truncate with an explicit marker — never silently cut.
    let redacted = codebro_core::tools::shell::redact_secrets_public(raw);
    if redacted.len() > MAX_IDENTITY_JSON_BYTES {
        let mut cut = MAX_IDENTITY_JSON_BYTES;
        while cut > 0 && !redacted.is_char_boundary(cut) {
            cut -= 1;
        }
        let mut out = redacted[..cut].to_string();
        out.push_str("…[truncated for index budget]");
        out
    } else {
        redacted
    }
}

/// One row of the `repo_indexes` table (10 columns).
type RepoIndexRow = (String, String, String, i64, String, i64, i64, i64, i64, i64);

impl ContextStore {
    /// Read the index row for a workspace. Absent rows return an
    /// in-memory UNKNOWN record (never fabricated READY).
    pub fn get_repo_index(
        &self,
        workspace_root: &str,
        now: u64,
    ) -> Result<RepoIndexRecord, ContextError> {
        let key = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            let row: Option<RepoIndexRow> = conn
                .query_row(
                    "SELECT workspace_root, repository_identity, index_status,
                            indexed_at, repository_revision, file_count,
                            symbol_count, edge_count, stale_count, updated_at
                     FROM repo_indexes WHERE workspace_root = ?1",
                    params![key],
                    |r| {
                        Ok((
                            r.get(0)?,
                            r.get(1)?,
                            r.get(2)?,
                            r.get(3)?,
                            r.get(4)?,
                            r.get(5)?,
                            r.get(6)?,
                            r.get(7)?,
                            r.get(8)?,
                            r.get(9)?,
                        ))
                    },
                )
                .optional()?;
            match row {
                None => Ok(RepoIndexRecord::unknown_for(&key, now)),
                Some((
                    ws,
                    identity,
                    status_raw,
                    indexed_at,
                    revision,
                    files,
                    symbols,
                    edges,
                    stale,
                    updated,
                )) => Ok(RepoIndexRecord {
                    workspace_root: ws,
                    repository_identity: identity,
                    index_status: RepoIndexStatus::parse(&status_raw)
                        .unwrap_or(RepoIndexStatus::Unknown),
                    indexed_at: indexed_at.max(0) as u64,
                    repository_revision: revision,
                    file_count: files.max(0) as usize,
                    symbol_count: symbols.max(0) as usize,
                    edge_count: edges.max(0) as usize,
                    stale_count: stale.max(0) as usize,
                    updated_at: updated.max(0) as u64,
                }),
            }
        })
    }

    /// Insert or replace the index row for a workspace. Workspace-scoped
    /// by canonical key; one row per workspace, never cross-workspace.
    pub fn upsert_repo_index(
        &self,
        workspace_root: &str,
        input: RepoIndexUpsert,
        now: u64,
    ) -> Result<RepoIndexRecord, ContextError> {
        let key = canonical_workspace_key(workspace_root);
        let identity = bound_identity_json(&input.repository_identity);
        let record = RepoIndexRecord {
            workspace_root: key.clone(),
            repository_identity: identity.clone(),
            index_status: input.index_status,
            indexed_at: input.indexed_at,
            repository_revision: input.repository_revision.clone(),
            file_count: input.file_count,
            symbol_count: input.symbol_count,
            edge_count: input.edge_count,
            stale_count: input.stale_count,
            updated_at: now,
        };
        let status_str = input.index_status.as_str().to_string();
        let revision = input.repository_revision.clone();
        let (fc, sc, ec, stc, iat) = (
            input.file_count as i64,
            input.symbol_count as i64,
            input.edge_count as i64,
            input.stale_count as i64,
            input.indexed_at as i64,
        );
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO repo_indexes (
                    workspace_root, repository_identity, index_status,
                    indexed_at, repository_revision, file_count,
                    symbol_count, edge_count, stale_count, updated_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(workspace_root) DO UPDATE SET
                    repository_identity = excluded.repository_identity,
                    index_status = excluded.index_status,
                    indexed_at = excluded.indexed_at,
                    repository_revision = excluded.repository_revision,
                    file_count = excluded.file_count,
                    symbol_count = excluded.symbol_count,
                    edge_count = excluded.edge_count,
                    stale_count = excluded.stale_count,
                    updated_at = excluded.updated_at",
                params![key, identity, status_str, iat, revision, fc, sc, ec, stc, now as i64,],
            )?;
            Ok(())
        })?;
        Ok(record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store_in(dir: &std::path::Path) -> ContextStore {
        ContextStore::at_state_dir(dir.to_path_buf())
    }

    #[test]
    fn absent_row_reads_as_unknown_never_ready() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let rec = store.get_repo_index("/repo-a", 100).unwrap();
        assert_eq!(rec.index_status, RepoIndexStatus::Unknown);
        assert_eq!(rec.file_count, 0);
    }

    #[test]
    fn upsert_round_trips_counts_and_status() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let rec = store
            .upsert_repo_index(
                "/repo-a",
                RepoIndexUpsert {
                    repository_identity: r#"{"project_id":"abc"}"#.to_string(),
                    index_status: RepoIndexStatus::Ready,
                    indexed_at: 123,
                    repository_revision: "rev1".to_string(),
                    file_count: 10,
                    symbol_count: 50,
                    edge_count: 20,
                    stale_count: 0,
                },
                200,
            )
            .unwrap();
        assert_eq!(rec.index_status, RepoIndexStatus::Ready);
        let back = store.get_repo_index("/repo-a", 300).unwrap();
        assert_eq!(back, rec);
    }

    #[test]
    fn workspace_isolation_holds() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        store
            .upsert_repo_index(
                "/repo-a",
                RepoIndexUpsert {
                    repository_identity: "{}".to_string(),
                    index_status: RepoIndexStatus::Ready,
                    indexed_at: 1,
                    repository_revision: "r".to_string(),
                    file_count: 5,
                    symbol_count: 5,
                    edge_count: 5,
                    stale_count: 0,
                },
                10,
            )
            .unwrap();
        // Workspace B sees UNKNOWN, never A's row.
        let b = store.get_repo_index("/repo-b", 10).unwrap();
        assert_eq!(b.index_status, RepoIndexStatus::Unknown);
    }

    #[test]
    fn canonicalisation_collapses_dotdot() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        store
            .upsert_repo_index(
                "/repo-a/sub/../",
                RepoIndexUpsert {
                    repository_identity: "{}".to_string(),
                    index_status: RepoIndexStatus::Ready,
                    indexed_at: 1,
                    repository_revision: "r".to_string(),
                    file_count: 1,
                    symbol_count: 1,
                    edge_count: 1,
                    stale_count: 0,
                },
                10,
            )
            .unwrap();
        let back = store.get_repo_index("/repo-a", 10).unwrap();
        assert_eq!(back.index_status, RepoIndexStatus::Ready);
    }

    #[test]
    fn identity_json_is_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let big = format!("{{\"k\":\"{}\"}}", "x".repeat(9000));
        let rec = store
            .upsert_repo_index(
                "/repo-a",
                RepoIndexUpsert {
                    repository_identity: big,
                    index_status: RepoIndexStatus::Ready,
                    indexed_at: 1,
                    repository_revision: "r".to_string(),
                    file_count: 1,
                    symbol_count: 1,
                    edge_count: 1,
                    stale_count: 0,
                },
                10,
            )
            .unwrap();
        assert!(rec
            .repository_identity
            .contains("…[truncated for index budget]"));
    }

    #[test]
    fn identity_json_redacts_secret_values_not_key_names() {
        // Regression: the old hand-rolled loop renamed `"token"` keys to
        // `"<redacted>"` while leaving the secret VALUE (and URL-embedded
        // git-remote credentials) in stored plaintext.
        let dir = tempfile::tempdir().unwrap();
        let store = store_in(dir.path());
        let raw = r#"{"project_id":"abc","git_remote":"https://user:s3cr3t-pw@github.com/org/repo.git","api_key": "sk-abcdefghijklmnop123456"}"#;
        let rec = store
            .upsert_repo_index(
                "/repo-a",
                RepoIndexUpsert {
                    repository_identity: raw.to_string(),
                    index_status: RepoIndexStatus::Ready,
                    indexed_at: 1,
                    repository_revision: "r".to_string(),
                    file_count: 1,
                    symbol_count: 1,
                    edge_count: 1,
                    stale_count: 0,
                },
                10,
            )
            .unwrap();
        assert!(
            !rec.repository_identity.contains("s3cr3t-pw"),
            "URL credential leaked: {}",
            rec.repository_identity
        );
        assert!(
            !rec.repository_identity
                .contains("sk-abcdefghijklmnop123456"),
            "key value leaked: {}",
            rec.repository_identity
        );
        // Non-secret structure survives redaction.
        assert!(rec.repository_identity.contains("project_id"));
    }
}

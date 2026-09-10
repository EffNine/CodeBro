//! [`ContextStore`]: transactional operations over the context database.
//!
//! All writes are validated, serialized on an in-process mutex (mirroring
//! the server's `mutation_lock` discipline), and synced to the FTS5 index
//! in the same transaction as the row write. The connection is opened
//! lazily on first use so constructing a store never touches the disk.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::db::{self, DbError};
use crate::retrieval::{ContextRetriever, RankedRecord, RecordQuery};
use crate::types::{
    validate_event, validate_record, Authority, ContextRecord, EventRecord, LifecycleStage,
    RecordKind, RecordScope, RecordStatus,
};
use crate::workspace::canonical_workspace_key;

#[derive(Debug, Error)]
pub enum ContextError {
    #[error("database error: {0}")]
    Db(#[from] DbError),
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("invalid record/event: {0}")]
    Validation(String),
    #[error("context store lock poisoned")]
    Poisoned,
    #[error("row decode error: {0}")]
    Decode(String),
}

/// A single connection guarded by a mutex. CodeBro is a single-writer
/// process; SQLite WAL + busy timeout makes cross-process readers safe.
pub struct ContextStore {
    db_path: PathBuf,
    conn: Mutex<Option<Connection>>,
}

impl std::fmt::Debug for ContextStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContextStore")
            .field("db_path", &self.db_path)
            .finish_non_exhaustive()
    }
}

impl ContextStore {
    /// Construct a store handle. The database is opened lazily on the first
    /// operation, so construction performs no I/O.
    pub fn new(db_path: PathBuf) -> Self {
        ContextStore {
            db_path,
            conn: Mutex::new(None),
        }
    }

    /// Default handle for the well-known `state.db` beside the CodeBro
    /// config directory. Constructors with an explicit path are used by
    /// hermetic tests and embedded deployments.
    pub fn at_state_dir(state_dir: PathBuf) -> Self {
        ContextStore::new(state_dir.join(db::STATE_DB_FILE))
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub(crate) fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> Result<T, ContextError>,
    ) -> Result<T, ContextError> {
        let mut guard = self.conn.lock().map_err(|_| ContextError::Poisoned)?;
        if guard.is_none() {
            *guard = Some(db::open_with_recovery(&self.db_path)?);
        }
        f(guard.as_ref().expect("connection just opened"))
    }

    // ── Records ─────────────────────────────────────────────────────────

    /// Insert a record, or fully replace an existing record with the same
    /// id (upsert). `created_at` survives an upsert; `updated_at` is always
    /// set to `now`. The FTS5 row is kept in sync in the same transaction.
    ///
    /// The record is normalized before validation: `workspace_root` is
    /// canonicalized (so `/repo`, `/repo/`, and symlinked aliases share one
    /// namespace) and cited evidence ids are resolved against the `events`
    /// table — inference citing fiction is refused, not stored.
    pub fn put_record(&self, record: &ContextRecord, now: u64) -> Result<(), ContextError> {
        let record = normalized_record(record)?;
        self.with_conn(|conn| {
            check_evidence(conn, &record)?;
            let tx = conn.unchecked_transaction()?;
            let created_at = match existing_created_at(&tx, &record.id)? {
                Some(existing) => existing,
                None => now,
            };
            tx.execute(
                "INSERT INTO context_records (
                    id, record_type, namespace, content, original_text, language,
                    authority, confidence, importance, scope, workspace_root, status,
                    lifecycle, supersedes, source, import_origin, evidence_json,
                    related_json, created_at, updated_at, expires_at, task_id,
                    extra_json
                 ) VALUES (
                    ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                    ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
                 )
                 ON CONFLICT(id) DO UPDATE SET
                    record_type = excluded.record_type,
                    namespace = excluded.namespace,
                    content = excluded.content,
                    original_text = excluded.original_text,
                    language = excluded.language,
                    authority = excluded.authority,
                    confidence = excluded.confidence,
                    importance = excluded.importance,
                    scope = excluded.scope,
                    workspace_root = excluded.workspace_root,
                    status = excluded.status,
                    lifecycle = excluded.lifecycle,
                    supersedes = excluded.supersedes,
                    source = excluded.source,
                    import_origin = excluded.import_origin,
                    evidence_json = excluded.evidence_json,
                    related_json = excluded.related_json,
                    created_at = context_records.created_at,
                    updated_at = excluded.updated_at,
                    expires_at = excluded.expires_at,
                    task_id = excluded.task_id,
                    extra_json = excluded.extra_json",
                params![
                    record.id,
                    record.kind.as_str(),
                    record.namespace,
                    record.content,
                    record.original_text.as_deref(),
                    record.language.as_deref(),
                    record.authority.as_str(),
                    record.confidence,
                    record.importance,
                    record.scope.as_str(),
                    record.workspace_root.as_deref(),
                    record.status.as_str(),
                    record.lifecycle.as_str(),
                    record.supersedes.as_deref(),
                    record.source.as_deref(),
                    record.import_origin.as_deref(),
                    serde_json::to_string(&record.evidence)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    serde_json::to_string(&record.related_ids)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    created_at,
                    now,
                    record.expires_at.map(|t| t as i64),
                    record.task_id.as_deref(),
                    record.extra_json.as_deref(),
                ],
            )?;
            sync_fts(&tx, &record)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Fetch a record by id (any status). Returns `None` when absent.
    pub fn get_record(&self, id: &str) -> Result<Option<ContextRecord>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT id, record_type, namespace, content, original_text, language,
                        authority, confidence, importance, scope, workspace_root, status,
                        lifecycle, supersedes, source, import_origin, evidence_json,
                        related_json, created_at, updated_at, expires_at, task_id,
                        extra_json
                 FROM context_records WHERE id = ?1",
                [id],
                row_to_record,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// Hard-remove a record and its FTS5 row. Soft transitions
    /// (supersede/reject/expire) are the preferred path for knowledge you
    /// want to stay reversible; removal is for cleanup of junk.
    pub fn remove_record(&self, id: &str) -> Result<bool, ContextError> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM context_records_fts WHERE record_id = ?1", [id])?;
            let removed = tx.execute("DELETE FROM context_records WHERE id = ?1", [id])?;
            tx.commit()?;
            Ok(removed > 0)
        })
    }

    /// Promote a record: atomically mark `old_id` superseded and insert the
    /// `replacement` (which must reference `old_id` in `supersedes`).
    /// This is how AI_INFERRED becomes USER_CONFIRMED without losing the
    /// audit trail.
    pub fn supersede_record(
        &self,
        old_id: &str,
        replacement: &ContextRecord,
        now: u64,
    ) -> Result<(), ContextError> {
        let replacement = normalized_record(replacement)?;
        if replacement.supersedes.as_deref() != Some(old_id) {
            return Err(ContextError::Validation(
                "replacement.supersedes must name the record it replaces".to_string(),
            ));
        }
        if replacement.status != RecordStatus::Active {
            return Err(ContextError::Validation(
                "a replacement record must be active".to_string(),
            ));
        }
        self.supersede_impl(old_id, &replacement, now)
    }

    /// Retire a record into a terminal state: like [`ContextStore::supersede_record`]
    /// but the replacement carries a terminal status (`Expired` for a
    /// completed intent, `Rejected` for a cancelled one) instead of staying
    /// active. The old row is still marked `Superseded` (not deleted), so
    /// the full history — attempt, completion, and what it replaced — stays
    /// queryable. The replacement itself must not be `Superseded`.
    pub fn retire_record(
        &self,
        old_id: &str,
        replacement: &ContextRecord,
        now: u64,
    ) -> Result<(), ContextError> {
        let replacement = normalized_record(replacement)?;
        if replacement.supersedes.as_deref() != Some(old_id) {
            return Err(ContextError::Validation(
                "replacement.supersedes must name the record it replaces".to_string(),
            ));
        }
        match replacement.status {
            RecordStatus::Expired | RecordStatus::Rejected => {}
            other => {
                return Err(ContextError::Validation(format!(
                    "a retiring replacement must be expired or rejected, not {other}"
                )));
            }
        }
        self.supersede_impl(old_id, &replacement, now)
    }

    /// Shared atomic core of supersede/retire: the predecessor must exist
    /// and be active (no double-supersede); both writes land in one
    /// transaction with the FTS5 row synced.
    fn supersede_impl(
        &self,
        old_id: &str,
        replacement: &ContextRecord,
        now: u64,
    ) -> Result<(), ContextError> {
        self.with_conn(|conn| {
            check_evidence(conn, replacement)?;
            let tx = conn.unchecked_transaction()?;
            let status: String = tx
                .query_row(
                    "SELECT status FROM context_records WHERE id = ?1",
                    [old_id],
                    |row| row.get(0),
                )
                .optional()?
                .ok_or_else(|| {
                    ContextError::Validation(format!("record {old_id} does not exist"))
                })?;
            if status != RecordStatus::Active.as_str() {
                return Err(ContextError::Validation(format!(
                    "only an active record can be superseded (record {old_id} is {status})"
                )));
            }
            tx.execute(
                "UPDATE context_records SET status = ?1, updated_at = ?2 WHERE id = ?3",
                params![RecordStatus::Superseded.as_str(), now, old_id],
            )?;
            insert_record_row(&tx, replacement, now)?;
            // The replacement is a new row: its FTS5 entry must be written
            // in the same transaction, or keyword recall silently loses the
            // confirmed record (the old row's FTS entry stays for
            // status-filtered audit queries).
            sync_fts(&tx, replacement)?;
            tx.commit()?;
            Ok(())
        })
    }

    /// Reject a record (status → rejected). Used by the future
    /// confirm/forget flows; keeps the row for the audit trail.
    pub fn reject_record(&self, id: &str, now: u64) -> Result<bool, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE context_records SET status = ?1, updated_at = ?2
                 WHERE id = ?3 AND status = ?4",
                params![
                    RecordStatus::Rejected.as_str(),
                    now,
                    id,
                    RecordStatus::Active.as_str()
                ],
            )?;
            Ok(updated > 0)
        })
    }

    /// Expire every active record whose `expires_at` has passed.
    /// Returns the number of records expired.
    pub fn expire_sweep(&self, now: u64) -> Result<usize, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE context_records SET status = ?1, updated_at = ?2
                 WHERE status = ?3 AND expires_at IS NOT NULL AND expires_at < ?4",
                params![
                    RecordStatus::Expired.as_str(),
                    now,
                    RecordStatus::Active.as_str(),
                    now
                ],
            )?;
            Ok(updated)
        })
    }

    /// Number of active records visible from a workspace: global records
    /// plus records scoped to that workspace. With no workspace, only
    /// global records count.
    pub fn count_visible(&self, workspace_root: Option<&str>) -> Result<usize, ContextError> {
        self.with_conn(|conn| {
            let count = match workspace_root {
                Some(ws) => conn.query_row(
                    "SELECT count(*) FROM context_records
                     WHERE status = ?1 AND (scope = ?2 OR workspace_root = ?3)",
                    params![
                        RecordStatus::Active.as_str(),
                        RecordScope::Global.as_str(),
                        ws
                    ],
                    |row| row.get::<_, i64>(0),
                )?,
                None => conn.query_row(
                    "SELECT count(*) FROM context_records
                     WHERE status = ?1 AND scope = ?2",
                    params![RecordStatus::Active.as_str(), RecordScope::Global.as_str()],
                    |row| row.get::<_, i64>(0),
                )?,
            };
            Ok(count as usize)
        })
    }

    /// Number of active records visible from a (workspace, task) viewpoint:
    /// global rows, the workspace's project rows, and — only with a
    /// matching `task_id` — that task's rows. This is the precise counting
    /// twin of the task-aware [`ContextRetriever::search`]; the older
    /// [`ContextStore::count_visible`] predates task identity and counts any
    /// workspace-matching row.
    pub fn count_visible_with_task(
        &self,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<usize, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        let task = task_id.map(|t| t.trim().to_string());
        self.with_conn(|conn| {
            let count: i64 = conn.query_row(
                "SELECT count(*) FROM context_records
                 WHERE status = ?1
                   AND (scope = ?2
                        OR (scope = ?3 AND workspace_root = ?4)
                        OR (scope = ?5 AND workspace_root = ?4 AND task_id = ?6))",
                params![
                    RecordStatus::Active.as_str(),
                    RecordScope::Global.as_str(),
                    RecordScope::Project.as_str(),
                    ws,
                    RecordScope::Task.as_str(),
                    task,
                ],
                |row| row.get(0),
            )?;
            Ok(count as usize)
        })
    }

    // ── Events ──────────────────────────────────────────────────────────

    /// Append an observation event. Returns the assigned event id (the id
    /// records cite as evidence). Payloads are size-bounded and digested.
    /// The event's workspace root is canonicalized like records, so
    /// evidence workspace matching compares stable keys.
    ///
    /// Oversized payloads are refused (P0 policy): callers that must never
    /// fail on long input (passive history capture) use
    /// [`ContextStore::record_history`], which truncates instead.
    pub fn append_event(&self, event: &EventRecord, now: u64) -> Result<i64, ContextError> {
        let mut event = event.clone();
        event.workspace_root = canonical_workspace_key(&event.workspace_root);
        event.task_id = event
            .task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        validate_event(&event).map_err(ContextError::Validation)?;
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let digest = event.payload.as_deref().map(|p| hex_sha256(p.as_bytes()));
            tx.execute(
                "INSERT INTO events (
                    session_id, workspace_root, task_id, kind, tool, path, outcome,
                    summary, payload_json, dedup_key, source, digest, created_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    event.session_id.as_deref(),
                    event.workspace_root,
                    event.task_id.as_deref(),
                    event.kind,
                    event.tool.as_deref(),
                    event.path.as_deref(),
                    event.outcome.as_deref(),
                    event.summary.as_deref(),
                    event.payload.as_deref(),
                    event.dedup_key.as_deref(),
                    event.source.as_deref(),
                    digest,
                    now
                ],
            )?;
            let id = tx.last_insert_rowid();
            sync_history_fts(
                &tx,
                id,
                event.summary.as_deref(),
                event.payload.as_deref(),
                &event.kind,
                event.tool.as_deref(),
                event.outcome.as_deref(),
            )?;
            tx.commit()?;
            Ok(id)
        })
    }

    /// Fetch an event by id.
    pub fn get_event(&self, id: i64) -> Result<Option<EventRecord>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {EVENT_COLUMNS} FROM events WHERE id = ?1"),
                [id],
                row_to_event,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// Most recent events for a workspace, newest first.
    pub fn list_events(
        &self,
        workspace_root: &str,
        limit: usize,
    ) -> Result<Vec<EventRecord>, ContextError> {
        let workspace_root = canonical_workspace_key(workspace_root);
        let limit = limit.clamp(1, 200) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM events WHERE workspace_root = ?1
                 ORDER BY created_at DESC, id DESC LIMIT ?2",
            ))?;
            let rows = stmt
                .query_map(params![workspace_root, limit], row_to_event)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }
}

// ── Internal helpers ──────────────────────────────────────────────────────

/// Normalize a record for storage: canonicalize `workspace_root` (so
/// `/repo`, `/repo/`, and symlinked aliases share one namespace) and trim
/// `task_id`, then run shape validation. Returns the storage-ready clone.
fn normalized_record(record: &ContextRecord) -> Result<ContextRecord, ContextError> {
    let mut normalized = record.clone();
    normalized.workspace_root = normalized
        .workspace_root
        .map(|ws| canonical_workspace_key(&ws));
    normalized.task_id = normalized
        .task_id
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty());
    validate_record(&normalized).map_err(ContextError::Validation)?;
    Ok(normalized)
}

/// Evidence existence check (P1 trust gate).
///
/// Shape validation only requires *non-empty* evidence for `AiInferred` /
/// `Observed` records; this resolves every cited id against the `events`
/// table: the id must parse as an event row id, the event must exist, and —
/// for workspace-scoped records — the event must belong to the same
/// canonical workspace (a project record cannot launder another project's
/// observation as its evidence). Global records cite existence only, since
/// a global preference may be grounded in work done in any workspace.
fn check_evidence(conn: &Connection, record: &ContextRecord) -> Result<(), ContextError> {
    for cited in &record.evidence {
        let event_id: i64 = cited.trim().parse().map_err(|_| {
            ContextError::Validation(format!(
                "evidence id {cited:?} is not a valid event id (expected the integer id returned when the evidence event was recorded)"
            ))
        })?;
        let event_ws: Option<String> = conn
            .query_row(
                "SELECT workspace_root FROM events WHERE id = ?1",
                [event_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?
            .ok_or_else(|| {
                ContextError::Validation(format!(
                    "evidence event {event_id} does not exist: record observations first, then cite their ids"
                ))
            })?;
        if let (Some(record_ws), Some(event_ws)) =
            (record.workspace_root.as_deref(), event_ws.as_deref())
        {
            if record_ws != event_ws {
                return Err(ContextError::Validation(format!(
                    "evidence event {event_id} belongs to workspace {event_ws:?}, not {record_ws:?}"
                )));
            }
        }
    }
    Ok(())
}

fn existing_created_at(conn: &Connection, id: &str) -> Result<Option<u64>, ContextError> {
    conn.query_row(
        "SELECT created_at FROM context_records WHERE id = ?1",
        [id],
        |row| row.get::<_, i64>(0),
    )
    .optional()
    .map(|v| v.map(|t| t as u64))
    .map_err(|e| ContextError::Decode(e.to_string()))
}

/// Shared INSERT used by `supersede_record` (fresh row, timestamps managed).
fn insert_record_row(
    conn: &Connection,
    record: &ContextRecord,
    now: u64,
) -> Result<(), ContextError> {
    conn.execute(
        "INSERT INTO context_records (
            id, record_type, namespace, content, original_text, language,
            authority, confidence, importance, scope, workspace_root, status,
            lifecycle, supersedes, source, import_origin, evidence_json,
            related_json, created_at, updated_at, expires_at, task_id,
            extra_json
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
            ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23
         )",
        params![
            record.id,
            record.kind.as_str(),
            record.namespace,
            record.content,
            record.original_text.as_deref(),
            record.language.as_deref(),
            record.authority.as_str(),
            record.confidence,
            record.importance,
            record.scope.as_str(),
            record.workspace_root.as_deref(),
            record.status.as_str(),
            record.lifecycle.as_str(),
            record.supersedes.as_deref(),
            record.source.as_deref(),
            record.import_origin.as_deref(),
            serde_json::to_string(&record.evidence)
                .map_err(|e| ContextError::Decode(e.to_string()))?,
            serde_json::to_string(&record.related_ids)
                .map_err(|e| ContextError::Decode(e.to_string()))?,
            now,
            now,
            record.expires_at.map(|t| t as i64),
            record.task_id.as_deref(),
            record.extra_json.as_deref(),
        ],
    )?;
    Ok(())
}

/// Keep the FTS5 row in lockstep with the record row (delete + reinsert in
/// the caller's transaction).
fn sync_fts(conn: &Connection, record: &ContextRecord) -> Result<(), ContextError> {
    conn.execute(
        "DELETE FROM context_records_fts WHERE record_id = ?1",
        [&record.id],
    )?;
    conn.execute(
        "INSERT INTO context_records_fts (content, namespace, original_text, record_id)
         VALUES (?1, ?2, ?3, ?4)",
        params![
            record.content,
            record.namespace,
            record.original_text.as_deref().unwrap_or(""),
            record.id
        ],
    )?;
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("{:x}", hasher.finalize())
}

/// Crate-visible digest helper for the history write path (same algorithm
/// as evidence digests: SHA-256 hex of the payload bytes).
pub(crate) fn hex_sha256_for(bytes: &[u8]) -> String {
    hex_sha256(bytes)
}

/// Canonical event column list (v3 shape). Single source for every
/// `SELECT … FROM events` so new columns cannot drift between readers.
pub(crate) const EVENT_COLUMNS: &str = "id, session_id, workspace_root, task_id, kind, tool,
    path, outcome, summary, payload_json, dedup_key, source, digest, created_at";

/// Crate-visible row mapper (the history and recall modules read events).
pub(crate) fn row_to_event_pub(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
    row_to_event(row)
}

/// Keep the derived history FTS5 row in lockstep with the event row.
/// `event_id` is the canonical row id; indexed text is the summary plus a
/// bounded payload excerpt plus kind/tool/outcome tokens. Runs inside the
/// caller's transaction — canonical and index commit atomically.
pub(crate) fn sync_history_fts(
    conn: &Connection,
    event_id: i64,
    summary: Option<&str>,
    payload: Option<&str>,
    kind: &str,
    tool: Option<&str>,
    outcome: Option<&str>,
) -> Result<(), ContextError> {
    conn.execute("DELETE FROM events_fts WHERE event_id = ?1", [event_id])
        .map_err(ContextError::Sqlite)?;
    // Payloads are already byte-bounded at write time; index only a
    // leading excerpt so giant tool output never bloats the index.
    let excerpt: String = payload.unwrap_or("").chars().take(2000).collect();
    conn.execute(
        "INSERT INTO events_fts (summary, text, kind, tool, outcome, event_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            summary.unwrap_or(""),
            excerpt,
            kind,
            tool.unwrap_or(""),
            outcome.unwrap_or(""),
            event_id
        ],
    )
    .map_err(ContextError::Sqlite)?;
    Ok(())
}

fn parse_list(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn row_to_record(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextRecord> {
    let original_text: Option<String> = row.get(4)?;
    let language: Option<String> = row.get(5)?;
    let workspace_root: Option<String> = row.get(10)?;
    let supersedes: Option<String> = row.get(13)?;
    let source: Option<String> = row.get(14)?;
    let import_origin: Option<String> = row.get(15)?;
    let expires_at: Option<i64> = row.get(20)?;
    // v2 columns (absent as NULL on rows written before the migration).
    let task_id: Option<String> = row.get(21)?;
    let extra_json: Option<String> = row.get(22)?;
    Ok(ContextRecord {
        id: row.get(0)?,
        kind: row
            .get::<_, String>(1)?
            .parse()
            .unwrap_or(RecordKind::Other),
        namespace: row.get(2)?,
        content: row.get(3)?,
        original_text,
        language,
        authority: row
            .get::<_, String>(6)?
            .parse()
            .unwrap_or(Authority::Observed),
        confidence: row.get(7)?,
        importance: row.get(8)?,
        scope: row
            .get::<_, String>(9)?
            .parse()
            .unwrap_or(RecordScope::Global),
        workspace_root,
        status: row
            .get::<_, String>(11)?
            .parse()
            .unwrap_or(RecordStatus::Active),
        lifecycle: row
            .get::<_, String>(12)?
            .parse()
            .unwrap_or(LifecycleStage::Observed),
        supersedes,
        source,
        import_origin,
        evidence: parse_list(&row.get::<_, String>(16)?),
        related_ids: parse_list(&row.get::<_, String>(17)?),
        created_at: row.get::<_, i64>(18)? as u64,
        updated_at: row.get::<_, i64>(19)? as u64,
        expires_at: expires_at.map(|t| t as u64),
        task_id,
        extra_json,
    })
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<EventRecord> {
    Ok(EventRecord {
        id: Some(row.get(0)?),
        session_id: row.get(1)?,
        workspace_root: row.get(2)?,
        task_id: row.get(3)?,
        kind: row.get(4)?,
        tool: row.get(5)?,
        path: row.get(6)?,
        outcome: row.get(7)?,
        summary: row.get(8)?,
        payload: row.get(9)?,
        dedup_key: row.get(10)?,
        source: row.get(11)?,
        digest: row.get(12)?,
        created_at: row.get::<_, i64>(13)? as u64,
    })
}

// ── Retrieval ─────────────────────────────────────────────────────────────

impl ContextRetriever for ContextStore {
    fn search(&self, query: &RecordQuery<'_>, now: u64) -> Result<Vec<RankedRecord>, ContextError> {
        let status = query
            .status
            .unwrap_or(RecordStatus::Active)
            .as_str()
            .to_string();
        let kind: Option<String> = query.kind.map(|k| k.as_str().to_string());
        // Canonicalize at the boundary so `/repo` and `/repo/` query one
        // namespace; task ids are matched exactly (trimmed).
        let ws: Option<String> = query.workspace_root.map(canonical_workspace_key);
        let task: Option<String> = query.task_id.map(|t| t.trim().to_string());
        let limit = query.limit.clamp(1, 200) as i64;

        // Visibility predicate (shared by both branches): global rows, the
        // workspace's project rows, and — only with a matching task id —
        // that task's rows. `task_id = NULL` never matches, so task rows
        // stay invisible until the caller names the task.
        const VISIBILITY: &str = "(scope = ?2 OR (scope = ?3 AND workspace_root = ?4) \
             OR (scope = ?5 AND workspace_root = ?4 AND task_id = ?6))";
        const COLUMNS: &str = "id, record_type, namespace, content, original_text, language,
                        authority, confidence, importance, scope, workspace_root, status,
                        lifecycle, supersedes, source, import_origin, evidence_json,
                        related_json, created_at, updated_at, expires_at, task_id,
                        extra_json";

        let keywords = crate::retrieval::query_tokens(&query.keywords);
        self.with_conn(move |conn| {
            let mut rows: Vec<RankedRecord> = if keywords.is_empty() {
                // Keyword-less retrieval: deterministic importance ordering.
                let sql = format!(
                    "SELECT {COLUMNS}
                     FROM context_records
                     WHERE status = ?1
                       AND {VISIBILITY}
                       AND (?7 IS NULL OR record_type = ?7)
                     ORDER BY importance DESC, updated_at DESC, id
                     LIMIT ?8"
                );
                let mut stmt = conn.prepare(&sql)?;
                let collected = stmt
                    .query_map(
                        params![
                            status,
                            RecordScope::Global.as_str(),
                            RecordScope::Project.as_str(),
                            ws,
                            RecordScope::Task.as_str(),
                            task,
                            kind,
                            limit
                        ],
                        row_to_record,
                    )?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                collected
                    .into_iter()
                    .map(|record| RankedRecord {
                        record,
                        bm25: None,
                        effective_confidence: 0.0,
                    })
                    .collect()
            } else {
                let match_expr = keywords
                    .iter()
                    .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
                    .collect::<Vec<_>>()
                    .join(" AND ");
                let sql = "SELECT cr.id, cr.record_type, cr.namespace, cr.content,
                            cr.original_text, cr.language, cr.authority, cr.confidence,
                            cr.importance, cr.scope, cr.workspace_root, cr.status,
                            cr.lifecycle, cr.supersedes, cr.source, cr.import_origin,
                            cr.evidence_json, cr.related_json, cr.created_at,
                            cr.updated_at, cr.expires_at, cr.task_id, cr.extra_json,
                            bm25(context_records_fts) AS rank
                     FROM context_records cr
                     JOIN context_records_fts ON context_records_fts.record_id = cr.id
                     WHERE cr.status = ?1
                       AND (cr.scope = ?2 OR (cr.scope = ?3 AND cr.workspace_root = ?4)
                            OR (cr.scope = ?5 AND cr.workspace_root = ?4 AND cr.task_id = ?6))
                       AND (?7 IS NULL OR cr.record_type = ?7)
                       AND context_records_fts MATCH ?8
                     ORDER BY rank, cr.id
                     LIMIT ?9";
                let mut stmt = conn.prepare(sql)?;
                let collected = stmt
                    .query_map(
                        params![
                            status,
                            RecordScope::Global.as_str(),
                            RecordScope::Project.as_str(),
                            ws,
                            RecordScope::Task.as_str(),
                            task,
                            kind,
                            match_expr,
                            limit
                        ],
                        |row| {
                            let record = row_to_record(row)?;
                            let rank: f64 = row.get(23)?;
                            Ok((record, rank))
                        },
                    )?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                collected
                    .into_iter()
                    .map(|(record, bm25)| RankedRecord {
                        record,
                        bm25: Some(bm25),
                        effective_confidence: 0.0,
                    })
                    .collect()
            };
            for ranked in rows.iter_mut() {
                ranked.effective_confidence =
                    crate::retrieval::decayed_confidence(&ranked.record, now);
            }
            Ok(rows)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::retrieval::RecordQuery;

    fn store(dir: &tempfile::TempDir) -> ContextStore {
        ContextStore::new(dir.path().join(db::STATE_DB_FILE))
    }

    fn user_record(id: &str, ns: &str, content: &str) -> ContextRecord {
        ContextRecord::new(
            id,
            RecordKind::Preference,
            ns,
            content,
            Authority::UserConfirmed,
        )
    }

    fn inferred_record(id: &str, ns: &str, content: &str, evidence: &[i64]) -> ContextRecord {
        let mut r = ContextRecord::new(
            id,
            RecordKind::Preference,
            ns,
            content,
            Authority::AiInferred,
        );
        r.evidence = evidence.iter().map(|e| format!("{e}")).collect();
        r
    }

    fn ws_record(id: &str, ws: &str, ns: &str, content: &str) -> ContextRecord {
        let mut r = user_record(id, ns, content);
        r.scope = RecordScope::Project;
        r.workspace_root = Some(ws.to_string());
        r
    }

    #[test]
    fn put_and_get_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut r = user_record(
            "ctx::1",
            "fp.communication.verbosity",
            "Prefers concise replies",
        );
        r.original_text = Some("jangan panjang sangat".to_string());
        r.language = Some("manglish".to_string());
        r.confidence = 0.9;
        r.lifecycle = LifecycleStage::Confirmed;
        store.put_record(&r, 1000).unwrap();

        let got = store.get_record("ctx::1").unwrap().unwrap();
        assert_eq!(got.content, r.content);
        assert_eq!(got.original_text.as_deref(), Some("jangan panjang sangat"));
        assert_eq!(got.language.as_deref(), Some("manglish"));
        assert_eq!(got.authority, Authority::UserConfirmed);
        assert_eq!(got.lifecycle, LifecycleStage::Confirmed);
        assert_eq!(got.created_at, 1000);
        assert_eq!(got.updated_at, 1000);
        assert!(got.expires_at.is_none());
    }

    #[test]
    fn upsert_preserves_created_at_and_bumps_updated_at() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut r = user_record("ctx::1", "fp.x", "first");
        store.put_record(&r, 1000).unwrap();
        r.content = "revised".to_string();
        store.put_record(&r, 2000).unwrap();

        let got = store.get_record("ctx::1").unwrap().unwrap();
        assert_eq!(got.created_at, 1000, "created_at survives an upsert");
        assert_eq!(got.updated_at, 2000);
        assert_eq!(got.content, "revised");
    }

    #[test]
    fn invalid_records_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        // Inference without evidence must be refused at the store.
        let inferred = inferred_record("ctx::i", "fp.x", "maybe", &[]);
        let err = store.put_record(&inferred, 1).unwrap_err();
        assert!(matches!(err, ContextError::Validation(_)));
        assert!(store.get_record("ctx::i").unwrap().is_none());

        // Empty content refused.
        let empty = user_record("ctx::e", "fp.x", "   ");
        assert!(matches!(
            store.put_record(&empty, 1),
            Err(ContextError::Validation(_))
        ));
    }

    #[test]
    fn supersede_keeps_audit_trail() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let event = store
            .append_event(
                &EventRecord {
                    id: None,
                    session_id: None,
                    workspace_root: "/work".to_string(),
                    task_id: None,
                    kind: "observed".to_string(),
                    tool: None,
                    path: None,
                    outcome: None,
                    summary: None,
                    payload: Some(r#"{"utterance":"jangan overengineer benda ni"}"#.to_string()),
                    dedup_key: None,
                    source: None,
                    digest: None,
                    created_at: 0,
                },
                10,
            )
            .unwrap();

        let weak = inferred_record(
            "ctx::weak",
            "fp.engineering.simplicity",
            "maybe simple",
            &[event],
        );
        store.put_record(&weak, 10).unwrap();

        let mut strong = user_record(
            "ctx::strong",
            "fp.engineering.simplicity",
            "Prefer the simplest reasonable implementation; avoid unnecessary abstraction",
        );
        strong.supersedes = Some("ctx::weak".to_string());
        strong.lifecycle = LifecycleStage::Confirmed;
        store.supersede_record("ctx::weak", &strong, 20).unwrap();

        let old = store.get_record("ctx::weak").unwrap().unwrap();
        assert_eq!(old.status, RecordStatus::Superseded);
        let new = store.get_record("ctx::strong").unwrap().unwrap();
        assert_eq!(new.status, RecordStatus::Active);
        assert_eq!(new.supersedes.as_deref(), Some("ctx::weak"));

        // The superseded record no longer surfaces as active.
        let active = store
            .search(
                &RecordQuery {
                    workspace_root: None,
                    task_id: None,
                    kind: None,
                    status: None,
                    keywords: Vec::new(),
                    limit: 10,
                },
                20,
            )
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].record.id, "ctx::strong");
    }

    #[test]
    fn supersede_requires_active_target_and_matching_link() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let r = user_record("ctx::a", "fp.x", "original");
        store.put_record(&r, 1).unwrap();

        let mut replacement = user_record("ctx::b", "fp.x", "replacement");
        replacement.supersedes = Some("ctx::missing".to_string());
        let err = store
            .supersede_record("ctx::a", &replacement, 2)
            .unwrap_err();
        assert!(err.to_string().contains("must name"));

        replacement.supersedes = Some("ctx::a".to_string());
        store.supersede_record("ctx::a", &replacement, 2).unwrap();
        // Second supersede of an already-superseded record is refused.
        let mut third = user_record("ctx::c", "fp.x", "third");
        third.supersedes = Some("ctx::a".to_string());
        let err = store.supersede_record("ctx::a", &third, 3).unwrap_err();
        assert!(err.to_string().contains("only an active record"));
    }

    #[test]
    fn expire_sweep_marks_stale_records() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut r = user_record("ctx::a", "fp.x", "transient");
        r.expires_at = Some(100);
        store.put_record(&r, 1).unwrap();
        let keep = user_record("ctx::b", "fp.x", "durable");
        store.put_record(&keep, 1).unwrap();

        assert_eq!(store.expire_sweep(50).unwrap(), 0);
        assert_eq!(store.expire_sweep(200).unwrap(), 1);
        let got = store.get_record("ctx::a").unwrap().unwrap();
        assert_eq!(got.status, RecordStatus::Expired);

        // Expired records are not returned by default (active-only) search.
        let visible = store.count_visible(None).unwrap();
        assert_eq!(visible, 1);
    }

    #[test]
    fn reject_marks_record_and_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .put_record(&user_record("ctx::a", "fp.x", "nope"), 1)
            .unwrap();
        assert!(store.reject_record("ctx::a", 2).unwrap());
        assert!(!store.reject_record("ctx::a", 3).unwrap());
        assert_eq!(
            store.get_record("ctx::a").unwrap().unwrap().status,
            RecordStatus::Rejected
        );
    }

    #[test]
    fn keyword_search_finds_indexed_records() {
        // Baseline: a stored record must be retrievable by keyword. Without
        // this, the FTS5 happy path itself is unproven.
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .put_record(&user_record("ctx::a", "fp.x", "unique token quetzal"), 1)
            .unwrap();
        let hits = store
            .search(
                &RecordQuery {
                    workspace_root: None,
                    task_id: None,
                    kind: None,
                    status: None,
                    keywords: vec!["quetzal".to_string()],
                    limit: 10,
                },
                2,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.id, "ctx::a");
    }

    #[test]
    fn superseded_replacement_remains_keyword_searchable() {
        // P1's confirm flow promotes AiInferred → UserConfirmed via
        // supersede. The replacement must be FTS-indexed, or keyword recall
        // silently loses exactly the knowledge that was just confirmed.
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let event = store
            .append_event(
                &EventRecord {
                    id: None,
                    session_id: None,
                    workspace_root: "/work".to_string(),
                    task_id: None,
                    kind: "observed".to_string(),
                    tool: None,
                    path: None,
                    outcome: None,
                    summary: None,
                    payload: None,
                    dedup_key: None,
                    source: None,
                    digest: None,
                    created_at: 0,
                },
                10,
            )
            .unwrap();
        let weak = inferred_record(
            "ctx::weak",
            "fp.engineering.simplicity",
            "maybe prefers simple code",
            &[event],
        );
        store.put_record(&weak, 10).unwrap();

        let mut strong = user_record(
            "ctx::strong",
            "fp.engineering.simplicity",
            "Prefers simplest reasonable quetzal implementation",
        );
        strong.supersedes = Some("ctx::weak".to_string());
        strong.lifecycle = LifecycleStage::Confirmed;
        store.supersede_record("ctx::weak", &strong, 20).unwrap();

        let hits = store
            .search(
                &RecordQuery {
                    workspace_root: None,
                    task_id: None,
                    kind: None,
                    status: None,
                    keywords: vec!["quetzal".to_string()],
                    limit: 10,
                },
                20,
            )
            .unwrap();
        assert_eq!(
            hits.len(),
            1,
            "confirmed replacement must be keyword-searchable"
        );
        assert_eq!(hits[0].record.id, "ctx::strong");
    }

    #[test]
    fn remove_deletes_row_and_fts_entry() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .put_record(&user_record("ctx::a", "fp.x", "unique token zebra"), 1)
            .unwrap();
        assert!(store.remove_record("ctx::a").unwrap());
        assert!(!store.remove_record("ctx::a").unwrap());
        let hits = store
            .search(
                &RecordQuery {
                    workspace_root: None,
                    task_id: None,
                    kind: None,
                    status: None,
                    keywords: vec!["zebra".to_string()],
                    limit: 10,
                },
                2,
            )
            .unwrap();
        assert!(hits.is_empty(), "removed record must not be searchable");
    }

    #[test]
    fn events_append_list_and_digest() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let payload = r#"{"command":"cargo test","verified":false}"#.to_string();
        let id = store
            .append_event(
                &EventRecord {
                    id: None,
                    session_id: Some("ses::1".to_string()),
                    workspace_root: "/work".to_string(),
                    task_id: None,
                    kind: "verification".to_string(),
                    tool: Some("sandbox_test".to_string()),
                    path: None,
                    outcome: Some("test_failure".to_string()),
                    summary: None,
                    payload: Some(payload.clone()),
                    dedup_key: None,
                    source: None,
                    digest: None,
                    created_at: 0,
                },
                100,
            )
            .unwrap();
        let got = store.get_event(id).unwrap().unwrap();
        assert_eq!(got.kind, "verification");
        assert_eq!(got.payload.as_deref(), Some(payload.as_str()));
        // Digest must be the sha256 of the payload bytes.
        let mut hasher = Sha256::new();
        hasher.update(payload.as_bytes());
        assert_eq!(
            got.digest.as_deref(),
            Some(format!("{:x}", hasher.finalize()).as_str())
        );

        let list = store.list_events("/work", 10).unwrap();
        assert_eq!(list.len(), 1);
        // Other workspaces never see this event.
        assert!(store.list_events("/elsewhere", 10).unwrap().is_empty());
    }

    #[test]
    fn oversized_event_payload_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let big = EventRecord {
            id: None,
            session_id: None,
            workspace_root: "/work".to_string(),
            task_id: None,
            kind: "verification".to_string(),
            tool: None,
            path: None,
            outcome: None,
            summary: None,
            payload: Some("x".repeat(crate::types::MAX_EVENT_PAYLOAD_BYTES + 1)),
            dedup_key: None,
            source: None,
            digest: None,
            created_at: 0,
        };
        assert!(matches!(
            store.append_event(&big, 1),
            Err(ContextError::Validation(_))
        ));
    }

    #[test]
    fn persistence_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(db::STATE_DB_FILE);
        {
            let store = ContextStore::new(path.clone());
            store
                .put_record(&user_record("ctx::a", "fp.x", "persistent fact"), 1)
                .unwrap();
            store
                .append_event(
                    &EventRecord {
                        id: None,
                        session_id: None,
                        workspace_root: "/work".to_string(),
                        task_id: None,
                        kind: "tool_called".to_string(),
                        tool: None,
                        path: None,
                        outcome: None,
                        summary: None,
                        payload: None,
                        dedup_key: None,
                        source: None,
                        digest: None,
                        created_at: 0,
                    },
                    1,
                )
                .unwrap();
        }
        // A fresh handle over the same file is a restart: state must survive.
        let store = ContextStore::new(path);
        assert!(store.get_record("ctx::a").unwrap().is_some());
        assert_eq!(store.list_events("/work", 10).unwrap().len(), 1);
    }

    #[test]
    fn concurrent_writers_are_serialized_and_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(store(&dir));
        let mut handles = Vec::new();
        for t in 0..8 {
            let store = std::sync::Arc::clone(&store);
            handles.push(std::thread::spawn(move || {
                for i in 0..25 {
                    let id = format!("ctx::t{t}i{i}");
                    store
                        .put_record(&user_record(&id, "fp.concurrency", "payload"), 1)
                        .unwrap();
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(store.count_visible(None).unwrap(), 8 * 25);
    }

    #[test]
    fn corrupt_underlying_file_recovers_transparently() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(db::STATE_DB_FILE);
        // Corrupt the file behind the store's back, then use the store: it
        // must quarantine and recreate rather than error or lose the handle.
        std::fs::write(&path, b"not sqlite").unwrap();
        let store = ContextStore::new(path.clone());
        store
            .put_record(&user_record("ctx::a", "fp.x", "after recovery"), 1)
            .unwrap();
        assert!(store.get_record("ctx::a").unwrap().is_some());
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(entries.iter().any(|n| n.contains("corrupt-")));
    }

    // ── P1 gates ──────────────────────────────────────────────────────

    fn observed_record(id: &str, evidence: &[i64]) -> ContextRecord {
        let mut r = ContextRecord::new(
            id,
            RecordKind::Preference,
            "fp.p1.probe",
            "observed probe content",
            Authority::Observed,
        );
        r.evidence = evidence.iter().map(|e| e.to_string()).collect();
        r
    }

    fn task_record(id: &str, ws: &str, task: &str) -> ContextRecord {
        let mut r = user_record(id, "fp.p1.tasked", "task override content");
        r.scope = RecordScope::Task;
        r.workspace_root = Some(ws.to_string());
        r.task_id = Some(task.to_string());
        r
    }

    fn mint_event(store: &ContextStore, ws: &str) -> i64 {
        store
            .append_event(
                &EventRecord {
                    id: None,
                    session_id: None,
                    workspace_root: ws.to_string(),
                    task_id: None,
                    kind: "agent_observation".to_string(),
                    tool: None,
                    path: None,
                    outcome: None,
                    summary: None,
                    payload: None,
                    dedup_key: None,
                    source: None,
                    digest: None,
                    created_at: 0,
                },
                5,
            )
            .unwrap()
    }

    #[test]
    fn evidence_citing_nonexistent_event_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        // No events exist: id 999 is fiction and must be refused.
        let err = store
            .put_record(&observed_record("ctx::ghost", &[999]), 10)
            .unwrap_err();
        assert!(err.to_string().contains("does not exist"), "got: {err}");
        assert!(store.get_record("ctx::ghost").unwrap().is_none());
    }

    #[test]
    fn evidence_citing_malformed_id_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut rec = observed_record("ctx::bad", &[]);
        rec.evidence = vec!["ev:fiction".to_string()];
        let err = store.put_record(&rec, 10).unwrap_err();
        assert!(
            err.to_string().contains("not a valid event id"),
            "got: {err}"
        );
    }

    #[test]
    fn evidence_from_another_workspace_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let foreign = mint_event(&store, "/elsewhere");
        // A project record for /work cannot launder /elsewhere's observation.
        let mut rec = observed_record("ctx::xws", &[foreign]);
        rec.scope = RecordScope::Project;
        rec.workspace_root = Some("/work".to_string());
        let err = store.put_record(&rec, 10).unwrap_err();
        assert!(
            err.to_string().contains("belongs to workspace"),
            "got: {err}"
        );
        // ... but the same event backs a global record (existence only).
        let global = observed_record("ctx::glob", &[foreign]);
        assert!(store.put_record(&global, 10).is_ok());
        // ... and a same-workspace project record.
        let mut local = observed_record("ctx::local", &[foreign]);
        local.scope = RecordScope::Project;
        local.workspace_root = Some("/elsewhere".to_string());
        assert!(store.put_record(&local, 10).is_ok());
    }

    #[test]
    fn supersede_with_bad_evidence_is_refused_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .put_record(&user_record("ctx::a", "fp.x", "original"), 1)
            .unwrap();
        let mut replacement = user_record("ctx::b", "fp.x", "replacement");
        replacement.supersedes = Some("ctx::a".to_string());
        // Forge an inferred replacement citing a nonexistent event: the
        // whole supersede must fail and the original stays active.
        let mut forged = inferred_record("ctx::b", "fp.x", "forged", &[4242]);
        forged.supersedes = Some("ctx::a".to_string());
        let err = store.supersede_record("ctx::a", &forged, 2).unwrap_err();
        assert!(err.to_string().contains("does not exist"), "got: {err}");
        assert_eq!(
            store.get_record("ctx::a").unwrap().unwrap().status,
            RecordStatus::Active
        );
        let _ = replacement;
    }

    #[test]
    fn workspace_roots_canonicalize_on_write_and_query() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let ws = dir.path().join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let canonical = ws.canonicalize().unwrap().display().to_string();

        // Written with a trailing slash + dot segment...
        let mut rec = user_record("ctx::canon", "fp.x", "canonical probe");
        rec.scope = RecordScope::Project;
        rec.workspace_root = Some(format!("{}/./", canonical));
        store.put_record(&rec, 1).unwrap();

        // ... stored canonical ...
        let got = store.get_record("ctx::canon").unwrap().unwrap();
        assert_eq!(got.workspace_root.as_deref(), Some(canonical.as_str()));

        // ... and found through any equivalent spelling, keyword or not.
        for spelling in [
            canonical.clone(),
            format!("{canonical}/"),
            format!("{canonical}/sub/.."),
        ] {
            let hits = store
                .search(
                    &RecordQuery {
                        workspace_root: Some(spelling.as_str()),
                        task_id: None,
                        kind: None,
                        status: None,
                        keywords: Vec::new(),
                        limit: 10,
                    },
                    2,
                )
                .unwrap();
            assert_eq!(hits.len(), 1, "spelling {spelling:?} must resolve");
            let kw = store
                .search(
                    &RecordQuery {
                        workspace_root: Some(spelling.as_str()),
                        task_id: None,
                        kind: None,
                        status: None,
                        keywords: vec!["canonical".to_string()],
                        limit: 10,
                    },
                    2,
                )
                .unwrap();
            assert_eq!(kw.len(), 1, "keyword spelling {spelling:?} must resolve");
        }
    }

    #[test]
    fn task_rows_require_task_identity_at_read_time() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .put_record(&task_record("ctx::t", "/work", "task-1"), 1)
            .unwrap();
        store
            .put_record(&user_record("ctx::g", "fp.p1.tasked", "global fallback"), 1)
            .unwrap();

        // No task named: the task row is invisible (both query paths).
        for keywords in [Vec::new(), vec!["task".to_string(), "override".to_string()]] {
            let hits = store
                .search(
                    &RecordQuery {
                        workspace_root: Some("/work"),
                        task_id: None,
                        kind: None,
                        status: None,
                        keywords,
                        limit: 10,
                    },
                    2,
                )
                .unwrap();
            let ids: Vec<&str> = hits.iter().map(|h| h.record.id.as_str()).collect();
            assert!(!ids.contains(&"ctx::t"), "task row must hide: {ids:?}");
        }
        // Matching task: visible.
        let hits = store
            .search(
                &RecordQuery {
                    workspace_root: Some("/work"),
                    task_id: Some("task-1"),
                    kind: None,
                    status: None,
                    keywords: Vec::new(),
                    limit: 10,
                },
                2,
            )
            .unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.record.id.as_str()).collect();
        assert!(ids.contains(&"ctx::t"), "own task row visible: {ids:?}");
        // Sibling task: invisible.
        let hits = store
            .search(
                &RecordQuery {
                    workspace_root: Some("/work"),
                    task_id: Some("task-2"),
                    kind: None,
                    status: None,
                    keywords: Vec::new(),
                    limit: 10,
                },
                2,
            )
            .unwrap();
        let ids: Vec<&str> = hits.iter().map(|h| h.record.id.as_str()).collect();
        assert!(!ids.contains(&"ctx::t"), "sibling task row hidden: {ids:?}");
    }

    #[test]
    fn task_id_and_extra_json_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        let mut rec = task_record("ctx::full", "/work", "task-9");
        rec.kind = RecordKind::Intent;
        rec.namespace = "intent.probe".to_string();
        rec.content = "Ship the probe".to_string();
        rec.extra_json =
            Some(r#"{"rationale":"why","priority":"high","intent_status":"active"}"#.to_string());
        store.put_record(&rec, 7).unwrap();
        let got = store.get_record("ctx::full").unwrap().unwrap();
        assert_eq!(got.task_id.as_deref(), Some("task-9"));
        assert!(got.extra_json.as_deref().unwrap().contains("high"));
        // Search decodes them too.
        let hits = store
            .search(
                &RecordQuery {
                    workspace_root: Some("/work"),
                    task_id: Some("task-9"),
                    kind: Some(RecordKind::Intent),
                    status: None,
                    keywords: Vec::new(),
                    limit: 10,
                },
                8,
            )
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].record.task_id.as_deref(), Some("task-9"));
    }

    #[test]
    fn precise_count_sees_task_rows_only_with_task() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(&dir);
        store
            .put_record(&task_record("ctx::t", "/work", "task-1"), 1)
            .unwrap();
        let mut p = user_record("ctx::p", "fp.p1.c", "project row");
        p.scope = RecordScope::Project;
        p.workspace_root = Some("/work".to_string());
        store.put_record(&p, 1).unwrap();
        store
            .put_record(&user_record("ctx::g", "fp.p1.c", "global row"), 1)
            .unwrap();

        assert_eq!(
            store.count_visible_with_task(Some("/work"), None).unwrap(),
            2
        );
        assert_eq!(
            store
                .count_visible_with_task(Some("/work"), Some("task-1"))
                .unwrap(),
            3
        );
        assert_eq!(
            store
                .count_visible_with_task(Some("/work"), Some("task-2"))
                .unwrap(),
            2
        );
        assert_eq!(store.count_visible_with_task(None, None).unwrap(), 1);
    }
}

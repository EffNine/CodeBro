//! SQLite bootstrap for the context store: schema, migrations, quarantine.
//!
//! Versioning mirrors the JSON-store discipline: a `PRAGMA user_version`
//! gate with sequential migrations, refusal of unknown (newer) schemas, and
//! quarantine-on-corruption instead of silent data loss.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::Connection;
use thiserror::Error;

/// File name of the context database inside the state directory.
pub const STATE_DB_FILE: &str = "state.db";
/// Current schema version. Bump on every additive schema change and append
/// a matching migration step in [`migrate`]. Steps are sequential: a v1
/// database applies only the v2+v3+v4 steps, never a re-run of the whole batch.
pub const SCHEMA_VERSION: i64 = 7;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "state.db schema version {found} is newer than supported {supported}; refusing to open"
    )]
    NewerSchema { found: i64, supported: i64 },
    #[error("state.db failed integrity/migration checks ({0}); quarantined and recreated")]
    Corrupt(String),
    #[error("database is unusable: {0}")]
    Unusable(String),
}

fn quarantine_path(db_path: &Path) -> PathBuf {
    // Mirror core's quarantine naming: `<name>.corrupt-<unix-ts>`, plus the
    // process id so two processes quarantining the same instant keep
    // distinct, forensic-complete debris instead of colliding on one name.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut name = db_path.as_os_str().to_owned();
    name.push(format!(".corrupt-{ts}-{}", std::process::id()));
    PathBuf::from(name)
}

/// Rename `path` (and its WAL sidecars, if any) out of the way. Never
/// deletes: a quarantined database stays recoverable for forensics.
fn quarantine(db_path: &Path) -> std::io::Result<()> {
    for sidecar in [db_path.to_path_buf(), wal_path(db_path), shm_path(db_path)] {
        if sidecar.exists() {
            std::fs::rename(&sidecar, quarantine_path(&sidecar))?;
        }
    }
    Ok(())
}

/// SQLite keeps `<name>-wal`/`<name>-shm` next to the database. A stale
/// WAL/SHM paired with a fresh database is a corruption hazard, so
/// quarantine must move all three together.
fn wal_path(db_path: &Path) -> PathBuf {
    let mut os = db_path.as_os_str().to_owned();
    os.push("-wal");
    PathBuf::from(os)
}

fn shm_path(db_path: &Path) -> PathBuf {
    let mut os = db_path.as_os_str().to_owned();
    os.push("-shm");
    PathBuf::from(os)
}

/// Open (creating if needed) and fully prepare a connection: WAL mode,
/// busy timeout, foreign keys, schema migration, integrity check.
pub(crate) fn open_checked(db_path: &Path) -> Result<Connection, DbError> {
    let parent = db_path
        .parent()
        .ok_or_else(|| DbError::Unusable("state.db path has no parent directory".to_string()))?;
    std::fs::create_dir_all(parent).map_err(|e| DbError::Io {
        path: parent.to_path_buf(),
        source: e,
    })?;

    let conn = Connection::open(db_path)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;

    let version = user_version(&conn)?;
    if version > SCHEMA_VERSION {
        return Err(DbError::NewerSchema {
            found: version,
            supported: SCHEMA_VERSION,
        });
    }
    if version < SCHEMA_VERSION {
        migrate(&conn)?;
    }
    verify_integrity(&conn)?;
    Ok(conn)
}

/// Whether an open failure is evidence that the database file itself is
/// corrupt (quarantine + recreate) rather than an environmental or
/// contention problem (propagate untouched).
///
/// Quarantine renames user data aside: it must only happen when the file
/// content is to blame. Lock contention (`DatabaseBusy`/`DatabaseLocked`),
/// permission/IO failures, or a full disk must never trigger a quarantine
/// of a healthy database — the caller degrades or retries instead.
pub(crate) fn should_quarantine(err: &DbError) -> bool {
    match err {
        // Integrity check failed on our own schema: the content is bad.
        DbError::Corrupt(_) => true,
        DbError::Sqlite(sqlite_err) => matches!(
            sqlite_err,
            rusqlite::Error::SqliteFailure(e, _)
                if matches!(
                    e.code,
                    rusqlite::ErrorCode::DatabaseCorrupt | rusqlite::ErrorCode::NotADatabase
                )
        ),
        // IO, unusable-path, and all other SQLite failures (busy, locked,
        // readonly, cannot-open, disk-full, misuse, ...): the file may be
        // perfectly healthy, so leave it alone.
        _ => false,
    }
}

/// Open a healthy database, quarantining and rebuilding when the file is
/// corrupt. A *newer* schema is never quarantined — that data is intact and
/// belongs to a newer CodeBro; it is refused with a clear error instead.
/// Non-corruption, non-transient failures (IO, permissions) propagate
/// without touching the file.
///
/// Transient failures are retried first (bounded): lock contention from
/// sibling processes (including FTS5 validation lines surfacing as busy)
/// and spurious check failures clear once writers settle, while
/// quarantining on a transient would destroy a healthy database — the
/// worst possible outcome. A genuinely corrupt file fails every attempt
/// and is still quarantined.
pub(crate) fn open_with_recovery(db_path: &Path) -> Result<Connection, DbError> {
    const OPEN_ATTEMPTS: usize = 6;
    const RETRY_DELAY_MS: u64 = 100;
    let mut first: Option<DbError> = None;
    for attempt in 0..OPEN_ATTEMPTS {
        match open_checked(db_path) {
            Ok(conn) => return Ok(conn),
            Err(DbError::NewerSchema { .. }) => {
                return Err(DbError::NewerSchema {
                    found: user_version_from_raw(db_path),
                    supported: SCHEMA_VERSION,
                })
            }
            Err(e) => {
                if !is_transient(&e) || attempt + 1 >= OPEN_ATTEMPTS {
                    first = Some(e);
                    break;
                }
                first = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS));
            }
        }
    }
    let first = first.expect("retry loop always records the last error");
    if should_quarantine(&first) {
        // Quiescence gate: quarantine destroys the live database, so it
        // must only fire on a file nobody else is actively changing. A
        // check that keeps failing while the file keeps changing is
        // contention (a sibling mid-write/recovery/quarantine), not proof
        // of corruption — withhold quarantine and report the error so the
        // caller degrades or retries against the settled file instead.
        // A vanished file means a sibling already quarantined: same answer.
        let before = file_fingerprint(db_path);
        std::thread::sleep(std::time::Duration::from_millis(RETRY_DELAY_MS * 2));
        if file_fingerprint(db_path) != before {
            return Err(first);
        }
    }
    if !should_quarantine(&first) {
        // No evidence the file content is corrupt (contention, IO,
        // permissions, ...): propagate and leave the file untouched.
        // Callers (e.g. the `context` MCP tool) degrade gracefully.
        return Err(first);
    }
    // Corrupt or half-written database: quarantine the evidence,
    // then build a fresh one. If the rebuild also fails, surface
    // the original error — never silently continue on a broken DB.
    if db_path.exists() {
        tracing::warn!("context state.db failed checks ({first}); quarantining and recreating");
        if let Err(qe) = quarantine(db_path) {
            tracing::error!("quarantine of corrupt state.db failed: {qe}");
            return Err(DbError::Unusable(format!(
                "quarantine failed after {first}: {qe}"
            )));
        }
    }
    open_checked(db_path).map_err(|second| {
        DbError::Unusable(format!(
            "recreate failed after original error ({first}): {second}"
        ))
    })
}

/// Whether an open failure is worth retrying (bounded): quarantinable
/// verdicts (a transient can mimic corruption) and lock contention
/// (a sibling writer holds the database). Anything else (IO,
/// permissions, newer schema — handled separately) is returned at once.
fn is_transient(err: &DbError) -> bool {
    if should_quarantine(err) {
        return true;
    }
    matches!(
        err,
        DbError::Sqlite(rusqlite::Error::SqliteFailure(e, _))
            if matches!(
                e.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            )
    )
}

/// Cheap identity snapshot of the database file for the quarantine
/// quiescence gate: (modification time, length). `None` when the file
/// cannot be stated (absent or unreadable) — a sibling may have just
/// quarantined it, which counts as change.
fn file_fingerprint(db_path: &Path) -> Option<(std::time::SystemTime, u64)> {
    std::fs::metadata(db_path)
        .and_then(|m| m.modified().map(|t| (t, m.len())))
        .ok()
}

fn user_version(conn: &Connection) -> Result<i64, DbError> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Read `user_version` from a file without full recovery semantics — used
/// only to report the found version after a refusal.
fn user_version_from_raw(db_path: &Path) -> i64 {
    Connection::open(db_path)
        .and_then(|c| c.query_row("PRAGMA user_version", [], |row| row.get(0)))
        .unwrap_or(-1)
}

fn verify_integrity(conn: &Connection) -> Result<(), DbError> {
    // Collect a few lines (not just the first row): on failure the first
    // offending line is the forensic evidence, and logging it costs
    // nothing on the healthy path.
    let mut stmt = conn.prepare("PRAGMA quick_check")?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(DbError::Sqlite)?;
    match rows.first().map(|s| s.trim()) {
        Some(first) if first.eq_ignore_ascii_case("ok") => Ok(()),
        _ => {
            // FTS5 index validation reports lock contention as check
            // OUTPUT ("unable to validate the inverted index for FTS5
            // table …: database is locked"), not as a query error — and
            // the busy timeout does not cover it. A lock line means the
            // check was inconclusive (siblings writing), never that the
            // content is corrupt: report busy so the caller degrades or
            // retries, and quarantine stays out of it entirely.
            if rows.iter().any(|line| {
                let lower = line.to_ascii_lowercase();
                lower.contains("database is locked")
                    || lower.contains("database table is locked")
                    || lower.contains("database is busy")
            }) {
                return Err(lock_contention_error());
            }
            let preview: Vec<&str> = rows.iter().take(3).map(|s| s.as_str()).collect();
            tracing::warn!(
                "state.db integrity check failed ({} line(s), first: {preview:?}); \
                 refusing to trust this database file",
                rows.len()
            );
            Err(DbError::Corrupt(format!(
                "quick_check reported {} problem(s), first: {}",
                rows.len(),
                preview.first().copied().unwrap_or("(no output)")
            )))
        }
    }
}

/// A busy error for check paths that surface contention as output rows
/// rather than as a failed query: same contract as a genuine
/// `SQLITE_BUSY` (retryable, never quarantinable).
fn lock_contention_error() -> DbError {
    DbError::Sqlite(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error {
            code: rusqlite::ErrorCode::DatabaseBusy,
            extended_code: 0,
        },
        "integrity check could not acquire locks (concurrent writers active)"
            .to_string()
            .into(),
    ))
}

/// Schema v1: durable context records + FTS5 index, observation events,
/// session clustering (reserved for the session-history phase).
///
/// Schema v2 (P1: fingerprint + intent): `task_id` binds task-scoped
/// records to their task (the task > project > global hierarchy needs an
/// identity, not just a scope label); `extra_json` carries extensible
/// semantic metadata (intent rationale / priority / status) without rigid
/// per-preference columns. Both additive and nullable: v1 rows read back
/// with `None`, v1 databases upgrade in place.
///
/// Schema v3 (P2: sessions + history + recall): `sessions` gains `task_id`
/// (task-bound sessions), `status` (active/completed/abandoned lifecycle),
/// `source`, `parent_session_id`, and `updated_at` (heartbeat for stale
/// detection); `events` gains `task_id`, `summary` (FTS-indexed human
/// line), `dedup_key` (idempotency), and `source`; a new derived
/// `events_fts` FTS5 index makes history searchable without touching the
/// canonical tables. All additive: v1/v2 rows upgrade in place with
/// `status='active'` and `updated_at=opened_at` backfills.
///
/// Schema v4 (P3: learning + inference): a new `learning_candidates` table
/// holds evaluated learning candidates (proposition, supporting and
/// contradicting evidence event ids, confidence, lifecycle status,
/// evaluation reason, persisted-inference backlink). Accepted inferences
/// themselves reuse the existing `context_records` table with
/// `authority = ai_inferred` — there is deliberately no second knowledge
/// store. The table starts empty on every upgrade: historical evidence
/// (events) is preserved, hypotheses are re-derived.
///
/// Schema v6 (P5: durable task runtime): two new tables. `tasks` holds
/// the engineering task records (opaque `task::<hex>` ids, canonical
/// workspace root, lifecycle status, optimistic version, worker lease
/// with fencing version, latest-checkpoint pointer, outcome).`
/// task_checkpoints` holds immutable resume snapshots (unique
/// `(task_id, version)`; the task's mutable pointer is the only way
/// "current" moves). All new tables with `IF NOT EXISTS` — crash-safe
/// resume, no backfill: tasks are created going forward.
///
/// Schema v7 (P6: engineering index metadata): one new table.
/// `repo_indexes` holds *derived* per-workspace index bookkeeping only:
/// canonical workspace root, repository identity JSON, lifecycle status,
/// generation timestamps, revision, and file/symbol/edge counts. It
/// never stores file contents, symbols, or graph edges — those live in
/// the per-project `.codebro/facts.json` (the canonical derived index).
/// Storing only counts + status keeps the user-level SQLite small,
/// workspace-scoped, and restart-safe while giving freshness,
/// cache-invalidation, and impact/health queries a single lookup.
fn migrate(conn: &Connection) -> Result<(), DbError> {
    let version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    let tx = conn.unchecked_transaction()?;
    if version < 1 {
        migrate_v1(&tx)?;
    }
    if version < 2 {
        migrate_v2(&tx)?;
    }
    if version < 3 {
        migrate_v3(&tx)?;
    }
    if version < 4 {
        migrate_v4(&tx)?;
    }
    if version < 5 {
        migrate_v5(&tx)?;
    }
    if version < 6 {
        migrate_v6(&tx)?;
    }
    if version < 7 {
        migrate_v7(&tx)?;
    }
    tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
    tx.commit()?;
    Ok(())
}

fn migrate_v1(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS context_records (
            id              TEXT PRIMARY KEY,
            record_type     TEXT NOT NULL,
            namespace       TEXT NOT NULL,
            content         TEXT NOT NULL,
            original_text   TEXT,
            language        TEXT,
            authority       TEXT NOT NULL,
            confidence      REAL NOT NULL,
            importance      REAL NOT NULL,
            scope           TEXT NOT NULL,
            workspace_root  TEXT,
            status          TEXT NOT NULL DEFAULT 'active',
            lifecycle       TEXT NOT NULL DEFAULT 'observed',
            supersedes      TEXT,
            source          TEXT,
            import_origin   TEXT,
            evidence_json   TEXT NOT NULL DEFAULT '[]',
            related_json    TEXT NOT NULL DEFAULT '[]',
            created_at      INTEGER NOT NULL,
            updated_at      INTEGER NOT NULL,
            expires_at      INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_records_scope
            ON context_records(scope, workspace_root, status);
        CREATE INDEX IF NOT EXISTS idx_records_namespace
            ON context_records(namespace, status);
        CREATE INDEX IF NOT EXISTS idx_records_updated
            ON context_records(updated_at DESC);

        CREATE TABLE IF NOT EXISTS events (
            id              INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id      TEXT,
            workspace_root  TEXT NOT NULL,
            kind            TEXT NOT NULL,
            tool            TEXT,
            path            TEXT,
            outcome         TEXT,
            payload_json    TEXT,
            digest          TEXT,
            created_at      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_events_ws_time
            ON events(workspace_root, created_at DESC);
        CREATE INDEX IF NOT EXISTS idx_events_session
            ON events(session_id, created_at);

        CREATE TABLE IF NOT EXISTS sessions (
            id                   TEXT PRIMARY KEY,
            workspace_root       TEXT NOT NULL,
            title                TEXT,
            opened_at            INTEGER NOT NULL,
            closed_at            INTEGER,
            end_reason           TEXT,
            event_count          INTEGER NOT NULL DEFAULT 0,
            context_snapshot_json TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_ws
            ON sessions(workspace_root, opened_at DESC);

        CREATE VIRTUAL TABLE IF NOT EXISTS context_records_fts USING fts5(
            content,
            namespace,
            original_text,
            record_id UNINDEXED,
            tokenize = 'unicode61'
        );
        "#,
    )?;
    Ok(())
}

fn migrate_v2(conn: &Connection) -> Result<(), DbError> {
    // `IF NOT EXISTS` is not supported on ADD COLUMN in older SQLite, so
    // probe the schema first: a partially-applied v2 step must resume,
    // never fail on the column it already added.
    let columns: Vec<String> = conn
        .prepare("PRAGMA table_info(context_records)")?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?;
    if !columns.iter().any(|c| c == "task_id") {
        conn.execute("ALTER TABLE context_records ADD COLUMN task_id TEXT", [])?;
    }
    if !columns.iter().any(|c| c == "extra_json") {
        conn.execute("ALTER TABLE context_records ADD COLUMN extra_json TEXT", [])?;
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_records_task
            ON context_records(task_id, status)",
        [],
    )?;
    Ok(())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>, DbError> {
    let sql = format!("PRAGMA table_info({table})");
    Ok(conn
        .prepare(&sql)?
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<Result<_, _>>()?)
}

fn migrate_v4(conn: &Connection) -> Result<(), DbError> {
    // New table ⇒ `IF NOT EXISTS` is the whole resume story: a crash
    // mid-step re-runs idempotently. No backfill: candidates are derived
    // from events, which every earlier migration preserves.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS learning_candidates (
            candidate_id       TEXT PRIMARY KEY,
            workspace_root     TEXT,
            task_id            TEXT,
            scope              TEXT NOT NULL,
            candidate_kind     TEXT NOT NULL,
            proposition        TEXT NOT NULL,
            namespace          TEXT NOT NULL,
            supporting_json    TEXT NOT NULL DEFAULT '[]',
            contradicting_json TEXT NOT NULL DEFAULT '[]',
            evidence_count     INTEGER NOT NULL DEFAULT 0,
            confidence         REAL NOT NULL DEFAULT 0.0,
            status             TEXT NOT NULL DEFAULT 'candidate',
            created_at         INTEGER NOT NULL,
            updated_at         INTEGER NOT NULL,
            expires_at         INTEGER,
            eval_reason        TEXT,
            inference_record_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_candidates_scope
            ON learning_candidates(scope, workspace_root, status);
        CREATE INDEX IF NOT EXISTS idx_candidates_status
            ON learning_candidates(status, updated_at);
        "#,
    )?;
    Ok(())
}

fn migrate_v5(conn: &Connection) -> Result<(), DbError> {
    // P4: skill lifecycle tables. All new tables with IF NOT EXISTS —
    // crash-safe resume. No backfill: skills are derived from learning
    // candidates, which earlier migrations preserve.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS skill_candidates (
            candidate_id                    TEXT PRIMARY KEY,
            workspace_root                  TEXT,
            task_id                         TEXT,
            scope                           TEXT NOT NULL,
            name                            TEXT NOT NULL,
            description                     TEXT NOT NULL,
            purpose                         TEXT NOT NULL,
            applicability_json              TEXT NOT NULL DEFAULT '{}',
            source_learning_candidates_json TEXT NOT NULL DEFAULT '[]',
            supporting_json                 TEXT NOT NULL DEFAULT '[]',
            contradicting_json              TEXT NOT NULL DEFAULT '[]',
            proposed_content                TEXT NOT NULL DEFAULT '',
            status                          TEXT NOT NULL DEFAULT 'candidate',
            confidence                      REAL NOT NULL DEFAULT 0.0,
            validation_json                 TEXT,
            created_at                      INTEGER NOT NULL,
            updated_at                      INTEGER NOT NULL,
            expires_at                      INTEGER,
            eval_reason                     TEXT,
            rejection_reason                TEXT,
            supersedes_skill                TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_skill_candidates_scope
            ON skill_candidates(scope, workspace_root, status);
        CREATE INDEX IF NOT EXISTS idx_skill_candidates_status
            ON skill_candidates(status, updated_at);
        CREATE INDEX IF NOT EXISTS idx_skill_candidates_name
            ON skill_candidates(name);

        CREATE TABLE IF NOT EXISTS skills (
            skill_id              TEXT PRIMARY KEY,
            workspace_root        TEXT,
            scope                 TEXT NOT NULL,
            name                  TEXT NOT NULL,
            description           TEXT NOT NULL,
            applicability_json    TEXT NOT NULL DEFAULT '{}',
            current_version       INTEGER NOT NULL DEFAULT 1,
            status                TEXT NOT NULL DEFAULT 'draft',
            confidence            REAL NOT NULL DEFAULT 0.0,
            health_json           TEXT NOT NULL DEFAULT '{}',
            source_candidate_id   TEXT,
            superseded_by         TEXT,
            created_at            INTEGER NOT NULL,
            updated_at            INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_skills_scope
            ON skills(scope, workspace_root, status);
        CREATE INDEX IF NOT EXISTS idx_skills_status
            ON skills(status);
        CREATE INDEX IF NOT EXISTS idx_skills_name
            ON skills(name);

        CREATE TABLE IF NOT EXISTS skill_versions (
            version_id            TEXT PRIMARY KEY,
            skill_id              TEXT NOT NULL,
            version_number        INTEGER NOT NULL,
            content               TEXT NOT NULL,
            content_hash          TEXT NOT NULL,
            source_candidate_id   TEXT,
            supporting_json       TEXT NOT NULL DEFAULT '[]',
            validation_json       TEXT,
            author                TEXT NOT NULL DEFAULT 'codebro',
            status                TEXT NOT NULL DEFAULT 'draft',
            created_at            INTEGER NOT NULL,
            parent_version        TEXT
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_skill_versions_skill
            ON skill_versions(skill_id, version_number);
        CREATE INDEX IF NOT EXISTS idx_skill_versions_status
            ON skill_versions(status);
        "#,
    )?;
    // Optimistic-concurrency anchor: the skill version a candidate was
    // validated against. Probe-first (v5 is unreleased; early dev DBs may
    // already carry the v5 tables without this column) — resume-safe.
    let candidate_cols = table_columns(conn, "skill_candidates")?;
    if !candidate_cols.iter().any(|c| c == "based_on_version") {
        conn.execute(
            "ALTER TABLE skill_candidates ADD COLUMN based_on_version INTEGER",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v6(conn: &Connection) -> Result<(), DbError> {
    // P5: durable engineering task runtime. All new tables with IF NOT
    // EXISTS — crash-safe resume. No backfill: tasks are created going
    // forward; P0–P4 rows are untouched.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS tasks (
            task_id                TEXT PRIMARY KEY,
            workspace_root         TEXT NOT NULL,
            title                  TEXT NOT NULL,
            description            TEXT,
            status                 TEXT NOT NULL DEFAULT 'pending',
            priority               TEXT NOT NULL DEFAULT 'medium',
            intent_record_id       TEXT,
            parent_task_id         TEXT,
            idempotency_key        TEXT,
            current_version        INTEGER NOT NULL DEFAULT 0,
            current_checkpoint_id  TEXT,
            lease_worker           TEXT,
            lease_expires_at       INTEGER,
            lease_version          INTEGER NOT NULL DEFAULT 0,
            lease_heartbeat_at     INTEGER,
            created_at             INTEGER NOT NULL,
            updated_at             INTEGER NOT NULL,
            started_at             INTEGER,
            paused_at              INTEGER,
            completed_at            INTEGER,
            validation_json        TEXT,
            skill_refs_json        TEXT NOT NULL DEFAULT '[]',
            outcome_json           TEXT,
            last_transition_at     INTEGER
        );
        CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_idempotency
            ON tasks(workspace_root, idempotency_key)
            WHERE idempotency_key IS NOT NULL;
        CREATE INDEX IF NOT EXISTS idx_tasks_ws_status
            ON tasks(workspace_root, status, updated_at DESC);
        CREATE INDEX IF NOT EXISTS idx_tasks_parent
            ON tasks(parent_task_id) WHERE parent_task_id IS NOT NULL;

        CREATE TABLE IF NOT EXISTS task_checkpoints (
            checkpoint_id          TEXT PRIMARY KEY,
            task_id                TEXT NOT NULL,
            version                INTEGER NOT NULL,
            summary                TEXT NOT NULL,
            state                  TEXT NOT NULL DEFAULT 'running',
            progress               TEXT,
            next_action            TEXT,
            validation_status      TEXT,
            metadata_json          TEXT NOT NULL DEFAULT '{}',
            lease_version          INTEGER NOT NULL DEFAULT 0,
            created_at             INTEGER NOT NULL,
            UNIQUE (task_id, version)
        );
        CREATE INDEX IF NOT EXISTS idx_task_checkpoints_task
            ON task_checkpoints(task_id, version DESC);
        "#,
    )?;
    Ok(())
}

fn migrate_v7(conn: &Connection) -> Result<(), DbError> {
    // P6: engineering index metadata. One new table with IF NOT EXISTS —
    // crash-safe resume, restart-safe, repeatable. No backfill: indexes
    // are recorded going forward as `reindex`/`init` completes; absent
    // rows mean UNKNOWN (never fabricated READY).
    //
    // Canonical/derived boundary: this table holds derived bookkeeping
    // (counts, status, revision) keyed by canonical workspace root. It
    // never stores file contents, symbols, or edges — those remain in
    // the per-project `.codebro/facts.json`.
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS repo_indexes (
            workspace_root        TEXT PRIMARY KEY,
            repository_identity   TEXT NOT NULL DEFAULT '{}',
            index_status          TEXT NOT NULL DEFAULT 'UNKNOWN',
            indexed_at            INTEGER NOT NULL DEFAULT 0,
            repository_revision   TEXT NOT NULL DEFAULT 'unknown',
            file_count            INTEGER NOT NULL DEFAULT 0,
            symbol_count          INTEGER NOT NULL DEFAULT 0,
            edge_count            INTEGER NOT NULL DEFAULT 0,
            stale_count           INTEGER NOT NULL DEFAULT 0,
            updated_at            INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_repo_indexes_status
            ON repo_indexes(index_status, updated_at DESC);
        "#,
    )?;
    Ok(())
}

fn migrate_v3(conn: &Connection) -> Result<(), DbError> {
    // Probe-first like v2: a crash mid-step must resume, never fail on the
    // column it already added.
    let session_cols = table_columns(conn, "sessions")?;
    for (column, ddl) in [
        ("task_id", "ALTER TABLE sessions ADD COLUMN task_id TEXT"),
        (
            "status",
            "ALTER TABLE sessions ADD COLUMN status TEXT NOT NULL DEFAULT 'active'",
        ),
        ("source", "ALTER TABLE sessions ADD COLUMN source TEXT"),
        (
            "parent_session_id",
            "ALTER TABLE sessions ADD COLUMN parent_session_id TEXT",
        ),
        (
            "updated_at",
            "ALTER TABLE sessions ADD COLUMN updated_at INTEGER",
        ),
    ] {
        if !session_cols.iter().any(|c| c == column) {
            conn.execute(ddl, [])?;
        }
    }
    // Pre-v3 rows predate the heartbeat: their last touch is their opening.
    conn.execute(
        "UPDATE sessions SET updated_at = opened_at WHERE updated_at IS NULL",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_task
            ON sessions(workspace_root, task_id, status)",
        [],
    )?;
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_sessions_status
            ON sessions(status, updated_at)",
        [],
    )?;

    let event_cols = table_columns(conn, "events")?;
    for (column, ddl) in [
        ("task_id", "ALTER TABLE events ADD COLUMN task_id TEXT"),
        ("summary", "ALTER TABLE events ADD COLUMN summary TEXT"),
        ("dedup_key", "ALTER TABLE events ADD COLUMN dedup_key TEXT"),
        ("source", "ALTER TABLE events ADD COLUMN source TEXT"),
    ] {
        if !event_cols.iter().any(|c| c == column) {
            conn.execute(ddl, [])?;
        }
    }
    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_events_task
            ON events(workspace_root, task_id, created_at DESC)",
        [],
    )?;
    // Idempotency keys are unique when present; NULLs stay distinct so
    // key-less passive events never collide.
    conn.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_events_dedup
            ON events(dedup_key) WHERE dedup_key IS NOT NULL",
        [],
    )?;

    // Derived history search index (canonical data stays in `events`).
    conn.execute(
        "CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
            summary,
            text,
            kind,
            tool,
            outcome,
            event_id UNINDEXED,
            tokenize = 'unicode61'
        )",
        [],
    )?;
    // Backfill: pre-v3 events become searchable (their stored text is
    // already redacted at write time by every writer since P0).
    let existing: i64 = conn.query_row("SELECT count(*) FROM events_fts", [], |r| r.get(0))?;
    if existing == 0 {
        let mut stmt =
            conn.prepare("SELECT id, summary, payload_json, kind, tool, outcome FROM events")?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        for (id, summary, payload, kind, tool, outcome) in &rows {
            let excerpt: String = payload
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(2000)
                .collect();
            conn.execute(
                "INSERT INTO events_fts (summary, text, kind, tool, outcome, event_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    summary.as_deref().unwrap_or(""),
                    excerpt,
                    kind,
                    tool.as_deref().unwrap_or(""),
                    outcome.as_deref().unwrap_or(""),
                    id
                ],
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db_path(dir: &std::path::Path) -> PathBuf {
        dir.join(STATE_DB_FILE)
    }

    #[test]
    fn fresh_open_creates_schema_at_current_version() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_checked(&db_path(dir.path())).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        // Tables exist and the FTS virtual table is queryable.
        let count: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let fts_count: i64 = conn
            .query_row("SELECT count(*) FROM context_records_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts_count, 0);
    }

    #[test]
    fn migration_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        open_checked(&path).unwrap();
        // Re-running on an already-migrated file must be a no-op.
        open_checked(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn newer_schema_is_refused_not_quarantined() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        {
            let conn = open_checked(&path).unwrap();
            conn.pragma_update(None, "user_version", SCHEMA_VERSION + 5)
                .unwrap();
        }
        match open_checked(&path) {
            Err(DbError::NewerSchema { found, supported }) => {
                assert_eq!(found, SCHEMA_VERSION + 5);
                assert_eq!(supported, SCHEMA_VERSION);
            }
            other => panic!("expected NewerSchema, got {other:?}"),
        }
        // The data file must be untouched (no quarantine rename).
        assert!(path.exists());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn corrupt_file_is_quarantined_and_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        std::fs::write(&path, b"this is not a sqlite database at all").unwrap();
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            entries.iter().any(|n| n.contains("state.db.corrupt-")),
            "corrupt database must be quarantined, not deleted: {entries:?}"
        );
        assert!(
            path.exists(),
            "a fresh database must replace the corrupt one"
        );
    }

    #[test]
    fn wal_sidecars_are_quarantined_together() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        std::fs::write(&path, b"garbage").unwrap();
        std::fs::write(wal_path(&path), b"stale-wal").unwrap();
        std::fs::write(shm_path(&path), b"stale-shm").unwrap();
        open_with_recovery(&path).unwrap();
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(names
            .iter()
            .any(|n| n.contains("state.db.corrupt-") && !n.ends_with("-wal")));
        assert!(
            !names.iter().any(|n| n == "state.db-wal"),
            "stale WAL must not survive next to the fresh database"
        );
        assert!(
            !names.iter().any(|n| n == "state.db-shm"),
            "stale SHM must not survive next to the fresh database"
        );
    }

    #[test]
    fn quarantine_only_on_evidence_of_corruption() {
        use rusqlite::ErrorCode;
        let failure = |code: ErrorCode| {
            DbError::Sqlite(rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error {
                    code,
                    extended_code: 0,
                },
                "probe".to_string().into(),
            ))
        };
        // Content-level corruption: quarantine.
        assert!(should_quarantine(&DbError::Corrupt("probe".to_string())));
        assert!(should_quarantine(&failure(ErrorCode::DatabaseCorrupt)));
        assert!(should_quarantine(&failure(ErrorCode::NotADatabase)));
        // Contention and environment: never quarantine a possibly-healthy DB.
        assert!(!should_quarantine(&failure(ErrorCode::DatabaseBusy)));
        assert!(!should_quarantine(&failure(ErrorCode::DatabaseLocked)));
        assert!(!should_quarantine(&failure(ErrorCode::ReadOnly)));
        assert!(!should_quarantine(&failure(ErrorCode::CannotOpen)));
        assert!(!should_quarantine(&failure(ErrorCode::DiskFull)));
        assert!(!should_quarantine(&failure(ErrorCode::ApiMisuse)));
        assert!(!should_quarantine(&DbError::Unusable("nope".to_string())));
    }

    #[test]
    fn non_corrupt_open_failure_leaves_files_untouched() {
        // `state.db` as a non-empty directory: SQLite cannot open it, but
        // the content is not "corrupt" — recovery must NOT rename the
        // operator's directory aside. The error propagates and callers
        // (e.g. the `context` tool) degrade to an empty section instead.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("blocker"), b"x").unwrap();
        assert!(
            open_with_recovery(&path).is_err(),
            "a directory is not an openable database"
        );
        assert!(path.is_dir(), "the directory must be left in place");
        assert!(
            path.join("blocker").exists(),
            "directory contents must be untouched"
        );
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains("corrupt-")),
            "nothing may be quarantined without corruption evidence: {names:?}"
        );
    }

    /// Build a genuine v1 database (P0 shape: no task_id / extra_json,
    /// user_version = 1) with one seeded record, as an upgrade fixture.
    fn v1_database(path: &std::path::Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            CREATE TABLE context_records (
                id              TEXT PRIMARY KEY,
                record_type     TEXT NOT NULL,
                namespace       TEXT NOT NULL,
                content         TEXT NOT NULL,
                original_text   TEXT,
                language        TEXT,
                authority       TEXT NOT NULL,
                confidence      REAL NOT NULL,
                importance      REAL NOT NULL,
                scope           TEXT NOT NULL,
                workspace_root  TEXT,
                status          TEXT NOT NULL DEFAULT 'active',
                lifecycle       TEXT NOT NULL DEFAULT 'observed',
                supersedes      TEXT,
                source          TEXT,
                import_origin   TEXT,
                evidence_json   TEXT NOT NULL DEFAULT '[]',
                related_json    TEXT NOT NULL DEFAULT '[]',
                created_at      INTEGER NOT NULL,
                updated_at      INTEGER NOT NULL,
                expires_at      INTEGER
            );
            CREATE TABLE events (
                id              INTEGER PRIMARY KEY AUTOINCREMENT,
                session_id      TEXT,
                workspace_root  TEXT NOT NULL,
                kind            TEXT NOT NULL,
                tool            TEXT,
                path            TEXT,
                outcome         TEXT,
                payload_json    TEXT,
                digest          TEXT,
                created_at      INTEGER NOT NULL
            );
            CREATE TABLE sessions (
                id                   TEXT PRIMARY KEY,
                workspace_root       TEXT NOT NULL,
                title                TEXT,
                opened_at            INTEGER NOT NULL,
                closed_at            INTEGER,
                end_reason           TEXT,
                event_count          INTEGER NOT NULL DEFAULT 0,
                context_snapshot_json TEXT
            );
            CREATE VIRTUAL TABLE context_records_fts USING fts5(
                content, namespace, original_text, record_id UNINDEXED,
                tokenize = 'unicode61'
            );
            INSERT INTO context_records (
                id, record_type, namespace, content, authority, confidence,
                importance, scope, created_at, updated_at
            ) VALUES (
                'ctx::p0-legacy', 'preference', 'fp.engineering.simplicity',
                'Prefer simple implementations', 'user_confirmed', 0.9,
                0.7, 'global', 100, 200
            );
            INSERT INTO context_records_fts (content, namespace, original_text, record_id)
                VALUES ('Prefer simple implementations', 'fp.engineering.simplicity', '', 'ctx::p0-legacy');
            PRAGMA user_version = 1;
            "#,
        )
        .unwrap();
    }

    #[test]
    fn fresh_database_has_v2_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        open_checked(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(context_records)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(cols.iter().any(|c| c == "task_id"), "columns: {cols:?}");
        assert!(cols.iter().any(|c| c == "extra_json"), "columns: {cols:?}");
    }

    #[test]
    fn v1_database_upgrades_to_v2_preserving_rows() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v1_database(&path);

        // Upgrade path: open (migration runs), not quarantine.
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);

        // The P0 row survives with NULL new columns (reads back as None).
        let (content, task_id, extra): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT content, task_id, extra_json FROM context_records WHERE id = 'ctx::p0-legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(content, "Prefer simple implementations");
        assert_eq!(task_id, None);
        assert_eq!(extra, None);

        // ... and stays keyword-searchable after the upgrade.
        let fts: i64 = conn
            .query_row(
                "SELECT count(*) FROM context_records_fts WHERE context_records_fts MATCH '\"simple\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts, 1);

        // No quarantine sidecars: this was a migration, not corruption.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains("corrupt-")),
            "upgrade must not quarantine: {names:?}"
        );
    }

    #[test]
    fn v2_migration_resumes_after_partial_application() {
        // A v2 step that added task_id but stopped before extra_json (crash
        // mid-migration) must resume, not fail on the existing column.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v1_database(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("ALTER TABLE context_records ADD COLUMN task_id TEXT", [])
                .unwrap();
            // user_version stays 1: the step never completed.
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    fn table_cols(conn: &Connection, table: &str) -> Vec<String> {
        conn.prepare(&format!("PRAGMA table_info({table})"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    }

    #[test]
    fn fresh_database_has_v3_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        open_checked(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let sessions = table_cols(&conn, "sessions");
        for col in [
            "task_id",
            "status",
            "source",
            "parent_session_id",
            "updated_at",
        ] {
            assert!(sessions.iter().any(|c| c == col), "sessions: {sessions:?}");
        }
        let events = table_cols(&conn, "events");
        for col in ["task_id", "summary", "dedup_key", "source"] {
            assert!(events.iter().any(|c| c == col), "events: {events:?}");
        }
        // The derived history index exists and is queryable empty.
        let fts: i64 = conn
            .query_row("SELECT count(*) FROM events_fts", [], |r| r.get(0))
            .unwrap();
        assert_eq!(fts, 0);
    }

    /// Seed a v1 database with history rows (an event + a session) so the
    /// v1→v3 upgrade proves P0/P1 history survives with searchable FTS.
    fn v1_database_with_history(path: &std::path::Path) {
        v1_database(path);
        let conn = Connection::open(path).unwrap();
        conn.execute_batch(
            r#"
            INSERT INTO events (session_id, workspace_root, kind, tool, outcome, payload_json, created_at)
                VALUES (NULL, '/proj', 'validation', 'sandbox_test', 'passed',
                        '{"command":"cargo test"}', 500);
            INSERT INTO sessions (id, workspace_root, title, opened_at, closed_at, end_reason, event_count)
                VALUES ('ses::legacy01', '/proj', 'old work', 400, NULL, NULL, 0);
            PRAGMA user_version = 1;
            "#,
        )
        .unwrap();
    }

    #[test]
    fn v1_database_upgrades_to_v3_preserving_history() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v1_database_with_history(&path);

        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);

        // Legacy session survives as active with a backfilled heartbeat.
        let (status, updated, opened): (String, i64, i64) = conn
            .query_row(
                "SELECT status, updated_at, opened_at FROM sessions WHERE id = 'ses::legacy01'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "active");
        assert_eq!(updated, opened);

        // Legacy event survives with NULL P2 columns...
        let (kind, task_id, summary): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT kind, task_id, summary FROM events WHERE workspace_root = '/proj'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(kind, "validation");
        assert_eq!(task_id, None);
        assert_eq!(summary, None);

        // ... and the v3 backfill made it FTS-searchable.
        let fts: i64 = conn
            .query_row(
                "SELECT count(*) FROM events_fts WHERE events_fts MATCH '\"validation\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(fts, 1);

        // Records FTS from the v1 era still intact.
        let records_fts: i64 = conn
            .query_row(
                "SELECT count(*) FROM context_records_fts WHERE context_records_fts MATCH '\"simple\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(records_fts, 1);
    }

    /// Build a genuine v2 database (v1 shape + record task_id/extra_json,
    /// user_version = 2) as a P1→P2 upgrade fixture.
    fn v2_database(path: &std::path::Path) {
        v1_database_with_history(path);
        // Apply only the v2 step, then pin the version at 2.
        let conn = Connection::open(path).unwrap();
        conn.execute("ALTER TABLE context_records ADD COLUMN task_id TEXT", [])
            .unwrap();
        conn.execute("ALTER TABLE context_records ADD COLUMN extra_json TEXT", [])
            .unwrap();
        conn.execute(
            "CREATE INDEX idx_records_task ON context_records(task_id, status)",
            [],
        )
        .unwrap();
        conn.execute("PRAGMA user_version = 2", []).unwrap();
    }

    #[test]
    fn v2_database_upgrades_to_v3() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);

        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let sessions = table_cols(&conn, "sessions");
        assert!(sessions.iter().any(|c| c == "status"));
        let count: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
        // The v1-seeded record row is still there (P0→P1→P2 chain intact).
        let records: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(records, 1);
    }

    #[test]
    fn v3_migration_resumes_after_partial_application() {
        // A v3 step that added the session task_id but crashed before the
        // rest must resume, not fail on the existing column.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute("ALTER TABLE sessions ADD COLUMN task_id TEXT", [])
                .unwrap();
            // user_version stays 2: the step never completed.
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let sessions = table_cols(&conn, "sessions");
        assert!(sessions.iter().any(|c| c == "updated_at"));
        let count: i64 = conn
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn fresh_database_has_v4_learning_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        open_checked(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        let cols: Vec<String> = conn
            .prepare("PRAGMA table_info(learning_candidates)")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for col in [
            "candidate_id",
            "scope",
            "candidate_kind",
            "proposition",
            "namespace",
            "supporting_json",
            "contradicting_json",
            "confidence",
            "status",
            "eval_reason",
            "inference_record_id",
        ] {
            assert!(cols.iter().any(|c| c == col), "candidates: {cols:?}");
        }
        let count: i64 = conn
            .query_row("SELECT count(*) FROM learning_candidates", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn v2_database_upgrades_to_v4_preserving_everything() {
        // P1 → P3 chain: records, history, and both FTS indexes survive;
        // the candidate table starts empty (hypotheses re-derive).
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);

        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);

        let records: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(records, 1);
        let events: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1);
        let records_fts: i64 = conn
            .query_row(
                "SELECT count(*) FROM context_records_fts WHERE context_records_fts MATCH '\"simple\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(records_fts, 1);
        let events_fts: i64 = conn
            .query_row(
                "SELECT count(*) FROM events_fts WHERE events_fts MATCH '\"validation\"'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(events_fts, 1);
        let candidates: i64 = conn
            .query_row("SELECT count(*) FROM learning_candidates", [], |r| r.get(0))
            .unwrap();
        assert_eq!(candidates, 0);

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains("corrupt-")),
            "upgrade must not quarantine: {names:?}"
        );
    }

    #[test]
    fn v4_migration_resumes_when_table_already_exists() {
        // A v4 step interrupted after CREATE TABLE but before the version
        // bump must resume idempotently (IF NOT EXISTS everywhere).
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS learning_candidates (
                    candidate_id TEXT PRIMARY KEY, scope TEXT NOT NULL,
                    candidate_kind TEXT NOT NULL, proposition TEXT NOT NULL,
                    namespace TEXT NOT NULL, supporting_json TEXT NOT NULL DEFAULT '[]',
                    contradicting_json TEXT NOT NULL DEFAULT '[]', evidence_count INTEGER NOT NULL DEFAULT 0,
                    confidence REAL NOT NULL DEFAULT 0.0, status TEXT NOT NULL DEFAULT 'candidate',
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    workspace_root TEXT, task_id TEXT, expires_at INTEGER,
                    eval_reason TEXT, inference_record_id TEXT);
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM learning_candidates", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn fresh_database_has_v6_task_shape() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        open_checked(&path).unwrap();
        let conn = Connection::open(&path).unwrap();
        for table in ["tasks", "task_checkpoints"] {
            let cols = table_cols(&conn, table);
            assert!(!cols.is_empty(), "{table} missing");
        }
        let tasks = table_cols(&conn, "tasks");
        for col in [
            "task_id",
            "workspace_root",
            "title",
            "status",
            "priority",
            "intent_record_id",
            "parent_task_id",
            "idempotency_key",
            "current_version",
            "current_checkpoint_id",
            "lease_worker",
            "lease_expires_at",
            "lease_version",
            "lease_heartbeat_at",
            "outcome_json",
        ] {
            assert!(tasks.iter().any(|c| c == col), "tasks: {tasks:?}");
        }
        let cps = table_cols(&conn, "task_checkpoints");
        for col in [
            "checkpoint_id",
            "task_id",
            "version",
            "summary",
            "state",
            "lease_version",
            "created_at",
        ] {
            assert!(cps.iter().any(|c| c == col), "task_checkpoints: {cps:?}");
        }
        // Both start empty: tasks are created going forward.
        for table in ["tasks", "task_checkpoints"] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0);
        }
    }

    #[test]
    fn v5_database_upgrades_to_v6_preserving_everything() {
        // P4 → P5 chain: skills/versions/learning/history/records all
        // survive; the task tables arrive empty; the skill tables keep
        // their shape.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        // v2_database already carries records+history; migrate to v5
        // by running the current migration, then pin at v5 via a seeded
        // skill row, then reopen (v6 step applies).
        v2_database(&path);
        {
            let conn = open_checked(&path).unwrap(); // migrates to v6
            let _ = conn;
        }
        // Re-pin to v5 with one skill row to prove v5 data survives the
        // v6 step: rewrite the version and insert P4 rows.
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                DELETE FROM tasks;
                DELETE FROM task_checkpoints;
                INSERT INTO skills (skill_id, scope, name, description,
                    current_version, status, confidence, created_at, updated_at)
                VALUES ('sk::e5c0', 'project', 'persist-skill', 'keeps v5 rows',
                    1, 'active', 0.8, 100, 200);
                INSERT INTO skill_versions (version_id, skill_id, version_number,
                    content, content_hash, status, created_at)
                VALUES ('sv::e5c0', 'sk::e5c0', 1, 'x', 'deadbeef', 'active', 100);
                PRAGMA user_version = 5;
                "#,
            )
            .unwrap();
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        // P0–P4 state intact.
        let records: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(records, 1);
        let events: i64 = conn
            .query_row("SELECT count(*) FROM events", [], |r| r.get(0))
            .unwrap();
        assert_eq!(events, 1);
        let skills: i64 = conn
            .query_row("SELECT count(*) FROM skills", [], |r| r.get(0))
            .unwrap();
        assert_eq!(skills, 1);
        let versions: i64 = conn
            .query_row("SELECT count(*) FROM skill_versions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(versions, 1);
        // Task tables exist and are empty.
        let tasks: i64 = conn
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tasks, 0);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains("corrupt-")),
            "upgrade must not quarantine: {names:?}"
        );
    }

    #[test]
    fn v6_migration_resumes_when_tables_already_exist() {
        // A v6 step interrupted after CREATE TABLE but before the version
        // bump must resume idempotently (IF NOT EXISTS everywhere).
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS tasks (
                    task_id TEXT PRIMARY KEY, workspace_root TEXT NOT NULL,
                    title TEXT NOT NULL, description TEXT,
                    status TEXT NOT NULL DEFAULT 'pending',
                    priority TEXT NOT NULL DEFAULT 'medium',
                    intent_record_id TEXT, parent_task_id TEXT, idempotency_key TEXT,
                    current_version INTEGER NOT NULL DEFAULT 0, current_checkpoint_id TEXT,
                    lease_worker TEXT, lease_expires_at INTEGER,
                    lease_version INTEGER NOT NULL DEFAULT 0, lease_heartbeat_at INTEGER,
                    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
                    started_at INTEGER, paused_at INTEGER, completed_at INTEGER,
                    validation_json TEXT, skill_refs_json TEXT NOT NULL DEFAULT '[]',
                    outcome_json TEXT, last_transition_at INTEGER);
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        // The v6 step completed: checkpoints table now exists too.
        let cps = table_cols(&conn, "task_checkpoints");
        assert!(!cps.is_empty());
        let count: i64 = conn
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn v6_database_upgrades_to_v7_preserving_everything() {
        // P5 → P6 chain: tasks, skills, learning, history, records all
        // survive; repo_indexes arrives empty; no quarantine.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);
        {
            // Migrate to v6 first, then pin back to v6 with a task row to
            // prove P5 data survives the v7 step.
            let conn = open_checked(&path).unwrap();
            let _ = conn;
        }
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                r#"
                INSERT INTO tasks (task_id, workspace_root, title, status,
                    priority, current_version, lease_version, created_at, updated_at)
                VALUES ('task::p6keep', '/repo-a', 'keep me', 'pending',
                    'medium', 0, 0, 100, 200);
                PRAGMA user_version = 6;
                "#,
            )
            .unwrap();
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        // P0–P5 state intact.
        let records: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(records, 1);
        let tasks: i64 = conn
            .query_row("SELECT count(*) FROM tasks", [], |r| r.get(0))
            .unwrap();
        assert_eq!(tasks, 1);
        // repo_indexes exists and starts empty.
        let idx: i64 = conn
            .query_row("SELECT count(*) FROM repo_indexes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(idx, 0);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains("corrupt-")),
            "upgrade must not quarantine: {names:?}"
        );
    }

    #[test]
    fn v7_migration_resumes_when_table_already_exists() {
        // A v7 step interrupted after CREATE TABLE but before the version
        // bump must resume idempotently.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v2_database(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(
                "CREATE TABLE IF NOT EXISTS repo_indexes (
                    workspace_root TEXT PRIMARY KEY,
                    repository_identity TEXT NOT NULL DEFAULT '{}',
                    index_status TEXT NOT NULL DEFAULT 'UNKNOWN',
                    indexed_at INTEGER NOT NULL DEFAULT 0,
                    repository_revision TEXT NOT NULL DEFAULT 'unknown',
                    file_count INTEGER NOT NULL DEFAULT 0,
                    symbol_count INTEGER NOT NULL DEFAULT 0,
                    edge_count INTEGER NOT NULL DEFAULT 0,
                    stale_count INTEGER NOT NULL DEFAULT 0,
                    updated_at INTEGER NOT NULL DEFAULT 0);
                 PRAGMA user_version = 6;",
            )
            .unwrap();
        }
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let cols = table_cols(&conn, "repo_indexes");
        assert!(cols.contains(&"workspace_root".to_string()));
        assert!(cols.contains(&"index_status".to_string()));
    }

    #[test]
    fn fresh_v7_database_has_repo_indexes() {
        let dir = tempfile::tempdir().unwrap();
        let conn = open_checked(&db_path(dir.path())).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let count: i64 = conn
            .query_row("SELECT count(*) FROM repo_indexes", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn v1_database_upgrades_straight_to_v7() {
        // Full chain v1 → v7 in one open: P0 rows survive, all P1–P6
        // tables exist.
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        v1_database(&path);
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let records: i64 = conn
            .query_row("SELECT count(*) FROM context_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(records, 1);
        for table in [
            "tasks",
            "task_checkpoints",
            "skills",
            "learning_candidates",
            "repo_indexes",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 0, "table {table} must exist and start empty");
        }
    }
}

#[cfg(test)]
mod concurrent_open_tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    fn db_path(dir: &std::path::Path) -> PathBuf {
        dir.join(STATE_DB_FILE)
    }

    /// P9 regression: concurrent opens from independent connections must
    /// never quarantine a healthy database. Before the fix, FTS5 index
    /// validation reported lock contention as integrity-check output
    /// rows ("unable to validate the inverted index …: database is
    /// locked"), which read as corruption and quarantined the live
    /// database — data effectively lost to a fresh empty file, cascading
    /// into sibling opens failing on the half-rebuilt replacement.
    /// Contention lines now report busy (retryable, never quarantinable),
    /// retries absorb the rest, and the quiescence gate withholds
    /// destruction while the file churns.
    #[test]
    fn concurrent_opens_never_quarantine_a_healthy_database() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        // Healthy v7 database with real content.
        {
            let conn = open_checked(&path).unwrap();
            conn.execute(
                "INSERT INTO sessions (id, workspace_root, task_id, title, status, source,
                    parent_session_id, opened_at, updated_at, closed_at, end_reason, event_count)
                 VALUES ('ses::probe', '/work', NULL, NULL, 'active', NULL, NULL, 1, 1, NULL, NULL, 0)",
                [],
            )
            .unwrap();
        }
        let barrier = Arc::new(Barrier::new(8));
        let failures = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let (path, barrier, failures) =
                    (path.clone(), Arc::clone(&barrier), Arc::clone(&failures));
                std::thread::spawn(move || {
                    barrier.wait();
                    // Production discipline: one connection per worker,
                    // opened once (with client-side backoff, like an MCP
                    // caller retrying a busy tool) and reused — servers do
                    // not open per statement.
                    let mut conn = None;
                    for attempt in 0..20 {
                        match open_with_recovery(&path) {
                            Ok(c) => {
                                conn = Some(c);
                                break;
                            }
                            Err(e) if attempt + 1 < 20 => {
                                std::thread::sleep(std::time::Duration::from_millis(25));
                                let _ = e;
                            }
                            Err(e) => {
                                failures.lock().unwrap().push(format!("open {i}: {e}"));
                                return;
                            }
                        }
                    }
                    let conn = conn.expect("open eventually succeeds");
                    for j in 0..10 {
                        // Interleave reads with concurrent writes.
                        if (i + j) % 3 == 0 {
                            if let Err(e) = conn.execute(
                                "UPDATE sessions SET updated_at = ?1 WHERE id = 'ses::probe'",
                                [1000 + j as i64],
                            ) {
                                failures.lock().unwrap().push(format!("write {i}/{j}: {e}"));
                            }
                        } else {
                            match conn.query_row("SELECT count(*) FROM sessions", [], |r| {
                                r.get::<_, i64>(0)
                            }) {
                                Ok(1) => {}
                                Ok(count) => failures
                                    .lock()
                                    .unwrap()
                                    .push(format!("read {i}/{j}: count={count}")),
                                Err(e) => {
                                    failures.lock().unwrap().push(format!("read {i}/{j}: {e}"))
                                }
                            }
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("no thread may panic");
        }
        // Primary invariant first: no quarantine debris, so any failure
        // below is diagnosable as itself rather than as data destruction.
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains(".corrupt-")),
            "healthy database must never be quarantined: {names:?}"
        );
        let failures = failures.lock().unwrap();
        assert!(
            failures.is_empty(),
            "concurrent opens must all succeed: {failures:?}"
        );
        // Content survived the storm.
        let conn = open_checked(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    /// Rabid opens under write contention must never destroy data: even
    /// when individual opens fail transiently (busy), the file itself is
    /// never quarantined. Errors are allowed here; destruction is not.
    #[test]
    fn rabid_opens_under_contention_never_quarantine() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        {
            let conn = open_checked(&path).unwrap();
            conn.execute(
                "INSERT INTO sessions (id, workspace_root, task_id, title, status, source,
                    parent_session_id, opened_at, updated_at, closed_at, end_reason, event_count)
                 VALUES ('ses::probe', '/work', NULL, NULL, 'active', NULL, NULL, 1, 1, NULL, NULL, 0)",
                [],
            )
            .unwrap();
        }
        let barrier = Arc::new(Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (path, barrier) = (path.clone(), Arc::clone(&barrier));
                std::thread::spawn(move || {
                    barrier.wait();
                    // Deliberately hostile: open + close + write every
                    // iteration, no backoff. Some opens may report busy;
                    // none may destroy the database.
                    for j in 0..15 {
                        let Ok(conn) = open_with_recovery(&path) else {
                            continue;
                        };
                        if j % 3 == 0 {
                            let _ = conn.execute(
                                "UPDATE sessions SET updated_at = ?1 WHERE id = 'ses::probe'",
                                [2000 + j as i64],
                            );
                        }
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().expect("no thread may panic");
        }
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            !names.iter().any(|n| n.contains(".corrupt-")),
            "contention must never trigger quarantine: {names:?}"
        );
        let conn = open_with_recovery(&path).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM sessions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1, "seeded content must survive contention");
    }

    /// Genuine corruption still quarantines (the retry path must not
    /// swallow real damage): scribble over the file, open, expect a
    /// fresh database plus quarantine debris.
    #[test]
    fn lock_contention_is_busy_never_quarantinable() {
        // The FTS5 lock-marker mapping: contention surfaces as a busy
        // error (retryable) and must never trigger quarantine.
        let busy = lock_contention_error();
        assert!(
            !should_quarantine(&busy),
            "contention must never quarantine"
        );
        assert!(is_transient(&busy), "contention must be retryable");
    }
    #[test]
    fn genuine_corruption_still_quarantines_after_retries() {
        let dir = tempfile::tempdir().unwrap();
        let path = db_path(dir.path());
        open_checked(&path).unwrap();
        std::fs::write(&path, b"definitely not a sqlite file at all").unwrap();
        let conn = open_with_recovery(&path).unwrap();
        assert_eq!(user_version(&conn).unwrap(), SCHEMA_VERSION);
        let names: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            names.iter().any(|n| n.contains("state.db.corrupt-")),
            "corrupt file must be quarantined: {names:?}"
        );
    }
}

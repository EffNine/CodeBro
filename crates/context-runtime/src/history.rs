//! P2 sessions + history: what happened during previous engineering work.
//!
//! P1 answers "what has the user explicitly told CodeBro"; this module
//! answers "what happened during previous work". It deliberately stops
//! there: no inference, no learning, no promotion — P3 decides later
//! whether an experience should become durable knowledge.
//!
//! # Model
//!
//! ```text
//! Session (one coherent period of OpenCode/CodeBro work)
//!   └── HistoryEvent (append-only; what happened, when, where, who, task)
//!         └── indexed in events_fts (derived FTS5; see recall.rs)
//! ```
//!
//! Sessions live in the `sessions` table (reserved since P0, extended by
//! the v3 migration with `task_id`, `status`, `source`,
//! `parent_session_id`, `updated_at`). History reuses [`EventRecord`] —
//! there is exactly one event abstraction — with the P2 taxonomy
//! ([`HistoryKind`]) as its validated vocabulary. The store-facing
//! [`HistoryInput`] is the single write seam; [`ContextStore::record_history`]
//! redacts, truncates, dedups, links the session, and syncs FTS in one
//! transaction.
//!
//! # Invariants
//!
//! - History is append-only: events are never updated or soft-retired.
//!   `forget` retires *records*, never history.
//! - An interrupted process leaves `Active` sessions behind. They are
//!   reported as stale (never auto-completed) via [`ContextStore::stale_sessions`].
//! - Session ids are opaque (`ses::<hex>`), never workspace paths or row ids.
//! - Every write path canonicalizes the workspace and redacts secrets
//!   before storage (canonical store *and* FTS).

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, OptionalExtension};

use crate::db;
use crate::store::{ContextError, ContextStore};
use crate::types::{
    EventRecord, MAX_DEDUP_KEY_CHARS, MAX_EVENT_PAYLOAD_BYTES, MAX_HISTORY_SOURCE_CHARS,
    MAX_HISTORY_SUMMARY_CHARS, MAX_TASK_ID_CHARS,
};
use crate::workspace::canonical_workspace_key;

/// Seconds after which an `Active` session with no touch is considered
/// stale (interrupted): 24 hours. Staleness is derived at read time, never
/// stored — an interrupted session is recoverable, never silently completed.
pub const STALE_AFTER_SECS: u64 = 24 * 3600;

/// Marker appended when a history summary or payload is truncated.
/// Matches the context-excerpt convention (`…[truncated …]` vocabulary).
pub const HISTORY_TRUNCATION_MARKER: &str = "…[truncated for history budget]";

/// Minimal session lifecycle.
///
/// There is deliberately no `Paused`/`Resumed`: a session is either open
/// (`Active`) or terminally closed (`Completed` for clean ends,
/// `Abandoned` for discarded work). Anything `Active` past
/// [`STALE_AFTER_SECS`] is *interrupted*, reported by
/// [`ContextStore::stale_sessions`], and still reopenable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    Active,
    Completed,
    Abandoned,
}

impl SessionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Active => "active",
            SessionStatus::Completed => "completed",
            SessionStatus::Abandoned => "abandoned",
        }
    }

    /// Whether this status is terminal (no further transitions allowed).
    pub fn is_terminal(&self) -> bool {
        !matches!(self, SessionStatus::Active)
    }
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for SessionStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "active" => Ok(SessionStatus::Active),
            "completed" => Ok(SessionStatus::Completed),
            "abandoned" => Ok(SessionStatus::Abandoned),
            other => Err(format!("unknown session status: {other}")),
        }
    }
}

/// A persistent session: one coherent period of OpenCode/CodeBro work.
///
/// Only fields useful to recall are kept: identity (opaque id), scope
/// (workspace + optional task), provenance (source, parent), lifecycle
/// (status + timestamps), and a cached event count. There is no model or
/// config metadata beyond `source` — CodeBro never selects models.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionRecord {
    /// Opaque stable id (`ses::<16 hex>`). Never a path or row id.
    pub id: String,
    /// Canonical workspace root this session belongs to.
    pub workspace_root: String,
    /// Task identity when the session is task-bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Human-readable title where available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Producer label (e.g. `mcp`, `mcp-passive`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Lifecycle status.
    pub status: SessionStatus,
    /// Id of the session this one continues, where applicable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Unix seconds: opened.
    pub started_at: u64,
    /// Unix seconds: last event or touch.
    pub updated_at: u64,
    /// Unix seconds: closed (terminal statuses only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    /// Why the session ended (free text, e.g. `user_done`, `superseded`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_reason: Option<String>,
    /// Cached number of linked history events (maintained by the store).
    pub event_count: u64,
}

impl SessionRecord {
    /// Whether this session looks interrupted as of `now`: still `Active`
    /// but untouched for longer than [`STALE_AFTER_SECS`].
    pub fn is_stale(&self, now: u64) -> bool {
        self.status == SessionStatus::Active
            && now.saturating_sub(self.updated_at) > STALE_AFTER_SECS
    }
}

/// The closed P2 history taxonomy: what happened, structurally.
///
/// Kept small on purpose. Structural capture ("this MCP operation happened
/// in project Y") is P2; semantic interpretation ("the user prefers X") is
/// P3 and must never be smuggled in through a kind label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    SessionStarted,
    SessionEnded,
    UserMessage,
    AssistantMessage,
    ToolExecution,
    ToolResult,
    Decision,
    ChangeApplied,
    Validation,
    Error,
    Observation,
    /// P5 task lifecycle: durable engineering work crossed a lifecycle
    /// boundary. The variants are structural (what happened to the task
    /// record), not semantic — interpretation stays with P3 learning.
    TaskCreated,
    TaskStarted,
    TaskPaused,
    TaskResumed,
    TaskCheckpoint,
    TaskValidationStarted,
    TaskValidationPassed,
    TaskValidationFailed,
    TaskCompleted,
    TaskFailed,
    TaskCancelled,
    /// P6 engineering intelligence: meaningful repository-index events.
    /// Only emitted for completed/failed index runs, analyses, and
    /// discoveries — never for routine internal operations (no recursion,
    /// no per-file events). Append-only like all history.
    IndexCompleted,
    IndexFailed,
    ImpactAnalyzed,
    HealthAnalyzed,
    RepositoryDiscovered,
    /// P9 engineering outcome: structured outcome evidence reported by
    /// OpenCode (or confirmed by the user) and bound to a durable task.
    /// The classification (success/partial/failure/rejected/superseded)
    /// travels in the event's outcome label + payload; interpretation
    /// stays with P3 learning. Append-only like all history.
    TaskOutcome,
}

impl HistoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            HistoryKind::SessionStarted => "session_started",
            HistoryKind::SessionEnded => "session_ended",
            HistoryKind::UserMessage => "user_message",
            HistoryKind::AssistantMessage => "assistant_message",
            HistoryKind::ToolExecution => "tool_execution",
            HistoryKind::ToolResult => "tool_result",
            HistoryKind::Decision => "decision",
            HistoryKind::ChangeApplied => "change_applied",
            HistoryKind::Validation => "validation",
            HistoryKind::Error => "error",
            HistoryKind::Observation => "observation",
            HistoryKind::TaskCreated => "task_created",
            HistoryKind::TaskStarted => "task_started",
            HistoryKind::TaskPaused => "task_paused",
            HistoryKind::TaskResumed => "task_resumed",
            HistoryKind::TaskCheckpoint => "task_checkpoint",
            HistoryKind::TaskValidationStarted => "task_validation_started",
            HistoryKind::TaskValidationPassed => "task_validation_passed",
            HistoryKind::TaskValidationFailed => "task_validation_failed",
            HistoryKind::TaskCompleted => "task_completed",
            HistoryKind::TaskFailed => "task_failed",
            HistoryKind::TaskCancelled => "task_cancelled",
            HistoryKind::IndexCompleted => "index_completed",
            HistoryKind::IndexFailed => "index_failed",
            HistoryKind::ImpactAnalyzed => "impact_analyzed",
            HistoryKind::HealthAnalyzed => "health_analyzed",
            HistoryKind::RepositoryDiscovered => "repository_discovered",
            HistoryKind::TaskOutcome => "task_outcome",
        }
    }

    /// Deterministic importance for recall ranking (higher first).
    /// Decisions, validations, and errors outrank chatter: recall answers
    /// "why did we choose this" and "did this fail before" before
    /// "what tool ran". P6 index events are mid-importance structural
    /// evidence: useful for "when was this indexed" without outranking
    /// decisions or validations.
    pub fn importance(&self) -> u8 {
        match self {
            HistoryKind::Decision => 100,
            HistoryKind::Validation => 80,
            HistoryKind::Error => 80,
            HistoryKind::TaskCompleted
            | HistoryKind::TaskFailed
            | HistoryKind::TaskCancelled
            | HistoryKind::TaskValidationPassed
            | HistoryKind::TaskValidationFailed => 80,
            HistoryKind::TaskOutcome => 80,
            HistoryKind::ChangeApplied => 70,
            HistoryKind::TaskCreated
            | HistoryKind::TaskStarted
            | HistoryKind::TaskPaused
            | HistoryKind::TaskResumed
            | HistoryKind::TaskValidationStarted => 60,
            HistoryKind::TaskCheckpoint => 50,
            HistoryKind::SessionStarted | HistoryKind::SessionEnded => 60,
            HistoryKind::UserMessage => 60,
            HistoryKind::AssistantMessage => 50,
            HistoryKind::ToolResult => 40,
            HistoryKind::ToolExecution => 30,
            HistoryKind::Observation => 20,
            HistoryKind::IndexCompleted
            | HistoryKind::IndexFailed
            | HistoryKind::ImpactAnalyzed
            | HistoryKind::HealthAnalyzed
            | HistoryKind::RepositoryDiscovered => 55,
        }
    }
}

impl std::fmt::Display for HistoryKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for HistoryKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "session_started" => Ok(HistoryKind::SessionStarted),
            "session_ended" => Ok(HistoryKind::SessionEnded),
            "user_message" => Ok(HistoryKind::UserMessage),
            "assistant_message" => Ok(HistoryKind::AssistantMessage),
            "tool_execution" => Ok(HistoryKind::ToolExecution),
            "tool_result" => Ok(HistoryKind::ToolResult),
            "decision" => Ok(HistoryKind::Decision),
            "change_applied" => Ok(HistoryKind::ChangeApplied),
            "validation" => Ok(HistoryKind::Validation),
            "error" => Ok(HistoryKind::Error),
            "observation" => Ok(HistoryKind::Observation),
            "task_created" => Ok(HistoryKind::TaskCreated),
            "task_started" => Ok(HistoryKind::TaskStarted),
            "task_paused" => Ok(HistoryKind::TaskPaused),
            "task_resumed" => Ok(HistoryKind::TaskResumed),
            "task_checkpoint" => Ok(HistoryKind::TaskCheckpoint),
            "task_validation_started" => Ok(HistoryKind::TaskValidationStarted),
            "task_validation_passed" => Ok(HistoryKind::TaskValidationPassed),
            "task_validation_failed" => Ok(HistoryKind::TaskValidationFailed),
            "task_completed" => Ok(HistoryKind::TaskCompleted),
            "task_failed" => Ok(HistoryKind::TaskFailed),
            "task_cancelled" => Ok(HistoryKind::TaskCancelled),
            "index_completed" => Ok(HistoryKind::IndexCompleted),
            "index_failed" => Ok(HistoryKind::IndexFailed),
            "impact_analyzed" => Ok(HistoryKind::ImpactAnalyzed),
            "health_analyzed" => Ok(HistoryKind::HealthAnalyzed),
            "repository_discovered" => Ok(HistoryKind::RepositoryDiscovered),
            "task_outcome" => Ok(HistoryKind::TaskOutcome),
            other => Err(format!("unknown history kind: {other}")),
        }
    }
}

/// Input to [`ContextStore::record_history`]: one meaningful historical event.
///
/// Structural capture only — what happened, when, where, who produced it,
/// what task/project it belonged to. Timestamps are explicit (`created_at`
/// override) so out-of-order arrivals keep deterministic history order;
/// when absent the store uses `now`.
#[derive(Debug, Clone)]
pub struct HistoryInput {
    pub session_id: Option<String>,
    pub workspace_root: String,
    pub task_id: Option<String>,
    pub kind: HistoryKind,
    pub tool: Option<String>,
    pub path: Option<String>,
    pub outcome: Option<String>,
    /// Short human-readable line (≤ [`MAX_HISTORY_SUMMARY_CHARS`]; longer
    /// input is truncated with a marker, never refused — passive capture
    /// must not fail on a long tool result).
    pub summary: Option<String>,
    /// Detail blob (≤ [`MAX_EVENT_PAYLOAD_BYTES`]; truncated with a marker
    /// when longer, metadata preserved).
    pub payload: Option<String>,
    /// Idempotency key: recording the same key twice returns the original
    /// event id instead of duplicating history.
    pub dedup_key: Option<String>,
    /// Producer label (e.g. `mcp:remember`).
    pub source: Option<String>,
    /// Explicit event time (out-of-order arrivals); `None` = `now`.
    pub created_at: Option<u64>,
}

impl HistoryInput {
    pub fn new(
        workspace_root: impl Into<String>,
        kind: HistoryKind,
        summary: impl Into<String>,
    ) -> Self {
        HistoryInput {
            session_id: None,
            workspace_root: workspace_root.into(),
            task_id: None,
            kind,
            tool: None,
            path: None,
            outcome: None,
            summary: Some(summary.into()),
            payload: None,
            dedup_key: None,
            source: None,
            created_at: None,
        }
    }
}

/// Metadata for opening a session.
#[derive(Debug, Clone, Default)]
pub struct OpenSession {
    pub task_id: Option<String>,
    pub title: Option<String>,
    pub source: Option<String>,
    pub parent_session_id: Option<String>,
}

/// Filter for listing sessions.
#[derive(Debug, Clone, Default)]
pub struct SessionFilter {
    pub task_id: Option<String>,
    pub status: Option<SessionStatus>,
    pub limit: usize,
}

// Process-local collision-avoidance counter for session ids. Restart-safe:
// the id embeds wall-clock time, and existence is checked before use.
static SESSION_COUNTER: AtomicU64 = AtomicU64::new(0);

fn mint_session_id(conn: &rusqlite::Connection, now: u64) -> Result<String, ContextError> {
    let pid = std::process::id() as u64;
    for _ in 0..1000 {
        let n = SESSION_COUNTER.fetch_add(1, Ordering::Relaxed);
        // Mix time + pid + counter: unique across restarts (time moves on),
        // across processes (pid), and within a process (counter).
        let mut x = now
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            .wrapping_add(pid.wrapping_mul(0xBF58_476D_1CE4_E5B9))
            .wrapping_add(n.wrapping_mul(0x94D0_49BB_1331_11EB));
        // xorshift64* finalizer for diffusion.
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        x = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        let candidate = format!("ses::{x:016x}");
        let exists: bool = conn
            .query_row("SELECT 1 FROM sessions WHERE id = ?1", [&candidate], |_| {
                Ok(())
            })
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?
            .is_some();
        if !exists {
            return Ok(candidate);
        }
    }
    Err(ContextError::Validation(
        "could not mint a unique session id after 1000 attempts".to_string(),
    ))
}

/// Insert one already-normalized event inside the caller's transaction:
/// dedup-key probe, session existence check + heartbeat bump, digest,
/// canonical row insert, FTS sync. The caller owns `commit`.
///
/// Returns `(event_id, duplicate)`. On duplicate nothing is written.
/// This is the in-transaction core shared by `record_history` (its own
/// transaction) and P5 task mutations (state row + event commit together
/// — a task transition must never land without its history event, and
/// an event must never outlive a rolled-back transition).
pub(crate) fn record_event_in_tx(
    tx: &rusqlite::Connection,
    event: &EventRecord,
    created_at: u64,
) -> Result<(i64, bool), ContextError> {
    if let Some(key) = event.dedup_key.as_deref() {
        // Idempotent replay: the same key returns the original id.
        let prior: Option<i64> = tx
            .query_row("SELECT id FROM events WHERE dedup_key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?;
        if let Some(id) = prior {
            return Ok((id, true));
        }
    }
    if let Some(sid) = event.session_id.as_deref() {
        let exists: bool = tx
            .query_row("SELECT 1 FROM sessions WHERE id = ?1", [sid], |_| Ok(()))
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?
            .is_some();
        if !exists {
            return Err(ContextError::Validation(format!(
                "history session {sid} does not exist: open it first"
            )));
        }
    }
    let digest = event
        .payload
        .as_deref()
        .map(|p| crate::store::hex_sha256_for(p.as_bytes()));
    tx.execute(
        "INSERT INTO events (session_id, workspace_root, task_id, kind, tool, path,
            outcome, summary, payload_json, dedup_key, source, digest, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
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
            created_at as i64,
        ],
    )?;
    let id = tx.last_insert_rowid();
    if let Some(sid) = event.session_id.as_deref() {
        tx.execute(
            "UPDATE sessions SET event_count = event_count + 1, updated_at = ?1
             WHERE id = ?2",
            params![created_at as i64, sid],
        )?;
    }
    crate::store::sync_history_fts(
        tx,
        id,
        event.summary.as_deref(),
        event.payload.as_deref(),
        &event.kind,
        event.tool.as_deref(),
        event.outcome.as_deref(),
    )?;
    Ok((id, false))
}

/// Redact then truncate a free-text history field. Redaction runs first so
/// a secret is never preserved by a truncation boundary, and truncation is
/// marked so excerpts are honest about being excerpts.
pub(crate) fn clean_text(raw: &str, max_chars: usize) -> String {
    let redacted = codebro_core::tools::shell::redact_secrets_public(raw);
    if redacted.chars().count() <= max_chars {
        return redacted;
    }
    format!(
        "{}{}",
        redacted.chars().take(max_chars).collect::<String>(),
        HISTORY_TRUNCATION_MARKER
    )
}

/// Truncate a payload by bytes (payloads are byte-bounded blobs).
pub(crate) fn clean_payload(raw: &str) -> String {
    let redacted = codebro_core::tools::shell::redact_secrets_public(raw);
    if redacted.len() <= MAX_EVENT_PAYLOAD_BYTES {
        return redacted;
    }
    // Cut on a char boundary, then mark.
    let mut cut = MAX_EVENT_PAYLOAD_BYTES;
    while cut > 0 && !redacted.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", &redacted[..cut], HISTORY_TRUNCATION_MARKER)
}

fn validate_history_input(input: &HistoryInput) -> Result<(), ContextError> {
    if input.workspace_root.trim().is_empty() {
        return Err(ContextError::Validation(
            "history workspace_root must not be empty".to_string(),
        ));
    }
    if let Some(task) = input.task_id.as_deref() {
        if task.trim().is_empty() {
            return Err(ContextError::Validation(
                "history task_id must not be blank when supplied".to_string(),
            ));
        }
        if task.chars().count() > MAX_TASK_ID_CHARS {
            return Err(ContextError::Validation(format!(
                "history task_id exceeds {MAX_TASK_ID_CHARS} characters"
            )));
        }
    }
    if let Some(key) = input.dedup_key.as_deref() {
        if key.trim().is_empty() {
            return Err(ContextError::Validation(
                "history dedup_key must not be blank when supplied".to_string(),
            ));
        }
        if key.chars().count() > MAX_DEDUP_KEY_CHARS {
            return Err(ContextError::Validation(format!(
                "history dedup_key exceeds {MAX_DEDUP_KEY_CHARS} characters"
            )));
        }
    }
    if let Some(source) = input.source.as_deref() {
        if source.chars().count() > MAX_HISTORY_SOURCE_CHARS {
            return Err(ContextError::Validation(format!(
                "history source exceeds {MAX_HISTORY_SOURCE_CHARS} characters"
            )));
        }
    }
    Ok(())
}

/// One canonical history row for FTS backfill/rebuild: (id, summary,
/// payload, kind, tool, outcome).
type HistoryFtsRow = (
    i64,
    Option<String>,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
);

pub(crate) fn row_to_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRecord> {
    let status_raw: String = row.get(4)?;
    Ok(SessionRecord {
        id: row.get(0)?,
        workspace_root: row.get(1)?,
        task_id: row.get(2)?,
        title: row.get(3)?,
        status: status_raw.parse().unwrap_or(SessionStatus::Active),
        source: row.get(5)?,
        parent_session_id: row.get(6)?,
        started_at: row.get::<_, i64>(7)? as u64,
        updated_at: row.get::<_, i64>(8)? as u64,
        ended_at: row.get::<_, Option<i64>>(9)?.map(|t| t as u64),
        end_reason: row.get(10)?,
        event_count: row.get::<_, i64>(11)? as u64,
    })
}

const SESSION_COLUMNS: &str = "id, workspace_root, task_id, title, status, source,
    parent_session_id, opened_at, updated_at, closed_at, end_reason, event_count";

impl ContextStore {
    // ── Sessions ──────────────────────────────────────────────────────

    /// Open a persistent session. The id is minted (`ses::<hex>`), the
    /// workspace canonicalized, and restarts are safe: reopening the same
    /// database never duplicates — callers resume via [`ContextStore::get_session`]
    /// or [`ContextStore::ensure_active_session`].
    pub fn open_session(
        &self,
        workspace_root: &str,
        meta: &OpenSession,
        now: u64,
    ) -> Result<SessionRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        if ws.is_empty() {
            return Err(ContextError::Validation(
                "session workspace_root must not be empty".to_string(),
            ));
        }
        let task_id = meta
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(parent) = meta.parent_session_id.as_deref() {
            if parent.trim().is_empty() {
                return Err(ContextError::Validation(
                    "parent_session_id must not be blank when supplied".to_string(),
                ));
            }
        }
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let id = mint_session_id(&tx, now)?;
            tx.execute(
                "INSERT INTO sessions (id, workspace_root, task_id, title, status, source,
                    parent_session_id, opened_at, updated_at, closed_at, end_reason, event_count)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, NULL, NULL, 0)",
                params![
                    id,
                    ws,
                    task_id.as_deref(),
                    meta.title.as_deref(),
                    SessionStatus::Active.as_str(),
                    meta.source.as_deref(),
                    meta.parent_session_id.as_deref(),
                    now as i64,
                ],
            )?;
            tx.commit()?;
            Ok(SessionRecord {
                id,
                workspace_root: ws,
                task_id,
                title: meta.title.clone(),
                source: meta.source.clone(),
                status: SessionStatus::Active,
                parent_session_id: meta.parent_session_id.clone(),
                started_at: now,
                updated_at: now,
                ended_at: None,
                end_reason: None,
                event_count: 0,
            })
        })
    }

    /// Fetch a session by id. Survives restarts: the row is durable.
    pub fn get_session(&self, id: &str) -> Result<Option<SessionRecord>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
                [id],
                row_to_session,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// Touch a session's `updated_at` (heartbeat). Returns false when absent.
    pub fn touch_session(&self, id: &str, now: u64) -> Result<bool, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
                params![now as i64, id],
            )?;
            Ok(updated > 0)
        })
    }

    /// Close a session terminally. Only `Active` sessions can close —
    /// a second close is refused (history must not rewrite an ending),
    /// and an unknown id errors. Crash-interrupted sessions are never
    /// auto-closed here; they surface via [`ContextStore::stale_sessions`].
    pub fn close_session(
        &self,
        id: &str,
        status: SessionStatus,
        reason: Option<&str>,
        now: u64,
    ) -> Result<SessionRecord, ContextError> {
        if !status.is_terminal() {
            return Err(ContextError::Validation(
                "close_session requires a terminal status (completed or abandoned)".to_string(),
            ));
        }
        self.with_conn(|conn| {
            let current: String = conn
                .query_row("SELECT status FROM sessions WHERE id = ?1", [id], |row| {
                    row.get(0)
                })
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?
                .ok_or_else(|| ContextError::Validation(format!("session {id} does not exist")))?;
            if current != SessionStatus::Active.as_str() {
                return Err(ContextError::Validation(format!(
                    "only an active session can be closed (session {id} is {current})"
                )));
            }
            conn.execute(
                "UPDATE sessions SET status = ?1, closed_at = ?2, updated_at = ?2, end_reason = ?3
                 WHERE id = ?4",
                params![status.as_str(), now as i64, reason, id],
            )?;
            let record = conn
                .query_row(
                    &format!("SELECT {SESSION_COLUMNS} FROM sessions WHERE id = ?1"),
                    [id],
                    row_to_session,
                )
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(record)
        })
    }

    /// List sessions for a workspace, newest first. Task and status narrow
    /// the view; without a task, task-bound sessions are included (session
    /// listing is inventory, not recall — recall enforces task invisibility).
    pub fn list_sessions(
        &self,
        workspace_root: &str,
        filter: &SessionFilter,
        _now: u64,
    ) -> Result<Vec<SessionRecord>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        let limit = filter.limit.clamp(1, 200) as i64;
        let status = filter.status.map(|s| s.as_str().to_string());
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions
                 WHERE workspace_root = ?1
                   AND (?2 IS NULL OR task_id = ?2)
                   AND (?3 IS NULL OR status = ?3)
                 ORDER BY opened_at DESC, id DESC LIMIT ?4"
            ))?;
            let rows = stmt
                .query_map(
                    params![ws, filter.task_id.as_deref(), status.as_deref(), limit],
                    row_to_session,
                )?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Newest active session for a (workspace, task) pair, if any.
    /// Task matching is exact: a task viewpoint never resumes another
    /// task's session.
    pub fn active_session_for(
        &self,
        workspace_root: &str,
        task_id: Option<&str>,
        _now: u64,
    ) -> Result<Option<SessionRecord>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        let task = task_id.map(str::trim).filter(|s| !s.is_empty());
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {SESSION_COLUMNS} FROM sessions
                     WHERE workspace_root = ?1
                       AND status = ?2
                       AND ((?3 IS NULL AND task_id IS NULL) OR task_id = ?3)
                     ORDER BY opened_at DESC, id DESC LIMIT 1"
                ),
                params![ws, SessionStatus::Active.as_str(), task],
                row_to_session,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// Resume the newest active session for (workspace, task), or open one.
    /// This is the restart-safe seam: a process that exits unexpectedly and
    /// restarts resumes the same session instead of duplicating it.
    /// Returns the session plus whether it was newly created.
    pub fn ensure_active_session(
        &self,
        workspace_root: &str,
        task_id: Option<&str>,
        source: Option<&str>,
        now: u64,
    ) -> Result<(SessionRecord, bool), ContextError> {
        if let Some(existing) = self.active_session_for(workspace_root, task_id, now)? {
            return Ok((existing, false));
        }
        let meta = OpenSession {
            task_id: task_id.map(str::to_string),
            title: None,
            source: source.map(str::to_string),
            parent_session_id: None,
        };
        self.open_session(workspace_root, &meta, now)
            .map(|s| (s, true))
    }

    /// Active sessions untouched for longer than [`STALE_AFTER_SECS`] —
    /// the crash-interruption inventory. Never auto-completed: an
    /// interrupted session "did not complete successfully" and must say so.
    pub fn stale_sessions(
        &self,
        workspace_root: Option<&str>,
        now: u64,
    ) -> Result<Vec<SessionRecord>, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        let cutoff = now.saturating_sub(STALE_AFTER_SECS) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {SESSION_COLUMNS} FROM sessions
                 WHERE status = ?1 AND updated_at < ?2
                   AND (?3 IS NULL OR workspace_root = ?3)
                 ORDER BY updated_at ASC, id ASC LIMIT 200"
            ))?;
            let rows = stmt
                .query_map(
                    params![SessionStatus::Active.as_str(), cutoff, ws.as_deref()],
                    row_to_session,
                )?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    // ── History ───────────────────────────────────────────────────────

    /// Record one meaningful historical event.
    ///
    /// The write seam for P2 history: canonicalizes the workspace, redacts
    /// secrets from summary and payload *before* storage (canonical row and
    /// FTS alike), truncates oversized text with a marker (history capture
    /// must not fail on a long tool result — use [`ContextStore::append_event`]
    /// when refusal is the right policy), links and touches the session
    /// (bumping its event count), and syncs the derived FTS index — all in
    /// one transaction.
    ///
    /// Idempotency: when `dedup_key` is supplied and was seen before, the
    /// original event id is returned with `duplicate = true` and nothing is
    /// written. Events may carry explicit `created_at` (out-of-order
    /// arrivals); ordering is always (`created_at`, `id`), never insertion
    /// order.
    ///
    /// Returns `(event_id, duplicate)`.
    pub fn record_history(
        &self,
        input: &HistoryInput,
        now: u64,
    ) -> Result<(i64, bool), ContextError> {
        validate_history_input(input)?;
        let ws = canonical_workspace_key(&input.workspace_root);
        if ws.is_empty() {
            return Err(ContextError::Validation(
                "history workspace_root must not be empty".to_string(),
            ));
        }
        let task_id = input
            .task_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let summary = input
            .summary
            .as_deref()
            .map(|s| clean_text(s, MAX_HISTORY_SUMMARY_CHARS))
            .filter(|s| !s.trim().is_empty());
        let payload = input
            .payload
            .as_deref()
            .map(clean_payload)
            .filter(|s| !s.is_empty());
        let created_at = input.created_at.unwrap_or(now);
        let event = EventRecord {
            id: None,
            session_id: input.session_id.clone(),
            workspace_root: ws,
            task_id,
            kind: input.kind.as_str().to_string(),
            tool: input.tool.clone(),
            path: input.path.clone(),
            outcome: input.outcome.clone(),
            summary,
            payload,
            dedup_key: input.dedup_key.clone(),
            source: input.source.clone(),
            digest: None,
            created_at,
        };
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let (id, dup) = record_event_in_tx(&tx, &event, created_at)?;
            if dup {
                // Idempotent replay committed nothing; the original row is
                // the answer.
                tx.commit()?;
                return Ok((id, true));
            }
            tx.commit()?;
            Ok((id, false))
        })
    }

    /// Events of one session in deterministic history order
    /// (`created_at`, then `id`) — insertion order is never assumed.
    pub fn list_session_events(
        &self,
        session_id: &str,
        limit: usize,
    ) -> Result<Vec<EventRecord>, ContextError> {
        let limit = limit.clamp(1, 500) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM events WHERE session_id = ?1
                 ORDER BY created_at ASC, id ASC LIMIT ?2",
                crate::store::EVENT_COLUMNS
            ))?;
            let rows = stmt
                .query_map(params![session_id, limit], crate::store::row_to_event_pub)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Rebuild the derived history FTS index from canonical events.
    /// Canonical history is authoritative; FTS is disposable. Returns the
    /// number of events re-indexed.
    pub fn rebuild_history_fts(&self) -> Result<usize, ContextError> {
        self.with_conn(|conn| {
            let rows: Vec<HistoryFtsRow> = {
                let mut stmt = conn
                    .prepare("SELECT id, summary, payload_json, kind, tool, outcome FROM events")?;
                let mapped = stmt.query_map([], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                })?;
                let collected: Vec<HistoryFtsRow> = mapped
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                collected
            };
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM events_fts", [])?;
            let mut count = 0;
            for (id, summary, payload, kind, tool, outcome) in &rows {
                crate::store::sync_history_fts(
                    &tx,
                    *id,
                    summary.as_deref(),
                    payload.as_deref(),
                    kind,
                    tool.as_deref(),
                    outcome.as_deref(),
                )?;
                count += 1;
            }
            tx.commit()?;
            Ok(count)
        })
    }
}

/// Number of history events visible from a workspace (passive-capture
/// smoke tests and the no-recursion invariant use this).
pub fn count_events_for_store(store: &ContextStore, workspace_root: &str) -> usize {
    store
        .list_events(workspace_root, 200)
        .map(|v| v.len())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, ContextStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        (dir, store)
    }

    fn history(ws: &str, kind: HistoryKind, summary: &str) -> HistoryInput {
        HistoryInput::new(ws, kind, summary)
    }

    #[test]
    fn session_kinds_and_statuses_roundtrip() {
        for k in [
            HistoryKind::SessionStarted,
            HistoryKind::SessionEnded,
            HistoryKind::UserMessage,
            HistoryKind::AssistantMessage,
            HistoryKind::ToolExecution,
            HistoryKind::ToolResult,
            HistoryKind::Decision,
            HistoryKind::ChangeApplied,
            HistoryKind::Validation,
            HistoryKind::Error,
            HistoryKind::Observation,
        ] {
            assert_eq!(k.to_string().parse::<HistoryKind>().unwrap(), k);
        }
        assert!("nope".parse::<HistoryKind>().is_err());
        for s in [
            SessionStatus::Active,
            SessionStatus::Completed,
            SessionStatus::Abandoned,
        ] {
            assert_eq!(s.to_string().parse::<SessionStatus>().unwrap(), s);
        }
        assert!("paused".parse::<SessionStatus>().is_err());
        assert!(SessionStatus::Completed.is_terminal());
        assert!(!SessionStatus::Active.is_terminal());
    }

    #[test]
    fn open_get_and_close_session() {
        let (_dir, store) = store();
        let meta = OpenSession {
            task_id: Some("task-1".to_string()),
            title: Some("sqlite decision".to_string()),
            source: Some("mcp".to_string()),
            parent_session_id: None,
        };
        let s = store.open_session("/work/repo", &meta, 1000).unwrap();
        assert!(s.id.starts_with("ses::"));
        assert_eq!(s.workspace_root, "/work/repo");
        assert_eq!(s.task_id.as_deref(), Some("task-1"));
        assert_eq!(s.status, SessionStatus::Active);
        assert_eq!(s.started_at, 1000);
        assert_eq!(s.event_count, 0);

        let got = store.get_session(&s.id).unwrap().unwrap();
        assert_eq!(got, s);

        let closed = store
            .close_session(&s.id, SessionStatus::Completed, Some("user_done"), 2000)
            .unwrap();
        assert_eq!(closed.status, SessionStatus::Completed);
        assert_eq!(closed.ended_at, Some(2000));
        assert_eq!(closed.end_reason.as_deref(), Some("user_done"));

        // Second close is refused: history never rewrites an ending.
        assert!(store
            .close_session(&s.id, SessionStatus::Abandoned, None, 3000)
            .is_err());
        // Closing with a non-terminal status is refused.
        let (_d2, s2) = (
            (),
            store
                .open_session("/work/repo", &OpenSession::default(), 3000)
                .unwrap(),
        );
        assert!(store
            .close_session(&s2.id, SessionStatus::Active, None, 3000)
            .is_err());
        // Unknown sessions error.
        assert!(store
            .close_session("ses::deadbeefdeadbeef", SessionStatus::Completed, None, 1)
            .is_err());
    }

    #[test]
    fn minted_session_ids_are_unique_portable_and_restart_safe() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(db::STATE_DB_FILE);
        let mut ids = std::collections::HashSet::new();
        let first: String;
        {
            let store = ContextStore::new(path.clone());
            for _ in 0..50 {
                let s = store
                    .open_session("/work", &OpenSession::default(), 1000)
                    .unwrap();
                assert!(!s.id.contains("/work"), "ids must be portable, not paths");
                assert!(ids.insert(s.id.clone()));
            }
            first = ids.iter().next().unwrap().clone();
        }
        // Restart: a fresh handle over the same file resumes the session.
        let store = ContextStore::new(path);
        assert!(store.get_session(&first).unwrap().is_some());
        let s = store
            .open_session("/work", &OpenSession::default(), 1000)
            .unwrap();
        assert!(!ids.contains(&s.id), "no duplicate after restart");
    }

    #[test]
    fn ensure_active_session_resumes_instead_of_duplicating() {
        let (_dir, store) = store();
        let (a, created) = store
            .ensure_active_session("/work", Some("t1"), Some("mcp"), 100)
            .unwrap();
        assert!(created);
        let (b, created) = store
            .ensure_active_session("/work", Some("t1"), Some("mcp"), 200)
            .unwrap();
        assert!(!created);
        assert_eq!(a.id, b.id);
        // A different task gets its own session; the workspace default too.
        let (c, created) = store
            .ensure_active_session("/work", Some("t2"), Some("mcp"), 200)
            .unwrap();
        assert!(created);
        assert_ne!(a.id, c.id);
        // After closing, ensure opens a fresh session (no resurrection).
        store
            .close_session(&a.id, SessionStatus::Completed, None, 300)
            .unwrap();
        let (d, created) = store
            .ensure_active_session("/work", Some("t1"), Some("mcp"), 400)
            .unwrap();
        assert!(created);
        assert_ne!(a.id, d.id);
    }

    #[test]
    fn interrupted_sessions_are_stale_never_completed() {
        let (_dir, store) = store();
        let s = store
            .open_session("/work", &OpenSession::default(), 1000)
            .unwrap();
        // Fresh: not stale.
        assert!(!s.is_stale(1000 + STALE_AFTER_SECS - 1));
        assert!(store
            .stale_sessions(Some("/work"), 1000)
            .unwrap()
            .is_empty());
        // Past the horizon: stale, still Active, still recoverable.
        let stale = store
            .stale_sessions(Some("/work"), 1000 + STALE_AFTER_SECS + 1)
            .unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].id, s.id);
        assert_eq!(stale[0].status, SessionStatus::Active);
        assert!(stale[0].is_stale(1000 + STALE_AFTER_SECS + 1));
        assert!(store.get_session(&s.id).unwrap().is_some());
        // A touch revives it.
        store
            .touch_session(&s.id, 1000 + STALE_AFTER_SECS + 2)
            .unwrap();
        assert!(store
            .stale_sessions(Some("/work"), 1000 + STALE_AFTER_SECS + 3)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn history_record_links_session_and_bumps_count() {
        let (_dir, store) = store();
        let s = store
            .open_session("/work", &OpenSession::default(), 10)
            .unwrap();
        let mut input = history(
            "/work",
            HistoryKind::Decision,
            "chose SQLite for local runtime",
        );
        input.session_id = Some(s.id.clone());
        input.outcome = Some("decided".to_string());
        input.source = Some("mcp:remember".to_string());
        let (id, dup) = store.record_history(&input, 20).unwrap();
        assert!(!dup);
        assert!(id > 0);
        let got = store.get_event(id).unwrap().unwrap();
        assert_eq!(got.kind, "decision");
        assert_eq!(got.session_id.as_deref(), Some(s.id.as_str()));
        assert_eq!(got.workspace_root, "/work");
        let s2 = store.get_session(&s.id).unwrap().unwrap();
        assert_eq!(s2.event_count, 1);
        assert_eq!(s2.updated_at, 20);
        // Unknown sessions are refused, not silently orphaned.
        let mut bad = history("/work", HistoryKind::Observation, "orphan");
        bad.session_id = Some("ses::0000000000000000".to_string());
        assert!(store.record_history(&bad, 21).is_err());
    }

    #[test]
    fn history_events_order_by_time_not_insertion() {
        let (_dir, store) = store();
        let s = store
            .open_session("/w", &OpenSession::default(), 1)
            .unwrap();
        for (summary, at) in [("third", 300), ("first", 100), ("second", 200)] {
            let mut input = history("/w", HistoryKind::Observation, summary);
            input.session_id = Some(s.id.clone());
            input.created_at = Some(at);
            store.record_history(&input, 999).unwrap();
        }
        let events = store.list_session_events(&s.id, 10).unwrap();
        let summaries: Vec<_> = events
            .iter()
            .map(|e| e.summary.as_deref().unwrap())
            .collect();
        assert_eq!(summaries, vec!["first", "second", "third"]);
    }

    #[test]
    fn duplicate_dedup_key_returns_original_id() {
        let (_dir, store) = store();
        let mut a = history("/w", HistoryKind::Validation, "cargo test passed");
        a.dedup_key = Some("ci:abc123".to_string());
        let (id1, dup1) = store.record_history(&a, 10).unwrap();
        assert!(!dup1);
        let mut b = history("/w", HistoryKind::Validation, "cargo test passed retry");
        b.dedup_key = Some("ci:abc123".to_string());
        let (id2, dup2) = store.record_history(&b, 11).unwrap();
        assert!(dup2);
        assert_eq!(id1, id2);
        assert_eq!(store.list_events("/w", 100).unwrap().len(), 1);
    }

    #[test]
    fn oversized_history_is_truncated_not_refused() {
        let (_dir, store) = store();
        let mut input = history("/w", HistoryKind::ToolResult, &"x".repeat(9000));
        input.payload = Some("y".repeat(40_000));
        let (id, _) = store.record_history(&input, 1).unwrap();
        let got = store.get_event(id).unwrap().unwrap();
        let summary = got.summary.unwrap();
        assert!(summary.ends_with(HISTORY_TRUNCATION_MARKER));
        assert!(summary.chars().count() <= MAX_HISTORY_SUMMARY_CHARS + 64);
        let payload = got.payload.unwrap();
        assert!(payload.ends_with(HISTORY_TRUNCATION_MARKER));
        assert!(payload.len() <= MAX_EVENT_PAYLOAD_BYTES + 64);
    }

    #[test]
    fn parent_session_links_continuation() {
        let (_dir, store) = store();
        let a = store
            .open_session("/w", &OpenSession::default(), 1)
            .unwrap();
        let b = store
            .open_session(
                "/w",
                &OpenSession {
                    parent_session_id: Some(a.id.clone()),
                    ..OpenSession::default()
                },
                2,
            )
            .unwrap();
        assert_eq!(b.parent_session_id.as_deref(), Some(a.id.as_str()));
    }

    #[test]
    fn secrets_never_reach_canonical_storage_or_fts() {
        let (_dir, store) = store();
        let secret = "sk-testsecretkey1234567890abcdef";
        let mut input = history(
            "/w",
            HistoryKind::ToolResult,
            &format!("deploy failed with api_key=\"{secret}\" bearer token"),
        );
        input.payload = Some(format!(
            "{{\"token\": \"ghp_abcdefghijklmnopqrst1234\", \"pw\": \"password={secret}\"}}"
        ));
        let (id, _) = store.record_history(&input, 1).unwrap();
        let got = store.get_event(id).unwrap().unwrap();
        let summary = got.summary.unwrap();
        let payload = got.payload.unwrap();
        assert!(
            !summary.contains(secret),
            "raw secret in summary: {summary}"
        );
        assert!(!payload.contains(secret), "raw secret in payload");
        assert!(!payload.contains("ghp_abcdefghijklmnopqrst1234"));
        assert!(summary.contains("[REDACTED]") || payload.contains("[REDACTED]"));
        // And absent from the derived FTS index too (raw table scan).
        let fts_hit: Vec<String> = store
            .recall(
                &crate::recall::RecallQuery {
                    query: "testsecretkey",
                    workspace_root: Some("/w"),
                    ..crate::recall::RecallQuery::default()
                },
                2,
            )
            .unwrap()
            .groups
            .into_iter()
            .flat_map(|g| g.hits.into_iter().map(|h| h.excerpt))
            .collect();
        assert!(
            fts_hit.iter().all(|e| !e.contains(secret)),
            "secret leaked through recall: {fts_hit:?}"
        );
    }

    #[test]
    fn binary_and_garbage_payloads_stay_bounded_and_valid() {
        let (_dir, store) = store();
        let garbage = "ok\x00\x01\x02 binary \u{1f600} emoji ".repeat(300) + &"z".repeat(20_000);
        let mut input = history("/w", HistoryKind::ToolResult, "weird output");
        input.payload = Some(garbage);
        let (id, _) = store.record_history(&input, 1).unwrap();
        let got = store.get_event(id).unwrap().unwrap();
        assert!(got.payload.unwrap().len() <= MAX_EVENT_PAYLOAD_BYTES + 64);
        // Database still valid and searchable afterwards.
        assert_eq!(store.list_events("/w", 10).unwrap().len(), 1);
    }

    #[test]
    fn workspace_variants_share_one_history() {
        // Canonicalization: trailing slashes and dot segments must not
        // split-brain sessions or history.
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let canonical = ws.canonicalize().unwrap().display().to_string();
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        let s = store
            .open_session(&format!("{canonical}/"), &OpenSession::default(), 1)
            .unwrap();
        assert_eq!(s.workspace_root, canonical);
        store
            .record_history(
                &history(
                    &format!("{canonical}/sub/.."),
                    HistoryKind::Decision,
                    "same project choice",
                ),
                2,
            )
            .unwrap();
        assert_eq!(store.list_events(&canonical, 10).unwrap().len(), 1);
        assert_eq!(
            store
                .list_events(&format!("{canonical}//"), 10)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn concurrent_history_writes_stay_consistent() {
        let dir = tempfile::tempdir().unwrap();
        let store = std::sync::Arc::new(ContextStore::new(dir.path().join(db::STATE_DB_FILE)));
        let s = store
            .open_session("/w", &OpenSession::default(), 1)
            .unwrap();
        let mut handles = Vec::new();
        for t in 0..8 {
            let store = std::sync::Arc::clone(&store);
            let sid = s.id.clone();
            handles.push(std::thread::spawn(move || {
                for i in 0..25 {
                    let mut e = history(
                        "/w",
                        HistoryKind::Observation,
                        &format!("t{t} event {i} concurrent history"),
                    );
                    e.session_id = Some(sid.clone());
                    store.record_history(&e, 10).unwrap();
                    // Reads during writes must never fail.
                    let _ = store.list_session_events(&sid, 500);
                }
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(store.list_session_events(&s.id, 500).unwrap().len(), 200);
        assert_eq!(store.get_session(&s.id).unwrap().unwrap().event_count, 200);
        // Restart after concurrent writes: everything durable.
        drop(store);
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        assert_eq!(store.list_session_events(&s.id, 500).unwrap().len(), 200);
    }

    #[test]
    fn two_connections_write_without_losing_history() {
        // Closest practical approximation of two processes: two handles
        // (two SQLite connections, WAL + busy timeout) racing writes.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(db::STATE_DB_FILE);
        let a = ContextStore::new(path.clone());
        let b = ContextStore::new(path.clone());
        let sa = a.open_session("/w", &OpenSession::default(), 1).unwrap();
        let (id_a, _) = a
            .record_history(
                &{
                    let mut e = history("/w", HistoryKind::Decision, "writer alpha decision");
                    e.session_id = Some(sa.id.clone());
                    e
                },
                2,
            )
            .unwrap();
        let (id_b, _) = b
            .record_history(
                &history("/w", HistoryKind::Observation, "writer beta observation"),
                3,
            )
            .unwrap();
        assert_ne!(id_a, id_b);
        assert!(a.get_event(id_b).unwrap().is_some());
        assert!(b.get_event(id_a).unwrap().is_some());
    }
}

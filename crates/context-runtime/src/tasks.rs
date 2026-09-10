//! P5: durable engineering task runtime.
//!
//! A task is a bounded piece of engineering work with a persistent
//! lifecycle, context, checkpoints, and outcome. It is NOT a chat
//! session, a context record, a skill, or a generic TODO: sessions may
//! end while a task continues, and a process may die while the task
//! remains recoverable.
//!
//! ```text
//! OpenCode (executor, reasoning, coding)
//!    │  task create/start/checkpoint/… through CodeBro
//!    ▼
//! tasks (SQLite state.db, schema v6)  ←── task_checkpoints (immutable)
//!    │  every transition + checkpoint writes a P2 history event
//!    ▼  (same transaction)
//! events / events_fts → P3 learning evidence → future context/skills
//! ```
//!
//! # Core model
//!
//! - [`TaskStatus`] strict state machine with a validated transition
//!   matrix; `resumed` is normalized into `running` (a transition, not a
//!   state) so the stored lifecycle stays minimal.
//! - [`TaskRecord`] holds identity (opaque `task::<hex>` id), workspace,
//!   intent/parent references (soft, non-duplicating), lifecycle
//!   timestamps, optimistic `current_version`, worker lease, validation
//!   state, and the terminal [`TaskOutcome`].
//! - [`TaskCheckpoint`] is bounded, immutable resume state (never a
//!   transcript): what was completed, what remains, what was decided,
//!   what failed, what should happen next. New checkpoint = new version
//!   row; the task's mutable pointer `current_checkpoint_id` moves
//!   forward only.
//! - Worker ownership is a lease: a worker id + expiry + fencing version.
//!   A stale worker (its `lease_version` fell behind) can never
//!   overwrite a newer owner's state — every mutation carries the
//!   caller's lease version and the store refuses mismatches.
//!
//! # Invariants (all store-enforced)
//!
//! - **Workspace isolation**: tasks are workspace-bound; every read and
//!   mutation re-verifies the canonical workspace. Cross-workspace
//!   access is refused.
//! - **State machine**: transitions are validated server-side against a
//!   fixed matrix; callers never supply the next status directly.
//! - **Completion gate**: `completed` requires a recorded validation
//!   result (`validating` → passed). A caller cannot skip validation.
//! - **Optimistic concurrency**: mutations carry `based_on_version`;
//!   stale writers are refused, never silently merged.
//! - **Fencing**: mutations carry the caller's `lease_version`; a
//!   worker whose lease was taken over (version advanced) is refused.
//! - **Immutability**: checkpoint rows are plain INSERT + unique
//!   `(task_id, version)`; published checkpoints are never modified.
//! - **Atomicity**: task row + checkpoint row + history event commit in
//!   one transaction — the durable state never contradicts the recorded
//!   history.
//! - **Redaction**: every free-text field is redacted through the P2
//!   history seam (`record_event_in_tx` re-runs the same redaction).
//! - **Request-driven**: no background worker, no scheduler, no polling.
//!   Stale RUNNING tasks are derived at read time and exposed as
//!   recoverable work; only an explicit `resume` continues them.
//!
//! # Relationship policy
//!
//! - **Sessions (P2)**: a task survives sessions. Task events reference
//!   the task id; sessions stay independent (their own table).
//! - **Intent (P1)**: a task may reference an intent record id. If the
//!   intent is terminal (completed/cancelled/superseded) the task
//!   continues but `inspect` surfaces the mismatch — CodeBro never
//!   silently destroys or rewrites intent history, and never forces the
//!   task to pause.
//! - **Skills (P4)**: usage association only (`skill_refs_json`); the
//!   task runtime never executes skills and never mutates skill health
//!   from task outcomes (one task outcome is evidence, not proof).
//! - **Learning (P3)**: task lifecycle events are ordinary history
//!   events; P3 consumes them through its existing evidence rules
//!   (kinds map into `validation`/`observation` groups). Task outcomes
//!   never bypass P3 trust.

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::params;
use rusqlite::Connection;
use rusqlite::OptionalExtension;

use crate::history::record_event_in_tx;
use crate::history::HistoryInput;
use crate::store::ContextError;
use crate::store::ContextStore;
use crate::types::EventRecord;
use crate::workspace::canonical_workspace_key;
use crate::HistoryKind;

// ── Bounds ─────────────────────────────────────────────────────────────────

/// Maximum characters in a task title.
pub const MAX_TASK_TITLE_CHARS: usize = 512;
/// Maximum characters in a task description.
pub const MAX_TASK_DESCRIPTION_CHARS: usize = 4096;
/// Maximum characters in a checkpoint summary.
pub const MAX_CHECKPOINT_SUMMARY_CHARS: usize = 4096;
/// Maximum characters in a checkpoint progress / next-action field.
pub const MAX_CHECKPOINT_FIELD_CHARS: usize = 4096;
/// Maximum characters in a checkpoint metadata JSON blob.
pub const MAX_CHECKPOINT_METADATA_CHARS: usize = 8192;
/// Maximum skill references a task may carry.
pub const MAX_TASK_SKILL_REFS: usize = 16;
/// Maximum characters in a skill reference string.
pub const MAX_SKILL_REF_CHARS: usize = 256;
/// Maximum characters in a P9 outcome summary (caller-authored).
pub const MAX_OUTCOME_SUMMARY_CHARS: usize = 2048;
/// Maximum characters in P9 outcome evidence detail (caller-authored).
pub const MAX_OUTCOME_EVIDENCE_CHARS: usize = 2048;
/// Maximum characters in a P9 outcome command identity (caller-authored).
pub const MAX_OUTCOME_COMMAND_CHARS: usize = 512;
/// Maximum changed-area references a P9 outcome may carry.
pub const MAX_OUTCOME_CHANGED_AREAS: usize = 32;
/// Maximum characters per P9 outcome changed-area reference.
pub const MAX_OUTCOME_CHANGED_AREA_CHARS: usize = 256;
/// Maximum characters in a P9 outcome idempotency key (caller-supplied).
pub const MAX_OUTCOME_DEDUP_KEY_CHARS: usize = 256;
/// Default worker-lease TTL (seconds). A worker that has not renewed
/// within this window can be taken over by an explicit resume.
pub const TASK_LEASE_TTL_SECS: u64 = 15 * 60;

// ── Lifecycle ─────────────────────────────────────────────────────────────

/// Task lifecycle status. `resumed` transitions normalize to `running`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Paused,
    Validating,
    Completed,
    Failed,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Running => "running",
            TaskStatus::Paused => "paused",
            TaskStatus::Validating => "validating",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Cancelled
        )
    }
}

impl std::fmt::Display for TaskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for TaskStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pending" => Ok(TaskStatus::Pending),
            "running" => Ok(TaskStatus::Running),
            "paused" => Ok(TaskStatus::Paused),
            "validating" => Ok(TaskStatus::Validating),
            "completed" => Ok(TaskStatus::Completed),
            "failed" => Ok(TaskStatus::Failed),
            "cancelled" => Ok(TaskStatus::Cancelled),
            other => Err(format!("unknown task status: {other}")),
        }
    }
}

/// The validated transition matrix. Entry (from, to) → allowed.
///
/// ```text
/// pending   → running, cancelled
/// running   → paused, validating, failed, cancelled
/// paused    → running (resume), cancelled
/// validating→ running (validation failed → resume work), completed,
///             failed, cancelled
/// completed/failed/cancelled → (terminal; nothing)
/// ```
pub fn task_transition_allowed(from: TaskStatus, to: TaskStatus) -> bool {
    matches!(
        (from, to),
        (TaskStatus::Pending, TaskStatus::Running)
            | (TaskStatus::Pending, TaskStatus::Cancelled)
            | (TaskStatus::Running, TaskStatus::Paused)
            | (TaskStatus::Running, TaskStatus::Validating)
            | (TaskStatus::Running, TaskStatus::Failed)
            | (TaskStatus::Running, TaskStatus::Cancelled)
            | (TaskStatus::Paused, TaskStatus::Running)
            | (TaskStatus::Paused, TaskStatus::Cancelled)
            | (TaskStatus::Validating, TaskStatus::Running)
            | (TaskStatus::Validating, TaskStatus::Completed)
            | (TaskStatus::Validating, TaskStatus::Failed)
            | (TaskStatus::Validating, TaskStatus::Cancelled)
    )
}

/// Task priority (organizational only; no scheduling semantics).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    High,
    Medium,
    Low,
}

impl TaskPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskPriority::High => "high",
            TaskPriority::Medium => "medium",
            TaskPriority::Low => "low",
        }
    }
}

impl std::str::FromStr for TaskPriority {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "high" => Ok(TaskPriority::High),
            "medium" | "med" => Ok(TaskPriority::Medium),
            "low" => Ok(TaskPriority::Low),
            other => Err(format!("unknown task priority: {other}")),
        }
    }
}

// ── Records ───────────────────────────────────────────────────────────────

/// One durable engineering task.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskRecord {
    /// Opaque id (`task::<16 hex>`), minted store-side. Never a path or
    /// a database rowid.
    pub task_id: String,
    /// Canonical workspace root (storage key; isolation boundary).
    pub workspace_root: String,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub priority: TaskPriority,
    /// Optional P1 intent record id (reference, never duplicated).
    pub intent_record_id: Option<String>,
    /// Optional parent task id (organizational nesting only).
    pub parent_task_id: Option<String>,
    /// Explicit idempotency key: creating with the same (workspace, key)
    /// returns the same task instead of duplicating.
    pub idempotency_key: Option<String>,
    /// Optimistic concurrency version; bumped on every mutation.
    pub current_version: u64,
    /// Mutable pointer to the latest immutable checkpoint.
    pub current_checkpoint_id: Option<String>,
    /// Lease owner (worker id) while running/validating.
    pub lease_worker: Option<String>,
    /// Lease expiry (unix seconds); expired leases are take-over-able.
    pub lease_expires_at: Option<u64>,
    /// Fencing token: incremented on every lease acquisition/transfer.
    /// Mutations must carry the caller's version; a stale worker is
    /// refused.
    pub lease_version: u64,
    /// Last lease heartbeat (unix seconds).
    pub lease_heartbeat_at: Option<u64>,
    pub created_at: u64,
    pub updated_at: u64,
    pub started_at: Option<u64>,
    pub paused_at: Option<u64>,
    pub completed_at: Option<u64>,
    /// Last validation state (`validating` record: command summary,
    /// result, evidence) — filled by `start_validation` and `record`
    /// validation results.
    pub validation: Option<TaskValidation>,
    /// Skills used by this task (reference-only association).
    pub skill_refs: Vec<String>,
    /// Terminal outcome, present on completed/failed tasks.
    pub outcome: Option<TaskOutcome>,
    /// When the last lifecycle transition happened.
    pub last_transition_at: Option<u64>,
    /// Derived at read time (never stored): a RUNNING/VALIDATING task
    /// whose lease expired is stale/interrupted — recoverable via
    /// explicit resume, never auto-completed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stale: Option<bool>,
}

impl TaskRecord {
    /// Derived staleness: a live-state task (running/validating) whose
    /// lease expired (or has no lease) is interrupted work. Paused tasks
    /// are intentional stops, never interruptions; pending and terminal
    /// tasks are never stale.
    pub fn is_stale(&self, now: u64) -> bool {
        matches!(self.status, TaskStatus::Running | TaskStatus::Validating)
            && self.lease_expires_at.map(|e| now >= e).unwrap_or(true)
    }

    /// Whether `worker` currently holds this task's lease.
    pub fn lease_owned_by(&self, worker: &str) -> bool {
        self.lease_worker.as_deref() == Some(worker)
    }
}

/// Validation state recorded on the task (the completion gate evidence).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskValidation {
    /// What was validated (command/test summary), bounded.
    pub what: String,
    /// `Some(passed|failed)` once a result was recorded; `None` while a
    /// validation run is in progress (seeded by `start_task_validation`).
    pub result: Option<TaskValidationResult>,
    /// When the validation state last changed.
    pub at: u64,
    /// Bounded evidence summary (command output digest, failing tests).
    pub evidence: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskValidationResult {
    Passed,
    Failed,
}

impl TaskValidationResult {
    pub fn as_str(&self) -> &'static str {
        match self {
            TaskValidationResult::Passed => "passed",
            TaskValidationResult::Failed => "failed",
        }
    }
}

/// Terminal outcome of a completed or failed task.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskOutcome {
    /// `completed` | `failed` — mirrors the terminal status.
    pub result: String,
    /// Bounded human summary of what the task achieved.
    pub summary: String,
    /// Areas/files/subsystems changed (bounded list).
    pub changed_areas: Vec<String>,
    /// Validation evidence id(s): the history event ids backing the
    /// outcome (for P3 learning, which reads them by its own rules).
    pub evidence_event_ids: Vec<i64>,
    pub completed_at: u64,
}

/// P9 engineering-outcome classification (closed vocabulary).
///
/// What the reported work taught us, structurally: a success to repeat,
/// a partial result to continue, a failure to avoid, a rejected approach
/// to stop, or a superseded approach replaced by something better.
/// Interpretation (whether this generalizes) stays with P3 learning;
/// this label only describes the single reported outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeClassification {
    Success,
    #[default]
    Partial,
    Failure,
    Rejected,
    Superseded,
}

impl OutcomeClassification {
    pub fn as_str(&self) -> &'static str {
        match self {
            OutcomeClassification::Success => "success",
            OutcomeClassification::Partial => "partial",
            OutcomeClassification::Failure => "failure",
            OutcomeClassification::Rejected => "rejected",
            OutcomeClassification::Superseded => "superseded",
        }
    }

    /// History outcome label for the evidence event. Polarity-safe under
    /// the existing learning mapping: every label keeps its natural
    /// polarity except `superseded`, which is stored as `replaced`
    /// (neutral) so an abandoned approach never reads as a success.
    pub fn history_outcome_label(&self) -> &'static str {
        match self {
            OutcomeClassification::Success => "success",
            OutcomeClassification::Partial => "partial",
            OutcomeClassification::Failure => "failure",
            OutcomeClassification::Rejected => "rejected",
            OutcomeClassification::Superseded => "replaced",
        }
    }
}

impl std::fmt::Display for OutcomeClassification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for OutcomeClassification {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim() {
            "success" => Ok(OutcomeClassification::Success),
            "partial" => Ok(OutcomeClassification::Partial),
            "failure" => Ok(OutcomeClassification::Failure),
            "rejected" => Ok(OutcomeClassification::Rejected),
            "superseded" => Ok(OutcomeClassification::Superseded),
            other => Err(format!(
                "unknown outcome classification: '{other}' — use success|partial|failure|rejected|superseded"
            )),
        }
    }
}

/// Input for [`ContextStore::record_task_outcome`]: one structured
/// outcome report. All free text is bounded and redacted at the write
/// seam; the caller restates oversize input (nothing is silently cut
/// before the history seam's own defensive truncation).
#[derive(Debug, Clone, Default)]
pub struct TaskOutcomeInput<'a> {
    /// What this single outcome was (required).
    pub classification: OutcomeClassification,
    /// Bounded human summary of what happened (required).
    pub summary: &'a str,
    /// Bounded evidence detail, e.g. failing-test names or an output
    /// digest (never full logs or transcripts).
    pub evidence: Option<&'a str>,
    /// Test/build command identity, e.g. `cargo test` (never output).
    pub command: Option<&'a str>,
    /// Command exit status, when the outcome reports a command.
    pub exit_code: Option<i32>,
    /// Files/subsystems touched (bounded list).
    pub changed_areas: Vec<String>,
    /// Caller speech act: set true only when the user explicitly stated
    /// or approved this outcome. Otherwise the report is recorded as
    /// `observed` (OpenCode-reported), never as user-confirmed truth.
    pub user_confirmed: bool,
    /// Explicit idempotency key: redelivering the same (task, key)
    /// returns the original event instead of duplicating history.
    pub dedup_key: Option<&'a str>,
}

/// The stored outcome-evidence handle returned by
/// [`ContextStore::record_task_outcome`].
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskOutcomeRecord {
    /// History event id of the outcome evidence (citable by future
    /// `AiInferred`/`Observed` records and P3 learning).
    pub event_id: i64,
    /// True when `dedup_key` replayed a previous submission (nothing
    /// written; the original event is the answer).
    pub duplicate: bool,
    pub classification: OutcomeClassification,
    /// `observed` (OpenCode-reported) or `user_confirmed` (explicit
    /// user speech act). Existing authority vocabulary, no new trust
    /// system.
    pub authority: String,
}

/// One immutable checkpoint: the durable state required to safely
/// resume engineering work. Bounded resume state, never a transcript.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskCheckpoint {
    /// Opaque id (`cp::<16 hex>`).
    pub checkpoint_id: String,
    pub task_id: String,
    /// Monotonic version within the task (1, 2, …). Immutable once
    /// written; progress produces a new version.
    pub version: u64,
    /// What has been completed so far (bounded).
    pub summary: String,
    /// Task status at checkpoint time (informational).
    pub state: TaskStatus,
    /// What remains.
    pub progress: Option<String>,
    /// What should happen next.
    pub next_action: Option<String>,
    /// Validation status snapshot (`passed` | `failed` | None).
    pub validation_status: Option<String>,
    /// Bounded JSON object of task-private metadata (files touched,
    /// decisions made, blockers). Redacted before storage.
    pub metadata_json: Option<String>,
    /// Fencing version of the worker that wrote it.
    pub lease_version: u64,
    pub created_at: u64,
}

// ── Id minting ────────────────────────────────────────────────────────────

// Process-local collision-avoidance counter (same scheme as session ids:
// time + pid + counter, existence-checked before use).
static TASK_ID_COUNTER: AtomicU64 = AtomicU64::new(0);
static CHECKPOINT_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

fn diffuse(mut x: u64) -> u64 {
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Mint a unique opaque task id (`task::<16 hex>`), existence-checked
/// against the tasks table within the caller's transaction.
fn mint_task_id(conn: &Connection, now: u64) -> Result<String, ContextError> {
    let pid = std::process::id() as u64;
    for _ in 0..1000 {
        let n = TASK_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let x = diffuse(
            now.wrapping_mul(0x9E37_79B9_7F4A_7C15)
                .wrapping_add(pid.wrapping_mul(0xBF58_476D_1CE4_E5B9))
                .wrapping_add(n.wrapping_mul(0x94D0_49BB_1331_11EB)),
        );
        let candidate = format!("task::{x:016x}");
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM tasks WHERE task_id = ?1",
                [&candidate],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?
            .is_some();
        if !exists {
            return Ok(candidate);
        }
    }
    Err(ContextError::Validation(
        "could not mint a unique task id after 1000 attempts".to_string(),
    ))
}

/// Mint a unique opaque checkpoint id (`cp::<16 hex>`).
fn mint_checkpoint_id(conn: &Connection, now: u64) -> Result<String, ContextError> {
    let pid = std::process::id() as u64;
    for _ in 0..1000 {
        let n = CHECKPOINT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let x = diffuse(
            now.wrapping_mul(0x5851_F42D_4C95_7F2D)
                .wrapping_add(pid.wrapping_mul(0xA24B_AED4_96D8_4FB5))
                .wrapping_add(n.wrapping_mul(0x9E37_79B9_7F4A_7C15)),
        );
        let candidate = format!("cp::{x:016x}");
        let exists: bool = conn
            .query_row(
                "SELECT 1 FROM task_checkpoints WHERE checkpoint_id = ?1",
                [&candidate],
                |_| Ok(()),
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?
            .is_some();
        if !exists {
            return Ok(candidate);
        }
    }
    Err(ContextError::Validation(
        "could not mint a unique checkpoint id after 1000 attempts".to_string(),
    ))
}

/// Mint a worker id (`wkr::<16 hex>`): identifies one server process's
/// task-ownership. One worker id per ContextStore server process is
/// expected (the MCP layer mints one at startup).
pub fn mint_worker_id() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    let n = WORKER_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let x = diffuse(
        now.wrapping_mul(0x6D2B_79F5_1F5D_69A4)
            .wrapping_add(pid.wrapping_mul(0xC2B2_AE3D_27D4_6F21))
            .wrapping_add(n.wrapping_mul(0x8E54_9C7F_93D1_2B6A)),
    );
    format!("wkr::{x:016x}")
}

static WORKER_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

// ── Validation helpers ────────────────────────────────────────────────────

fn redact(s: &str) -> String {
    codebro_core::tools::shell::redact_secrets_public(s)
}

fn validate_title(title: &str) -> Result<(), ContextError> {
    if title.trim().is_empty() {
        return Err(ContextError::Validation(
            "task title must not be empty".to_string(),
        ));
    }
    if title.chars().count() > MAX_TASK_TITLE_CHARS {
        return Err(ContextError::Validation(format!(
            "task title exceeds {MAX_TASK_TITLE_CHARS} characters"
        )));
    }
    Ok(())
}

fn validate_skill_refs(refs: &[String]) -> Result<(), ContextError> {
    if refs.len() > MAX_TASK_SKILL_REFS {
        return Err(ContextError::Validation(format!(
            "task carries more than {MAX_TASK_SKILL_REFS} skill references"
        )));
    }
    for r in refs {
        if r.trim().is_empty() || r.chars().count() > MAX_SKILL_REF_CHARS {
            return Err(ContextError::Validation(
                "skill references must be non-empty and ≤ 256 chars".to_string(),
            ));
        }
    }
    Ok(())
}

// ── Row mappers ───────────────────────────────────────────────────────────

const TASK_COLUMNS: &str = "task_id, workspace_root, title, description, status, priority,
    intent_record_id, parent_task_id, idempotency_key, current_version,
    current_checkpoint_id, lease_worker, lease_expires_at, lease_version,
    lease_heartbeat_at, created_at, updated_at, started_at, paused_at,
    completed_at, validation_json, skill_refs_json, outcome_json,
    last_transition_at";

fn row_to_task(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskRecord> {
    let status: String = row.get(4)?;
    let priority: String = row.get(5)?;
    let validation_json: Option<String> = row.get(20)?;
    let skill_refs_json: String = row.get(21)?;
    let outcome_json: Option<String> = row.get(22)?;
    Ok(TaskRecord {
        task_id: row.get(0)?,
        workspace_root: row.get(1)?,
        title: row.get(2)?,
        description: row.get(3)?,
        status: status.parse().unwrap_or(TaskStatus::Pending),
        priority: priority.parse().unwrap_or(TaskPriority::Medium),
        intent_record_id: row.get(6)?,
        parent_task_id: row.get(7)?,
        idempotency_key: row.get(8)?,
        current_version: row.get::<_, i64>(9)? as u64,
        current_checkpoint_id: row.get(10)?,
        lease_worker: row.get(11)?,
        lease_expires_at: row.get::<_, Option<i64>>(12)?.map(|t| t as u64),
        lease_version: row.get::<_, i64>(13)? as u64,
        lease_heartbeat_at: row.get::<_, Option<i64>>(14)?.map(|t| t as u64),
        created_at: row.get::<_, i64>(15)? as u64,
        updated_at: row.get::<_, i64>(16)? as u64,
        started_at: row.get::<_, Option<i64>>(17)?.map(|t| t as u64),
        paused_at: row.get::<_, Option<i64>>(18)?.map(|t| t as u64),
        completed_at: row.get::<_, Option<i64>>(19)?.map(|t| t as u64),
        validation: validation_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok()),
        skill_refs: serde_json::from_str(&skill_refs_json).unwrap_or_default(),
        outcome: outcome_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok()),
        last_transition_at: row.get::<_, Option<i64>>(23)?.map(|t| t as u64),
        stale: None,
    })
}

const CHECKPOINT_COLUMNS: &str = "checkpoint_id, task_id, version, summary, state, progress,
    next_action, validation_status, metadata_json, lease_version, created_at";

fn row_to_checkpoint(row: &rusqlite::Row<'_>) -> rusqlite::Result<TaskCheckpoint> {
    let state: String = row.get(4)?;
    // The checkpoint INSERT stores `""` for an absent metadata blob (the
    // column is NOT NULL); normalize it back so persisted reads equal
    // the value the create call returned.
    let metadata_json: Option<String> = row.get(8)?;
    Ok(TaskCheckpoint {
        checkpoint_id: row.get(0)?,
        task_id: row.get(1)?,
        version: row.get::<_, i64>(2)? as u64,
        summary: row.get(3)?,
        state: state.parse().unwrap_or(TaskStatus::Running),
        progress: row.get(5)?,
        next_action: row.get(6)?,
        validation_status: row.get(7)?,
        metadata_json: metadata_json.filter(|m| !m.is_empty()),
        lease_version: row.get::<_, i64>(9)? as u64,
        created_at: row.get::<_, i64>(10)? as u64,
    })
}

// ── Store API ─────────────────────────────────────────────────────────────

/// Shared context for a task mutation: who is mutating, which task, and
/// the concurrency/lease anchors. Bundles the seven arguments every
/// mutation repeats.
#[derive(Debug, Clone)]
pub struct TaskMutationCtx<'a> {
    /// Canonicalizable workspace root of the caller.
    pub workspace_root: &'a str,
    /// Target task id.
    pub task_id: &'a str,
    /// Calling worker id (from `mint_worker_id`).
    pub worker: &'a str,
    /// The lease fencing version the caller believes is current.
    pub lease_version: u64,
    /// Optimistic-concurrency anchor (the task version this mutation is
    /// based on); `None` skips the check (used by trusted internal paths).
    pub based_on_version: Option<u64>,
    /// Mutation time (unix seconds).
    pub now: u64,
}

impl<'a> TaskMutationCtx<'a> {
    fn check_stale(&self, task: &TaskRecord) -> Result<(), ContextError> {
        if let Some(anchor) = self.based_on_version {
            if anchor != task.current_version {
                return Err(ContextError::Validation(format!(
                    "stale task mutation: based on version {anchor} but the task is at \
                     version {} — re-read and retry",
                    task.current_version
                )));
            }
        }
        Ok(())
    }
}

/// Input for creating a task.
#[derive(Debug, Clone, Default)]
pub struct NewTask {
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<TaskPriority>,
    pub intent_record_id: Option<String>,
    pub parent_task_id: Option<String>,
    pub idempotency_key: Option<String>,
    pub skill_refs: Vec<String>,
}

/// Input for [`ContextStore::create_task_checkpoint`]: the bounded
/// resume-state fields. Checkpoints are caller-authored, so oversize is
/// refused (the caller can restate; silent truncation would corrupt
/// resume state).
#[derive(Debug, Clone, Copy)]
pub struct CheckpointInput<'a> {
    /// What has been completed (required).
    pub summary: &'a str,
    /// What remains.
    pub progress: Option<&'a str>,
    /// What should happen next.
    pub next_action: Option<&'a str>,
    /// Validation status snapshot (e.g. "passed").
    pub validation_status: Option<&'a str>,
    /// Bounded JSON object of task-private metadata.
    pub metadata_json: Option<&'a str>,
}

/// Event description for the history row a transition writes.
struct TaskEvent {
    kind: HistoryKind,
    /// Task-bound session id (from P2 `ensure_active_session`), when the
    /// session ensure succeeded before the mutation transaction.
    session_id: Option<String>,
    summary: String,
    outcome: Option<&'static str>,
    payload: Option<String>,
}

/// Derive the history kind for a `to` transition given the from-state:
/// `paused → running` is a resume; a fresh `pending → running` is a
/// start; an interrupted-lease takeover back to running is also a
/// resume.
fn transition_event_kind(from: TaskStatus, to: TaskStatus) -> HistoryKind {
    match (from, to) {
        (_, TaskStatus::Pending) => HistoryKind::TaskCreated,
        (TaskStatus::Paused, TaskStatus::Running) => HistoryKind::TaskResumed,
        (TaskStatus::Running, TaskStatus::Running) => HistoryKind::TaskResumed,
        // Resume of interrupted validating work is a resume of the task
        // (the validation failure path emits its own distinct event).
        (TaskStatus::Validating, TaskStatus::Running) => HistoryKind::TaskResumed,
        (_, TaskStatus::Running) => HistoryKind::TaskStarted,
        (_, TaskStatus::Paused) => HistoryKind::TaskPaused,
        (_, TaskStatus::Validating) => HistoryKind::TaskValidationStarted,
        (_, TaskStatus::Completed) => HistoryKind::TaskCompleted,
        (_, TaskStatus::Failed) => HistoryKind::TaskFailed,
        (_, TaskStatus::Cancelled) => HistoryKind::TaskCancelled,
    }
}

fn history_event_for_transition(
    task_id: &str,
    title: &str,
    from: TaskStatus,
    to: TaskStatus,
    session_id: Option<String>,
    now: u64,
) -> TaskEvent {
    let kind = transition_event_kind(from, to);
    let outcome = match to {
        TaskStatus::Completed => Some("completed"),
        TaskStatus::Failed => Some("failed"),
        TaskStatus::Cancelled => Some("cancelled"),
        TaskStatus::Validating => Some("validating"),
        _ => None,
    };
    let summary = match kind {
        HistoryKind::TaskCreated => format!("task created: {title}"),
        HistoryKind::TaskStarted => format!("task started: {title}"),
        HistoryKind::TaskResumed => format!("task resumed: {title}"),
        HistoryKind::TaskPaused => format!("task paused: {title}"),
        HistoryKind::TaskValidationStarted => format!("task validation started: {title}"),
        HistoryKind::TaskCompleted => format!("task completed: {title}"),
        HistoryKind::TaskFailed => format!("task failed: {title}"),
        HistoryKind::TaskCancelled => format!("task cancelled: {title}"),
        _ => format!("task transition: {title}"),
    };
    let payload = serde_json::json!({
        "task_id": task_id,
        "from_status": from.as_str(),
        "to_status": to.as_str(),
        "at": now,
    })
    .to_string();
    TaskEvent {
        kind,
        session_id,
        summary,
        outcome,
        payload: Some(payload),
    }
}

/// Write a task event through the P2 history seam inside the caller's
/// transaction (redaction + FTS + dedup included).
fn record_task_event_in_tx(
    tx: &Connection,
    task: &TaskRecord,
    event: &TaskEvent,
    now: u64,
) -> Result<i64, ContextError> {
    let summary = crate::history::clean_text(event.summary.as_str(), 2000)
        .trim()
        .to_string();
    let payload = event
        .payload
        .as_deref()
        .map(crate::history::clean_payload)
        .filter(|p| !p.trim().is_empty());
    let record = EventRecord {
        id: None,
        session_id: event.session_id.clone(),
        workspace_root: task.workspace_root.clone(),
        task_id: Some(task.task_id.clone()),
        kind: event.kind.as_str().to_string(),
        tool: Some("task".to_string()),
        path: None,
        outcome: event.outcome.map(str::to_string),
        summary: Some(summary),
        payload,
        dedup_key: None,
        source: Some("mcp:task".to_string()),
        digest: None,
        created_at: now,
    };
    let (id, dup) = record_event_in_tx(tx, &record, now)?;
    let _ = dup; // task events are never deduplicated (each is distinct)
    Ok(id)
}

impl ContextStore {
    /// Ensure the task-bound session (P2) exists before a mutation
    /// transaction and return its id for the lifecycle event binding.
    /// Best-effort: a failed session ensure degrades to an unlinked
    /// event (the task row remains the source of truth).
    fn task_session_id(&self, ws: &str, task_id: &str, now: u64) -> Option<String> {
        self.ensure_active_session(ws, Some(task_id), Some("mcp:task"), now)
            .ok()
            .map(|(session, _)| session.id)
    }

    /// Redact a field and refuse oversized input (refusal policy for
    /// caller-authored task/checkpoint fields — the caller can restate;
    /// silent truncation would corrupt resume state).
    fn redacted_bounded(raw: &str, max_chars: usize, what: &str) -> Result<String, ContextError> {
        let redacted = redact(raw);
        if redacted.chars().count() > max_chars {
            return Err(ContextError::Validation(format!(
                "{what} exceeds {max_chars} characters"
            )));
        }
        Ok(redacted)
    }

    // ── Creation & lookup ─────────────────────────────────────────────

    /// Create a durable engineering task. With `idempotency_key` set, an
    /// existing task under the same (workspace, key) is returned
    /// instead of a duplicate. Titles are NOT deduplicated.
    pub fn create_task(
        &self,
        workspace_root: &str,
        input: &NewTask,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        if ws.is_empty() {
            return Err(ContextError::Validation(
                "task workspace_root must not be empty".to_string(),
            ));
        }
        validate_title(&input.title)?;
        if let Some(desc) = input.description.as_deref() {
            if desc.trim().is_empty() {
                return Err(ContextError::Validation(
                    "task description must not be blank when supplied".to_string(),
                ));
            }
        }
        validate_skill_refs(&input.skill_refs)?;
        let priority = input.priority.unwrap_or(TaskPriority::Medium);
        let title = Self::redacted_bounded(&input.title, MAX_TASK_TITLE_CHARS, "task title")?;
        let description = match input.description.as_deref() {
            Some(d) => Some(Self::redacted_bounded(
                d,
                MAX_TASK_DESCRIPTION_CHARS,
                "task description",
            )?),
            None => None,
        };

        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            // Idempotent create: same (workspace, key) returns the
            // existing task.
            if let Some(key) = input.idempotency_key.as_deref() {
                if !key.trim().is_empty() {
                    let existing = tx
                        .query_row(
                            &format!(
                                "SELECT {TASK_COLUMNS} FROM tasks
                                 WHERE workspace_root = ?1 AND idempotency_key = ?2"
                            ),
                            params![ws, key.trim()],
                            row_to_task,
                        )
                        .optional()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    if let Some(task) = existing {
                        tx.commit()?;
                        return Ok(task);
                    }
                }
            }
            let task_id = mint_task_id(&tx, now)?;
            let task = TaskRecord {
                task_id: task_id.clone(),
                workspace_root: ws.clone(),
                title,
                description,
                status: TaskStatus::Pending,
                priority,
                intent_record_id: input
                    .intent_record_id
                    .clone()
                    .filter(|s| !s.trim().is_empty()),
                parent_task_id: input
                    .parent_task_id
                    .clone()
                    .filter(|s| !s.trim().is_empty()),
                idempotency_key: input
                    .idempotency_key
                    .clone()
                    .map(|k| k.trim().to_string())
                    .filter(|k| !k.is_empty()),
                current_version: 0,
                current_checkpoint_id: None,
                lease_worker: None,
                lease_expires_at: None,
                lease_version: 0,
                lease_heartbeat_at: None,
                created_at: now,
                updated_at: now,
                started_at: None,
                paused_at: None,
                completed_at: None,
                validation: None,
                // Redact free-text skill references at the write seam:
                // refs are caller-supplied strings surfaced verbatim by
                // read paths (task snapshots, P6 resolution, P7 briefs),
                // so a pasted secret must never persist.
                skill_refs: input.skill_refs.iter().map(|r| redact(r)).collect(),
                outcome: None,
                last_transition_at: Some(now),
                stale: None,
            };
            tx.execute(
                "INSERT INTO tasks (task_id, workspace_root, title, description, status,
                    priority, intent_record_id, parent_task_id, idempotency_key,
                    current_version, created_at, updated_at, skill_refs_json,
                    last_transition_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 0, ?10, ?10, ?11, ?10)",
                params![
                    task.task_id,
                    task.workspace_root,
                    task.title,
                    task.description,
                    task.status.as_str(),
                    task.priority.as_str(),
                    task.intent_record_id,
                    task.parent_task_id,
                    task.idempotency_key,
                    now as i64,
                    serde_json::to_string(&task.skill_refs)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                ],
            )?;
            // Create cannot ensure a task-bound session up front (the
            // id does not exist until this transaction commits); the
            // created event is linked on the next mutation instead.
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                TaskStatus::Pending,
                TaskStatus::Pending,
                None,
                now,
            );
            record_task_event_in_tx(&tx, &task, &event, now)?;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Fetch a task by id, enforcing workspace isolation: a task from
    /// another workspace is `None` (invisible), never an error leak.
    pub fn get_task(
        &self,
        workspace_root: &str,
        task_id: &str,
    ) -> Result<Option<TaskRecord>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE task_id = ?1"),
                [task_id],
                row_to_task,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
            .map(|task| task.filter(|t| t.workspace_root == ws))
        })
    }

    /// List tasks for a workspace (status/priority filters, bounded,
    /// checkpoint-light: no checkpoint payloads in list results).
    pub fn list_tasks(
        &self,
        workspace_root: &str,
        status: Option<TaskStatus>,
        parent_task_id: Option<&str>,
        limit: usize,
        now: u64,
    ) -> Result<Vec<TaskRecord>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        let limit = limit.clamp(1, 100) as i64;
        let parent = parent_task_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        self.with_conn(|conn| {
            let sql = match (status, &parent) {
                (Some(_), Some(_)) => format!(
                    "SELECT {TASK_COLUMNS} FROM tasks
                     WHERE workspace_root = ?1 AND status = ?2 AND parent_task_id = ?3
                     ORDER BY updated_at DESC, task_id LIMIT ?4"
                ),
                (Some(_), None) => format!(
                    "SELECT {TASK_COLUMNS} FROM tasks
                     WHERE workspace_root = ?1 AND status = ?2
                     ORDER BY updated_at DESC, task_id LIMIT ?3"
                ),
                (None, Some(_)) => format!(
                    "SELECT {TASK_COLUMNS} FROM tasks
                     WHERE workspace_root = ?1 AND parent_task_id = ?2
                     ORDER BY updated_at DESC, task_id LIMIT ?3"
                ),
                (None, None) => format!(
                    "SELECT {TASK_COLUMNS} FROM tasks
                     WHERE workspace_root = ?1
                     ORDER BY updated_at DESC, task_id LIMIT ?2"
                ),
            };
            let mut rows: Vec<TaskRecord> = match (status, &parent) {
                (Some(s), Some(p)) => {
                    let mut stmt = conn.prepare(&sql)?;
                    let collected = stmt
                        .query_map(params![ws, s.as_str(), p, limit], row_to_task)
                        .map_err(|e| ContextError::Decode(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    collected
                }
                (Some(s), None) => {
                    let mut stmt = conn.prepare(&sql)?;
                    let collected = stmt
                        .query_map(params![ws, s.as_str(), limit], row_to_task)
                        .map_err(|e| ContextError::Decode(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    collected
                }
                (None, Some(p)) => {
                    let mut stmt = conn.prepare(&sql)?;
                    let collected = stmt
                        .query_map(params![ws, p, limit], row_to_task)
                        .map_err(|e| ContextError::Decode(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    collected
                }
                (None, None) => {
                    let mut stmt = conn.prepare(&sql)?;
                    let collected = stmt
                        .query_map(params![ws, limit], row_to_task)
                        .map_err(|e| ContextError::Decode(e.to_string()))?
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    collected
                }
            };
            for t in rows.iter_mut() {
                t.stale = Some(t.is_stale(now));
            }
            Ok(rows)
        })
    }

    /// Detect recoverable work: non-terminal tasks whose lease expired
    /// (interrupted RUNNING/VALIDATING work). Never auto-completes,
    /// never deletes; the caller decides whether to resume.
    pub fn stale_tasks(
        &self,
        workspace_root: &str,
        now: u64,
    ) -> Result<Vec<TaskRecord>, ContextError> {
        let mut rows = self.list_tasks(workspace_root, None, None, 100, now)?;
        rows.retain(|t| t.is_stale(now));
        Ok(rows)
    }

    // ── Mutation core ──────────────────────────────────────────────────

    /// Load a task inside a transaction, enforcing workspace isolation.
    fn load_task_for(tx: &Connection, ws: &str, task_id: &str) -> Result<TaskRecord, ContextError> {
        let task = tx
            .query_row(
                &format!("SELECT {TASK_COLUMNS} FROM tasks WHERE task_id = ?1"),
                [task_id],
                row_to_task,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))?
            .ok_or_else(|| ContextError::Validation(format!("task {task_id} does not exist")))?;
        if task.workspace_root != ws {
            return Err(ContextError::Validation(format!(
                "task {task_id} belongs to another workspace"
            )));
        }
        Ok(task)
    }

    /// Shared transition core: validate the matrix, apply the new
    /// status/timestamps, bump the version, write the history event —
    /// one transaction. Terminal transitions and pause release the
    /// lease in the SAME transaction (no crash window where a completed
    /// task still claims an owner). Returns the updated record.
    fn transition_task_impl(
        &self,
        workspace_root: &str,
        task_id: &str,
        to: TaskStatus,
        based_on_version: Option<u64>,
        now: u64,
        lease_ctx: Option<(&str, u64)>,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        // Task-bound session first (separate transaction; harmless if
        // the mutation later fails), then the mutation itself.
        let task_session = self.task_session_id(&ws, task_id, now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, task_id)?;

            // Optimistic concurrency: refuse stale writers.
            if let Some(anchor) = based_on_version {
                if anchor != task.current_version {
                    return Err(ContextError::Validation(format!(
                        "stale task mutation: based on version {anchor} but the task is at \
                         version {} — re-read and retry",
                        task.current_version
                    )));
                }
            }

            // Lease enforcement: while the task is live and held by
            // another worker, refuse. A worker whose fencing version
            // fell behind is always refused.
            if let Some((worker, caller_lease_version)) = lease_ctx {
                Self::enforce_lease(&mut task, worker, caller_lease_version, now)?;
            }

            // State machine validation.
            if !task_transition_allowed(task.status, to) {
                return Err(ContextError::Validation(format!(
                    "invalid task transition: {} → {}",
                    task.status.as_str(),
                    to.as_str()
                )));
            }

            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                task.status,
                to,
                task_session.clone(),
                now,
            );
            task.status = to;
            task.updated_at = now;
            task.last_transition_at = Some(now);
            let releases_lease = matches!(
                to,
                TaskStatus::Paused
                    | TaskStatus::Completed
                    | TaskStatus::Failed
                    | TaskStatus::Cancelled
            );
            match to {
                TaskStatus::Running => {
                    task.started_at.get_or_insert(now);
                }
                TaskStatus::Paused => task.paused_at = Some(now),
                TaskStatus::Completed => {
                    // Completion gate: a passed validation result must
                    // exist on the task.
                    let passed = task
                        .validation
                        .as_ref()
                        .map(|v| v.result == Some(TaskValidationResult::Passed))
                        .unwrap_or(false);
                    if !passed {
                        return Err(ContextError::Validation(
                            "task cannot complete without a recorded passed validation — \
                             use validate first"
                                .to_string(),
                        ));
                    }
                    task.completed_at = Some(now);
                }
                TaskStatus::Failed | TaskStatus::Cancelled => {
                    task.completed_at = Some(now);
                }
                _ => {}
            }
            if releases_lease {
                task.lease_worker = None;
                task.lease_expires_at = None;
                task.lease_heartbeat_at = None;
            }

            let validation_json = task
                .validation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let outcome_json = task
                .outcome
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;

            tx.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2, started_at = ?3,
                    paused_at = ?4, completed_at = ?5, current_version = current_version + 1,
                    validation_json = ?6, outcome_json = ?7, last_transition_at = ?2,
                    lease_worker = ?8, lease_expires_at = ?9, lease_heartbeat_at = ?10
                 WHERE task_id = ?11",
                params![
                    task.status.as_str(),
                    now as i64,
                    task.started_at.map(|t| t as i64),
                    task.paused_at.map(|t| t as i64),
                    task.completed_at.map(|t| t as i64),
                    validation_json,
                    outcome_json,
                    task.lease_worker,
                    task.lease_expires_at.map(|t| t as i64),
                    task.lease_heartbeat_at.map(|t| t as i64),
                    task.task_id,
                ],
            )?;
            task.current_version += 1;
            record_task_event_in_tx(&tx, &task, &event, now)?;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Lease enforcement for live-state mutations.
    ///
    /// Rules:
    /// 1. A caller must carry its worker id and the task's lease
    ///    version it believes is current. If the task's fencing version
    ///    advanced past the caller's (a takeover happened), the caller
    ///    is stale — refuse.
    /// 2. While the task names an owner, only that owner may mutate live
    ///    state — even after the lease expired. Anyone else must take
    ///    over through explicit `resume` (which fences the version
    ///    forward). A forged or guessed lease version never grants
    ///    access: ownership, not the caller's number, arbitrates.
    /// 3. A live-state transition by a worker other than the lease
    ///    holder is refused while the lease is unexpired.
    fn enforce_lease(
        task: &mut TaskRecord,
        worker: &str,
        caller_lease_version: u64,
        now: u64,
    ) -> Result<(), ContextError> {
        // Fencing: a caller whose version fell behind cannot mutate.
        if caller_lease_version < task.lease_version {
            return Err(ContextError::Validation(format!(
                "stale worker {worker}: lease version {caller_lease_version} predates the \
                 task's fencing version {} — another worker owns this task",
                task.lease_version
            )));
        }
        // Ownership: a named owner keeps exclusive mutation rights until
        // an explicit resume transfers them — expiry alone never opens
        // the task to direct mutation.
        if let Some(holder) = task.lease_worker.as_deref() {
            if holder != worker {
                let live = task.lease_expires_at.map(|e| now < e).unwrap_or(false);
                if live {
                    return Err(ContextError::Validation(format!(
                        "task is leased by another worker ({:?}) until its lease expires",
                        task.lease_worker
                    )));
                }
                return Err(ContextError::Validation(format!(
                    "task is owned by worker {holder}; its lease expired — \
                     resume explicitly to take over",
                )));
            }
        }
        Ok(())
    }

    /// Acquire/renew the task lease for `worker` inside the caller's
    /// transaction. A take-over (different worker, expired lease)
    /// increments `lease_version` — the fencing token.
    fn acquire_lease_in_tx(
        tx: &Connection,
        task: &mut TaskRecord,
        worker: &str,
        now: u64,
    ) -> Result<(), ContextError> {
        let current = task.lease_worker.as_deref();
        let expired = task.lease_expires_at.map(|e| now >= e).unwrap_or(true);
        match current {
            Some(w) if w == worker => {
                // Renewal: same fencing version, new expiry.
                task.lease_expires_at = Some(now + TASK_LEASE_TTL_SECS);
                task.lease_heartbeat_at = Some(now);
            }
            Some(_) if !expired => {
                return Err(ContextError::Validation(format!(
                    "task is actively leased by worker {current:?}; wait for the lease to \
                     expire or resume explicitly after expiry",
                )));
            }
            _ => {
                // Fresh lease or take-over after expiry: fence forward.
                task.lease_version += 1;
                task.lease_worker = Some(worker.to_string());
                task.lease_expires_at = Some(now + TASK_LEASE_TTL_SECS);
                task.lease_heartbeat_at = Some(now);
            }
        }
        tx.execute(
            "UPDATE tasks SET lease_worker = ?1, lease_expires_at = ?2,
                lease_version = ?3, lease_heartbeat_at = ?4
             WHERE task_id = ?5",
            params![
                task.lease_worker,
                task.lease_expires_at.map(|t| t as i64),
                task.lease_version as i64,
                task.lease_heartbeat_at.map(|t| t as i64),
                task.task_id,
            ],
        )?;
        Ok(())
    }

    // ── Lifecycle actions ──────────────────────────────────────────────

    /// Start a pending task: acquires the lease for `worker`, transitions
    /// pending → running.
    pub fn start_task(
        &self,
        workspace_root: &str,
        task_id: &str,
        worker: &str,
        based_on_version: Option<u64>,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, task_id, now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, task_id)?;
            if let Some(anchor) = based_on_version {
                if anchor != task.current_version {
                    return Err(ContextError::Validation(format!(
                        "stale task mutation: based on version {anchor} but the task is at \
                         version {} — re-read and retry",
                        task.current_version
                    )));
                }
            }
            if task.status != TaskStatus::Pending {
                return Err(ContextError::Validation(format!(
                    "only a pending task can start (task is {})",
                    task.status.as_str()
                )));
            }
            Self::acquire_lease_in_tx(&tx, &mut task, worker, now)?;
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                TaskStatus::Pending,
                TaskStatus::Running,
                task_session.clone(),
                now,
            );
            task.status = TaskStatus::Running;
            task.updated_at = now;
            task.started_at = Some(now);
            task.last_transition_at = Some(now);
            tx.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2, started_at = ?2,
                    current_version = current_version + 1, last_transition_at = ?2,
                    lease_worker = ?3, lease_expires_at = ?4, lease_version = ?5,
                    lease_heartbeat_at = ?2
                 WHERE task_id = ?6",
                params![
                    task.status.as_str(),
                    now as i64,
                    task.lease_worker,
                    task.lease_expires_at.map(|t| t as i64),
                    task.lease_version as i64,
                    task.task_id,
                ],
            )?;
            task.current_version += 1;
            record_task_event_in_tx(&tx, &task, &event, now)?;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Pause a running task (running → paused). The lease is released in
    /// the same transaction (pause is an intentional stop, not a crash).
    pub fn pause_task(
        &self,
        workspace_root: &str,
        task_id: &str,
        worker: &str,
        lease_version: u64,
        based_on_version: Option<u64>,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        self.transition_task_impl(
            workspace_root,
            task_id,
            TaskStatus::Paused,
            based_on_version,
            now,
            Some((worker, lease_version)),
        )
    }

    /// Resume a paused or interrupted (stale-lease) task: acquires the
    /// lease (fencing forward on take-over) and transitions → running.
    pub fn resume_task(
        &self,
        workspace_root: &str,
        task_id: &str,
        worker: &str,
        based_on_version: Option<u64>,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, task_id, now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, task_id)?;
            if let Some(anchor) = based_on_version {
                if anchor != task.current_version {
                    return Err(ContextError::Validation(format!(
                        "stale task mutation: based on version {anchor} but the task is at \
                         version {} — re-read and retry",
                        task.current_version
                    )));
                }
            }
            // Resume is legal from paused, or from an interrupted
            // running/validating state (expired/absent lease).
            let resumable = task.status == TaskStatus::Paused
                || ((task.status == TaskStatus::Running || task.status == TaskStatus::Validating)
                    && task.is_stale(now));
            if !resumable {
                return Err(ContextError::Validation(format!(
                    "task in state {} with a live lease cannot be resumed here",
                    task.status.as_str()
                )));
            }
            Self::acquire_lease_in_tx(&tx, &mut task, worker, now)?;
            let from = task.status;
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                from,
                TaskStatus::Running,
                task_session.clone(),
                now,
            );
            task.status = TaskStatus::Running;
            task.updated_at = now;
            task.started_at.get_or_insert(now);
            task.last_transition_at = Some(now);
            tx.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2, started_at = ?3,
                    current_version = current_version + 1, last_transition_at = ?2,
                    lease_worker = ?4, lease_expires_at = ?5, lease_version = ?6,
                    lease_heartbeat_at = ?2
                 WHERE task_id = ?7",
                params![
                    task.status.as_str(),
                    now as i64,
                    task.started_at.map(|t| t as i64),
                    task.lease_worker,
                    task.lease_expires_at.map(|t| t as i64),
                    task.lease_version as i64,
                    task.task_id,
                ],
            )?;
            task.current_version += 1;
            record_task_event_in_tx(&tx, &task, &event, now)?;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Begin validation (running → validating). Requires the lease.
    /// The `what` (command/test summary) is recorded on the task's
    /// validation state with its result pending in `record_task_validation`.
    pub fn start_task_validation(
        &self,
        ctx: &TaskMutationCtx<'_>,
        what: &str,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(ctx.workspace_root);
        let what_bounded = Self::redacted_bounded(what, 512, "validation what")?;
        if what_bounded.trim().is_empty() {
            return Err(ContextError::Validation(
                "validation 'what' must not be empty".to_string(),
            ));
        }
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, ctx.task_id, ctx.now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, ctx.task_id)?;
            ctx.check_stale(&task)?;
            if task.status != TaskStatus::Running {
                return Err(ContextError::Validation(format!(
                    "validation starts from running (task is {})",
                    task.status.as_str()
                )));
            }
            Self::enforce_lease(&mut task, ctx.worker, ctx.lease_version, ctx.now)?;
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                TaskStatus::Running,
                TaskStatus::Validating,
                task_session.clone(),
                ctx.now,
            );
            task.status = TaskStatus::Validating;
            task.updated_at = ctx.now;
            task.last_transition_at = Some(ctx.now);
            // Seed the validation state (what is being validated, result
            // pending); the result lands in record_task_validation.
            task.validation = Some(TaskValidation {
                what: what_bounded.clone(),
                result: None,
                at: ctx.now,
                evidence: None,
            });
            let validation_json = task
                .validation
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            tx.execute(
                "UPDATE tasks SET status = ?1, updated_at = ?2, last_transition_at = ?2,
                    validation_json = ?3, current_version = current_version + 1
                 WHERE task_id = ?4",
                params![
                    task.status.as_str(),
                    ctx.now as i64,
                    validation_json,
                    task.task_id,
                ],
            )?;
            task.current_version += 1;
            record_task_event_in_tx(&tx, &task, &event, ctx.now)?;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Record a validation result and apply the gated transition:
    /// passed → (stays validating for `complete_task`) or the caller
    /// completes; failed → back to running. Stores the validation on
    /// the task and writes the corresponding history event in one
    /// transaction.
    pub fn record_task_validation(
        &self,
        ctx: &TaskMutationCtx<'_>,
        what: &str,
        result: TaskValidationResult,
        evidence: Option<&str>,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(ctx.workspace_root);
        let what_bounded = Self::redacted_bounded(what, 512, "validation what")?;
        let evidence_bounded = match evidence {
            Some(e) => Some(Self::redacted_bounded(e, 2048, "validation evidence")?),
            None => None,
        };
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, ctx.task_id, ctx.now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, ctx.task_id)?;
            ctx.check_stale(&task)?;
            if task.status != TaskStatus::Validating {
                return Err(ContextError::Validation(format!(
                    "validation results are recorded while validating (task is {})",
                    task.status.as_str()
                )));
            }
            Self::enforce_lease(&mut task, ctx.worker, ctx.lease_version, ctx.now)?;

            task.validation = Some(TaskValidation {
                what: what_bounded.clone(),
                result: Some(result),
                at: ctx.now,
                evidence: evidence_bounded.clone(),
            });
            let validation_json = serde_json::to_string(&task.validation)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let (event, new_status) = match result {
                TaskValidationResult::Passed => (
                    TaskEvent {
                        kind: HistoryKind::TaskValidationPassed,
                        session_id: task_session.clone(),
                        summary: format!("task validation passed: {what_bounded}"),
                        outcome: Some("passed"),
                        payload: Some(
                            serde_json::json!({
                                "task_id": task.task_id,
                                "what": what_bounded,
                                "result": "passed",
                            })
                            .to_string(),
                        ),
                    },
                    // Passed validation leaves the task validating: the
                    // completion gate is now armed, `complete_task`
                    // performs the terminal transition.
                    TaskStatus::Validating,
                ),
                TaskValidationResult::Failed => (
                    TaskEvent {
                        kind: HistoryKind::TaskValidationFailed,
                        session_id: task_session.clone(),
                        summary: format!("task validation failed: {what_bounded}"),
                        outcome: Some("failed"),
                        payload: Some(
                            serde_json::json!({
                                "task_id": task.task_id,
                                "what": what_bounded,
                                "result": "failed",
                                "evidence": evidence_bounded,
                            })
                            .to_string(),
                        ),
                    },
                    // Validation failed: back to running (fix and retry).
                    TaskStatus::Running,
                ),
            };
            task.status = new_status;
            task.updated_at = ctx.now;
            tx.execute(
                "UPDATE tasks SET status = ?1, validation_json = ?2, updated_at = ?3,
                    current_version = current_version + 1
                 WHERE task_id = ?4",
                params![
                    task.status.as_str(),
                    validation_json,
                    ctx.now as i64,
                    task.task_id,
                ],
            )?;
            task.current_version += 1;
            record_task_event_in_tx(&tx, &task, &event, ctx.now)?;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Complete a validating task (validating → completed) with a rich
    /// outcome. The completion gate refuses when the last validation
    /// result was not `passed`.
    pub fn complete_task(
        &self,
        ctx: &TaskMutationCtx<'_>,
        outcome_summary: &str,
        changed_areas: Vec<String>,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(ctx.workspace_root);
        let summary = Self::redacted_bounded(outcome_summary, 2048, "outcome summary")?;
        let mut changed = Vec::new();
        for area in changed_areas.into_iter().take(32) {
            if area.trim().is_empty() {
                continue;
            }
            changed.push(Self::redacted_bounded(&area, 256, "changed area")?);
        }
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, ctx.task_id, ctx.now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, ctx.task_id)?;
            ctx.check_stale(&task)?;
            if task.status != TaskStatus::Validating {
                return Err(ContextError::Validation(format!(
                    "completion happens from validating (task is {}); record a passed \
                     validation first",
                    task.status.as_str()
                )));
            }
            Self::enforce_lease(&mut task, ctx.worker, ctx.lease_version, ctx.now)?;
            let passed = task
                .validation
                .as_ref()
                .map(|v| v.result == Some(TaskValidationResult::Passed))
                .unwrap_or(false);
            if !passed {
                return Err(ContextError::Validation(
                    "task cannot complete: the recorded validation result is not passed"
                        .to_string(),
                ));
            }
            // The completion event's id becomes outcome evidence.
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                TaskStatus::Validating,
                TaskStatus::Completed,
                task_session.clone(),
                ctx.now,
            );
            let event_id = record_task_event_in_tx(&tx, &task, &event, ctx.now)?;
            task.outcome = Some(TaskOutcome {
                result: "completed".to_string(),
                summary: summary.clone(),
                changed_areas: changed.clone(),
                evidence_event_ids: vec![event_id],
                completed_at: ctx.now,
            });
            let outcome_json = serde_json::to_string(&task.outcome)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            task.status = TaskStatus::Completed;
            task.completed_at = Some(ctx.now);
            task.updated_at = ctx.now;
            task.last_transition_at = Some(ctx.now);
            // Terminal: release the lease.
            tx.execute(
                "UPDATE tasks SET status = ?1, outcome_json = ?2, completed_at = ?3,
                    updated_at = ?4, current_version = current_version + 1,
                    last_transition_at = ?4, lease_worker = NULL, lease_expires_at = NULL,
                    lease_heartbeat_at = NULL
                 WHERE task_id = ?5",
                params![
                    task.status.as_str(),
                    outcome_json,
                    ctx.now as i64,
                    ctx.now as i64,
                    task.task_id,
                ],
            )?;
            task.current_version += 1;
            task.lease_worker = None;
            task.lease_expires_at = None;
            task.lease_heartbeat_at = None;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Fail a running/validating task (→ failed), preserving failure
    /// evidence in the outcome. Failure is evidence, not completion.
    pub fn fail_task(
        &self,
        ctx: &TaskMutationCtx<'_>,
        reason: &str,
    ) -> Result<TaskRecord, ContextError> {
        let reason_bounded = Self::redacted_bounded(reason, 2048, "failure reason")?;
        let ws = canonical_workspace_key(ctx.workspace_root);
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, ctx.task_id, ctx.now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, ctx.task_id)?;
            ctx.check_stale(&task)?;
            if !matches!(task.status, TaskStatus::Running | TaskStatus::Validating) {
                return Err(ContextError::Validation(format!(
                    "only live work can fail (task is {})",
                    task.status.as_str()
                )));
            }
            Self::enforce_lease(&mut task, ctx.worker, ctx.lease_version, ctx.now)?;
            if !task_transition_allowed(task.status, TaskStatus::Failed) {
                return Err(ContextError::Validation(format!(
                    "invalid task transition: {} → failed",
                    task.status.as_str()
                )));
            }
            // The transition event is the failure evidence anchor.
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                task.status,
                TaskStatus::Failed,
                task_session.clone(),
                ctx.now,
            );
            let event_id = record_task_event_in_tx(&tx, &task, &event, ctx.now)?;
            task.outcome = Some(TaskOutcome {
                result: "failed".to_string(),
                summary: reason_bounded,
                changed_areas: Vec::new(),
                evidence_event_ids: vec![event_id],
                completed_at: ctx.now,
            });
            let outcome_json = serde_json::to_string(&task.outcome)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            task.status = TaskStatus::Failed;
            task.completed_at = Some(ctx.now);
            task.updated_at = ctx.now;
            task.last_transition_at = Some(ctx.now);
            task.lease_worker = None;
            task.lease_expires_at = None;
            task.lease_heartbeat_at = None;
            tx.execute(
                "UPDATE tasks SET status = ?1, outcome_json = ?2, completed_at = ?3,
                    updated_at = ?3, last_transition_at = ?3,
                    current_version = current_version + 1,
                    lease_worker = NULL, lease_expires_at = NULL, lease_heartbeat_at = NULL
                 WHERE task_id = ?4",
                params![
                    task.status.as_str(),
                    outcome_json,
                    ctx.now as i64,
                    task.task_id
                ],
            )?;
            task.current_version += 1;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Cancel a non-terminal task (→ cancelled). Allowed without a
    /// lease (cancellation is a supervisory action).
    pub fn cancel_task(
        &self,
        workspace_root: &str,
        task_id: &str,
        reason: Option<&str>,
        based_on_version: Option<u64>,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        let reason_bounded = match reason {
            Some(r) => Some(Self::redacted_bounded(r, 2048, "cancellation reason")?),
            None => None,
        };
        let ws = canonical_workspace_key(workspace_root);
        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, task_id, now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, task_id)?;
            if let Some(anchor) = based_on_version {
                if anchor != task.current_version {
                    return Err(ContextError::Validation(format!(
                        "stale task mutation: based on version {anchor} but the task is at \
                         version {} — re-read and retry",
                        task.current_version
                    )));
                }
            }
            if !task_transition_allowed(task.status, TaskStatus::Cancelled) {
                return Err(ContextError::Validation(format!(
                    "invalid task transition: {} → cancelled",
                    task.status.as_str()
                )));
            }
            let event = history_event_for_transition(
                &task.task_id,
                &task.title,
                task.status,
                TaskStatus::Cancelled,
                task_session.clone(),
                now,
            );
            record_task_event_in_tx(&tx, &task, &event, now)?;
            if let Some(reason) = reason_bounded.clone() {
                task.outcome = Some(TaskOutcome {
                    result: "cancelled".to_string(),
                    summary: reason,
                    changed_areas: Vec::new(),
                    evidence_event_ids: Vec::new(),
                    completed_at: now,
                });
            }
            let outcome_json = task
                .outcome
                .as_ref()
                .map(serde_json::to_string)
                .transpose()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            task.status = TaskStatus::Cancelled;
            task.completed_at = Some(now);
            task.updated_at = now;
            task.last_transition_at = Some(now);
            task.lease_worker = None;
            task.lease_expires_at = None;
            task.lease_heartbeat_at = None;
            tx.execute(
                "UPDATE tasks SET status = ?1, outcome_json = ?2, completed_at = ?3,
                    updated_at = ?3, last_transition_at = ?3,
                    current_version = current_version + 1,
                    lease_worker = NULL, lease_expires_at = NULL, lease_heartbeat_at = NULL
                 WHERE task_id = ?4",
                params![task.status.as_str(), outcome_json, now as i64, task.task_id],
            )?;
            task.current_version += 1;
            tx.commit()?;
            Ok(task)
        })
    }

    // ── P9 outcomes ────────────────────────────────────────────────────

    /// Record structured outcome evidence bound to a durable task (P9).
    ///
    /// This is the engineering-outcome ingestion seam: OpenCode performed
    /// work, observed results, and reports them; CodeBro persists the
    /// report as task-bound history evidence. It deliberately mutates NO
    /// task row and performs NO state transition:
    ///
    /// - state transition (`complete`/`fail`/…) — what happened to the task;
    /// - outcome evidence (this method) — what was observed, with what
    ///   classification and authority;
    /// - user confirmation (`user_confirmed=true`) — the user stated or
    ///   approved the outcome (caller speech act, like `remember`);
    /// - inferred lesson — P3 learning's job once evidence accumulates.
    ///
    /// Rules:
    /// - The task must exist in the caller's workspace (isolation
    ///   enforced; cross-workspace ids are refused like every other
    ///   task mutation).
    /// - Any task status is accepted, including terminal ones: user
    ///   confirmation routinely arrives after completion.
    /// - No lease is required and none is granted: appending evidence
    ///   never changes who owns the task.
    /// - Every free-text field is redacted before storage (the same
    ///   authority every other task write seam uses); the history seam
    ///   additionally truncates oversize text with an honest marker.
    /// - Idempotency is explicit: with `dedup_key` set, redelivering the
    ///   same (task, key) returns the original event id with
    ///   `duplicate=true` and writes nothing. The key is namespaced by
    ///   task id, so two tasks never collide on the same caller key.
    /// - Determinism: identical inputs (modulo `now`) produce identical
    ///   summaries and payloads; only the store-assigned event id and
    ///   timestamp vary, as with all history.
    pub fn record_task_outcome(
        &self,
        workspace_root: &str,
        task_id: &str,
        input: &TaskOutcomeInput<'_>,
        now: u64,
    ) -> Result<TaskOutcomeRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        if ws.is_empty() {
            return Err(ContextError::Validation(
                "task workspace_root must not be empty".to_string(),
            ));
        }
        if task_id.trim().is_empty() {
            return Err(ContextError::Validation(
                "task_id must not be empty".to_string(),
            ));
        }
        let summary =
            Self::redacted_bounded(input.summary, MAX_OUTCOME_SUMMARY_CHARS, "outcome summary")?;
        if summary.trim().is_empty() {
            return Err(ContextError::Validation(
                "outcome summary must not be empty".to_string(),
            ));
        }
        let evidence = match input.evidence {
            Some(e) => Some(Self::redacted_bounded(
                e,
                MAX_OUTCOME_EVIDENCE_CHARS,
                "outcome evidence",
            )?),
            None => None,
        };
        let command = match input.command {
            Some(c) => {
                let bounded =
                    Self::redacted_bounded(c, MAX_OUTCOME_COMMAND_CHARS, "outcome command")?;
                if bounded.trim().is_empty() {
                    return Err(ContextError::Validation(
                        "outcome command must not be blank when supplied".to_string(),
                    ));
                }
                Some(bounded)
            }
            None => None,
        };
        let mut changed = Vec::new();
        for area in input.changed_areas.iter().take(MAX_OUTCOME_CHANGED_AREAS) {
            if area.trim().is_empty() {
                continue;
            }
            changed.push(Self::redacted_bounded(
                area,
                MAX_OUTCOME_CHANGED_AREA_CHARS,
                "outcome changed area",
            )?);
        }
        let dedup_key = match input.dedup_key {
            Some(k) => {
                if k.trim().is_empty() {
                    return Err(ContextError::Validation(
                        "outcome dedup_key must not be blank when supplied".to_string(),
                    ));
                }
                if k.chars().count() > MAX_OUTCOME_DEDUP_KEY_CHARS {
                    return Err(ContextError::Validation(format!(
                        "outcome dedup_key exceeds {MAX_OUTCOME_DEDUP_KEY_CHARS} characters"
                    )));
                }
                Some(format!("task_outcome:{}:{}", task_id.trim(), k.trim()))
            }
            None => None,
        };
        let authority = if input.user_confirmed {
            "user_confirmed"
        } else {
            "observed"
        };
        // History outcome label, polarity-safe under the existing
        // `outcome_polarity` mapping: success/failure/rejected keep their
        // natural polarity, partial is neutral (mixed result claims
        // nothing), and superseded is recorded as `replaced` (neutral) so
        // an abandoned approach can never read as a success pattern. The
        // verbatim classification always survives in the summary/payload.
        let outcome_label = input.classification.history_outcome_label();
        let mut summary_line = format!(
            "task outcome [{}] ({authority}): {summary}",
            input.classification.as_str()
        );
        if let Some(cmd) = command.as_deref() {
            summary_line.push_str(&format!(" | command `{cmd}`"));
            if let Some(code) = input.exit_code {
                summary_line.push_str(&format!(" exit {code}"));
            }
        }
        let payload_raw = serde_json::json!({
            "task_id": task_id.trim(),
            "classification": input.classification.as_str(),
            "authority": authority,
            "summary": summary,
            "evidence": evidence,
            "command": command,
            "exit_code": input.exit_code,
            "changed_areas": changed,
            "user_confirmed": input.user_confirmed,
            "at": now,
        })
        .to_string();
        // Defense in depth: inputs are already bounded, but the combined
        // blob is capped at the event-payload budget deterministically.
        let payload = crate::history::clean_payload(&payload_raw);
        // Same for the composed summary line: field bounds compose past
        // the history budget (a 2048-char summary plus a 512-char command
        // identity), so the history seam's truncation-with-marker applies
        // here exactly as it does for every P5 task transition event.
        let summary_line =
            crate::history::clean_text(&summary_line, crate::types::MAX_HISTORY_SUMMARY_CHARS);
        // Task-bound session first (separate transaction; harmless if the
        // mutation later fails), mirroring every other task write path.
        let task_session = self.task_session_id(&ws, task_id.trim(), now);
        self.with_conn(|conn| {
            // IMMEDIATE rather than deferred (the P5 transition paths use
            // deferred): the write lock is acquired up front, so
            // concurrent cross-process writers serialize on the busy
            // timeout instead of snapshotting a read view and then failing
            // the check-then-act upgrade with `database is locked`. The
            // transaction stays short (one probe + one insert), and the
            // same serialization makes the dedup-key idempotency check
            // atomic across processes. Rollback discipline is explicit:
            // every error path below rolls back before returning.
            conn.execute_batch("BEGIN IMMEDIATE")?;
            let result: Result<TaskOutcomeRecord, ContextError> = (|| {
                // Existence + workspace isolation. The row itself is untouched.
                let task = Self::load_task_for(conn, &ws, task_id.trim())?;
                let record = EventRecord {
                    id: None,
                    session_id: task_session.clone(),
                    workspace_root: task.workspace_root.clone(),
                    task_id: Some(task.task_id.clone()),
                    kind: HistoryKind::TaskOutcome.as_str().to_string(),
                    tool: Some("task".to_string()),
                    path: None,
                    outcome: Some(outcome_label.to_string()),
                    summary: Some(summary_line.clone()),
                    payload: Some(payload.clone()),
                    dedup_key,
                    source: Some("mcp:task".to_string()),
                    digest: None,
                    created_at: now,
                };
                let (id, duplicate) = record_event_in_tx(conn, &record, now)?;
                Ok(TaskOutcomeRecord {
                    event_id: id,
                    duplicate,
                    classification: input.classification,
                    authority: authority.to_string(),
                })
            })();
            match result {
                Ok(rec) => {
                    conn.execute_batch("COMMIT")?;
                    Ok(rec)
                }
                Err(e) => {
                    let _ = conn.execute_batch("ROLLBACK");
                    Err(e)
                }
            }
        })
    }

    // ── Checkpoints ────────────────────────────────────────────────────

    /// Create a new immutable checkpoint (new version) and move the
    /// task's pointer in one transaction. The task row, checkpoint row,
    /// and history event commit together — the pointer can never
    /// reference a nonexistent checkpoint.
    pub fn create_task_checkpoint(
        &self,
        ctx: &TaskMutationCtx<'_>,
        input: &CheckpointInput<'_>,
    ) -> Result<TaskCheckpoint, ContextError> {
        let summary = input.summary;
        let progress = input.progress;
        let next_action = input.next_action;
        let validation_status = input.validation_status;
        let metadata_json = input.metadata_json;
        let ws = canonical_workspace_key(ctx.workspace_root);
        let summary_bounded =
            Self::redacted_bounded(summary, MAX_CHECKPOINT_SUMMARY_CHARS, "checkpoint summary")?;
        if summary_bounded.trim().is_empty() {
            return Err(ContextError::Validation(
                "checkpoint summary must not be empty".to_string(),
            ));
        }
        let progress_bounded = match progress {
            Some(p) => Some(Self::redacted_bounded(
                p,
                MAX_CHECKPOINT_FIELD_CHARS,
                "checkpoint progress",
            )?),
            None => None,
        };
        let next_bounded = match next_action {
            Some(p) => Some(Self::redacted_bounded(
                p,
                MAX_CHECKPOINT_FIELD_CHARS,
                "checkpoint next_action",
            )?),
            None => None,
        };
        let validation_bounded = match validation_status {
            Some(v) => Some(Self::redacted_bounded(
                v,
                64,
                "checkpoint validation_status",
            )?),
            None => None,
        };
        let metadata_bounded = match metadata_json {
            Some(m) => Some(Self::redacted_bounded(
                m,
                MAX_CHECKPOINT_METADATA_CHARS,
                "checkpoint metadata",
            )?),
            None => None,
        };
        if let Some(m) = metadata_bounded.as_deref() {
            let parsed: serde_json::Value = serde_json::from_str(m).map_err(|e| {
                ContextError::Validation(format!("metadata must be a JSON object: {e}"))
            })?;
            if !parsed.is_object() {
                return Err(ContextError::Validation(
                    "metadata must be a JSON object".to_string(),
                ));
            }
        }

        // Task-bound session first (separate transaction;
        // harmless if the mutation later fails),
        let task_session = self.task_session_id(&ws, ctx.task_id, ctx.now);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, ctx.task_id)?;
            ctx.check_stale(&task)?;
            if task.status.is_terminal() {
                return Err(ContextError::Validation(format!(
                    "terminal task cannot gain checkpoints (task is {})",
                    task.status.as_str()
                )));
            }
            Self::enforce_lease(&mut task, ctx.worker, ctx.lease_version, ctx.now)?;

            // Version = last checkpoint version + 1 (immutable, unique).
            let next_version: i64 = tx
                .query_row(
                    "SELECT COALESCE(MAX(version), 0) + 1 FROM task_checkpoints WHERE task_id = ?1",
                    [&task.task_id],
                    |row| row.get(0),
                )
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let checkpoint_id = mint_checkpoint_id(&tx, ctx.now)?;
            tx.execute(
                "INSERT INTO task_checkpoints (checkpoint_id, task_id, version, summary,
                    state, progress, next_action, validation_status, metadata_json,
                    lease_version, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    checkpoint_id,
                    task.task_id,
                    next_version,
                    summary_bounded,
                    task.status.as_str(),
                    progress_bounded,
                    next_bounded,
                    validation_bounded,
                    metadata_bounded.clone().unwrap_or_default(),
                    task.lease_version as i64,
                    ctx.now as i64,
                ],
            )?;
            // Move the pointer + bump the task version in the same
            // transaction. The lease is heartbeated only when the caller
            // is the lease holder: a checkpoint by anyone else (or on a
            // holderless paused/pending task) must never invent or extend
            // lease state behind the owner's back.
            if task.lease_worker.as_deref() == Some(ctx.worker) {
                tx.execute(
                    "UPDATE tasks SET current_checkpoint_id = ?1, current_version = current_version + 1,
                        updated_at = ?2, lease_expires_at = ?3, lease_heartbeat_at = ?2
                     WHERE task_id = ?4",
                    params![
                        checkpoint_id,
                        ctx.now as i64,
                        (ctx.now + TASK_LEASE_TTL_SECS) as i64,
                        task.task_id,
                    ],
                )?;
            } else {
                tx.execute(
                    "UPDATE tasks SET current_checkpoint_id = ?1, current_version = current_version + 1,
                        updated_at = ?2
                     WHERE task_id = ?3",
                    params![checkpoint_id, ctx.now as i64, task.task_id,],
                )?;
            }
            let checkpoint = TaskCheckpoint {
                checkpoint_id: checkpoint_id.clone(),
                task_id: task.task_id.clone(),
                version: next_version as u64,
                summary: summary_bounded,
                state: task.status,
                progress: progress_bounded,
                next_action: next_bounded,
                validation_status: validation_bounded,
                metadata_json: metadata_bounded,
                lease_version: task.lease_version,
                created_at: ctx.now,
            };
            let event = TaskEvent {
                kind: HistoryKind::TaskCheckpoint,
                session_id: task_session.clone(),
                summary: format!(
                    "checkpoint v{next_version}: {summary_line}",
                    summary_line = checkpoint.summary.chars().take(120).collect::<String>()
                ),
                outcome: None,
                payload: Some(
                    serde_json::json!({
                        "task_id": task.task_id,
                        "checkpoint_id": checkpoint_id,
                        "version": next_version,
                    })
                    .to_string(),
                ),
            };
            record_task_event_in_tx(&tx, &task, &event, ctx.now)?;
            tx.commit()?;
            Ok(checkpoint)
        })
    }

    /// Fetch the latest checkpoint (or a specific version) for a task,
    /// workspace-checked.
    pub fn get_task_checkpoint(
        &self,
        workspace_root: &str,
        task_id: &str,
        version: Option<u64>,
    ) -> Result<Option<TaskCheckpoint>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            // Workspace isolation: the task must belong to the caller's
            // workspace before its checkpoints are visible.
            let owner: Option<String> = conn
                .query_row(
                    "SELECT workspace_root FROM tasks WHERE task_id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            match owner {
                Some(ws_root) if ws_root == ws => {}
                _ => return Ok(None),
            }
            match version {
                Some(v) => conn
                    .query_row(
                        &format!(
                            "SELECT {CHECKPOINT_COLUMNS} FROM task_checkpoints
                             WHERE task_id = ?1 AND version = ?2"
                        ),
                        params![task_id, v as i64],
                        row_to_checkpoint,
                    )
                    .optional()
                    .map_err(|e| ContextError::Decode(e.to_string())),
                None => conn
                    .query_row(
                        &format!(
                            "SELECT {CHECKPOINT_COLUMNS} FROM task_checkpoints
                             WHERE task_id = ?1 ORDER BY version DESC LIMIT 1"
                        ),
                        [task_id],
                        row_to_checkpoint,
                    )
                    .optional()
                    .map_err(|e| ContextError::Decode(e.to_string())),
            }
        })
    }

    /// List checkpoint headers (id + version + created_at, no payloads)
    /// for a task, workspace-checked.
    pub fn list_task_checkpoints(
        &self,
        workspace_root: &str,
        task_id: &str,
    ) -> Result<Vec<TaskCheckpoint>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            let owner: Option<String> = conn
                .query_row(
                    "SELECT workspace_root FROM tasks WHERE task_id = ?1",
                    [task_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            match owner {
                Some(ws_root) if ws_root == ws => {}
                _ => return Ok(Vec::new()),
            }
            let mut stmt = conn.prepare(&format!(
                "SELECT {CHECKPOINT_COLUMNS} FROM task_checkpoints
                 WHERE task_id = ?1 ORDER BY version DESC LIMIT 100"
            ))?;
            let rows = stmt
                .query_map([task_id], row_to_checkpoint)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Renew (heartbeat) the caller's lease on a live task. Refused for
    /// stale workers (fencing) and non-holders — and refused once the
    /// lease already expired: recovery after expiry goes through explicit
    /// `resume` (which fences the version forward), never a silent renew.
    pub fn heartbeat_task_lease(
        &self,
        workspace_root: &str,
        task_id: &str,
        worker: &str,
        lease_version: u64,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, task_id)?;
            Self::enforce_lease(&mut task, worker, lease_version, now)?;
            if task.lease_worker.as_deref() != Some(worker) {
                return Err(ContextError::Validation(format!(
                    "worker {worker} does not hold the lease on task {task_id}"
                )));
            }
            if task.lease_expires_at.map(|e| now >= e).unwrap_or(true) {
                return Err(ContextError::Validation(
                    "task lease expired — resume explicitly to re-acquire it".to_string(),
                ));
            }
            tx.execute(
                "UPDATE tasks SET lease_expires_at = ?1, lease_heartbeat_at = ?2,
                    updated_at = ?2
                 WHERE task_id = ?3",
                params![(now + TASK_LEASE_TTL_SECS) as i64, now as i64, task_id],
            )?;
            task.lease_expires_at = Some(now + TASK_LEASE_TTL_SECS);
            task.lease_heartbeat_at = Some(now);
            task.updated_at = now;
            tx.commit()?;
            Ok(task)
        })
    }

    /// Update the task's skill association (reference-only). Bounded and
    /// redacted; never mutates skill lifecycle/health.
    pub fn set_task_skill_refs(
        &self,
        workspace_root: &str,
        task_id: &str,
        skill_refs: Vec<String>,
        based_on_version: Option<u64>,
        now: u64,
    ) -> Result<TaskRecord, ContextError> {
        // Redact at the write seam (same policy as create_task): refs are
        // free text surfaced verbatim by every reader.
        let skill_refs: Vec<String> = skill_refs.iter().map(|r| redact(r)).collect();
        validate_skill_refs(&skill_refs)?;
        let ws = canonical_workspace_key(workspace_root);
        let refs_json =
            serde_json::to_string(&skill_refs).map_err(|e| ContextError::Decode(e.to_string()))?;
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut task = Self::load_task_for(&tx, &ws, task_id)?;
            if let Some(anchor) = based_on_version {
                if anchor != task.current_version {
                    return Err(ContextError::Validation(format!(
                        "stale task mutation: based on version {anchor} but the task is at \
                         version {} — re-read and retry",
                        task.current_version
                    )));
                }
            }
            tx.execute(
                "UPDATE tasks SET skill_refs_json = ?1, updated_at = ?2,
                    current_version = current_version + 1
                 WHERE task_id = ?3",
                params![refs_json, now as i64, task_id],
            )?;
            task.skill_refs = skill_refs;
            task.current_version += 1;
            task.updated_at = now;
            tx.commit()?;
            Ok(task)
        })
    }

    /// P6 task ↔ skill debt resolution (read-time only).
    ///
    /// Background: tasks carry opaque `skill_refs` (usage association:
    /// "this task used skill X") while P4 skills may have task-scoped
    /// *records* (lifecycle scoped to a task via `task_id`). These are
    /// two different concepts:
    ///
    /// - `skill_refs`: an opaque, bounded list of skill ids/names the
    ///   executor claims the task used. Never dereferenced on write,
    ///   never mutates skill health.
    /// - Task-scoped skill rows: P4 lifecycle state (`skill_candidates`
    ///   / `skills` with `task_id`) governed by workspace/scope rules.
    ///
    /// This function safely resolves the first concept against the
    /// second at *read time* for display: each ref is matched (exact
    /// `skill_id`, else exact `name`) against workspace-visible skills
    /// (same canonical workspace or global scope). Unmatched refs are
    /// returned as opaque strings — never invented, never hidden.
    /// No writes, no joins that cross workspaces, deterministic ordering,
    /// bounded output (≤16 refs by validation).
    pub fn resolve_task_skill_refs(
        &self,
        workspace_root: &str,
        task_id: &str,
    ) -> Result<SkillRefResolution, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            let task = Self::load_task_for(conn, &ws, task_id)?;
            let mut resolved: Vec<ResolvedSkillRef> = Vec::new();
            let mut unresolved: Vec<String> = Vec::new();
            for r in &task.skill_refs {
                // Exact skill_id match first (workspace-scoped or global).
                let by_id: Option<(String, String, String)> = conn
                    .query_row(
                        "SELECT skill_id, name, status FROM skills
                         WHERE skill_id = ?1 AND (workspace_root = ?2 OR scope = 'global')
                         LIMIT 1",
                        params![r, ws],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                if let Some((skill_id, name, status)) = by_id {
                    resolved.push(ResolvedSkillRef {
                        reference: r.clone(),
                        skill_id: Some(skill_id),
                        name: Some(name),
                        status: Some(status),
                    });
                    continue;
                }
                // Exact name match (workspace-scoped preferred, else global).
                let by_name: Option<(String, String, String)> = conn
                    .query_row(
                        "SELECT skill_id, name, status FROM skills
                         WHERE name = ?1 AND (workspace_root = ?2 OR scope = 'global')
                         ORDER BY CASE WHEN workspace_root = ?2 THEN 0 ELSE 1 END
                         LIMIT 1",
                        params![r, ws],
                        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                    )
                    .optional()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                match by_name {
                    Some((skill_id, name, status)) => resolved.push(ResolvedSkillRef {
                        reference: r.clone(),
                        skill_id: Some(skill_id),
                        name: Some(name),
                        status: Some(status),
                    }),
                    None => unresolved.push(r.clone()),
                }
            }
            resolved.sort_by(|a, b| a.reference.cmp(&b.reference));
            unresolved.sort();
            Ok(SkillRefResolution {
                resolved,
                unresolved,
            })
        })
    }

    /// Compose the bounded resume snapshot for a task: task identity,
    /// latest checkpoint, recent task-scoped history (bounded), active
    /// skill association, and validation state. Never a transcript
    /// replay; bounded like every other context surface.
    pub fn task_resume_snapshot(
        &self,
        workspace_root: &str,
        task_id: &str,
        now: u64,
    ) -> Result<TaskResumeSnapshot, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        self.with_conn(|conn| {
            let task = Self::load_task_for(conn, &ws, task_id)?;
            let mut task = task;
            task.stale = Some(task.is_stale(now));
            let latest_checkpoint = conn
                .query_row(
                    &format!(
                        "SELECT {CHECKPOINT_COLUMNS} FROM task_checkpoints
                         WHERE task_id = ?1 ORDER BY version DESC LIMIT 1"
                    ),
                    [&task.task_id],
                    row_to_checkpoint,
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            // Recent task-scoped events (bounded: 10, oldest-first).
            let recent: Vec<(i64, Option<String>)> = {
                let mut stmt = conn.prepare(
                    "SELECT id, summary FROM events WHERE task_id = ?1
                     ORDER BY created_at DESC, id DESC LIMIT 10",
                )?;
                let collected = stmt
                    .query_map([&task.task_id], |row| {
                        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
                    })
                    .map_err(|e| ContextError::Decode(e.to_string()))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                collected
            };
            let recent_events: Vec<TaskResumeEvent> = recent
                .into_iter()
                .rev()
                .map(|(id, summary)| TaskResumeEvent {
                    event_id: id,
                    summary: summary.unwrap_or_default(),
                })
                .collect();
            // Intent reference (soft): surface a terminal-intent note
            // without rewriting intent history. A missing intent record
            // is surfaced as such (dangling reference), never fatal.
            let intent_note: Option<String> = match task.intent_record_id.clone() {
                None => None,
                Some(rid) => {
                    // A completed/cancelled intent keeps its original row
                    // as `superseded` with a terminal successor (the P1
                    // audit trail: `supersedes` points backward). The
                    // newest successor's status is the intent's live
                    // state; without a successor, the row's own status.
                    let successor: Option<String> = conn
                        .query_row(
                            "SELECT id FROM context_records WHERE supersedes = ?1
                             ORDER BY updated_at DESC, id LIMIT 1",
                            [&rid],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    let latest_id = successor.unwrap_or_else(|| rid.clone());
                    let status: Option<String> = conn
                        .query_row(
                            "SELECT status FROM context_records WHERE id = ?1",
                            [&latest_id],
                            |row| row.get(0),
                        )
                        .optional()
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    let note = match status.as_deref() {
                        None => format!("referenced intent {rid} no longer exists"),
                        Some("expired") => format!("referenced intent {rid} is completed"),
                        Some("rejected") => {
                            format!("referenced intent {rid} is cancelled/rejected")
                        }
                        Some(other) => format!("referenced intent {rid} is {other}"),
                    };
                    Some(note)
                }
            };
            Ok(TaskResumeSnapshot {
                skill_refs: task.skill_refs.clone(),
                task,
                latest_checkpoint,
                recent_events,
                intent_note,
            })
        })
    }
}

/// Bounded resume event summary.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskResumeEvent {
    pub event_id: i64,
    pub summary: String,
}

/// The bounded context snapshot returned by resume/inspect.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TaskResumeSnapshot {
    pub task: TaskRecord,
    pub latest_checkpoint: Option<TaskCheckpoint>,
    pub recent_events: Vec<TaskResumeEvent>,
    pub skill_refs: Vec<String>,
    pub intent_note: Option<String>,
}

/// P6 read-time resolution of a task's opaque `skill_refs` against
/// workspace-visible skills. Display-only: no writes, no execution.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedSkillRef {
    /// The original opaque reference string.
    pub reference: String,
    /// Matched skill id (None when unresolved).
    pub skill_id: Option<String>,
    /// Matched skill name (None when unresolved).
    pub name: Option<String>,
    /// Matched skill status (None when unresolved).
    pub status: Option<String>,
}

/// Read-time skill association: resolved matches + opaque leftovers.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SkillRefResolution {
    pub resolved: Vec<ResolvedSkillRef>,
    pub unresolved: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    fn store_at(dir: &std::path::Path) -> ContextStore {
        ContextStore::new(dir.join(db::STATE_DB_FILE))
    }

    fn new_task(title: &str) -> NewTask {
        NewTask {
            title: title.to_string(),
            ..Default::default()
        }
    }

    /// Drive a task to running under a fresh worker in /repo-a.
    fn running_task(dir: &std::path::Path) -> (ContextStore, TaskRecord, String) {
        let s = store_at(dir);
        let worker = mint_worker_id();
        let t = s
            .create_task("/repo-a", &new_task("implement X"), 100)
            .unwrap();
        let t = s
            .start_task("/repo-a", &t.task_id, &worker, None, 110)
            .unwrap();
        (s, t, worker)
    }

    fn ctx<'a>(
        task_id: &'a str,
        worker: &'a str,
        lease_version: u64,
        based_on: Option<u64>,
        now: u64,
    ) -> TaskMutationCtx<'a> {
        TaskMutationCtx {
            workspace_root: "/repo-a",
            task_id,
            worker,
            lease_version,
            based_on_version: based_on,
            now,
        }
    }

    fn ctx_ws<'a>(
        ws: &'a str,
        task_id: &'a str,
        worker: &'a str,
        lease_version: u64,
        now: u64,
    ) -> TaskMutationCtx<'a> {
        TaskMutationCtx {
            workspace_root: ws,
            task_id,
            worker,
            lease_version,
            based_on_version: None,
            now,
        }
    }

    fn cp_input<'a>(
        summary: &'a str,
        progress: Option<&'a str>,
        next_action: Option<&'a str>,
        metadata_json: Option<&'a str>,
    ) -> CheckpointInput<'a> {
        CheckpointInput {
            summary,
            progress,
            next_action,
            validation_status: None,
            metadata_json,
        }
    }

    /// Drive a running task through validation → passed result.
    fn validate_passed(s: &ContextStore, task: &TaskRecord, worker: &str, now: u64) -> TaskRecord {
        s.start_task_validation(
            &ctx(&task.task_id, worker, task.lease_version, None, now),
            "cargo test",
        )
        .unwrap();
        s.record_task_validation(
            &ctx(&task.task_id, worker, task.lease_version, None, now + 10),
            "cargo test",
            TaskValidationResult::Passed,
            Some("42 passed"),
        )
        .unwrap()
    }

    // ── State machine ─────────────────────────────────────────────────

    #[test]
    fn transition_matrix_is_strict() {
        let allowed = [
            (TaskStatus::Pending, TaskStatus::Running),
            (TaskStatus::Pending, TaskStatus::Cancelled),
            (TaskStatus::Running, TaskStatus::Paused),
            (TaskStatus::Running, TaskStatus::Validating),
            (TaskStatus::Running, TaskStatus::Failed),
            (TaskStatus::Running, TaskStatus::Cancelled),
            (TaskStatus::Paused, TaskStatus::Running),
            (TaskStatus::Paused, TaskStatus::Cancelled),
            (TaskStatus::Validating, TaskStatus::Running),
            (TaskStatus::Validating, TaskStatus::Completed),
            (TaskStatus::Validating, TaskStatus::Failed),
            (TaskStatus::Validating, TaskStatus::Cancelled),
        ];
        for (from, to) in allowed {
            assert!(
                task_transition_allowed(from, to),
                "{from} → {to} must be allowed"
            );
        }
        let all = [
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Validating,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ];
        for from in all {
            for to in all {
                if !allowed.contains(&(from, to)) && from != to {
                    assert!(
                        !task_transition_allowed(from, to),
                        "{from} → {to} must be rejected"
                    );
                }
            }
        }
        for s in all {
            assert!(!task_transition_allowed(s, s), "{s} → {s}");
        }
    }

    #[test]
    fn statuses_roundtrip() {
        for s in [
            TaskStatus::Pending,
            TaskStatus::Running,
            TaskStatus::Paused,
            TaskStatus::Validating,
            TaskStatus::Completed,
            TaskStatus::Failed,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(s.to_string().parse::<TaskStatus>().unwrap(), s);
        }
        assert!("nonsense".parse::<TaskStatus>().is_err());
        assert!(TaskStatus::Completed.is_terminal());
        assert!(TaskStatus::Failed.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::Running.is_terminal());
    }

    // ── Create / list / idempotency ───────────────────────────────────

    #[test]
    fn create_start_full_lifecycle_with_events() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        assert_eq!(task.status, TaskStatus::Running);
        assert_eq!(task.lease_worker.as_deref(), Some(worker.as_str()));
        assert!(task.lease_expires_at.is_some());
        assert_eq!(task.current_version, 1);

        let cp = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, task.lease_version, None, 120),
                &cp_input(
                    "runtime core done",
                    Some("MCP wiring remains"),
                    Some("wire the task tool"),
                    Some(r#"{"files":["crates/tasks.rs"]}"#),
                ),
            )
            .unwrap();
        assert_eq!(cp.version, 1);
        let after = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(
            after.current_checkpoint_id.as_deref(),
            Some(cp.checkpoint_id.as_str())
        );

        let t = validate_passed(&s, &task, &worker, 130);
        assert_eq!(t.status, TaskStatus::Validating);
        let t = s
            .complete_task(
                &ctx(&task.task_id, &worker, t.lease_version, None, 150),
                "task done",
                vec!["crates/tasks.rs".into()],
            )
            .unwrap();
        assert_eq!(t.status, TaskStatus::Completed);
        assert!(t.lease_worker.is_none(), "terminal tasks release the lease");
        let outcome = t.outcome.expect("outcome present");
        assert_eq!(outcome.result, "completed");
        assert_eq!(outcome.changed_areas, vec!["crates/tasks.rs"]);
        assert_eq!(outcome.evidence_event_ids.len(), 1);

        let mut events = s.list_events("/repo-a", 50).unwrap();
        events.reverse();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "task_created",
                "task_started",
                "task_checkpoint",
                "task_validation_started",
                "task_validation_passed",
                "task_completed",
            ]
        );
    }

    #[test]
    fn validation_failure_returns_to_running() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        s.start_task_validation(
            &ctx(&task.task_id, &worker, task.lease_version, None, 120),
            "cargo test",
        )
        .unwrap();
        let t = s
            .record_task_validation(
                &ctx(&task.task_id, &worker, task.lease_version, None, 130),
                "cargo test",
                TaskValidationResult::Failed,
                Some("3 failures"),
            )
            .unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        let events = s.list_events("/repo-a", 10).unwrap();
        assert!(events.iter().any(|e| e.kind == "task_validation_failed"));
    }

    #[test]
    fn completion_gate_requires_passed_validation() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        // complete from running: refused.
        let err = s
            .complete_task(
                &ctx(&task.task_id, &worker, task.lease_version, None, 120),
                "no",
                vec![],
            )
            .unwrap_err();
        assert!(err.to_string().contains("validating"), "{err}");
        // validate → failed → back to running; complete still refused.
        s.start_task_validation(
            &ctx(&task.task_id, &worker, task.lease_version, None, 120),
            "cargo test",
        )
        .unwrap();
        let t = s
            .record_task_validation(
                &ctx(&task.task_id, &worker, task.lease_version, None, 130),
                "cargo test",
                TaskValidationResult::Failed,
                None,
            )
            .unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        let err = s
            .complete_task(
                &ctx(&task.task_id, &worker, t.lease_version, None, 140),
                "no",
                vec![],
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("validating") || err.to_string().contains("transition"),
            "{err}"
        );
    }

    #[test]
    fn idempotency_key_deduplicates_creates() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let mut input = new_task("same task");
        input.idempotency_key = Some("fix-gpu-init".into());
        let a = s.create_task("/repo-a", &input, 100).unwrap();
        let b = s.create_task("/repo-a", &input, 200).unwrap();
        assert_eq!(a.task_id, b.task_id);
        let c = s
            .create_task("/repo-a", &new_task("similar title"), 300)
            .unwrap();
        let d = s
            .create_task("/repo-a", &new_task("similar title"), 400)
            .unwrap();
        assert_ne!(c.task_id, d.task_id);
        let e = s.create_task("/repo-b", &input, 500).unwrap();
        assert_ne!(a.task_id, e.task_id);
        assert_eq!(
            s.list_tasks("/repo-a", None, None, 100, 600).unwrap().len(),
            3
        );
    }

    #[test]
    fn task_ids_are_opaque_and_safe() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let t = s.create_task("/repo-a", &new_task("t"), 100).unwrap();
        assert!(t.task_id.starts_with("task::"));
        assert_eq!(t.task_id.len(), "task::".len() + 16);
        assert!(!t.task_id.contains('/'));
        assert!(s.create_task("/repo-a", &new_task("  "), 100).is_err());
        assert!(s
            .create_task(
                "/repo-a",
                &new_task(&"x".repeat(MAX_TASK_TITLE_CHARS + 1)),
                100
            )
            .is_err());
    }

    // ── Workspace isolation ───────────────────────────────────────────

    #[test]
    fn workspace_isolation_holds_at_every_seam() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let worker_b = mint_worker_id();
        let _ = &worker_b;

        assert!(s.get_task("/repo-b", &task.task_id).unwrap().is_none());
        assert!(s
            .list_tasks("/repo-b", None, None, 100, 120)
            .unwrap()
            .is_empty());
        assert!(s.stale_tasks("/repo-b", 120).unwrap().is_empty());
        assert!(s
            .get_task_checkpoint("/repo-b", &task.task_id, None)
            .unwrap()
            .is_none());
        assert!(s
            .list_task_checkpoints("/repo-b", &task.task_id)
            .unwrap()
            .is_empty());

        let calls: Vec<Result<(), ContextError>> = vec![
            s.start_task("/repo-b", &task.task_id, &worker, None, 120)
                .map(|_| ()),
            s.pause_task("/repo-b", &task.task_id, &worker, 1, None, 120)
                .map(|_| ()),
            s.resume_task("/repo-b", &task.task_id, &worker, None, 120)
                .map(|_| ()),
            s.start_task_validation(&ctx_ws("/repo-b", &task.task_id, &worker, 1, 120), "x")
                .map(|_| ()),
            s.complete_task(
                &ctx_ws("/repo-b", &task.task_id, &worker, 1, 120),
                "x",
                vec![],
            )
            .map(|_| ()),
            s.fail_task(&ctx_ws("/repo-b", &task.task_id, &worker, 1, 120), "x")
                .map(|_| ()),
            s.cancel_task("/repo-b", &task.task_id, Some("x"), None, 120)
                .map(|_| ()),
            s.create_task_checkpoint(
                &ctx_ws("/repo-b", &task.task_id, &worker, 1, 120),
                &cp_input("x", None, None, None),
            )
            .map(|_| ()),
            s.set_task_skill_refs("/repo-b", &task.task_id, vec!["sk::1".into()], None, 120)
                .map(|_| ()),
        ];
        for call in calls {
            let err = call.unwrap_err();
            assert!(
                err.to_string().contains("another workspace")
                    || err.to_string().contains("does not exist"),
                "cross-workspace call must be refused: {err}"
            );
        }
        let still = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(still.status, TaskStatus::Running);
    }

    // ── Optimistic concurrency ────────────────────────────────────────

    #[test]
    fn stale_based_on_version_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let read_a = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        let read_b = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        s.pause_task(
            "/repo-a",
            &task.task_id,
            &worker,
            read_a.lease_version,
            Some(read_a.current_version),
            120,
        )
        .unwrap();
        let err = s
            .pause_task(
                "/repo-a",
                &task.task_id,
                &worker,
                read_b.lease_version,
                Some(read_b.current_version),
                130,
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        // Checkpoint race on the same anchor: exactly one wins.
        let resumed = s
            .resume_task("/repo-a", &task.task_id, &worker, None, 140)
            .unwrap();
        let anchor = resumed.current_version;
        let cp1 = s
            .create_task_checkpoint(
                &ctx(
                    &resumed.task_id,
                    &worker,
                    resumed.lease_version,
                    Some(anchor),
                    150,
                ),
                &cp_input("a", None, None, None),
            )
            .unwrap();
        let err = s
            .create_task_checkpoint(
                &ctx(
                    &resumed.task_id,
                    &worker,
                    resumed.lease_version,
                    Some(anchor),
                    160,
                ),
                &cp_input("b", None, None, None),
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        assert_eq!(cp1.version, 1);
        assert_eq!(
            s.list_task_checkpoints("/repo-a", &task.task_id)
                .unwrap()
                .len(),
            1
        );
    }

    // ── Lease / fencing ────────────────────────────────────────────────

    #[test]
    fn second_worker_cannot_mutate_while_lease_live() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker_a) = running_task(dir.path());
        let worker_b = mint_worker_id();
        let err = s
            .pause_task(
                "/repo-a",
                &task.task_id,
                &worker_b,
                task.lease_version,
                None,
                120,
            )
            .map(|_| ())
            .unwrap_err();
        assert!(
            err.to_string().contains("leased by another worker"),
            "{err}"
        );
        let err = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker_b, task.lease_version, None, 120),
                &cp_input("x", None, None, None),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("leased by another worker"),
            "{err}"
        );
        let err = s
            .resume_task("/repo-a", &task.task_id, &worker_b, None, 120)
            .unwrap_err();
        assert!(
            err.to_string().contains("lease") || err.to_string().contains("resume"),
            "{err}"
        );
        let err = s
            .heartbeat_task_lease("/repo-a", &task.task_id, &worker_b, task.lease_version, 120)
            .unwrap_err();
        assert!(
            err.to_string().contains("does not hold")
                || err.to_string().contains("leased by another worker"),
            "{err}"
        );
        let _ = worker_a;
    }

    #[test]
    fn expired_lease_allows_takeover_and_fencing_refuses_stale_worker() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker_a) = running_task(dir.path());
        let lease_version_a = task.lease_version;
        let now_after_expiry = task.lease_expires_at.unwrap() + 1;

        let worker_b = mint_worker_id();
        let taken = s
            .resume_task("/repo-a", &task.task_id, &worker_b, None, now_after_expiry)
            .unwrap();
        assert!(
            taken.lease_version > lease_version_a,
            "fencing advances on takeover"
        );
        assert_eq!(taken.lease_worker.as_deref(), Some(worker_b.as_str()));

        // A (old lease version) is fenced out of everything.
        let calls: Vec<Result<(), ContextError>> = vec![
            s.pause_task(
                "/repo-a",
                &task.task_id,
                &worker_a,
                lease_version_a,
                None,
                now_after_expiry + 10,
            )
            .map(|_| ()),
            s.create_task_checkpoint(
                &ctx(
                    &task.task_id,
                    &worker_a,
                    lease_version_a,
                    None,
                    now_after_expiry + 10,
                ),
                &cp_input("stale write", None, None, None),
            )
            .map(|_| ()),
            s.heartbeat_task_lease(
                "/repo-a",
                &task.task_id,
                &worker_a,
                lease_version_a,
                now_after_expiry + 10,
            )
            .map(|_| ()),
        ];
        for call in calls {
            let err = call.unwrap_err();
            assert!(
                err.to_string().contains("stale worker"),
                "fenced worker must be refused with a stale-worker error: {err}"
            );
        }
        let fresh = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert!(
            fresh.current_checkpoint_id.is_none(),
            "A's stale checkpoint must not exist"
        );
    }

    #[test]
    fn heartbeat_extends_the_lease() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let before = task.lease_expires_at.unwrap();
        let beat = s
            .heartbeat_task_lease("/repo-a", &task.task_id, &worker, task.lease_version, 120)
            .unwrap();
        assert!(beat.lease_expires_at.unwrap() >= before);
        assert!(!beat.is_stale(120));
        assert!(beat.is_stale(beat.lease_expires_at.unwrap() + 1));
    }

    // ── Interruption / recovery ───────────────────────────────────────

    #[test]
    fn interrupted_running_task_is_stale_and_recoverable() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, _worker) = running_task(dir.path());
        let later = task.lease_expires_at.unwrap() + 10;
        let stale = s.stale_tasks("/repo-a", later).unwrap();
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].task_id, task.task_id);
        let on_disk = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(on_disk.status, TaskStatus::Running);
        let new_worker = mint_worker_id();
        let resumed = s
            .resume_task("/repo-a", &task.task_id, &new_worker, None, later)
            .unwrap();
        assert_eq!(resumed.status, TaskStatus::Running);
        assert_eq!(resumed.lease_worker.as_deref(), Some(new_worker.as_str()));
        assert!(!resumed.is_stale(later));
    }

    #[test]
    fn resume_snapshot_is_bounded_and_complete() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let lv = task.lease_version;
        s.create_task_checkpoint(
            &ctx(&task.task_id, &worker, lv, None, 120),
            &cp_input(
                "core done",
                Some("tests remain"),
                Some("run tests"),
                Some(r#"{"f":"x"}"#),
            ),
        )
        .unwrap();
        s.create_task_checkpoint(
            &ctx(&task.task_id, &worker, lv, None, 130),
            &cp_input("tests done", None, None, None),
        )
        .unwrap();
        let snapshot = s
            .task_resume_snapshot("/repo-a", &task.task_id, 140)
            .unwrap();
        assert_eq!(snapshot.task.task_id, task.task_id);
        let cp = snapshot.latest_checkpoint.expect("latest checkpoint");
        assert_eq!(cp.version, 2);
        assert_eq!(cp.summary, "tests done");
        assert!(snapshot.recent_events.len() >= 3);
        assert!(snapshot.recent_events.len() <= 10, "bounded to 10");
        let ids: Vec<i64> = snapshot.recent_events.iter().map(|e| e.event_id).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);
        assert!(s
            .task_resume_snapshot("/repo-b", &task.task_id, 140)
            .is_err());
    }

    // ── Checkpoints ───────────────────────────────────────────────────

    #[test]
    fn checkpoints_are_immutable_versions() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let lv = task.lease_version;
        let cp1 = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, lv, None, 110),
                &cp_input("v1", None, None, None),
            )
            .unwrap();
        let cp2 = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, lv, None, 120),
                &cp_input("v2", None, None, None),
            )
            .unwrap();
        assert_eq!(cp1.version, 1);
        assert_eq!(cp2.version, 2);
        assert_ne!(cp1.checkpoint_id, cp2.checkpoint_id);
        let fetched_v1 = s
            .get_task_checkpoint("/repo-a", &task.task_id, Some(1))
            .unwrap()
            .unwrap();
        assert_eq!(fetched_v1.summary, "v1");
        let list = s.list_task_checkpoints("/repo-a", &task.task_id).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].version, 2);
        let task_after = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(
            task_after.current_checkpoint_id.as_deref(),
            Some(cp2.checkpoint_id.as_str())
        );
        s.cancel_task(
            "/repo-a",
            &task.task_id,
            Some("done differently"),
            None,
            130,
        )
        .unwrap();
        let err = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, lv, None, 140),
                &cp_input("late", None, None, None),
            )
            .unwrap_err();
        assert!(err.to_string().contains("terminal"), "{err}");
    }

    #[test]
    fn checkpoint_validation_rejects_junk() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let c = |summary: &str, meta: Option<&str>| {
            s.create_task_checkpoint(
                &ctx(&task.task_id, &worker, task.lease_version, None, 110),
                &cp_input(summary, None, None, meta),
            )
        };
        assert!(c("  ", None).is_err());
        assert!(c("s", Some("not json")).is_err());
        assert!(c("s", Some("[1,2]")).is_err());
        assert!(c(&"x".repeat(MAX_CHECKPOINT_SUMMARY_CHARS + 1), None).is_err());
    }

    // ── Security: redaction, bounds ────────────────────────────────────

    #[test]
    fn secrets_are_redacted_from_checkpoints_and_events() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let poisoned = format!("used api_key={} to deploy", "A".repeat(24));
        let cp = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, task.lease_version, None, 120),
                &cp_input(&poisoned, None, None, None),
            )
            .unwrap();
        assert!(
            !cp.summary.contains(&"A".repeat(24)),
            "secret must be redacted: {}",
            cp.summary
        );
        let events = s.list_events("/repo-a", 10).unwrap();
        for e in &events {
            assert!(!e.summary.as_deref().unwrap_or("").contains(&"A".repeat(24)));
            assert!(!e.payload.as_deref().unwrap_or("").contains(&"A".repeat(24)));
        }
        let t = s
            .create_task(
                "/repo-a",
                &NewTask {
                    title: "deploy work".into(),
                    description: Some(poisoned.clone()),
                    ..Default::default()
                },
                130,
            )
            .unwrap();
        assert!(!t.description.unwrap().contains(&"A".repeat(24)));
    }

    #[test]
    fn skill_refs_are_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, _worker) = running_task(dir.path());
        let many: Vec<String> = (0..MAX_TASK_SKILL_REFS + 1)
            .map(|i| format!("sk::{i}"))
            .collect();
        assert!(s
            .set_task_skill_refs("/repo-a", &task.task_id, many, None, 120)
            .is_err());
        let ok = s
            .set_task_skill_refs("/repo-a", &task.task_id, vec!["sk::abc".into()], None, 120)
            .unwrap();
        assert_eq!(ok.skill_refs, vec!["sk::abc".to_string()]);
    }

    #[test]
    fn skill_refs_resolve_at_read_time_without_crossing_workspaces() {
        // P6 task↔skill debt: skill_refs (opaque usage association) vs
        // task-scoped skill rows (P4 lifecycle) are different concepts;
        // resolution happens at read time, workspace-scoped, bounded.
        let dir = tempfile::tempdir().unwrap();
        let (s, task, _worker) = running_task(dir.path());
        s.set_task_skill_refs(
            "/repo-a",
            &task.task_id,
            vec!["review-helper".to_string(), "opaque-unknown".to_string()],
            None,
            160,
        )
        .unwrap();
        let res = s.resolve_task_skill_refs("/repo-a", &task.task_id).unwrap();
        // With no matching skills, both refs stay opaque — never invented,
        // never hidden, deterministically ordered.
        assert_eq!(
            res.unresolved,
            vec!["opaque-unknown".to_string(), "review-helper".to_string()],
            "{res:?}"
        );
        assert!(res.resolved.is_empty());
        // Cross-workspace: tasks are workspace-bound, so resolution from
        // another workspace is refused entirely (no leakage).
        assert!(s.resolve_task_skill_refs("/repo-b", &task.task_id).is_err());
    }

    // ── Intent / parent / priority ─────────────────────────────────────

    #[test]
    fn parent_and_intent_references_are_soft() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let parent = s.create_task("/repo-a", &new_task("parent"), 100).unwrap();
        let mut child_input = new_task("child");
        child_input.parent_task_id = Some(parent.task_id.clone());
        child_input.intent_record_id = Some("ctx::intent-1".into());
        child_input.priority = Some(TaskPriority::High);
        let child = s.create_task("/repo-a", &child_input, 110).unwrap();
        assert_eq!(
            child.parent_task_id.as_deref(),
            Some(parent.task_id.as_str())
        );
        assert_eq!(child.priority, TaskPriority::High);
        let snapshot = s
            .task_resume_snapshot("/repo-a", &child.task_id, 120)
            .unwrap();
        assert_eq!(snapshot.task.task_id, child.task_id);
    }

    #[test]
    fn terminal_intent_is_surfaced_in_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let record = crate::types::ContextRecord::new(
            "ctx::intent-done",
            crate::types::RecordKind::Intent,
            "intent.work",
            "ship the thing",
            crate::types::Authority::UserConfirmed,
        );
        s.put_record(&record, 100).unwrap();
        let mut input = new_task("t");
        input.intent_record_id = Some("ctx::intent-done".into());
        let task = s.create_task("/repo-a", &input, 110).unwrap();
        let snapshot = s
            .task_resume_snapshot("/repo-a", &task.task_id, 120)
            .unwrap();
        assert_eq!(
            snapshot.intent_note.as_deref(),
            Some("referenced intent ctx::intent-done is active")
        );
        let mut replacement = record.clone();
        replacement.id = "ctx::intent-done-2".into();
        replacement.supersedes = Some("ctx::intent-done".into());
        replacement.status = crate::types::RecordStatus::Expired;
        s.retire_record("ctx::intent-done", &replacement, 130)
            .unwrap();
        let snapshot = s
            .task_resume_snapshot("/repo-a", &task.task_id, 140)
            .unwrap();
        assert_eq!(
            snapshot.intent_note.as_deref(),
            Some("referenced intent ctx::intent-done is completed")
        );
        let still = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(still.status, TaskStatus::Pending);
    }

    // ── P3 integration: task events are learning evidence ──────────────

    #[test]
    fn task_events_bind_to_task_sessions() {
        // Phase 7: task events reuse P2 session infrastructure — mutations
        // after creation bind to the task's session (task_id == session's
        // task binding; never task_id == session_id).
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let events = s.list_events("/repo-a", 10).unwrap();
        let started = events
            .iter()
            .find(|e| e.kind == "task_started")
            .expect("started event");
        let session_id = started.session_id.clone().expect("session-linked");
        assert!(!session_id.is_empty() && session_id != task.task_id);
        let session = s.get_session(&session_id).unwrap().expect("session exists");
        assert_eq!(session.task_id.as_deref(), Some(task.task_id.as_str()));
        // The task's session history is queryable.
        let session_events = s.list_session_events(&session_id, 10).unwrap();
        assert!(session_events
            .iter()
            .all(|e| e.session_id.as_deref() == Some(session_id.as_str())));
        // The started event is session-linked; the created event (which
        // could not ensure a session up front) stays reachable via the
        // task id alone — both orderings queryable.
        assert!(session_events.iter().any(|e| e.kind == "task_started"));
        // Created event (pre-session) is still reachable via task id.
        let _ = worker;
    }

    #[test]
    fn task_completion_events_are_valid_learning_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        s.create_task_checkpoint(
            &ctx(&task.task_id, &worker, task.lease_version, None, 110),
            &cp_input("done", None, None, None),
        )
        .unwrap();
        let t = validate_passed(&s, &task, &worker, 120);
        s.complete_task(
            &ctx(&task.task_id, &worker, t.lease_version, None, 140),
            "done",
            vec![],
        )
        .unwrap();

        assert_eq!(
            crate::learning::kind_group("task_completed"),
            crate::learning::KindGroup::Validation
        );
        assert_eq!(
            crate::learning::kind_group("task_validation_passed"),
            crate::learning::KindGroup::Validation
        );
        assert_eq!(
            crate::learning::kind_group("task_validation_failed"),
            crate::learning::KindGroup::Validation
        );
        assert_eq!(
            crate::learning::kind_group("task_failed"),
            crate::learning::KindGroup::Validation
        );
        assert_eq!(
            crate::learning::kind_group("task_started"),
            crate::learning::KindGroup::Observation
        );
        let events = s.list_events("/repo-a", 10).unwrap();
        let completed = events
            .iter()
            .find(|e| e.kind == "task_completed")
            .expect("completion event");
        assert_eq!(
            crate::learning::outcome_polarity(completed.outcome.as_deref()),
            crate::learning::OutcomePolarity::Success
        );
        let fts: i64 = s
            .with_conn(|conn| {
                Ok(conn.query_row(
                    "SELECT count(*) FROM events_fts WHERE events_fts MATCH '\"completed\"'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert!(fts >= 1, "task events must be FTS-searchable");
    }

    // ── Restart durability ─────────────────────────────────────────────

    #[test]
    fn tasks_survive_store_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let cp = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, task.lease_version, None, 120),
                &cp_input("state", None, None, None),
            )
            .unwrap();
        let reopened = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        let t = reopened
            .get_task("/repo-a", &task.task_id)
            .unwrap()
            .unwrap();
        assert_eq!(t.status, TaskStatus::Running);
        assert_eq!(t.lease_worker.as_deref(), Some(worker.as_str()));
        assert_eq!(
            t.current_checkpoint_id.as_deref(),
            Some(cp.checkpoint_id.as_str())
        );
        let events = reopened.list_events("/repo-a", 10).unwrap();
        assert!(events.len() >= 3, "history survives restart");
    }

    #[test]
    fn list_filters_by_status_and_parent() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let parent = s.create_task("/repo-a", &new_task("parent"), 100).unwrap();
        let mut child_input = new_task("child");
        child_input.parent_task_id = Some(parent.task_id.clone());
        let child = s.create_task("/repo-a", &child_input, 110).unwrap();
        s.create_task("/repo-a", &new_task("other"), 120).unwrap();
        s.start_task("/repo-a", &parent.task_id, &mint_worker_id(), None, 130)
            .unwrap();

        assert_eq!(
            s.list_tasks("/repo-a", Some(TaskStatus::Running), None, 100, 140)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            s.list_tasks("/repo-a", Some(TaskStatus::Pending), None, 100, 140)
                .unwrap()
                .len(),
            2
        );
        let kids = s
            .list_tasks("/repo-a", None, Some(&parent.task_id), 100, 140)
            .unwrap();
        assert_eq!(kids.len(), 1);
        assert_eq!(kids[0].task_id, child.task_id);
        // List rows carry derived staleness without checkpoint payloads.
        assert!(kids[0].stale.is_some());
    }

    #[test]
    fn mutation_lock_serializes_concurrent_transition_attempts() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let results: Vec<Result<TaskCheckpoint, ContextError>> = (0..4)
            .map(|i| {
                s.create_task_checkpoint(
                    &ctx(
                        &task.task_id,
                        &worker,
                        task.lease_version,
                        Some(task.current_version),
                        120 + i,
                    ),
                    &cp_input(&format!("cp-{i}"), None, None, None),
                )
            })
            .collect();
        let ok = results.iter().filter(|r| r.is_ok()).count();
        let stale = results.iter().filter(|r| r.is_err()).count();
        assert_eq!(ok, 1, "exactly one writer wins");
        assert_eq!(stale, 3, "the rest are refused as stale");
    }

    // ── Adversarial audit (P5 post-implementation) ──────────────────────
    //
    // Each test below attacks a concrete P5 guarantee: durability,
    // consistency, isolation, fencing, or recoverability. They pin the
    // audited behavior so regressions fail loudly.

    #[test]
    fn audit_paused_tasks_are_not_stale() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let expiry = task.lease_expires_at.unwrap();
        let paused = s
            .pause_task(
                "/repo-a",
                &task.task_id,
                &worker,
                task.lease_version,
                None,
                120,
            )
            .unwrap();
        assert_eq!(paused.status, TaskStatus::Paused);
        // Pausing releases the lease; an intentionally paused task is not
        // interrupted work, even far past the old lease expiry.
        let fetched = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert!(!fetched.is_stale(expiry + 10_000));
        assert!(s
            .stale_tasks("/repo-a", expiry + 10_000)
            .unwrap()
            .is_empty());
        let listed = s
            .list_tasks("/repo-a", None, None, 100, expiry + 10_000)
            .unwrap();
        let row = listed.iter().find(|t| t.task_id == task.task_id).unwrap();
        assert_eq!(row.stale, Some(false));
    }

    #[test]
    fn audit_expired_lease_requires_explicit_resume_for_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, _worker_a) = running_task(dir.path());
        let after = task.lease_expires_at.unwrap() + 1;
        let worker_b = mint_worker_id();
        // Worker B holds the current fencing version but not the lease.
        // Every live-state mutation without an explicit resume is refused.
        let err = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker_b, task.lease_version, None, after),
                &cp_input("takeover write", None, None, None),
            )
            .unwrap_err();
        assert!(err.to_string().contains("resume"), "{err}");
        let err = s
            .pause_task(
                "/repo-a",
                &task.task_id,
                &worker_b,
                task.lease_version,
                None,
                after,
            )
            .unwrap_err();
        assert!(err.to_string().contains("resume"), "{err}");
        let err = s
            .start_task_validation(
                &ctx(&task.task_id, &worker_b, task.lease_version, None, after),
                "cargo test",
            )
            .unwrap_err();
        assert!(err.to_string().contains("resume"), "{err}");
        // The documented recovery path works and fences the lease forward.
        let taken = s
            .resume_task("/repo-a", &task.task_id, &worker_b, None, after)
            .unwrap();
        assert!(taken.lease_version > task.lease_version);
        assert_eq!(taken.lease_worker.as_deref(), Some(worker_b.as_str()));
        // …and the new owner can checkpoint again.
        s.create_task_checkpoint(
            &ctx(
                &taken.task_id,
                &worker_b,
                taken.lease_version,
                None,
                after + 10,
            ),
            &cp_input("owner write", None, None, None),
        )
        .unwrap();
    }

    #[test]
    fn audit_forged_lease_version_grants_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, _worker_a) = running_task(dir.path());
        let worker_b = mint_worker_id();
        // A forged *higher* fencing version does not bypass live-lease
        // exclusivity…
        let err = s
            .create_task_checkpoint(
                &ctx(
                    &task.task_id,
                    &worker_b,
                    task.lease_version + 100,
                    None,
                    120,
                ),
                &cp_input("forged write", None, None, None),
            )
            .unwrap_err();
        assert!(
            err.to_string().contains("leased by another worker"),
            "{err}"
        );
        // …and does not bypass the explicit-resume rule after expiry either.
        let after = task.lease_expires_at.unwrap() + 1;
        let err = s
            .pause_task(
                "/repo-a",
                &task.task_id,
                &worker_b,
                task.lease_version + 100,
                None,
                after,
            )
            .unwrap_err();
        assert!(err.to_string().contains("resume"), "{err}");
        // The task is untouched by both attempts.
        let still = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(still.status, TaskStatus::Running);
        assert!(still.current_checkpoint_id.is_none());
    }

    #[test]
    fn audit_heartbeat_refuses_expired_lease() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let after = task.lease_expires_at.unwrap() + 1;
        // The holder cannot silently renew an expired lease: recovery must
        // go through explicit resume (which fences the version forward).
        let err = s
            .heartbeat_task_lease("/repo-a", &task.task_id, &worker, task.lease_version, after)
            .unwrap_err();
        assert!(
            err.to_string().contains("expired") || err.to_string().contains("resume"),
            "{err}"
        );
        // Resume still recovers the task for the same worker (a
        // self-resume with no competing owner renews in place — no
        // fencing bump is needed because nobody else took over).
        let resumed = s
            .resume_task("/repo-a", &task.task_id, &worker, None, after)
            .unwrap();
        assert_eq!(resumed.lease_worker.as_deref(), Some(worker.as_str()));
        assert!(!resumed.is_stale(after));
        // …and a live heartbeat works again afterwards.
        s.heartbeat_task_lease(
            "/repo-a",
            &task.task_id,
            &worker,
            resumed.lease_version,
            after + 10,
        )
        .unwrap();
    }

    #[test]
    fn audit_holderless_checkpoint_creates_no_lease() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        // Pending tasks carry no lease; checkpointing one must not mint
        // lease state (no owner, no expiry) behind the caller's back.
        let pending = s
            .create_task("/repo-a", &new_task("pending work"), 100)
            .unwrap();
        assert!(pending.lease_worker.is_none());
        let worker = mint_worker_id();
        s.create_task_checkpoint(
            &ctx(&pending.task_id, &worker, pending.lease_version, None, 110),
            &cp_input("early notes", None, None, None),
        )
        .unwrap();
        let fetched = s.get_task("/repo-a", &pending.task_id).unwrap().unwrap();
        assert!(fetched.lease_worker.is_none(), "no owner invented");
        assert!(fetched.lease_expires_at.is_none(), "no expiry invented");
        assert!(fetched.current_checkpoint_id.is_some());
        // Same for paused tasks (pause releases the lease).
        let t = s
            .create_task("/repo-a", &new_task("pausable"), 120)
            .unwrap();
        let t = s
            .start_task("/repo-a", &t.task_id, &worker, None, 130)
            .unwrap();
        s.pause_task("/repo-a", &t.task_id, &worker, t.lease_version, None, 140)
            .unwrap();
        s.create_task_checkpoint(
            &ctx(&t.task_id, &worker, t.lease_version, None, 150),
            &cp_input("paused notes", None, None, None),
        )
        .unwrap();
        let fetched = s.get_task("/repo-a", &t.task_id).unwrap().unwrap();
        assert_eq!(fetched.status, TaskStatus::Paused);
        assert!(fetched.lease_worker.is_none());
        assert!(fetched.lease_expires_at.is_none());
    }

    #[test]
    fn audit_terminal_tasks_refuse_every_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let t = validate_passed(&s, &task, &worker, 120);
        let done = s
            .complete_task(
                &ctx(&task.task_id, &worker, t.lease_version, None, 140),
                "done",
                vec![],
            )
            .unwrap();
        assert_eq!(done.status, TaskStatus::Completed);
        let lv = done.lease_version;
        assert!(s
            .start_task("/repo-a", &task.task_id, &worker, None, 150)
            .is_err());
        assert!(s
            .resume_task("/repo-a", &task.task_id, &worker, None, 150)
            .is_err());
        assert!(s
            .pause_task("/repo-a", &task.task_id, &worker, lv, None, 150)
            .is_err());
        assert!(s
            .start_task_validation(&ctx(&task.task_id, &worker, lv, None, 150), "x")
            .is_err());
        assert!(s
            .record_task_validation(
                &ctx(&task.task_id, &worker, lv, None, 150),
                "x",
                TaskValidationResult::Passed,
                None
            )
            .is_err());
        assert!(s
            .complete_task(&ctx(&task.task_id, &worker, lv, None, 150), "x", vec![])
            .is_err());
        assert!(s
            .fail_task(&ctx(&task.task_id, &worker, lv, None, 150), "x")
            .is_err());
        assert!(s
            .cancel_task("/repo-a", &task.task_id, Some("x"), None, 150)
            .is_err());
        assert!(s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, lv, None, 150),
                &cp_input("late", None, None, None)
            )
            .is_err());
        assert!(s
            .heartbeat_task_lease("/repo-a", &task.task_id, &worker, lv, 150)
            .is_err());
        // Paused work cannot skip validation either.
        let t2 = s.create_task("/repo-a", &new_task("second"), 160).unwrap();
        let t2 = s
            .start_task("/repo-a", &t2.task_id, &worker, None, 170)
            .unwrap();
        s.pause_task("/repo-a", &t2.task_id, &worker, t2.lease_version, None, 180)
            .unwrap();
        let err = s
            .complete_task(
                &ctx(&t2.task_id, &worker, t2.lease_version, None, 190),
                "skip",
                vec![],
            )
            .unwrap_err();
        assert!(err.to_string().contains("validating"), "{err}");
        // Failed work cannot be completed without revalidation either.
        let t3 = s.create_task("/repo-a", &new_task("third"), 200).unwrap();
        let t3 = s
            .start_task("/repo-a", &t3.task_id, &worker, None, 210)
            .unwrap();
        s.fail_task(
            &ctx(&t3.task_id, &worker, t3.lease_version, None, 220),
            "broke",
        )
        .unwrap();
        assert!(s
            .complete_task(
                &ctx(&t3.task_id, &worker, t3.lease_version, None, 230),
                "x",
                vec![]
            )
            .is_err());
        assert!(s
            .resume_task("/repo-a", &t3.task_id, &worker, None, 230)
            .is_err());
    }

    #[test]
    fn audit_concurrent_checkpoints_single_threaded_winner() {
        let dir = tempfile::tempdir().unwrap();
        let (store, task, worker) = running_task(dir.path());
        let store = std::sync::Arc::new(store);
        let anchor = task.current_version;
        let lease_version = task.lease_version;
        let task_id = task.task_id.clone();
        let worker = worker.clone();
        let results = std::thread::scope(|scope| {
            (0..4)
                .map(|i| {
                    let store = std::sync::Arc::clone(&store);
                    let task_id = task_id.clone();
                    let worker = worker.clone();
                    scope
                        .spawn(move || {
                            store
                                .create_task_checkpoint(
                                    &TaskMutationCtx {
                                        workspace_root: "/repo-a",
                                        task_id: &task_id,
                                        worker: &worker,
                                        lease_version,
                                        based_on_version: Some(anchor),
                                        now: 150,
                                    },
                                    &CheckpointInput {
                                        summary: "race write",
                                        progress: None,
                                        next_action: None,
                                        validation_status: None,
                                        metadata_json: None,
                                    },
                                )
                                .map(|_| i)
                                .map_err(|e| e.to_string())
                        })
                        .join()
                        .expect("thread joins")
                })
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results.iter().filter(|r| r.is_ok()).count(),
            1,
            "{results:?}"
        );
        for err in results.iter().filter_map(|r| r.as_ref().err()) {
            assert!(err.contains("stale"), "{err}");
        }
        assert_eq!(
            store
                .list_task_checkpoints("/repo-a", &task_id)
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn audit_concurrent_resume_single_winner() {
        let dir = tempfile::tempdir().unwrap();
        let (store, task, _worker) = running_task(dir.path());
        let store = std::sync::Arc::new(store);
        let after = task.lease_expires_at.unwrap() + 1;
        let task_id = task.task_id.clone();
        let results = std::thread::scope(|scope| {
            (0..4)
                .map(|_| {
                    let store = std::sync::Arc::clone(&store);
                    let task_id = task_id.clone();
                    scope
                        .spawn(move || {
                            let w = mint_worker_id();
                            store
                                .resume_task("/repo-a", &task_id, &w, None, after)
                                .map(|t| t.lease_worker.unwrap_or_default())
                                .map_err(|e| e.to_string())
                        })
                        .join()
                        .expect("thread joins")
                })
                .collect::<Vec<_>>()
        });
        let winners: Vec<_> = results.iter().filter_map(|r| r.as_ref().ok()).collect();
        assert_eq!(winners.len(), 1, "exactly one takeover wins: {results:?}");
        // The losers hold no lease: the persisted owner is the winner.
        let persisted = store.get_task("/repo-a", &task_id).unwrap().unwrap();
        assert_eq!(persisted.lease_worker.as_deref(), Some(winners[0].as_str()));
    }

    #[test]
    fn audit_stale_anchors_refused_on_all_versioned_mutations() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let stale_anchor = task.current_version;
        // Advance the task once (checkpoint bumps the version).
        let advanced = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, task.lease_version, None, 120),
                &cp_input("advance", None, None, None),
            )
            .unwrap();
        assert!(advanced.version == 1);
        let fresh = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert!(fresh.current_version > stale_anchor);
        // Every versioned mutation with the old anchor is refused.
        let err = s
            .start_task_validation(
                &TaskMutationCtx {
                    workspace_root: "/repo-a",
                    task_id: &task.task_id,
                    worker: &worker,
                    lease_version: fresh.lease_version,
                    based_on_version: Some(stale_anchor),
                    now: 130,
                },
                "cargo test",
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        let err = s
            .cancel_task(
                "/repo-a",
                &task.task_id,
                Some("late"),
                Some(stale_anchor),
                130,
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        let err = s
            .set_task_skill_refs(
                "/repo-a",
                &task.task_id,
                vec!["sk::x".into()],
                Some(stale_anchor),
                130,
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        let err = s
            .create_task_checkpoint(
                &ctx(
                    &task.task_id,
                    &worker,
                    fresh.lease_version,
                    Some(stale_anchor),
                    130,
                ),
                &cp_input("stale write", None, None, None),
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
    }

    #[test]
    fn audit_redaction_covers_all_free_text() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let ghp = "ghp_abcdefghij1234567890XY";
        let pw = "password=hunter2hunter2hunter2";
        let t = s
            .create_task(
                "/repo-a",
                &NewTask {
                    title: format!("ship {ghp}"),
                    description: Some(format!("uses {pw}")),
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        assert!(!t.title.contains(ghp));
        assert!(!t.description.unwrap().contains("hunter2"));
        let worker = mint_worker_id();
        let t = s
            .start_task("/repo-a", &t.task_id, &worker, None, 110)
            .unwrap();
        s.create_task_checkpoint(
            &ctx(&t.task_id, &worker, t.lease_version, None, 120),
            &CheckpointInput {
                summary: "progress",
                progress: Some(&format!("token {ghp}")),
                next_action: Some(&format!("rotate {pw}")),
                validation_status: None,
                metadata_json: Some(&format!(r#"{{"note":"{ghp}"}}"#)),
            },
        )
        .unwrap();
        s.start_task_validation(
            &ctx(&t.task_id, &worker, t.lease_version, None, 130),
            &format!("run with {ghp}"),
        )
        .unwrap();
        s.record_task_validation(
            &ctx(&t.task_id, &worker, t.lease_version, None, 140),
            "cargo test",
            TaskValidationResult::Failed,
            Some(&format!("denied by {pw}")),
        )
        .unwrap();
        s.fail_task(
            &ctx(&t.task_id, &worker, t.lease_version, None, 150),
            &format!("broke {ghp}"),
        )
        .unwrap();
        // Canonical rows, checkpoints, events, and the resume snapshot
        // must all be free of both sentinels.
        let fetched = s.get_task("/repo-a", &t.task_id).unwrap().unwrap();
        let row_json = serde_json::to_string(&fetched).unwrap();
        assert!(!row_json.contains(ghp), "{row_json}");
        assert!(!row_json.contains("hunter2"), "{row_json}");
        let cps = s.list_task_checkpoints("/repo-a", &t.task_id).unwrap();
        assert!(!cps.is_empty());
        for cp in &cps {
            let j = serde_json::to_string(cp).unwrap();
            assert!(!j.contains(ghp), "{j}");
            assert!(!j.contains("hunter2"), "{j}");
        }
        for e in s.list_events("/repo-a", 50).unwrap() {
            let j = serde_json::to_string(&e).unwrap();
            assert!(!j.contains(ghp), "{j}");
            assert!(!j.contains("hunter2"), "{j}");
        }
        let snapshot = s.task_resume_snapshot("/repo-a", &t.task_id, 160).unwrap();
        let j = serde_json::to_string(&snapshot).unwrap();
        assert!(!j.contains(ghp), "{j}");
        assert!(!j.contains("hunter2"), "{j}");
    }

    #[test]
    fn audit_validating_state_recovers_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let worker = mint_worker_id();
        let task_id = {
            let s = store_at(dir.path());
            let t = s
                .create_task("/repo-a", &new_task("validate me"), 100)
                .unwrap();
            let t = s
                .start_task("/repo-a", &t.task_id, &worker, None, 110)
                .unwrap();
            s.start_task_validation(
                &ctx(&t.task_id, &worker, t.lease_version, None, 120),
                "cargo test",
            )
            .unwrap();
            t.task_id.clone()
        };
        // A restart preserves the validating state and its pending result.
        let reopened = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        let t = reopened.get_task("/repo-a", &task_id).unwrap().unwrap();
        assert_eq!(t.status, TaskStatus::Validating);
        assert_eq!(t.validation.as_ref().and_then(|v| v.result), None);
        // Completion without a passed result is still refused…
        let err = reopened
            .complete_task(
                &ctx(&task_id, &worker, t.lease_version, None, 130),
                "x",
                vec![],
            )
            .unwrap_err();
        assert!(err.to_string().contains("not passed"), "{err}");
        // …and the interrupted validation recovers through resume, which
        // requires revalidation (the old pending result cannot complete
        // the post-resume state).
        let after = t.lease_expires_at.unwrap() + 1;
        let resumed = reopened
            .resume_task("/repo-a", &task_id, &worker, None, after)
            .unwrap();
        assert_eq!(resumed.status, TaskStatus::Running);
        let err = reopened
            .complete_task(
                &ctx(&task_id, &worker, resumed.lease_version, None, after + 10),
                "x",
                vec![],
            )
            .unwrap_err();
        assert!(err.to_string().contains("validating"), "{err}");
        reopened
            .start_task_validation(
                &ctx(&task_id, &worker, resumed.lease_version, None, after + 20),
                "cargo test",
            )
            .unwrap();
        reopened
            .record_task_validation(
                &ctx(&task_id, &worker, resumed.lease_version, None, after + 30),
                "cargo test",
                TaskValidationResult::Passed,
                None,
            )
            .unwrap();
        let done = reopened
            .complete_task(
                &ctx(&task_id, &worker, resumed.lease_version, None, after + 40),
                "green",
                vec![],
            )
            .unwrap();
        assert_eq!(done.status, TaskStatus::Completed);
    }

    #[test]
    fn audit_cross_task_checkpoint_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let worker = mint_worker_id();
        let a = s.create_task("/repo-a", &new_task("alpha"), 100).unwrap();
        let a = s
            .start_task("/repo-a", &a.task_id, &worker, None, 110)
            .unwrap();
        s.create_task_checkpoint(
            &ctx(&a.task_id, &worker, a.lease_version, None, 120),
            &cp_input("alpha state", None, None, None),
        )
        .unwrap();
        let b = s.create_task("/repo-a", &new_task("beta"), 130).unwrap();
        // B sees none of A's checkpoints through any accessor.
        assert!(s
            .get_task_checkpoint("/repo-a", &b.task_id, None)
            .unwrap()
            .is_none());
        assert!(s
            .get_task_checkpoint("/repo-a", &b.task_id, Some(1))
            .unwrap()
            .is_none());
        assert!(s
            .list_task_checkpoints("/repo-a", &b.task_id)
            .unwrap()
            .is_empty());
        // A's checkpoints resolve only under A's id.
        assert_eq!(
            s.get_task_checkpoint("/repo-a", &a.task_id, Some(1))
                .unwrap()
                .unwrap()
                .summary,
            "alpha state"
        );
        let snapshot_b = s.task_resume_snapshot("/repo-a", &b.task_id, 140).unwrap();
        assert!(snapshot_b.latest_checkpoint.is_none());
        assert!(snapshot_b
            .recent_events
            .iter()
            .all(|e| !e.summary.contains("alpha")));
    }

    #[test]
    fn audit_returned_state_matches_persisted_state() {
        // P4's audit found an API/persistence divergence; every P5
        // mutation must read back exactly what it returned.
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let persisted = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(persisted.current_version, task.current_version);
        assert_eq!(persisted.lease_version, task.lease_version);
        assert_eq!(persisted.lease_worker, task.lease_worker);
        let cp = s
            .create_task_checkpoint(
                &ctx(&task.task_id, &worker, task.lease_version, None, 120),
                &cp_input("state", Some("rest"), Some("next"), None),
            )
            .unwrap();
        let persisted = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(
            persisted.current_checkpoint_id.as_deref(),
            Some(cp.checkpoint_id.as_str())
        );
        let read_back = s
            .get_task_checkpoint("/repo-a", &task.task_id, Some(cp.version))
            .unwrap()
            .unwrap();
        assert_eq!(read_back, cp);
        let v = s
            .start_task_validation(
                &ctx(&task.task_id, &worker, task.lease_version, None, 130),
                "cargo test",
            )
            .unwrap();
        let persisted = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(persisted.status, v.status);
        assert_eq!(persisted.current_version, v.current_version);
        assert_eq!(persisted.validation, v.validation);
        let r = s
            .record_task_validation(
                &ctx(&task.task_id, &worker, v.lease_version, None, 140),
                "cargo test",
                TaskValidationResult::Passed,
                Some("ok"),
            )
            .unwrap();
        let persisted = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(persisted.validation, r.validation);
        let done = s
            .complete_task(
                &ctx(&task.task_id, &worker, r.lease_version, None, 150),
                "shipped",
                vec!["x".into()],
            )
            .unwrap();
        let persisted = s.get_task("/repo-a", &task.task_id).unwrap().unwrap();
        assert_eq!(persisted.status, TaskStatus::Completed);
        assert_eq!(persisted.outcome, done.outcome);
        assert_eq!(persisted.current_version, done.current_version);
    }

    #[test]
    fn audit_outcome_integrity_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let worker = mint_worker_id();
        let (first_id, evidence_id) = {
            let s = store_at(dir.path());
            let t = s.create_task("/repo-a", &new_task("first"), 100).unwrap();
            let t = s
                .start_task("/repo-a", &t.task_id, &worker, None, 110)
                .unwrap();
            let v = validate_passed(&s, &t, &worker, 120);
            let done = s
                .complete_task(
                    &ctx(&t.task_id, &worker, v.lease_version, None, 130),
                    "first done",
                    vec!["a".into()],
                )
                .unwrap();
            (
                t.task_id.clone(),
                done.outcome.unwrap().evidence_event_ids[0],
            )
        };
        let reopened = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        let t = reopened.get_task("/repo-a", &first_id).unwrap().unwrap();
        let outcome = t.outcome.expect("outcome survives restart");
        assert_eq!(outcome.result, "completed");
        assert_eq!(outcome.summary, "first done");
        assert_eq!(outcome.evidence_event_ids, vec![evidence_id]);
        // The evidence event exists and belongs to this task.
        let events = reopened.list_events("/repo-a", 50).unwrap();
        let anchor = events
            .iter()
            .find(|e| e.id == Some(evidence_id))
            .expect("evidence event exists");
        assert_eq!(anchor.task_id.as_deref(), Some(first_id.as_str()));
        // A second task cannot disturb the first outcome.
        let t2 = reopened
            .create_task("/repo-a", &new_task("second"), 140)
            .unwrap();
        let t2 = reopened
            .start_task("/repo-a", &t2.task_id, &worker, None, 150)
            .unwrap();
        reopened
            .fail_task(
                &ctx(&t2.task_id, &worker, t2.lease_version, None, 160),
                "broke",
            )
            .unwrap();
        let still = reopened.get_task("/repo-a", &first_id).unwrap().unwrap();
        assert_eq!(still.outcome.unwrap().summary, "first done");
    }

    #[test]
    fn audit_idempotency_key_ignores_later_titles() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let mut first = new_task("original title");
        first.idempotency_key = Some("deploy-prod".into());
        let a = s.create_task("/repo-a", &first, 100).unwrap();
        let mut replay = new_task("attacker-controlled title");
        replay.idempotency_key = Some("deploy-prod".into());
        let b = s.create_task("/repo-a", &replay, 200).unwrap();
        assert_eq!(a.task_id, b.task_id);
        assert_eq!(b.title, "original title");
        assert_eq!(
            s.list_events("/repo-a", 50)
                .unwrap()
                .iter()
                .filter(|e| e.kind == "task_created")
                .count(),
            1
        );
    }

    #[test]
    fn audit_cancel_is_supervisory_but_version_pinned() {
        // Audit decision: cancel intentionally needs no lease (a
        // supervisor can stop live work), but a stale version anchor is
        // still refused so blind cancels cannot win races silently.
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let stale_anchor = task.current_version;
        s.create_task_checkpoint(
            &ctx(&task.task_id, &worker, task.lease_version, None, 120),
            &cp_input("advance", None, None, None),
        )
        .unwrap();
        let err = s
            .cancel_task(
                "/repo-a",
                &task.task_id,
                Some("stop"),
                Some(stale_anchor),
                130,
            )
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        let cancelled = s
            .cancel_task("/repo-a", &task.task_id, Some("stop"), None, 140)
            .unwrap();
        assert_eq!(cancelled.status, TaskStatus::Cancelled);
        assert!(cancelled.lease_worker.is_none());
    }

    #[test]
    fn audit_oversized_outcome_and_refs_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let (s, task, worker) = running_task(dir.path());
        let t = validate_passed(&s, &task, &worker, 120);
        let err = s
            .complete_task(
                &ctx(&task.task_id, &worker, t.lease_version, None, 130),
                &"x".repeat(2049),
                vec![],
            )
            .unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
        let err = s
            .complete_task(
                &ctx(&task.task_id, &worker, t.lease_version, None, 130),
                "ok",
                vec!["x".repeat(257)],
            )
            .unwrap_err();
        assert!(err.to_string().contains("exceeds"), "{err}");
        assert!(s
            .set_task_skill_refs("/repo-a", &task.task_id, vec!["".into()], None, 130)
            .is_err());
        assert!(s
            .set_task_skill_refs("/repo-a", &task.task_id, vec!["x".repeat(257)], None, 130)
            .is_err());
        // Skill refs are opaque labels: they round-trip without touching
        // the P4 skill lifecycle tables.
        s.set_task_skill_refs(
            "/repo-a",
            &task.task_id,
            vec!["review-checklist".into()],
            None,
            130,
        )
        .unwrap();
        let skills: i64 = s
            .with_conn(|conn| Ok(conn.query_row("SELECT count(*) FROM skills", [], |r| r.get(0))?))
            .unwrap();
        let candidates: i64 = s
            .with_conn(|conn| {
                Ok(conn.query_row("SELECT count(*) FROM skill_candidates", [], |r| r.get(0))?)
            })
            .unwrap();
        assert_eq!((skills, candidates), (0, 0));
    }

    // ── P9 outcomes ───────────────────────────────────────────────────

    fn outcome_input<'a>(
        classification: OutcomeClassification,
        summary: &'a str,
    ) -> TaskOutcomeInput<'a> {
        TaskOutcomeInput {
            classification,
            summary,
            ..Default::default()
        }
    }

    #[test]
    fn outcome_classifications_roundtrip_and_label_polarity() {
        for c in [
            OutcomeClassification::Success,
            OutcomeClassification::Partial,
            OutcomeClassification::Failure,
            OutcomeClassification::Rejected,
            OutcomeClassification::Superseded,
        ] {
            assert_eq!(c.to_string().parse::<OutcomeClassification>().unwrap(), c);
        }
        assert!("triumph".parse::<OutcomeClassification>().is_err());
        // Polarity-safe history labels: superseded must never read as a
        // success under the existing outcome_polarity mapping.
        assert_eq!(
            OutcomeClassification::Success.history_outcome_label(),
            "success"
        );
        assert_eq!(
            OutcomeClassification::Partial.history_outcome_label(),
            "partial"
        );
        assert_eq!(
            OutcomeClassification::Failure.history_outcome_label(),
            "failure"
        );
        assert_eq!(
            OutcomeClassification::Rejected.history_outcome_label(),
            "rejected"
        );
        assert_eq!(
            OutcomeClassification::Superseded.history_outcome_label(),
            "replaced"
        );
        assert_eq!(
            crate::learning::outcome_polarity(Some("success")),
            crate::learning::OutcomePolarity::Success
        );
        assert_eq!(
            crate::learning::outcome_polarity(Some("failure")),
            crate::learning::OutcomePolarity::Failure
        );
        assert_eq!(
            crate::learning::outcome_polarity(Some("rejected")),
            crate::learning::OutcomePolarity::Failure
        );
        assert_eq!(
            crate::learning::outcome_polarity(Some("partial")),
            crate::learning::OutcomePolarity::Neutral
        );
        assert_eq!(
            crate::learning::outcome_polarity(Some("replaced")),
            crate::learning::OutcomePolarity::Neutral
        );
    }

    #[test]
    fn outcome_records_task_bound_evidence_without_transition() {
        let dir = tempfile::tempdir().unwrap();
        let (s, t, _) = running_task(dir.path());
        let rec = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &outcome_input(
                    OutcomeClassification::Failure,
                    "integration test failed because API contract differs",
                ),
                200,
            )
            .unwrap();
        assert!(!rec.duplicate);
        assert_eq!(rec.classification, OutcomeClassification::Failure);
        assert_eq!(rec.authority, "observed");
        // The task row is untouched: same status, same version.
        let after = s.get_task("/repo-a", &t.task_id).unwrap().unwrap();
        assert_eq!(after.status, TaskStatus::Running);
        assert_eq!(after.current_version, t.current_version);
        // The evidence is a task-bound history event.
        let event = s.get_event(rec.event_id).unwrap().unwrap();
        assert_eq!(event.kind, "task_outcome");
        assert_eq!(event.task_id.as_deref(), Some(t.task_id.as_str()));
        assert_eq!(event.outcome.as_deref(), Some("failure"));
        assert!(event.summary.as_deref().unwrap().contains("API contract"));
        // …and it surfaces in the bounded resume snapshot.
        let snap = s.task_resume_snapshot("/repo-a", &t.task_id, 200).unwrap();
        assert!(snap
            .recent_events
            .iter()
            .any(|e| e.event_id == rec.event_id));
    }

    #[test]
    fn outcome_accepts_terminal_tasks_and_user_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let (s, t, worker) = running_task(dir.path());
        let t = validate_passed(&s, &t, &worker, 200);
        let t = s
            .complete_task(
                &ctx(&t.task_id, &worker, t.lease_version, None, 220),
                "done",
                vec!["src/lib.rs".into()],
            )
            .unwrap();
        assert_eq!(t.status, TaskStatus::Completed);
        // User confirmation after completion: allowed, no lease, terminal.
        let rec = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Success,
                    summary: "user confirmed the fix resolves the issue",
                    user_confirmed: true,
                    ..Default::default()
                },
                300,
            )
            .unwrap();
        assert_eq!(rec.authority, "user_confirmed");
        let event = s.get_event(rec.event_id).unwrap().unwrap();
        assert_eq!(event.outcome.as_deref(), Some("success"));
        assert!(event
            .summary
            .as_deref()
            .unwrap()
            .contains("(user_confirmed)"));
        let payload = event.payload.unwrap();
        assert!(payload.contains("\"authority\":\"user_confirmed\""));
    }

    #[test]
    fn outcome_dedup_key_is_idempotent_per_task() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let a = s.create_task("/repo-a", &new_task("task A"), 100).unwrap();
        let b = s.create_task("/repo-a", &new_task("task B"), 100).unwrap();
        let first = s
            .record_task_outcome(
                "/repo-a",
                &a.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Partial,
                    summary: "half the migration is done",
                    dedup_key: Some("k1"),
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        assert!(!first.duplicate);
        let replay = s
            .record_task_outcome(
                "/repo-a",
                &a.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Partial,
                    summary: "half the migration is done",
                    dedup_key: Some("k1"),
                    ..Default::default()
                },
                201,
            )
            .unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.event_id, first.event_id);
        // Same caller key on another task is a distinct outcome.
        let other = s
            .record_task_outcome(
                "/repo-a",
                &b.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Partial,
                    summary: "half the migration is done",
                    dedup_key: Some("k1"),
                    ..Default::default()
                },
                202,
            )
            .unwrap();
        assert!(!other.duplicate);
        assert_ne!(other.event_id, first.event_id);
        // Without a key every delivery is a new event (documented).
        let no_key = s
            .record_task_outcome(
                "/repo-a",
                &a.task_id,
                &outcome_input(OutcomeClassification::Partial, "half the migration is done"),
                203,
            )
            .unwrap();
        assert!(!no_key.duplicate);
        assert_ne!(no_key.event_id, first.event_id);
    }

    #[test]
    fn outcome_rejects_unknown_tasks_and_cross_workspace_ids() {
        let dir = tempfile::tempdir().unwrap();
        let s = store_at(dir.path());
        let t = s.create_task("/repo-a", &new_task("real"), 100).unwrap();
        let missing = s.record_task_outcome(
            "/repo-a",
            "task::0000000000000000",
            &outcome_input(OutcomeClassification::Success, "done"),
            200,
        );
        assert!(missing.unwrap_err().to_string().contains("does not exist"));
        // Same id from another workspace: refused, never leaked.
        let cross = s.record_task_outcome(
            "/repo-b",
            &t.task_id,
            &outcome_input(OutcomeClassification::Success, "done"),
            200,
        );
        assert!(cross.unwrap_err().to_string().contains("another workspace"));
    }

    #[test]
    fn outcome_bounds_are_refused_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let (s, t, _) = running_task(dir.path());
        assert!(s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &outcome_input(OutcomeClassification::Success, "   "),
                200
            )
            .is_err());
        assert!(s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &outcome_input(
                    OutcomeClassification::Success,
                    &"x".repeat(MAX_OUTCOME_SUMMARY_CHARS + 1)
                ),
                200
            )
            .unwrap_err()
            .to_string()
            .contains("exceeds"));
        assert!(s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: "bad",
                    evidence: Some(&"y".repeat(MAX_OUTCOME_EVIDENCE_CHARS + 1)),
                    ..Default::default()
                },
                200
            )
            .is_err());
        assert!(s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: "bad",
                    command: Some("   "),
                    ..Default::default()
                },
                200
            )
            .is_err());
        assert!(s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Success,
                    summary: "ok",
                    dedup_key: Some("   "),
                    ..Default::default()
                },
                200
            )
            .is_err());
    }

    #[test]
    fn outcome_redacts_secrets_at_write_and_in_fts() {
        let dir = tempfile::tempdir().unwrap();
        let (s, t, _) = running_task(dir.path());
        let secret = "sk-testsecretkey1234567890abcdef";
        let rec = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: &format!("deploy failed with api_key=\"{secret}\""),
                    evidence: Some(&format!("token ghp_abcdefghijklmnopqrst1234 pw={secret}")),
                    command: Some("cargo test"),
                    changed_areas: vec![format!("src/{secret}.rs")],
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        let event = s.get_event(rec.event_id).unwrap().unwrap();
        let summary = event.summary.unwrap();
        let payload = event.payload.unwrap();
        assert!(!summary.contains(secret), "raw secret in summary");
        assert!(!payload.contains(secret), "raw secret in payload");
        assert!(!payload.contains("ghp_abcdefghijklmnopqrst1234"));
        assert!(summary.contains("[REDACTED]") || payload.contains("[REDACTED]"));
    }

    #[test]
    fn outcome_structured_evidence_round_trips_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let (s, t, _) = running_task(dir.path());
        let input = TaskOutcomeInput {
            classification: OutcomeClassification::Failure,
            summary: "cargo test failed on contract",
            evidence: Some("integration_contract: expected 200 got 500"),
            command: Some("cargo test"),
            exit_code: Some(101),
            changed_areas: vec!["src/api.rs".into(), "tests/contract.rs".into()],
            ..Default::default()
        };
        let a = s
            .record_task_outcome("/repo-a", &t.task_id, &input, 200)
            .unwrap();
        let b = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    dedup_key: Some("det"),
                    ..input.clone()
                },
                200,
            )
            .unwrap();
        let ea = s.get_event(a.event_id).unwrap().unwrap();
        let eb = s.get_event(b.event_id).unwrap().unwrap();
        // Same logical inputs ⇒ same summary shape and payload content
        // modulo the event `at` marker (identical `now` ⇒ identical).
        assert_eq!(ea.summary, eb.summary);
        assert_eq!(ea.payload, eb.payload);
        assert!(ea
            .summary
            .as_deref()
            .unwrap()
            .contains("command `cargo test` exit 101"));
        let payload: serde_json::Value =
            serde_json::from_str(ea.payload.as_deref().unwrap()).unwrap();
        assert_eq!(payload["classification"], "failure");
        assert_eq!(payload["authority"], "observed");
        assert_eq!(payload["exit_code"], 101);
        assert_eq!(payload["changed_areas"][0], "src/api.rs");
    }

    #[test]
    fn outcome_replay_wins_first_write_and_keeps_original_content() {
        // Idempotency is first-write-wins: redelivering the same key with
        // different content returns the ORIGINAL event untouched.
        let dir = tempfile::tempdir().unwrap();
        let (s, t, _) = running_task(dir.path());
        let first = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: "original report",
                    dedup_key: Some("k"),
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        let replay = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Success,
                    summary: "contradictory redelivery",
                    dedup_key: Some("k"),
                    ..Default::default()
                },
                201,
            )
            .unwrap();
        assert!(replay.duplicate);
        assert_eq!(replay.event_id, first.event_id);
        let event = s.get_event(first.event_id).unwrap().unwrap();
        assert!(event
            .summary
            .as_deref()
            .unwrap()
            .contains("original report"));
        assert_eq!(event.outcome.as_deref(), Some("failure"));
    }

    #[test]
    fn outcome_maximal_inputs_stay_bounded_with_honest_markers() {
        // Every bound maxed at once: the stored rows stay within the
        // history budgets (defensive truncation marks, never refusal,
        // never unbounded growth).
        let dir = tempfile::tempdir().unwrap();
        let (s, t, _) = running_task(dir.path());
        let areas: Vec<String> = (0..64).map(|i| format!("src/module_{i:03}.rs")).collect();
        let rec = s
            .record_task_outcome(
                "/repo-a",
                &t.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: &"s".repeat(MAX_OUTCOME_SUMMARY_CHARS),
                    evidence: Some(&"e".repeat(MAX_OUTCOME_EVIDENCE_CHARS)),
                    command: Some(&"c".repeat(MAX_OUTCOME_COMMAND_CHARS)),
                    exit_code: Some(-1),
                    changed_areas: areas,
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        let event = s.get_event(rec.event_id).unwrap().unwrap();
        let summary = event.summary.unwrap();
        assert!(
            summary.chars().count() <= crate::types::MAX_HISTORY_SUMMARY_CHARS + 64,
            "summary bounded: {}",
            summary.chars().count()
        );
        let payload = event.payload.unwrap();
        assert!(
            payload.len() <= crate::types::MAX_EVENT_PAYLOAD_BYTES + 64,
            "payload bounded: {}",
            payload.len()
        );
        // 32-area cap enforced (64 supplied).
        let parsed: serde_json::Value = serde_json::from_str(
            payload.trim_end_matches(crate::history::HISTORY_TRUNCATION_MARKER),
        )
        .unwrap_or(serde_json::json!({}));
        if let Some(list) = parsed.get("changed_areas").and_then(|v| v.as_array()) {
            assert!(list.len() <= MAX_OUTCOME_CHANGED_AREAS);
        }
    }
}

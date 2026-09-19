//! Real human-in-the-loop approval for the skill lifecycle (Phases 4–8).
//!
//! The pre-existing approval gate was an internal boolean/state flag
//! (`user_confirmed=true` on `skill approve`). This module upgrades it into
//! an actionable interaction protocol that OpenCode — which owns the
//! user-facing interaction — can present naturally, while CodeBro owns the
//! approval state machine:
//!
//! ```text
//! OpenCode                    CodeBro                         Human
//!    │                           │                               │
//!    │  skill request_approval   │                               │
//!    │ ────────────────────────► │                               │
//!    │  status: needs_input     │                               │
//!    │  + question + options     │                               │
//!    │ ◄──────────────────────── │                               │
//!    │                           │      "Create the skill?"       │
//!    │ ────────────────────────────────────────────────────────► │
//!    │                           │     approve / reject /        │
//!    │                           │     modify / defer            │
//!    │ ◄──────────────────────────────────────────────────────── │
//!    │  skill respond            │                               │
//!    │ ────────────────────────► │                               │
//!    │  validated + published /  │                               │
//!    │  rejected / deferred /    │                               │
//!    │  modified + revalidated   │                               │
//! ```
//!
//! # Design rules
//!
//! - **No parallel lifecycle.** Approval requests point at the existing
//!   [`SkillCandidate`](crate::skills::SkillCandidate) rows and feed the
//!   existing gates (validation, confidence floor, secret scan, workspace
//!   confinement, stale-version anchors, publish safeguards). Nothing here
//!   publishes on its own.
//! - **Explicit response semantics.** `approve` publishes through the
//!   existing gates; `reject` persists rejection without publishing;
//!   `defer` persists a deferred decision resumable later; `modify`
//!   captures the human instruction, applies it to a *new* candidate
//!   lineage (candidate ids bind content, so modified content mints a new
//!   id), revalidates it, and requires approval again — a modified
//!   candidate is never silently published.
//! - **Replay and stale-writer protection.** A request is single-use
//!   (pending → terminal exactly once); the response handler verifies the
//!   request is still pending, the candidate/version still matches (content
//!   hash + `based_on_version` anchor), and the workspace/task scope still
//!   matches.
//! - **Restart-safe.** Requests live in `state.db` (`skill_approval_requests`,
//!   schema v8), so a pending approval survives a hard kill.
//! - **No UI.** CodeBro emits the structured interaction
//!   ([`ApprovalInteraction`]); OpenCode renders it with its native
//!   question/permission UI.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::path::Path;
use std::str::FromStr;

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::skills::{
    content_hash, mint_skill_candidate_id, validate_skill_content, SkillCandidate,
    SkillCandidateStatus, SkillScope, SKILL_CANDIDATE_TTL_SECS,
};
use crate::store::{ContextError, ContextStore};
use crate::workspace::canonical_workspace_key;

// ─── Constants ────────────────────────────────────────────────────────────

/// Interaction kind emitted for skill approvals (OpenCode renders it).
pub const SKILL_APPROVAL_KIND: &str = "skill_approval";
/// Status value telling OpenCode's agent loop it must not claim completion.
pub const STATUS_NEEDS_INPUT: &str = "needs_input";
/// Maximum characters kept per human modification instruction.
pub const MAX_MODIFICATION_CHARS: usize = 2000;
/// Maximum characters kept per approval question.
pub const MAX_QUESTION_CHARS: usize = 500;
/// Request TTL: 30 days without a response → expired by sweep.
pub const SKILL_APPROVAL_TTL_SECS: u64 = 30 * 86_400;

// ─── Types ────────────────────────────────────────────────────────────────

/// Lifecycle status of an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Rejected,
    Deferred,
    Superseded,
    Expired,
}

impl ApprovalStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalStatus::Pending => "pending",
            ApprovalStatus::Approved => "approved",
            ApprovalStatus::Rejected => "rejected",
            ApprovalStatus::Deferred => "deferred",
            ApprovalStatus::Superseded => "superseded",
            ApprovalStatus::Expired => "expired",
        }
    }
}

impl std::fmt::Display for ApprovalStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ApprovalStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "pending" => Ok(ApprovalStatus::Pending),
            "approved" => Ok(ApprovalStatus::Approved),
            "rejected" => Ok(ApprovalStatus::Rejected),
            "deferred" => Ok(ApprovalStatus::Deferred),
            "superseded" => Ok(ApprovalStatus::Superseded),
            "expired" => Ok(ApprovalStatus::Expired),
            other => Err(format!("unknown approval status: {other}")),
        }
    }
}

/// Human response to an approval request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ApprovalResponse {
    Approve,
    Reject,
    Modify,
    Defer,
}

impl ApprovalResponse {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApprovalResponse::Approve => "approve",
            ApprovalResponse::Reject => "reject",
            ApprovalResponse::Modify => "modify",
            ApprovalResponse::Defer => "defer",
        }
    }
}

impl std::fmt::Display for ApprovalResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for ApprovalResponse {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "approve" => Ok(ApprovalResponse::Approve),
            "reject" => Ok(ApprovalResponse::Reject),
            "modify" => Ok(ApprovalResponse::Modify),
            "defer" => Ok(ApprovalResponse::Defer),
            other => Err(format!(
                "unknown approval response '{other}': use approve, reject, modify, or defer"
            )),
        }
    }
}

/// The structured interaction OpenCode presents to the human.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalInteraction {
    pub kind: String,
    pub question: String,
    pub options: Vec<String>,
}

/// A persisted human-in-the-loop approval request.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SkillApprovalRequest {
    /// Opaque id: `apr::<16 hex>`.
    pub request_id: String,
    pub candidate_id: String,
    /// Content hash of the candidate at request time (stale detection).
    pub candidate_content_hash: String,
    /// Version anchor of the candidate at request time (stale detection).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub based_on_version: Option<u32>,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Proposed action (currently always `approve`/`publish`).
    pub proposed_action: String,
    pub question: String,
    #[serde(default)]
    pub options: Vec<String>,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modification: Option<String>,
    /// Successor request after a `modify` (the re-approval requirement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor_request_id: Option<String>,
    /// Predecessor request when this request came from a `modify`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_request_id: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub responded_at: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl SkillApprovalRequest {
    /// Machine-readable interaction envelope for OpenCode's agent loop.
    pub fn interaction(&self) -> ApprovalInteraction {
        ApprovalInteraction {
            kind: SKILL_APPROVAL_KIND.to_string(),
            question: self.question.clone(),
            options: self.options.clone(),
        }
    }

    pub fn is_pending(&self) -> bool {
        self.status == ApprovalStatus::Pending.as_str()
    }
}

/// Outcome of consuming an approval response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ApprovalOutcome {
    pub request_id: String,
    pub response: String,
    pub status: String,
    /// The request after the transition (terminal, never pending).
    pub request: SkillApprovalRequest,
    /// New candidate minted by a `modify` (the modified proposal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_candidate_id: Option<String>,
    /// Successor request minted by a `modify` (re-approval required).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub successor_request_id: Option<String>,
}

// ─── ID minting ───────────────────────────────────────────────────────────

/// Mint an approval-request id. Time participates so repeated requests for
/// the same candidate mint distinct rows (each interaction is its own
/// auditable event); the candidate binding is enforced by columns, not ids.
pub fn mint_approval_request_id(candidate_id: &str, proposed_action: &str, now: u64) -> String {
    // Process id + counter disambiguate same-second requests in one process;
    // the wall clock disambiguates restarts.
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(
        format!(
            "{candidate_id}|{proposed_action}|{now}|{}|{n}",
            std::process::id()
        )
        .as_bytes(),
    );
    format!("apr::{:016x}", {
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    })
}

// ─── Store methods ────────────────────────────────────────────────────────

const APPROVAL_COLUMNS: &str = "request_id, candidate_id, candidate_content_hash, \
     based_on_version, workspace_root, task_id, proposed_action, question, \
     options_json, status, response, modification, successor_request_id, \
     parent_request_id, created_at, updated_at, responded_at, expires_at";

fn row_to_approval(row: &rusqlite::Row<'_>) -> rusqlite::Result<SkillApprovalRequest> {
    let options_json: String = row.get(8)?;
    Ok(SkillApprovalRequest {
        request_id: row.get(0)?,
        candidate_id: row.get(1)?,
        candidate_content_hash: row.get(2)?,
        based_on_version: row.get::<_, Option<i64>>(3)?.map(|v| v as u32),
        workspace_root: row.get(4)?,
        task_id: row.get(5)?,
        proposed_action: row.get(6)?,
        question: row.get(7)?,
        options: serde_json::from_str(&options_json).unwrap_or_default(),
        status: row.get(9)?,
        response: row.get(10)?,
        modification: row.get(11)?,
        successor_request_id: row.get(12)?,
        parent_request_id: row.get(13)?,
        created_at: row.get::<_, i64>(14)? as u64,
        updated_at: row.get::<_, i64>(15)? as u64,
        responded_at: row.get::<_, Option<i64>>(16)?.map(|t| t as u64),
        expires_at: row.get::<_, Option<i64>>(17)?.map(|t| t as u64),
    })
}

/// Default human-facing question for a skill-approval request.
pub fn default_approval_question(skill_name: &str, proposed_action: &str) -> String {
    let q = format!("Create the proposed skill '{skill_name}' ({proposed_action})?");
    q.chars().take(MAX_QUESTION_CHARS).collect()
}

pub fn default_approval_options() -> Vec<String> {
    vec![
        ApprovalResponse::Approve.as_str().to_string(),
        ApprovalResponse::Reject.as_str().to_string(),
        ApprovalResponse::Modify.as_str().to_string(),
        ApprovalResponse::Defer.as_str().to_string(),
    ]
}

impl ContextStore {
    /// Create a human-in-the-loop approval request for a skill candidate.
    ///
    /// Gates (all refusals, never silent):
    /// - the candidate must exist and be `validated` (only validated
    ///   content is approvable — the existing lifecycle gate);
    /// - the requesting workspace/task scope must match the candidate's
    ///   (project/task candidates never leak across workspaces);
    /// - at most one unexpired pending request per candidate+action: a
    ///   second request for the same candidate returns the existing pending
    ///   row (idempotent — OpenCode retries must not mint duplicate
    ///   interactions); expired rows are invisible to dedup so post-expiry
    ///   retries mint a fresh request.
    pub fn create_skill_approval_request(
        &self,
        candidate_id: &str,
        requesting_workspace: &str,
        task_id: Option<&str>,
        proposed_action: &str,
        question: Option<&str>,
        now: u64,
    ) -> Result<SkillApprovalRequest, ContextError> {
        let candidate = self.get_skill_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("skill candidate not found: {candidate_id}"))
        })?;
        if candidate.status != SkillCandidateStatus::Validated.as_str() {
            return Err(ContextError::Validation(format!(
                "candidate is '{}': approval requests require status 'validated' \
                 (candidate → evaluating → draft → validated → approval)",
                candidate.status
            )));
        }
        let req_ws = canonical_workspace_key(requesting_workspace);
        let task = task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        // Scope check mirrors the candidate-visibility rule: global rows are
        // approvable from anywhere; project/task rows only from their own
        // workspace (task rows additionally need the task id).
        match candidate.scope.as_str() {
            "global" => {}
            "project" => {
                let cand_ws = candidate.workspace_root.as_deref().unwrap_or("");
                if canonical_workspace_key(cand_ws) != req_ws {
                    return Err(ContextError::Validation(format!(
                        "workspace mismatch: candidate belongs to {cand_ws}, \
                         approval was requested from {req_ws}"
                    )));
                }
            }
            "task" => {
                let cand_ws = candidate.workspace_root.as_deref().unwrap_or("");
                if canonical_workspace_key(cand_ws) != req_ws {
                    return Err(ContextError::Validation(format!(
                        "workspace mismatch: candidate belongs to {cand_ws}, \
                         approval was requested from {req_ws}"
                    )));
                }
                let cand_task = candidate.task_id.as_deref().unwrap_or("");
                if task.as_deref().unwrap_or("") != cand_task {
                    return Err(ContextError::Validation(
                        "task mismatch: a task-scoped candidate needs its own task context"
                            .to_string(),
                    ));
                }
            }
            other => {
                return Err(ContextError::Validation(format!(
                    "candidate has unknown scope '{other}'"
                )));
            }
        }

        let content_hash_now = content_hash(&candidate.proposed_content);
        let action = if proposed_action.trim().is_empty() {
            "approve"
        } else {
            proposed_action.trim()
        }
        .to_string();

        self.with_conn(|conn| {
            // Idempotency: an unexpired pending request for the same
            // candidate+action from the same workspace is the same
            // interaction — return it. Expired rows are invisible here so
            // a retry after expiry mints a fresh request (the expired row
            // stays for audit; respond refuses it — see expiry guard).
            let existing: Option<SkillApprovalRequest> = conn
                .query_row(
                    &format!(
                        "SELECT {APPROVAL_COLUMNS} FROM skill_approval_requests \
                          WHERE candidate_id = ?1 AND proposed_action = ?2 \
                            AND workspace_root = ?3 AND status = 'pending' \
                            AND (expires_at IS NULL OR expires_at > ?4) \
                          ORDER BY created_at DESC LIMIT 1"
                    ),
                    params![candidate_id, action, req_ws, now as i64],
                    row_to_approval,
                )
                .optional()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            if let Some(existing) = existing {
                // Same content: genuine retry. Changed content underneath a
                // pending request is stale — refuse so the human never
                // approves something other than what they were shown.
                if existing.candidate_content_hash == content_hash_now {
                    return Ok(existing);
                }
                return Err(ContextError::Validation(format!(
                    "candidate {candidate_id} changed since approval request {} \
                     was created — withdraw it (respond reject/defer) and request again",
                    existing.request_id
                )));
            }

            let question = question
                .map(|q| q.trim().to_string())
                .filter(|q| !q.is_empty())
                .unwrap_or_else(|| default_approval_question(&candidate.name, &action));
            let question: String = question.chars().take(MAX_QUESTION_CHARS).collect();
            let options = default_approval_options();
            let request = SkillApprovalRequest {
                request_id: mint_approval_request_id(candidate_id, &action, now),
                candidate_id: candidate_id.to_string(),
                candidate_content_hash: content_hash_now,
                based_on_version: candidate.based_on_version,
                workspace_root: req_ws,
                task_id: task,
                proposed_action: action,
                question,
                options: options.clone(),
                status: ApprovalStatus::Pending.as_str().to_string(),
                response: None,
                modification: None,
                successor_request_id: None,
                parent_request_id: None,
                created_at: now,
                updated_at: now,
                responded_at: None,
                expires_at: Some(now + SKILL_APPROVAL_TTL_SECS),
            };
            conn.execute(
                "INSERT INTO skill_approval_requests (
                    request_id, candidate_id, candidate_content_hash,
                    based_on_version, workspace_root, task_id, proposed_action,
                    question, options_json, status, response, modification,
                    successor_request_id, parent_request_id,
                    created_at, updated_at, responded_at, expires_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                           ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    request.request_id,
                    request.candidate_id,
                    request.candidate_content_hash,
                    request.based_on_version.map(|v| v as i64),
                    request.workspace_root,
                    request.task_id.as_deref(),
                    request.proposed_action,
                    request.question,
                    serde_json::to_string(&options)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    request.status,
                    request.response.as_deref(),
                    request.modification.as_deref(),
                    request.successor_request_id.as_deref(),
                    request.parent_request_id.as_deref(),
                    request.created_at as i64,
                    request.updated_at as i64,
                    request.responded_at.map(|t| t as i64),
                    request.expires_at.map(|t| t as i64),
                ],
            )?;
            Ok(request)
        })
    }

    /// Fetch an approval request by id (any status, any workspace — scope is
    /// enforced by the caller, which knows the requesting workspace).
    pub fn get_skill_approval_request(
        &self,
        request_id: &str,
    ) -> Result<Option<SkillApprovalRequest>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {APPROVAL_COLUMNS} FROM skill_approval_requests \
                     WHERE request_id = ?1"
                ),
                [request_id],
                row_to_approval,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// List pending approval requests visible from a workspace (global rows
    /// are stored with the requesting workspace at creation, so visibility
    /// is a direct match; task rows additionally filter by task id).
    pub fn list_pending_skill_approval_requests(
        &self,
        workspace_root: &str,
        task_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<SkillApprovalRequest>, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        let task = task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let limit = limit.clamp(1, 50) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {APPROVAL_COLUMNS} FROM skill_approval_requests \
                 WHERE status = 'pending' AND workspace_root = ?1 \
                   AND (?2 IS NULL OR task_id IS NULL OR task_id = ?2) \
                 ORDER BY created_at ASC, request_id ASC LIMIT ?3"
            ))?;
            let rows = stmt
                .query_map(params![ws, task.as_deref(), limit], row_to_approval)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Consume an approval response.
    ///
    /// Verification (in order, every failure is a refusal):
    /// 1. the request exists;
    /// 2. the request has not expired (expired requests are never
    ///    consumable — request again for the current content);
    /// 3. the request is still pending (replay of a consumed request is
    ///    refused — a request transitions exactly once);
    /// 4. the workspace/task scope matches the request;
    /// 5. the response names a valid option;
    /// 6. the candidate still exists, is still `validated`, still carries
    ///    the requested content hash, and still anchors the requested
    ///    version (stale candidates are refused, never published).
    ///
    /// Effects per response (all through the existing lifecycle gates):
    /// - `approve`: [`ContextStore::approve_skill_candidate`] (validation,
    ///   confidence floor, workspace confinement, name conflicts,
    ///   stale-version anchors, publish safeguards) — needs `skill_root`;
    /// - `reject`: candidate → `rejected` (audit preserved);
    /// - `defer`: candidate → `deferred` (resumable via re-evaluation);
    /// - `modify`: the human instruction is captured; a *new* candidate
    ///   lineage carries the modified content (explicit `modified_content`
    ///   when supplied, otherwise the old content plus a bounded
    ///   human-directive trailer — deterministic without an LLM);
    ///   the new candidate is revalidated through
    ///   [`ContextStore::evaluate_candidate_content`], and a successor
    ///   request is minted so the modified result requires approval again.
    ///   The modified candidate is never published by this call.
    #[allow(clippy::too_many_arguments)]
    pub fn respond_to_skill_approval_request(
        &self,
        request_id: &str,
        requesting_workspace: &str,
        task_id: Option<&str>,
        response: ApprovalResponse,
        modification: Option<&str>,
        modified_content: Option<&str>,
        skill_root: Option<&Path>,
        now: u64,
    ) -> Result<ApprovalOutcome, ContextError> {
        let request = self
            .get_skill_approval_request(request_id)?
            .ok_or_else(|| {
                ContextError::Validation(format!("approval request not found: {request_id}"))
            })?;
        // Expiry enforcement: approval TTLs have no sweeping scheduler
        // (request-driven runtime), so expiry is enforced inline on every
        // response attempt — an expired request is never consumable,
        // whatever its stored status.
        if let Some(exp) = request.expires_at {
            if now >= exp {
                // Best-effort janitor: flip the row to expired so pending
                // listings converge. Refusal is unconditional either way,
                // so a janitor failure still cannot publish.
                let _ = self.expire_skill_approval_requests(now);
                return Err(ContextError::Validation(format!(
                    "approval request {request_id} expired — request again for the current content"
                )));
            }
        }
        if !request.is_pending() {
            return Err(ContextError::Validation(format!(
                "approval request {request_id} is '{}': requests are single-use and \
                 this one was already consumed — list pending requests for the current state",
                request.status
            )));
        }
        let req_ws = canonical_workspace_key(requesting_workspace);
        if req_ws != request.workspace_root {
            return Err(ContextError::Validation(format!(
                "workspace mismatch: request belongs to {}, response came from {req_ws}",
                request.workspace_root
            )));
        }
        let task = task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        if request.task_id.is_some() && task != request.task_id {
            return Err(ContextError::Validation(
                "task mismatch: a task-scoped approval needs its own task context".to_string(),
            ));
        }

        // Stale-candidate checks run before any effect.
        let candidate = self
            .get_skill_candidate(&request.candidate_id)?
            .ok_or_else(|| {
                ContextError::Validation(format!(
                    "candidate {} is gone — the approval has nothing to act on",
                    request.candidate_id
                ))
            })?;
        if candidate.status != SkillCandidateStatus::Validated.as_str() {
            return Err(ContextError::Validation(format!(
                "candidate is '{}': approval requires status 'validated' — \
                 re-validate before responding",
                candidate.status
            )));
        }
        if content_hash(&candidate.proposed_content) != request.candidate_content_hash {
            return Err(ContextError::Validation(format!(
                "stale approval: candidate {} changed since request {request_id} \
                 was created — request again for the current content",
                request.candidate_id
            )));
        }
        if candidate.based_on_version != request.based_on_version {
            return Err(ContextError::Validation(format!(
                "stale approval: candidate version anchor moved since request \
                 {request_id} — request again for the current lineage"
            )));
        }

        match response {
            ApprovalResponse::Approve => {
                let skill_root = skill_root.ok_or_else(|| {
                    ContextError::Validation("approve needs the skill publication root".to_string())
                })?;
                // The existing publish gates run here — approval feeds them,
                // never bypasses them.
                self.approve_skill_candidate(
                    &request.candidate_id,
                    Some(&req_ws),
                    skill_root,
                    now,
                )?;
                let updated = self.transition_approval_request(
                    request_id,
                    ApprovalStatus::Approved,
                    response,
                    None,
                    None,
                    now,
                )?;
                Ok(ApprovalOutcome {
                    request_id: request_id.to_string(),
                    response: response.as_str().to_string(),
                    status: ApprovalStatus::Approved.as_str().to_string(),
                    request: updated,
                    new_candidate_id: None,
                    successor_request_id: None,
                })
            }
            ApprovalResponse::Reject => {
                let reason = modification
                    .map(|m| m.trim().to_string())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "rejected by human approver".to_string());
                self.transition_skill_candidate(
                    &request.candidate_id,
                    SkillCandidateStatus::Rejected,
                    Some(&reason),
                    now,
                )?;
                let updated = self.transition_approval_request(
                    request_id,
                    ApprovalStatus::Rejected,
                    response,
                    Some(&reason),
                    None,
                    now,
                )?;
                Ok(ApprovalOutcome {
                    request_id: request_id.to_string(),
                    response: response.as_str().to_string(),
                    status: ApprovalStatus::Rejected.as_str().to_string(),
                    request: updated,
                    new_candidate_id: None,
                    successor_request_id: None,
                })
            }
            ApprovalResponse::Defer => {
                let reason = modification
                    .map(|m| m.trim().to_string())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| "deferred by human approver".to_string());
                self.transition_skill_candidate(
                    &request.candidate_id,
                    SkillCandidateStatus::Deferred,
                    Some(&reason),
                    now,
                )?;
                let updated = self.transition_approval_request(
                    request_id,
                    ApprovalStatus::Deferred,
                    response,
                    Some(&reason),
                    None,
                    now,
                )?;
                Ok(ApprovalOutcome {
                    request_id: request_id.to_string(),
                    response: response.as_str().to_string(),
                    status: ApprovalStatus::Deferred.as_str().to_string(),
                    request: updated,
                    new_candidate_id: None,
                    successor_request_id: None,
                })
            }
            ApprovalResponse::Modify => {
                let instruction = modification
                    .map(|m| m.trim().to_string())
                    .filter(|m| !m.is_empty())
                    .ok_or_else(|| {
                        ContextError::Validation(
                            "modify requires a modification instruction describing the change"
                                .to_string(),
                        )
                    })?;
                if instruction.chars().count() > MAX_MODIFICATION_CHARS {
                    return Err(ContextError::Validation(format!(
                        "modification exceeds {MAX_MODIFICATION_CHARS} characters"
                    )));
                }
                // New content: explicit replacement wins; otherwise append a
                // bounded human-directive trailer (deterministic without an
                // LLM — the content change is real, so the id changes and
                // revalidation + re-approval are structurally required).
                let new_content = match modified_content
                    .map(|c| c.to_string())
                    .filter(|c| !c.trim().is_empty())
                {
                    Some(explicit) => explicit,
                    None => format!(
                        "{}\n\n## Human modification\n\n{}\n",
                        candidate.proposed_content, instruction
                    ),
                };
                let scope =
                    SkillScope::from_str(&candidate.scope).map_err(ContextError::Validation)?;
                let new_candidate_id = mint_skill_candidate_id(
                    &scope,
                    candidate.workspace_root.as_deref(),
                    candidate.task_id.as_deref(),
                    &candidate.name,
                    &new_content,
                );
                if new_candidate_id == candidate.candidate_id {
                    return Err(ContextError::Validation(
                        "modification produced identical content — nothing to re-approve"
                            .to_string(),
                    ));
                }
                let validation = validate_skill_content(
                    &new_content,
                    &candidate.name,
                    candidate.workspace_root.as_deref(),
                );
                let new_candidate = SkillCandidate {
                    candidate_id: new_candidate_id.clone(),
                    workspace_root: candidate.workspace_root.clone(),
                    task_id: candidate.task_id.clone(),
                    scope: candidate.scope.clone(),
                    name: candidate.name.clone(),
                    description: candidate.description.clone(),
                    purpose: candidate.purpose.clone(),
                    applicability: candidate.applicability.clone(),
                    source_learning_candidates: candidate.source_learning_candidates.clone(),
                    supporting_evidence: candidate.supporting_evidence.clone(),
                    contradicting_evidence: candidate.contradicting_evidence.clone(),
                    proposed_content: new_content,
                    status: SkillCandidateStatus::Candidate.as_str().to_string(),
                    confidence: candidate.confidence,
                    validation: Some(validation),
                    eval_reason: Some(format!(
                        "modified from {} per human instruction: {}",
                        candidate.candidate_id,
                        instruction.chars().take(240).collect::<String>()
                    )),
                    rejection_reason: None,
                    supersedes_skill: candidate.supersedes_skill.clone(),
                    based_on_version: {
                        // Re-anchor to the live lineage: the modified
                        // proposal builds on today's version, not the
                        // parent's anchor.
                        let sid = crate::skills::mint_skill_id(
                            &scope,
                            candidate.workspace_root.as_deref(),
                            &candidate.name,
                        );
                        self.get_skill(&sid)?
                            .filter(|s| s.status == "active")
                            .map(|s| s.current_version)
                    },
                    created_at: now,
                    updated_at: now,
                    expires_at: Some(now + SKILL_CANDIDATE_TTL_SECS),
                };
                self.insert_skill_candidate(&new_candidate)?;
                // Revalidate through the existing automated pass
                // (candidate → evaluating → draft → validated when valid).
                // An invalid modification stays a candidate: it cannot be
                // approved until it passes validation — never published
                // silently.
                let revalidated = self.evaluate_candidate_content(&new_candidate_id, now)?;
                // The automated pass records its own eval notes; restore the
                // modification provenance on top so the human instruction
                // stays traceable on the candidate itself (not only on the
                // successor request's parent link).
                let provenance_note = format!(
                    "modified from {} per human instruction: {}",
                    candidate.candidate_id,
                    instruction.chars().take(240).collect::<String>()
                );
                self.with_conn(|conn| {
                    conn.execute(
                        "UPDATE skill_candidates SET eval_reason = ?2, updated_at = ?3
                         WHERE candidate_id = ?1",
                        params![
                            new_candidate_id,
                            format!(
                                "{provenance_note} | {}",
                                revalidated.eval_reason.as_deref().unwrap_or("")
                            ),
                            now as i64
                        ],
                    )?;
                    Ok::<_, ContextError>(())
                })?;
                // Successor request requires a validated candidate; an
                // invalid modification leaves the parent candidate
                // `validated` so a fresh request can still recover the
                // lineage (only the request is consumed as superseded).
                let successor = if revalidated.status == SkillCandidateStatus::Validated.as_str() {
                    Some(self.create_skill_approval_request_inner(
                        &revalidated,
                        &request,
                        &instruction,
                        now,
                    )?)
                } else {
                    None
                };
                let successor_id = successor.as_ref().map(|r| r.request_id.clone());
                // Parent lineage closure: a valid modification forks a
                // successor that carries the content forward, so the parent
                // candidate is superseded — it must never stay approvable
                // alongside its own replacement (two validated lineages for
                // one skill would fork the publish path).
                if successor.is_some() {
                    self.transition_skill_candidate(
                        &request.candidate_id,
                        SkillCandidateStatus::Superseded,
                        Some(&format!(
                            "superseded by modified successor {new_candidate_id}"
                        )),
                        now,
                    )?;
                }
                let updated = self.transition_approval_request(
                    request_id,
                    ApprovalStatus::Superseded,
                    response,
                    Some(&instruction),
                    successor_id.as_deref(),
                    now,
                )?;
                Ok(ApprovalOutcome {
                    request_id: request_id.to_string(),
                    response: response.as_str().to_string(),
                    status: ApprovalStatus::Superseded.as_str().to_string(),
                    request: updated,
                    new_candidate_id: Some(new_candidate_id),
                    successor_request_id: successor_id,
                })
            }
        }
    }

    /// Mint the successor request for a validated modified candidate,
    /// linked to its parent (re-approval requirement, provenance chain).
    fn create_skill_approval_request_inner(
        &self,
        candidate: &SkillCandidate,
        parent: &SkillApprovalRequest,
        instruction: &str,
        now: u64,
    ) -> Result<SkillApprovalRequest, ContextError> {
        let question = format!(
            "Approve the modified skill '{}'? (changed per your instruction: {})",
            candidate.name,
            instruction.chars().take(200).collect::<String>()
        );
        let request = SkillApprovalRequest {
            request_id: mint_approval_request_id(
                &candidate.candidate_id,
                &parent.proposed_action,
                now,
            ),
            candidate_id: candidate.candidate_id.clone(),
            candidate_content_hash: content_hash(&candidate.proposed_content),
            based_on_version: candidate.based_on_version,
            workspace_root: parent.workspace_root.clone(),
            task_id: parent.task_id.clone(),
            proposed_action: parent.proposed_action.clone(),
            question: question.chars().take(MAX_QUESTION_CHARS).collect(),
            options: default_approval_options(),
            status: ApprovalStatus::Pending.as_str().to_string(),
            response: None,
            modification: None,
            successor_request_id: None,
            parent_request_id: Some(parent.request_id.clone()),
            created_at: now,
            updated_at: now,
            responded_at: None,
            expires_at: Some(now + SKILL_APPROVAL_TTL_SECS),
        };
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO skill_approval_requests (
                    request_id, candidate_id, candidate_content_hash,
                    based_on_version, workspace_root, task_id, proposed_action,
                    question, options_json, status, response, modification,
                    successor_request_id, parent_request_id,
                    created_at, updated_at, responded_at, expires_at
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12,
                           ?13, ?14, ?15, ?16, ?17, ?18)",
                params![
                    request.request_id,
                    request.candidate_id,
                    request.candidate_content_hash,
                    request.based_on_version.map(|v| v as i64),
                    request.workspace_root,
                    request.task_id.as_deref(),
                    request.proposed_action,
                    request.question,
                    serde_json::to_string(&request.options)
                        .map_err(|e| ContextError::Decode(e.to_string()))?,
                    request.status,
                    request.response.as_deref(),
                    request.modification.as_deref(),
                    request.successor_request_id.as_deref(),
                    request.parent_request_id.as_deref(),
                    request.created_at as i64,
                    request.updated_at as i64,
                    request.responded_at.map(|t| t as i64),
                    request.expires_at.map(|t| t as i64),
                ],
            )?;
            Ok(())
        })?;
        Ok(request)
    }

    /// Mark a request terminal (exactly-once transition out of pending).
    fn transition_approval_request(
        &self,
        request_id: &str,
        status: ApprovalStatus,
        response: ApprovalResponse,
        modification: Option<&str>,
        successor_request_id: Option<&str>,
        now: u64,
    ) -> Result<SkillApprovalRequest, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE skill_approval_requests SET status = ?2, response = ?3,
                        modification = COALESCE(?4, modification),
                        successor_request_id = COALESCE(?5, successor_request_id),
                        updated_at = ?6, responded_at = ?6
                 WHERE request_id = ?1 AND status = 'pending'",
                params![
                    request_id,
                    status.as_str(),
                    response.as_str(),
                    modification,
                    successor_request_id,
                    now as i64,
                ],
            )?;
            if updated == 0 {
                return Err(ContextError::Validation(format!(
                    "approval request {request_id} is no longer pending — \
                     it was already consumed (replay refused)"
                )));
            }
            Ok(())
        })?;
        self.get_skill_approval_request(request_id)?.ok_or_else(|| {
            ContextError::Validation("approval request vanished after transition".into())
        })
    }

    /// Expire stale pending requests. Terminal rows are never rewritten.
    pub fn expire_skill_approval_requests(&self, now: u64) -> Result<usize, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE skill_approval_requests SET status = 'expired', updated_at = ?1
                 WHERE status = 'pending' AND expires_at IS NOT NULL AND expires_at < ?1",
                [now as i64],
            )?;
            Ok(updated)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_checked, STATE_DB_FILE};
    use crate::skills::{SkillApplicability, SkillCandidateStatus};

    fn test_store(dir: &std::path::Path) -> ContextStore {
        let db_path = dir.join(STATE_DB_FILE);
        open_checked(&db_path).unwrap();
        ContextStore::new(db_path)
    }

    fn test_now() -> u64 {
        1_700_000_000
    }

    fn valid_content(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: A test skill for {name}\n---\n\n\
             # Purpose\n\nTest procedure body."
        )
    }

    fn seed_validated(store: &ContextStore, id: &str, name: &str, ws: &str) -> SkillCandidate {
        let c = SkillCandidate {
            candidate_id: id.to_string(),
            workspace_root: Some(ws.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: name.to_string(),
            description: format!("Skill {name}"),
            purpose: "Testing".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: vec![],
            supporting_evidence: vec![],
            contradicting_evidence: vec![],
            proposed_content: valid_content(name),
            status: "candidate".to_string(),
            confidence: 0.75,
            validation: None,
            eval_reason: None,
            rejection_reason: None,
            supersedes_skill: None,
            based_on_version: None,
            created_at: test_now(),
            updated_at: test_now(),
            expires_at: None,
        };
        store.insert_skill_candidate(&c).unwrap();
        store
            .transition_skill_candidate(id, SkillCandidateStatus::Evaluating, None, test_now() + 1)
            .unwrap();
        store
            .promote_candidate_to_draft(id, &valid_content(name), test_now() + 2)
            .unwrap();
        store
            .transition_skill_candidate(id, SkillCandidateStatus::Validated, None, test_now() + 3)
            .unwrap()
    }

    #[test]
    fn request_emits_question_and_options() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        seed_validated(&store, "sc::q001", "review-helper", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::q001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        assert!(req.is_pending());
        assert!(
            req.question.contains("review-helper"),
            "got: {}",
            req.question
        );
        assert_eq!(req.options, vec!["approve", "reject", "modify", "defer"]);
        let ix = req.interaction();
        assert_eq!(ix.kind, SKILL_APPROVAL_KIND);
    }

    #[test]
    fn duplicate_request_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        seed_validated(&store, "sc::idem001", "idem-skill", "/repo");
        let a = store
            .create_skill_approval_request(
                "sc::idem001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        let b = store
            .create_skill_approval_request(
                "sc::idem001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 11,
            )
            .unwrap();
        assert_eq!(a.request_id, b.request_id);
    }

    #[test]
    fn wrong_workspace_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        seed_validated(&store, "sc::ws001", "ws-skill", "/repo-a");
        // Request from the owning workspace works.
        store
            .create_skill_approval_request(
                "sc::ws001",
                "/repo-a",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        // Another workspace cannot even request.
        assert!(store
            .create_skill_approval_request(
                "sc::ws001",
                "/repo-b",
                None,
                "approve",
                None,
                test_now() + 11
            )
            .is_err());
    }

    #[test]
    fn replay_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        seed_validated(&store, "sc::replay001", "replay-skill", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::replay001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Reject,
                Some("nope"),
                None,
                None,
                test_now() + 20,
            )
            .unwrap();
        let again = store.respond_to_skill_approval_request(
            &req.request_id,
            "/repo",
            None,
            ApprovalResponse::Approve,
            None,
            None,
            None,
            test_now() + 21,
        );
        assert!(again.unwrap_err().to_string().contains("already consumed"));
    }

    fn skills_dir(dir: &std::path::Path) -> std::path::PathBuf {
        let p = dir.join("skills");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn approve_continues_lifecycle_and_publishes() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        seed_validated(&store, "sc::appr001", "approve-me", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::appr001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        let outcome = store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Approve,
                None,
                None,
                Some(&skills),
                test_now() + 20,
            )
            .unwrap();
        assert_eq!(outcome.status, "approved");
        // The existing lifecycle ran: candidate is active, skill + version exist.
        let cand = store.get_skill_candidate("sc::appr001").unwrap().unwrap();
        assert_eq!(cand.status, "active");
        let skill = store
            .get_skill_by_name("approve-me")
            .unwrap()
            .expect("published");
        assert_eq!(skill.status, "active");
        assert!(skills.join("approve-me").join("SKILL.md").exists());
    }

    #[test]
    fn reject_prevents_publish_and_persists() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        seed_validated(&store, "sc::rej002", "reject-me", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::rej002",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        let outcome = store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Reject,
                Some("not useful"),
                None,
                None,
                test_now() + 20,
            )
            .unwrap();
        assert_eq!(outcome.status, "rejected");
        let cand = store.get_skill_candidate("sc::rej002").unwrap().unwrap();
        assert_eq!(cand.status, "rejected");
        assert!(store.get_skill_by_name("reject-me").unwrap().is_none());
        assert!(!skills.join("reject-me").join("SKILL.md").exists());
    }

    #[test]
    fn defer_persists_deferred_state_and_resumes() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        seed_validated(&store, "sc::def001", "defer-me", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::def001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        let outcome = store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Defer,
                Some("later"),
                None,
                None,
                test_now() + 20,
            )
            .unwrap();
        assert_eq!(outcome.status, "deferred");
        let cand = store.get_skill_candidate("sc::def001").unwrap().unwrap();
        assert_eq!(cand.status, "deferred");
        // Resume path: deferred re-evaluates toward validated again.
        let re = store
            .evaluate_candidate_content("sc::def001", test_now() + 30)
            .unwrap();
        assert_eq!(re.status, "validated");
    }

    #[test]
    fn modify_changes_candidate_revalidates_and_requires_reapproval() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        seed_validated(&store, "sc::mod001", "modify-me", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::mod001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        let new_content =
            valid_content("modify-me").replace("Test procedure body.", "Revised procedure body.");
        let outcome = store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Modify,
                Some("make it project-specific"),
                Some(&new_content),
                None,
                test_now() + 20,
            )
            .unwrap();
        // Parent request is consumed as superseded; nothing was published.
        assert_eq!(outcome.status, "superseded");
        assert!(store.get_skill_by_name("modify-me").unwrap().is_none());
        let new_id = outcome.new_candidate_id.clone().expect("new lineage");
        assert_ne!(new_id, "sc::mod001");
        // The modified candidate was revalidated through the existing pass.
        let re = store.get_skill_candidate(&new_id).unwrap().unwrap();
        assert_eq!(re.status, "validated");
        assert!(re
            .eval_reason
            .as_deref()
            .unwrap_or("")
            .contains("sc::mod001"));
        // Re-approval is structurally required: a successor request exists
        // and the old request can never publish.
        let succ_id = outcome.successor_request_id.clone().expect("successor");
        let succ = store.get_skill_approval_request(&succ_id).unwrap().unwrap();
        assert!(succ.is_pending());
        assert_eq!(
            succ.parent_request_id.as_deref(),
            Some(req.request_id.as_str())
        );
        // The old request is consumed — approving through it is refused.
        assert!(store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Approve,
                None,
                None,
                Some(&skills),
                test_now() + 30,
            )
            .is_err());
        // The successor approval publishes the *modified* content.
        let done = store
            .respond_to_skill_approval_request(
                &succ_id,
                "/repo",
                None,
                ApprovalResponse::Approve,
                None,
                None,
                Some(&skills),
                test_now() + 40,
            )
            .unwrap();
        assert_eq!(done.status, "approved");
        let published = std::fs::read_to_string(skills.join("modify-me").join("SKILL.md")).unwrap();
        assert!(published.contains("Revised procedure body."));
    }

    #[test]
    fn expired_request_is_refused_not_consumed() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        seed_validated(&store, "sc::exp001", "expiry-skill", "/repo");
        let created_at = test_now() + 10;
        let req = store
            .create_skill_approval_request("sc::exp001", "/repo", None, "approve", None, created_at)
            .unwrap();
        assert!(req.is_pending());
        // Past the 30-day TTL the request is unconsumable: approve,
        // reject, and defer are all refused with an expiry error.
        let past_expiry = created_at + SKILL_APPROVAL_TTL_SECS + 1;
        for response in [
            ApprovalResponse::Approve,
            ApprovalResponse::Reject,
            ApprovalResponse::Defer,
        ] {
            let err = store
                .respond_to_skill_approval_request(
                    &req.request_id,
                    "/repo",
                    None,
                    response,
                    None,
                    None,
                    Some(&skills),
                    past_expiry,
                )
                .unwrap_err();
            assert!(err.to_string().contains("expired"), "got: {err}");
        }
        // Nothing was published or transitioned by the refused attempts.
        assert!(store.get_skill_by_name("expiry-skill").unwrap().is_none());
        assert_eq!(
            store
                .get_skill_candidate("sc::exp001")
                .unwrap()
                .unwrap()
                .status,
            "validated"
        );
        // The janitor flipped the row to expired.
        assert_eq!(
            store
                .get_skill_approval_request(&req.request_id)
                .unwrap()
                .unwrap()
                .status,
            "expired"
        );
        // A retry after expiry mints a fresh request (dedup ignores the
        // expired row) and the fresh request is consumable.
        let fresh = store
            .create_skill_approval_request(
                "sc::exp001",
                "/repo",
                None,
                "approve",
                None,
                past_expiry + 1,
            )
            .unwrap();
        assert_ne!(fresh.request_id, req.request_id);
        assert!(fresh.is_pending());
        let done = store
            .respond_to_skill_approval_request(
                &fresh.request_id,
                "/repo",
                None,
                ApprovalResponse::Approve,
                None,
                None,
                Some(&skills),
                past_expiry + 2,
            )
            .unwrap();
        assert_eq!(done.status, "approved");
    }

    #[test]
    fn modify_supersedes_parent_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        seed_validated(&store, "sc::modpar001", "modify-parent", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::modpar001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        let new_content = valid_content("modify-parent")
            .replace("Test procedure body.", "Revised procedure body.");
        let outcome = store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Modify,
                Some("make it project-specific"),
                Some(&new_content),
                None,
                test_now() + 20,
            )
            .unwrap();
        assert!(outcome.successor_request_id.is_some());
        // The parent candidate is superseded by its successor: it must
        // never stay approvable alongside its own replacement.
        let parent = store.get_skill_candidate("sc::modpar001").unwrap().unwrap();
        assert_eq!(parent.status, "superseded");
        // Direct publication of the parent is refused — the successor
        // lineage is the only live path.
        let skills = skills_dir(dir.path());
        let err = store
            .approve_skill_candidate("sc::modpar001", Some("/repo"), &skills, test_now() + 30)
            .unwrap_err();
        assert!(err.to_string().contains("superseded"), "got: {err}");
    }

    #[test]
    fn stale_candidate_is_refused_not_published() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        seed_validated(&store, "sc::stale001", "stale-skill", "/repo");
        let req = store
            .create_skill_approval_request(
                "sc::stale001",
                "/repo",
                None,
                "approve",
                None,
                test_now() + 10,
            )
            .unwrap();
        // The candidate moves on (published directly): the pending request
        // now points at a non-validated lineage — stale, never published
        // through the old request.
        store
            .approve_skill_candidate("sc::stale001", Some("/repo"), &skills, test_now() + 15)
            .unwrap();
        let err = store
            .respond_to_skill_approval_request(
                &req.request_id,
                "/repo",
                None,
                ApprovalResponse::Approve,
                None,
                None,
                Some(&skills),
                test_now() + 20,
            )
            .unwrap_err();
        assert!(err.to_string().contains("validated"), "got: {err}");
    }

    #[test]
    fn pending_approval_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join(STATE_DB_FILE);
        open_checked(&db_path).unwrap();
        let req_id = {
            let store = ContextStore::new(db_path.clone());
            // Seed through the store (same helpers, scoped to this block).
            let c = SkillCandidate {
                candidate_id: "sc::persist001".to_string(),
                workspace_root: Some("/repo".to_string()),
                task_id: None,
                scope: "project".to_string(),
                name: "persist-skill".to_string(),
                description: "Skill persist-skill".to_string(),
                purpose: "Testing".to_string(),
                applicability: SkillApplicability::default(),
                source_learning_candidates: vec![],
                supporting_evidence: vec![],
                contradicting_evidence: vec![],
                proposed_content: valid_content("persist-skill"),
                status: "candidate".to_string(),
                confidence: 0.75,
                validation: None,
                eval_reason: None,
                rejection_reason: None,
                supersedes_skill: None,
                based_on_version: None,
                created_at: test_now(),
                updated_at: test_now(),
                expires_at: None,
            };
            store.insert_skill_candidate(&c).unwrap();
            store
                .transition_skill_candidate(
                    "sc::persist001",
                    SkillCandidateStatus::Evaluating,
                    None,
                    test_now() + 1,
                )
                .unwrap();
            store
                .promote_candidate_to_draft(
                    "sc::persist001",
                    &valid_content("persist-skill"),
                    test_now() + 2,
                )
                .unwrap();
            store
                .transition_skill_candidate(
                    "sc::persist001",
                    SkillCandidateStatus::Validated,
                    None,
                    test_now() + 3,
                )
                .unwrap();
            let req = store
                .create_skill_approval_request(
                    "sc::persist001",
                    "/repo",
                    None,
                    "approve",
                    None,
                    test_now() + 10,
                )
                .unwrap();
            req.request_id.clone()
        };
        // Reopen (new handle, same file — the hard-kill recovery path).
        let reopened = ContextStore::new(db_path);
        let req = reopened
            .get_skill_approval_request(&req_id)
            .unwrap()
            .expect("survives restart");
        assert!(req.is_pending());
        assert_eq!(req.candidate_id, "sc::persist001");
        let pending = reopened
            .list_pending_skill_approval_requests("/repo", None, 10)
            .unwrap();
        assert!(pending.iter().any(|r| r.request_id == req_id));
    }
}

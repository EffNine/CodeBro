//! P13 skill evolution v1: detect recurring weaknesses in an active skill,
//! propose an evidence-backed successor version, and evolve it through the
//! existing human-approval lifecycle.
//!
//! ```text
//! ACTIVE skill v1
//!         │
//!         ▼
//! real execution evidence (skill-linked history, same workspace,
//!                         newer than the active version)
//!         │
//!         ▼
//! deterministic weakness detection (this module — ≥3 linked failures,
//!         contradiction gates, recurring-signal gate, health veto)
//!         │
//!         ▼
//! P3 learning candidate (FailurePattern, evidence-cited)
//!         │
//!         ▼
//! P3 evaluation (accepted-only trust boundary, existing machinery)
//!         │
//!         ▼
//! P4 skill candidate (existing row shape: supersedes_skill +
//!         based_on_version anchor the v1 → v2 lineage; no new tables)
//!         │
//!         ▼
//! automated validation (existing `evaluate_candidate_content`)
//!         │
//!         ▼
//! human approval (`request_approval` → needs_input → `respond`)
//!         │
//!         ▼
//! ACTIVE skill v2 (v1 rows immutable, rollback reuses `rollback_skill`)
//! ```
//!
//! # What evolution is (and is not)
//!
//! - **Explicitly invoked.** OpenCode calls `skill detect_evolution`;
//!   nothing runs in the background. No scheduler, daemon, watcher,
//!   executor, or automatic publish.
//! - **The human remains the final authority.** A detected weakness becomes
//!   a validated candidate at most. Publication still requires an explicit
//!   human `approve` through the existing approval protocol; `reject` and
//!   `defer` leave v1 active; `modify` mints a fresh lineage that must be
//!   revalidated and re-approved.
//! - **An evolution candidate is a skill candidate.** It reuses the
//!   `skill_candidates` row (plus `skill_versions` on approval) with
//!   `supersedes_skill` pointing at the source skill and `based_on_version`
//!   anchoring the source version. No new database subsystem, no schema
//!   change: stale anchors, duplicate lineages, replay protection, and
//!   restart survival all come from the existing lifecycle.
//! - **Deterministic.** Weakness classification is token/outcome-label
//!   matching over canonical history: no LLM, no embeddings, no network.
//!   Re-running on unchanged history converges (same content → same
//!   deterministic ids, idempotent refresh), never duplicates.
//! - **Conservative.** A single failure — or two — is noise, never a
//!   candidate. Unrelated failures (other tools, other skills, other
//!   workspaces) never count. Successes contradict: contested evidence
//!   blocks the proposal. A healthy skill (per its own usage counters) is
//!   never evolved no matter what the detector sees.
//! - **Version-scoped.** Only executions strictly newer than the active
//!   version's activation count. Old-lineage failures cannot re-trigger
//!   evolution of a version that already addressed them, and a rollback
//!   does not immediately re-propose (its window restarts). The bound is
//!   strict (`>`, not `>=`): timestamps have second resolution, so an
//!   execution recorded in the same second as an activation is
//!   conservatively excluded rather than risk attributing pre-activation
//!   evidence to the new version.
//!
//! # Minimum signal (all required for a validated evolution candidate)
//!
//! - The source skill is `active` with a recorded active version.
//! - At least [`EVOLUTION_MIN_FAILURES`] skill-linked failure executions in
//!   the same workspace, newer than the active version.
//! - No strong contradiction: linked successes neither outweigh the
//!   failures nor reach half of them (mirrors the P3 contested rule), and
//!   the skill's own health counters — when assessable — report degraded.
//! - A recurring signal: the failures share an outcome label or a
//!   meaningful token in at least two executions (one shared word across
//!   unrelated failures is not a pattern).
//! - Accepted P3 evaluation with confidence at or above the skill approval
//!   floor ([`crate::skills::SKILL_APPROVAL_MIN_CONFIDENCE`]).
//! - No equivalent candidate already in review and no lineage advance
//!   since detection (deterministic ids converge; anchors refuse stale).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::learning::{
    CandidateKind, CandidateStatus, LearnScope, LearningCandidate, CANDIDATE_TTL_SECS,
    MAX_DETECTION_EVENTS,
};
use crate::skills::{
    content_hash, mint_skill_candidate_id, mint_skill_id, validate_skill_content, Skill,
    SkillApplicability, SkillCandidate, SkillCandidateStatus, SkillScope,
    SKILL_APPROVAL_MIN_CONFIDENCE, SKILL_CANDIDATE_TTL_SECS,
};
use crate::store::{ContextError, ContextStore, EVENT_COLUMNS};
use crate::workspace::canonical_workspace_key;

// ─── Constants ──────────────────────────────────────────────────────────

/// Minimum skill-linked failure executions before a weakness may be
/// proposed. One failure is an incident; two are a coincidence; three are
/// the smallest recurrence worth a human's attention.
pub const EVOLUTION_MIN_FAILURES: usize = 3;
/// Maximum evolution candidates minted per detector run (ranked
/// worst-health-first, so output stays bounded and model-friendly).
pub const EVOLUTION_MAX_CANDIDATES_PER_RUN: usize = 3;
/// Maximum evolution views echoed per report (reason views included).
pub const EVOLUTION_MAX_VIEWS: usize = 8;
/// Maximum event ids stored per learning candidate side (supporting /
/// contradicting). Enough provenance for audit without unbounded rows.
pub const EVOLUTION_MAX_EVIDENCE_IDS: usize = 100;
/// Maximum event ids echoed per candidate in the MCP view (the DB row may
/// hold more; the wire view stays small).
pub const EVOLUTION_MAX_VIEW_IDS: usize = 20;

/// Report status values (machine-readable, model-friendly).
pub const STATUS_CANDIDATE_FOUND: &str = "candidate_found";
pub const STATUS_ALREADY_EXISTS: &str = "already_exists";
pub const STATUS_LEARNING_ONLY: &str = "learning_only";
pub const STATUS_NO_CANDIDATES: &str = "no_candidates";

/// Deterministic weakness kinds (v1 taxonomy, conservative by design).
pub const WEAKNESS_VERIFICATION_GAP: &str = "verification_gap";
pub const WEAKNESS_TIMEOUT_GAP: &str = "timeout_gap";
pub const WEAKNESS_RELIABILITY_GAP: &str = "reliability_gap";

/// Tokens that mark a verification-shaped weakness (lowercase).
const VERIFICATION_TOKENS: &[&str] = &[
    "verification",
    "verify",
    "verifying",
    "verified",
    "validation",
    "validate",
    "validating",
    "validated",
    "test",
    "tests",
    "testing",
    "tested",
    "readback",
    "confirm",
    "confirmed",
];
/// Tokens that mark a timeout-shaped weakness (lowercase).
const TIMEOUT_TOKENS: &[&str] = &[
    "timeout",
    "timeouts",
    "timed",
    "deadline",
    "deadlines",
    "hanging",
    "hung",
];
/// Outcome labels that read as verification failures (lowercase).
const VERIFICATION_OUTCOMES: &[&str] = &[
    "test_failure",
    "validation_failed",
    "task_validation_failed",
];
/// Outcome labels that read as timeouts (lowercase).
const TIMEOUT_OUTCOMES: &[&str] = &["timeout"];
/// Noise words excluded from the recurring-signal scan: outcome
/// vocabulary (present in every failure summary by construction), the
/// linkage itself (the skill name is in every linked summary), and generic
/// glue. Without this filter every failure set would "recur".
const SIGNAL_STOPWORDS: &[&str] = &[
    "skill",
    "skills",
    "execution",
    "executions",
    "executed",
    "recorded",
    "recording",
    "success",
    "successful",
    "succeed",
    "succeeded",
    "failure",
    "failures",
    "failed",
    "fail",
    "failing",
    "passed",
    "pass",
    "passing",
    "error",
    "errors",
    "version",
    "with",
    "from",
    "that",
    "this",
    "have",
    "been",
    "were",
    "into",
    "over",
    "after",
    "before",
    "during",
    "again",
    "what",
    "when",
    "there",
    "their",
    "them",
    "then",
    "than",
    "your",
    "about",
    "into",
    // Provenance vocabulary: every captured skill-use event carries an
    // authority payload (`observed`), so it recurs by construction and is
    // linkage — not signal.
    "authority",
    "observed",
];

// ─── Types ──────────────────────────────────────────────────────────────

/// A classified recurring weakness: what is wrong with v1, in bounded
/// human-reviewable form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvolutionWeakness {
    /// One of `verification_gap` | `timeout_gap` | `reliability_gap`.
    pub kind: String,
    /// One-line label for views and questions.
    pub label: String,
    /// What is wrong with v1 (1–2 sentences, evidence-shaped).
    pub description: String,
    /// What v2 changes (1–2 sentences, the concrete delta).
    pub proposed_change: String,
    /// Why that change should address the weakness.
    pub rationale: String,
}

/// One detector outcome for a single active skill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionCandidateView {
    /// Source skill name and id (the v1 lineage).
    pub source_skill: String,
    pub source_skill_id: String,
    /// Active version the evidence was scoped to (the `based_on_version`).
    pub source_version: u32,
    /// The version the proposal would publish (source + 1).
    pub proposed_version: u32,
    /// Classified weakness (`none` when no candidate was produced).
    pub weakness_kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weakness: Option<EvolutionWeakness>,
    /// Skill-linked failures in the version window.
    pub observations: usize,
    /// Supporting event count (failures, stored on the candidates).
    pub supporting_events: usize,
    /// Contradicting event count considered (successes, stored and discounted).
    pub contradicting_events: usize,
    /// Post-evaluation learning confidence (bounded, two decimals).
    pub confidence: f64,
    /// `project` | `global` (v1 skills only; task skills are skipped).
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learning_status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_status: Option<String>,
    /// `created_validated` | `duplicate_candidate_in_review` |
    /// `skipped_terminal` | `lineage_advanced` | `learning_only_*` |
    /// `validation_failed` | `insufficient_evidence` | `contested` |
    /// `contradicted` | `no_recurring_signal` | `healthy_no_evolution` |
    /// `not_active` | `no_active_version` | `unsupported_scope`.
    pub outcome: String,
    pub next_action: String,
    /// Bounded sample of supporting event ids (audit pointers, capped).
    #[serde(default)]
    pub evidence_sample: Vec<i64>,
    /// Content identity: v1 hash and proposed v2 hash (deterministic).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_content_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed_content_hash: Option<String>,
    /// Health snapshot at detection (simple v1/v2 evidence comparison).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_success_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_failure_count: Option<u64>,
    /// Suggested human question for `request_approval` (OpenCode renders it;
    /// CodeBro never asks directly).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_question: Option<String>,
}

/// Bounded, model-friendly result of one detector run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvolutionReport {
    pub status: String,
    pub workspace_root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_selector: Option<String>,
    /// Active skills examined this run (before view caps).
    pub skills_considered: usize,
    #[serde(default)]
    pub candidates: Vec<EvolutionCandidateView>,
    pub next_action: String,
    pub note: String,
}

// ─── Internal event view ────────────────────────────────────────────────

/// One skill-linked history event as the detector sees it.
///
/// `pub(crate)` so the P15 validation pass reuses the exact same linkage
/// (same SQL, same name/id matching, same workspace discipline) instead of
/// reimplementing it and drifting.
pub(crate) struct LinkedEvent {
    pub(crate) id: i64,
    pub(crate) session_id: Option<String>,
    pub(crate) kind: String,
    pub(crate) outcome: Option<String>,
    pub(crate) summary: String,
    pub(crate) payload: Option<String>,
    pub(crate) created_at: u64,
}

/// Fetch skill-linked execution events: `tool = 'skill'` rows in this
/// workspace whose summary or payload names the skill (id or name,
/// case-insensitive). Structural linkage — unrelated tool runs and other
/// skills' executions never match, so they can never become support.
///
/// `pub(crate)` for P15 reuse (same linkage, no drift).
pub(crate) fn fetch_linked_events(
    conn: &rusqlite::Connection,
    workspace_root: &str,
    skill: &Skill,
) -> Result<Vec<LinkedEvent>, ContextError> {
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM events \
         WHERE workspace_root = ?1 AND tool = 'skill' \
         ORDER BY created_at ASC, id ASC LIMIT ?2"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_root, MAX_DETECTION_EVENTS as i64],
            crate::store::row_to_event_pub,
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ContextError::Decode(e.to_string()))?;
    let name_lower = skill.name.to_ascii_lowercase();
    let id_lower = skill.skill_id.to_ascii_lowercase();
    let mut out = Vec::new();
    for event in rows {
        // Defense in depth: re-apply the workspace predicate in Rust.
        if event.workspace_root != workspace_root {
            continue;
        }
        let id = match event.id {
            Some(id) if id > 0 => id,
            _ => continue,
        };
        let summary = event.summary.clone().unwrap_or_default();
        let payload = event.payload.clone().unwrap_or_default();
        let haystack = format!("{summary}\n{payload}").to_ascii_lowercase();
        if !haystack.contains(&name_lower) && !haystack.contains(&id_lower) {
            continue;
        }
        out.push(LinkedEvent {
            id,
            session_id: event.session_id.clone(),
            kind: event.kind.clone(),
            outcome: event.outcome.clone(),
            summary,
            payload: event.payload.clone(),
            created_at: event.created_at,
        });
    }
    Ok(out)
}

// ─── Weakness taxonomy ──────────────────────────────────────────────────

/// Lowercase alphanumeric tokens of length ≥ 4, minus noise.
fn signal_tokens(text: &str) -> BTreeSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 4)
        .map(|t| t.to_ascii_lowercase())
        .filter(|t| !SIGNAL_STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Find a recurring signal across the failure set: a shared outcome label
/// (≥2 executions) or a shared meaningful token in the failure summaries
/// (≥2 executions). Only summaries are scanned: payloads are structured
/// linkage metadata (skill ids, authority markers) whose keys would recur
/// by construction. Returns the recurring outcome labels and tokens for
/// classification.
fn recurring_signal(failures: &[&LinkedEvent], skill_name: &str) -> (Vec<String>, Vec<String>) {
    let mut label_counts: HashMap<String, usize> = HashMap::new();
    for e in failures {
        let label = e
            .outcome
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if !label.is_empty() {
            *label_counts.entry(label).or_default() += 1;
        }
    }
    let mut labels: Vec<String> = label_counts
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(l, _)| l)
        .collect();
    labels.sort();

    let name_tokens = signal_tokens(&skill_name.replace('-', " "));
    let mut token_hits: HashMap<String, usize> = HashMap::new();
    for e in failures {
        let mut toks = signal_tokens(&e.summary);
        // The linkage itself is not a signal: drop skill-name tokens.
        for t in &name_tokens {
            toks.remove(t);
        }
        for t in toks {
            *token_hits.entry(t).or_default() += 1;
        }
    }
    let mut tokens: Vec<String> = token_hits
        .into_iter()
        .filter(|(_, n)| *n >= 2)
        .map(|(t, _)| t)
        .collect();
    tokens.sort();
    (labels, tokens)
}

/// Classify a recurring failure set into the v1 weakness taxonomy.
/// Priority is fixed: verification-shaped evidence wins over
/// timeout-shaped evidence; anything else is an honestly-labeled generic
/// reliability gap (no localized cause is claimed that the evidence does
/// not support).
fn classify_weakness(
    skill_name: &str,
    source_version: u32,
    labels: &[String],
    tokens: &[String],
    failure_count: usize,
) -> EvolutionWeakness {
    let token_set: HashSet<&str> = tokens.iter().map(String::as_str).collect();
    let has_verification_outcome = labels
        .iter()
        .any(|l| VERIFICATION_OUTCOMES.contains(&l.as_str()));
    let has_timeout_outcome = labels
        .iter()
        .any(|l| TIMEOUT_OUTCOMES.contains(&l.as_str()));
    let has_verification_token = VERIFICATION_TOKENS.iter().any(|t| token_set.contains(t));
    let has_timeout_token = TIMEOUT_TOKENS.iter().any(|t| token_set.contains(t));

    if has_verification_outcome || has_verification_token {
        EvolutionWeakness {
            kind: WEAKNESS_VERIFICATION_GAP.to_string(),
            label: "verification step is missing in repeated executions".to_string(),
            description: format!(
                "Skill '{skill_name}' v{source_version} repeatedly fails around verification \
                 ({failure_count} linked failures): executions finish without the mandatory \
                 read-back check and test pass the procedure requires."
            ),
            proposed_change: format!(
                "Add a mandatory read-back verification section to '{skill_name}' v{}: \
                 read back changed files after every apply and confirm the verification \
                 passes before claiming done.",
                source_version + 1
            ),
            rationale: "The recurring failures share verification-shaped evidence \
                (test/validation outcomes or verify language in the failure reports), so an \
                explicit non-skippable verification step addresses the observed gap rather \
                than an unrelated part of the procedure."
                .to_string(),
        }
    } else if has_timeout_outcome || has_timeout_token {
        EvolutionWeakness {
            kind: WEAKNESS_TIMEOUT_GAP.to_string(),
            label: "timeout handling is missing in repeated executions".to_string(),
            description: format!(
                "Skill '{skill_name}' v{source_version} repeatedly fails on timeouts \
                 ({failure_count} linked failures): long-running steps have no explicit \
                 deadline, retry, or partial-progress policy."
            ),
            proposed_change: format!(
                "Add an explicit timeout-handling section to '{skill_name}' v{}: \
                 bound long-running steps, retry once on timeout, and report partial \
                 progress instead of hanging.",
                source_version + 1
            ),
            rationale: "The recurring failures share timeout-shaped evidence, so an \
                explicit deadline/retry/report policy addresses the observed gap."
                .to_string(),
        }
    } else {
        EvolutionWeakness {
            kind: WEAKNESS_RELIABILITY_GAP.to_string(),
            label: "recurring failures without an isolated localized cause".to_string(),
            description: format!(
                "Skill '{skill_name}' v{source_version} has {failure_count} linked failures \
                 with no single isolated cause: the failure reports share a recurrence \
                 signal but do not localize to verification or timeouts."
            ),
            proposed_change: format!(
                "Add a failure-hardening checklist to '{skill_name}' v{}: validate inputs \
                 before acting, verify after acting, and report failures as task evidence \
                 instead of claiming success.",
                source_version + 1
            ),
            rationale: "No localized cause was isolated, so v2 adds only a conservative \
                hardening checklist around the unchanged v1 procedure — no v1 step is \
                rewritten on evidence that does not justify rewriting it."
                .to_string(),
        }
    }
}

/// Whether an outcome label reads as verification-shaped (lowercase match).
pub(crate) fn is_verification_outcome(outcome: Option<&str>) -> bool {
    let label = outcome.unwrap_or("").trim().to_ascii_lowercase();
    VERIFICATION_OUTCOMES.contains(&label.as_str())
}

/// Whether an outcome label reads as timeout-shaped (lowercase match).
pub(crate) fn is_timeout_outcome(outcome: Option<&str>) -> bool {
    let label = outcome.unwrap_or("").trim().to_ascii_lowercase();
    TIMEOUT_OUTCOMES.contains(&label.as_str())
}

/// Whether a failure summary carries verification-shaped language.
/// Scans the summary only (payloads are structured linkage metadata whose
/// keys would recur by construction) using the same token vocabulary as
/// the detector, minus the linkage itself.
pub(crate) fn summary_has_verification_token(summary: &str, skill_name: &str) -> bool {
    let mut toks = signal_tokens(summary);
    for t in signal_tokens(&skill_name.replace('-', " ")) {
        toks.remove(&t);
    }
    VERIFICATION_TOKENS.iter().any(|t| toks.contains(*t))
}

/// Whether a failure summary carries timeout-shaped language (same
/// summary-only discipline as verification).
pub(crate) fn summary_has_timeout_token(summary: &str, skill_name: &str) -> bool {
    let mut toks = signal_tokens(summary);
    for t in signal_tokens(&skill_name.replace('-', " ")) {
        toks.remove(&t);
    }
    TIMEOUT_TOKENS.iter().any(|t| toks.contains(*t))
}

/// Whether one failure execution is evidence of the named weakness kind.
/// Verification wins over timeout (same priority as the detector); the
/// generic reliability gap treats every failure as targeted; unknown kinds
/// never match (no guessing).
pub(crate) fn is_targeted_failure(
    weakness_kind: &str,
    outcome: Option<&str>,
    summary: &str,
    skill_name: &str,
) -> bool {
    match weakness_kind {
        k if k == WEAKNESS_VERIFICATION_GAP => {
            is_verification_outcome(outcome) || summary_has_verification_token(summary, skill_name)
        }
        k if k == WEAKNESS_TIMEOUT_GAP => {
            is_timeout_outcome(outcome) || summary_has_timeout_token(summary, skill_name)
        }
        k if k == WEAKNESS_RELIABILITY_GAP => true,
        _ => false,
    }
}

/// Whether a weakness-kind string names the v1 taxonomy.
pub(crate) fn is_known_weakness_kind(kind: &str) -> bool {
    matches!(
        kind,
        k if k == WEAKNESS_VERIFICATION_GAP
            || k == WEAKNESS_TIMEOUT_GAP
            || k == WEAKNESS_RELIABILITY_GAP
    )
}

// ─── V2 generation ──────────────────────────────────────────────────────

/// URL-safe slug for namespaces: lowercase alphanumerics, single hyphens.
fn slugify(s: &str) -> String {
    let mut slug = String::new();
    for ch in s.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    slug
}

/// Deterministic learning-candidate id for an evolution hypothesis. The
/// `p13-skill-evolution` domain separator keeps these ids disjoint from P3
/// topic-pair ids and P11 reuse ids (same table, different identity space).
fn mint_evolution_learning_id(
    workspace_root: &str,
    skill_id: &str,
    source_version: u32,
    weakness_kind: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        format!("p13-skill-evolution|project|{workspace_root}||failure_pattern|{skill_id}|v{source_version}|{weakness_kind}")
            .as_bytes(),
    );
    format!("lc::{:016x}", {
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    })
}

/// Generate the deterministic proposed Skill v2: the complete v1 content,
/// preserved verbatim, plus one appended evidence-backed hardening section.
///
/// Stable by construction: a function of (v1 content, weakness kind,
/// source version) only — never volatile counts, confidence snapshots, or
/// event ids — so re-detection converges on one lineage instead of minting
/// a candidate per run. (Volatile counts live in the candidate's
/// description/purpose/evidence columns, which re-proposals refresh
/// idempotently without changing identity.)
pub fn evolution_skill_content(
    v1_content: &str,
    source_version: u32,
    weakness: &EvolutionWeakness,
) -> String {
    let dst = source_version + 1;
    let section = match weakness.kind.as_str() {
        WEAKNESS_VERIFICATION_GAP => "### Mandatory read-back verification\n\n\
             1. After every apply, read back the changed file(s) and confirm the edit landed as intended.\n\
             2. Run the relevant verification (tests/build) and confirm it passes before claiming done.\n\
             3. If verification fails, do not claim success — report the failure as task evidence.\n\n\
             ### Constraints\n\n\
             - Never claim completion without passing verification.\n\
             - A skipped verification step is a workflow failure, not a shortcut.\n"
            .to_string(),
        WEAKNESS_TIMEOUT_GAP => "### Timeout handling\n\n\
             1. Bound every long-running step with an explicit deadline before starting it.\n\
             2. On timeout, retry once; if the retry also times out, stop and report partial progress.\n\
             3. Never hang silently waiting for a step that already exceeded its deadline.\n\n\
             ### Constraints\n\n\
             - Every long-running step needs a deadline stated up front.\n\
             - Partial progress is reported as task evidence, never discarded.\n"
            .to_string(),
        _ => "### Failure-hardening checklist\n\n\
             1. Validate inputs before acting; refuse to proceed on invalid input.\n\
             2. Verify after acting; confirm the outcome before claiming done.\n\
             3. On failure, report what failed as task evidence instead of claiming success.\n\n\
             ### Constraints\n\n\
             - No v1 procedure step is changed by this section; it only adds checks around it.\n\
             - A failed check blocks the success claim for that execution.\n"
            .to_string(),
    };
    let base = v1_content.trim_end().to_string();
    format!(
        "{base}\n\n## Evolution hardening (v{source_version} → v{dst}, evidence-backed)\n\n\
         > Detected weakness in v{source_version} (`{kind}`): {label}.\n\
         >\n\
         > {change}\n\
         >\n\
         > Rationale: {rationale}\n\
         >\n\
         > Full event citations live in the skill registry, not in this file.\n\n\
         {section}",
        kind = weakness.kind,
        label = weakness.label,
        change = weakness.proposed_change,
        rationale = weakness.rationale,
    )
}

/// Suggested human question for the evolution approval (OpenCode renders it
/// through the existing approval protocol; CodeBro never asks directly).
pub fn evolution_approval_question(skill_name: &str, source_version: u32) -> String {
    format!(
        "Skill '{skill_name}' v{source_version} has recurring failures. Create proposed v{}?",
        source_version + 1
    )
}

// ─── Detector ───────────────────────────────────────────────────────────

impl ContextStore {
    /// P13 entry point: detect recurring weaknesses in workspace-visible
    /// active skills and route qualifying weaknesses through the existing
    /// learning → skill candidate → validate pipeline. Explicitly invoked
    /// (no background work); read-only when nothing qualifies.
    ///
    /// `skill_selector` optionally names one skill (id or name); when
    /// absent, every workspace-visible active project/global skill is
    /// examined (task-scoped skills are skipped in v1 — see the module
    /// docs). `task_id` is accepted for future task-scoped support and
    /// currently unused.
    pub fn detect_skill_evolution(
        &self,
        workspace_root: &str,
        skill_selector: Option<&str>,
        _task_id: Option<&str>,
        now: u64,
    ) -> Result<EvolutionReport, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        if ws.is_empty() {
            return Err(ContextError::Validation(
                "evolution detection requires a workspace_root".to_string(),
            ));
        }
        let selector = skill_selector
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let targets: Vec<Skill> = match selector.as_deref() {
            Some(sel) => {
                let found = self
                    .get_skill(sel)?
                    .or(self.get_skill_by_name(sel)?.filter(|s| {
                        s.scope == "global"
                            || s.workspace_root.as_deref().map(canonical_workspace_key)
                                == Some(ws.clone())
                    }));
                match found {
                    Some(s) => vec![s],
                    None => {
                        return Err(ContextError::Validation(format!(
                            "skill not found (by id or name, visible from this workspace): {sel}"
                        )));
                    }
                }
            }
            None => self
                .list_skills(Some(&ws), None, 100)?
                .into_iter()
                .filter(|s| s.status == "active")
                .filter(|s| matches!(s.scope.as_str(), "project" | "global"))
                .collect(),
        };

        let mut candidates = Vec::new();
        for skill in &targets {
            let view = self.evolve_one_skill(skill, &ws, now)?;
            candidates.push(view);
        }
        // Worst-health-first (most failures), then name — deterministic.
        candidates.sort_by(|a, b| {
            b.observations
                .cmp(&a.observations)
                .then_with(|| a.source_skill.cmp(&b.source_skill))
        });
        let skills_considered = targets.len();
        let truncated = candidates.len() > EVOLUTION_MAX_VIEWS;
        candidates.truncate(EVOLUTION_MAX_VIEWS);

        let created = candidates
            .iter()
            .filter(|c| c.outcome == "created_validated")
            .count();
        let duplicates = candidates
            .iter()
            .filter(|c| {
                matches!(
                    c.outcome.as_str(),
                    "duplicate_candidate_in_review"
                        | "skipped_terminal"
                        | "lineage_advanced"
                        | "no_change"
                )
            })
            .count();
        let learning_only = candidates
            .iter()
            .filter(|c| c.outcome.starts_with("learning_only"))
            .count();
        let (status, next_action) = if created > 0 {
            (
                STATUS_CANDIDATE_FOUND.to_string(),
                "Request human approval: call skill request_approval for each \
                 validated evolution candidate (pass its suggested_question as \
                 question so the human sees the v1 → v2 framing), present its \
                 interaction question to the human with the approve/reject/ \
                 modify/defer options, then call skill respond with the human \
                 answer. Never publish without approval."
                    .to_string(),
            )
        } else if !candidates.is_empty() && duplicates == candidates.len() {
            (
                STATUS_ALREADY_EXISTS.to_string(),
                "Nothing new to approve: an equivalent evolution candidate is \
                 already in review, the lineage already advanced, or the terminal \
                 verdict stands. Use skill applicable + skill_context for \
                 contextual reuse of the current version."
                    .to_string(),
            )
        } else if learning_only > 0 {
            (
                STATUS_LEARNING_ONLY.to_string(),
                "Weaknesses were accepted as learning but carry too little \
                 confidence to propose yet. Accumulate more linked failure \
                 evidence, then re-run detect_evolution."
                    .to_string(),
            )
        } else {
            (
                STATUS_NO_CANDIDATES.to_string(),
                "No recurring skill weakness found in this workspace. An \
                 evolution candidate needs at least 3 skill-linked failure \
                 executions of the same active version with a recurring signal \
                 and no strong contradiction."
                    .to_string(),
            )
        };
        if truncated {
            candidates.push(EvolutionCandidateView {
                source_skill: "…".to_string(),
                source_skill_id: "…".to_string(),
                source_version: 0,
                proposed_version: 0,
                weakness_kind: "none".to_string(),
                weakness: None,
                observations: 0,
                supporting_events: 0,
                contradicting_events: 0,
                confidence: 0.0,
                scope: "project".to_string(),
                learning_candidate_id: None,
                learning_status: None,
                skill_candidate_id: None,
                skill_status: None,
                outcome: "truncated".to_string(),
                next_action: "Re-run detect_evolution with skill_selector naming one skill."
                    .to_string(),
                evidence_sample: Vec::new(),
                source_content_hash: None,
                proposed_content_hash: None,
                source_success_count: None,
                source_failure_count: None,
                suggested_question: None,
            });
        }
        Ok(EvolutionReport {
            status,
            workspace_root: ws,
            skill_selector: selector,
            skills_considered,
            candidates,
            next_action,
            note: "P13 detector: explicitly invoked, deterministic, evidence-backed. \
                   Detector observations become P3 learning first; only accepted \
                   learning seeds evolution candidates; only validated candidates \
                   with explicit human approval publish. No automatic publish, no \
                   approval bypass, no background evolution."
                .to_string(),
        })
    }

    /// Examine one active skill for a recurring weakness. Soft gates return
    /// a reason view (never an error); only structural failures (missing
    /// version rows, DB errors) are `Err`.
    fn evolve_one_skill(
        &self,
        skill: &Skill,
        workspace_root: &str,
        now: u64,
    ) -> Result<EvolutionCandidateView, ContextError> {
        let base_view = |outcome: &str, next_action: &str| EvolutionCandidateView {
            source_skill: skill.name.clone(),
            source_skill_id: skill.skill_id.clone(),
            source_version: skill.current_version,
            proposed_version: skill.current_version + 1,
            weakness_kind: "none".to_string(),
            weakness: None,
            observations: 0,
            supporting_events: 0,
            contradicting_events: 0,
            confidence: 0.0,
            scope: skill.scope.clone(),
            learning_candidate_id: None,
            learning_status: None,
            skill_candidate_id: None,
            skill_status: None,
            outcome: outcome.to_string(),
            next_action: next_action.to_string(),
            evidence_sample: Vec::new(),
            source_content_hash: None,
            proposed_content_hash: None,
            source_success_count: Some(skill.health.success_count),
            source_failure_count: Some(skill.health.failure_count),
            suggested_question: None,
        };

        // Gate 0: only active skills evolve; only project/global scopes in v1.
        if skill.status != "active" {
            return Ok(base_view(
                "not_active",
                "Only active skills evolve: deprecated/superseded lineages never execute.",
            ));
        }
        if !matches!(skill.scope.as_str(), "project" | "global") {
            return Ok(base_view(
                "unsupported_scope",
                "P13 v1 evolves project/global skills; task-scoped evolution is not supported yet.",
            ));
        }
        // Workspace confinement for project skills (defense in depth: the
        // list query already scopes, the selector path needs this check).
        if skill.scope == "project" {
            let owns = skill.workspace_root.as_deref().map(canonical_workspace_key)
                == Some(workspace_root.to_string());
            if !owns {
                return Ok(base_view(
                    "not_active",
                    "Project skill belongs to another workspace — does not leak across workspaces.",
                ));
            }
        }

        // The active version anchors both the evidence window and the
        // optimistic-concurrency lineage (`based_on_version`).
        let active_version = self.get_active_skill_version(&skill.skill_id)?;
        let active_version = match active_version {
            Some(v) => v,
            None => {
                return Ok(base_view(
                    "no_active_version",
                    "No recorded active version: restore the version history before evolving.",
                ));
            }
        };

        // Skill-linked executions, scoped to the current version window.
        let linked = self.with_conn(|conn| fetch_linked_events(conn, workspace_root, skill))?;
        // Another skill's name in the report means the execution is evidence
        // about that skill, not this one — exclude before counting.
        let peer_names: Vec<String> = self
            .list_skills(Some(workspace_root), None, 100)?
            .into_iter()
            .filter(|s| s.skill_id != skill.skill_id && s.status == "active")
            .map(|s| s.name.to_ascii_lowercase())
            .collect();
        let mut supporting: Vec<&LinkedEvent> = Vec::new();
        let mut contradicting: Vec<&LinkedEvent> = Vec::new();
        for e in linked
            .iter()
            .filter(|e| e.created_at > active_version.created_at)
        {
            let haystack = format!("{}\n{}", e.summary, e.payload.as_deref().unwrap_or(""))
                .to_ascii_lowercase();
            if peer_names
                .iter()
                .any(|n| !n.is_empty() && haystack.contains(n))
            {
                continue;
            }
            match crate::learning::outcome_polarity(e.outcome.as_deref()) {
                crate::learning::OutcomePolarity::Failure => supporting.push(e),
                crate::learning::OutcomePolarity::Success => contradicting.push(e),
                crate::learning::OutcomePolarity::Neutral => {}
            }
        }
        let s = supporting.len();
        let c = contradicting.len();

        // Gate 1: at least three linked failures (single/double failures
        // are incidents, never evolution material).
        if s < EVOLUTION_MIN_FAILURES {
            return Ok(base_view(
                "insufficient_evidence",
                &format!(
                    "Only {s} skill-linked failure(s) in the v{} window (need at least \
                     {EVOLUTION_MIN_FAILURES}): a single failure is an incident, not a \
                     recurring weakness. Record more executions via skill health, then re-run.",
                    skill.current_version
                ),
            ));
        }
        // Gate 2: contradiction (mirrors the P3 contested/majority rules on
        // the linked sets: successes that keep up with failures veto).
        if c > s {
            let mut v = base_view(
                "contradicted",
                "Linked successes outweigh linked failures: the evidence weighs against a \
                 recurring weakness. No candidate.",
            );
            v.observations = s;
            v.supporting_events = s;
            v.contradicting_events = c;
            return Ok(v);
        }
        if c * 2 >= s {
            let mut v = base_view(
                "contested",
                "Linked successes contest the failures (contradictions reach half of \
                 support): preserved as evidence, not surfaced as a candidate.",
            );
            v.observations = s;
            v.supporting_events = s;
            v.contradicting_events = c;
            return Ok(v);
        }
        // Gate 3: health veto (the skill's own counters, when assessable).
        // Health can only veto, never create: unassessed skills rely on
        // history alone.
        if skill.health.is_assessable() && !skill.health.is_degraded() {
            let mut v = base_view(
                "healthy_no_evolution",
                "The skill's own usage counters report healthy despite linked failures: \
                 successes dominate overall, so no evolution is proposed.",
            );
            v.observations = s;
            v.supporting_events = s;
            v.contradicting_events = c;
            return Ok(v);
        }

        // Gate 4: recurring signal (shared outcome label or shared token in
        // ≥2 failures). Unrelated failures that merely share the skill name
        // do not pass.
        let (labels, tokens) = recurring_signal(&supporting, &skill.name);
        if labels.is_empty() && tokens.is_empty() {
            let mut v = base_view(
                "no_recurring_signal",
                "Linked failures share neither an outcome label nor a meaningful token: \
                 they read as unrelated incidents, not one recurring weakness. No candidate.",
            );
            v.observations = s;
            v.supporting_events = s;
            v.contradicting_events = c;
            return Ok(v);
        }
        let weakness = classify_weakness(&skill.name, skill.current_version, &labels, &tokens, s);

        // Phase 4: route through the existing P3 learning pipeline (project
        // scope = the evidence scope; the inference stays workspace-scoped
        // even for global skills, which keeps the acceptance bar honest).
        let mut supporting_ids: Vec<i64> = supporting.iter().map(|e| e.id).collect();
        let mut contradicting_ids: Vec<i64> = contradicting.iter().map(|e| e.id).collect();
        supporting_ids.sort_unstable();
        supporting_ids.dedup();
        contradicting_ids.sort_unstable();
        contradicting_ids.dedup();
        supporting_ids.truncate(EVOLUTION_MAX_EVIDENCE_IDS);
        contradicting_ids.truncate(EVOLUTION_MAX_EVIDENCE_IDS);
        let learning_id = mint_evolution_learning_id(
            workspace_root,
            &skill.skill_id,
            skill.current_version,
            &weakness.kind,
        );
        let slug = slugify(&skill.name);
        let namespace: String = format!("learn.failure-pattern.evolution-{slug}-{}", weakness.kind)
            .chars()
            .take(200)
            .collect();
        let proposition = format!(
            "The active skill '{}' v{} has repeatedly failed in this project \
             ({} linked failures, {} linked successes considered; recurring signal: {}): \
             {}",
            skill.name,
            skill.current_version,
            supporting_ids.len(),
            contradicting_ids.len(),
            if labels.is_empty() {
                tokens.join(", ")
            } else {
                labels.join(", ")
            },
            weakness.description,
        );
        let learning = LearningCandidate {
            candidate_id: learning_id.clone(),
            workspace_root: Some(workspace_root.to_string()),
            task_id: None,
            scope: LearnScope::Project.to_string(),
            kind: CandidateKind::FailurePattern.as_str().to_string(),
            proposition,
            namespace,
            supporting_evidence: supporting_ids.clone(),
            contradicting_evidence: contradicting_ids.clone(),
            confidence: 0.65,
            status: CandidateStatus::Candidate.as_str().to_string(),
            created_at: now,
            updated_at: now,
            expires_at: Some(now.saturating_add(CANDIDATE_TTL_SECS)),
            eval_reason: Some(
                "proposed by the P13 skill-evolution detector from recurring skill-linked \
                 failures; not yet evaluated"
                    .to_string(),
            ),
            inference_record_id: None,
        };
        self.upsert_candidate(&learning)?;
        let stored = self.get_candidate(&learning_id)?.ok_or_else(|| {
            ContextError::Decode("evolution learning candidate vanished after upsert".to_string())
        })?;
        let evidence_sample: Vec<i64> = supporting_ids
            .iter()
            .copied()
            .take(EVOLUTION_MAX_VIEW_IDS)
            .collect();
        let learning_view = |outcome: &str, next_action: &str, lc: &LearningCandidate| {
            let mut v = base_view(outcome, next_action);
            v.weakness_kind = weakness.kind.clone();
            v.weakness = Some(weakness.clone());
            v.observations = s;
            v.supporting_events = supporting_ids.len();
            v.contradicting_events = contradicting_ids.len();
            v.confidence = lc.confidence;
            v.learning_candidate_id = Some(lc.candidate_id.clone());
            v.learning_status = Some(lc.status.clone());
            v.evidence_sample = evidence_sample.clone();
            v.source_content_hash = Some(active_version.content_hash.clone());
            v
        };
        match stored.status.as_str() {
            "rejected" | "superseded" | "expired" => {
                return Ok(learning_view(
                    "skipped_terminal",
                    "A prior verdict (rejected/superseded/expired) on this exact weakness \
                     stands; new evidence must arrive as a new signal.",
                    &stored,
                ));
            }
            _ => {}
        }
        let evaluated = match self.evaluate_candidate(&learning_id, now) {
            Ok(done) => done,
            Err(e) if e.to_string().contains("never rewrites this verdict") => {
                return Ok(learning_view(
                    "skipped_terminal",
                    "This weakness carries a terminal verdict; re-detection does not rewrite it.",
                    &stored,
                ));
            }
            Err(e) => {
                return Ok(learning_view(
                    "learning_failed",
                    &format!("Learning evaluation errored ({e}); fix history scope and retry."),
                    &stored,
                ));
            }
        };
        if evaluated.status != CandidateStatus::Accepted.as_str() {
            return Ok(learning_view(
                &format!("learning_only_{}", evaluated.status),
                "Accepted learning required before any evolution proposal: this weakness \
                 stays a hypothesis. Accumulate more linked failures, then re-run \
                 detect_evolution.",
                &evaluated,
            ));
        }
        if evaluated.confidence < SKILL_APPROVAL_MIN_CONFIDENCE {
            return Ok(learning_view(
                "learning_only_weak_confidence",
                "Learning accepted but below the skill approval floor: no candidate \
                 minted. Accumulate more linked failures, then re-run detect_evolution.",
                &evaluated,
            ));
        }

        // Phase 5: deterministic V2 (complete successor, stable content).
        let v2_content =
            evolution_skill_content(&active_version.content, skill.current_version, &weakness);
        if content_hash(&v2_content) == active_version.content_hash {
            let mut v = learning_view(
                "no_change",
                "The generated successor is identical to the active version: nothing to propose.",
                &evaluated,
            );
            v.proposed_content_hash = Some(content_hash(&v2_content));
            return Ok(v);
        }
        let scope = SkillScope::from_str(&skill.scope).map_err(ContextError::Validation)?;
        let candidate_id = mint_skill_candidate_id(
            &scope,
            skill.workspace_root.as_deref(),
            None,
            &skill.name,
            &v2_content,
        );

        // Duplicate / lineage guards before inserting (deterministic ids
        // make re-detection converge; anchors refuse stale proposals).
        let fresh_skill = self.get_skill(&skill.skill_id)?;
        match fresh_skill {
            Some(current) if current.current_version != skill.current_version => {
                let mut v = learning_view(
                    "lineage_advanced",
                    "The skill lineage advanced since this detection read it: re-run \
                     detect_evolution against the current version.",
                    &evaluated,
                );
                v.skill_status = Some(current.status.clone());
                v.proposed_content_hash = Some(content_hash(&v2_content));
                return Ok(v);
            }
            None => {
                return Ok(learning_view(
                    "not_active",
                    "The source skill vanished during detection; nothing to evolve.",
                    &evaluated,
                ));
            }
            _ => {}
        }
        if let Some(existing) = self.get_skill_candidate(&candidate_id)? {
            let terminal = matches!(
                existing.status.as_str(),
                "rejected" | "superseded" | "expired"
            );
            let mut v = learning_view(
                if terminal {
                    "skipped_terminal"
                } else {
                    "duplicate_candidate_in_review"
                },
                "An equivalent evolution proposal already exists: validate it and request \
                 approval instead of proposing again (re-detection converges, never duplicates).",
                &evaluated,
            );
            v.skill_candidate_id = Some(existing.candidate_id.clone());
            v.skill_status = Some(existing.status.clone());
            v.proposed_content_hash = Some(content_hash(&v2_content));
            return Ok(v);
        }

        let description = format!(
            "Evolution of '{}' v{} → v{}: {} (evidence-backed, confidence {:.2}).",
            skill.name,
            skill.current_version,
            skill.current_version + 1,
            weakness.label,
            evaluated.confidence
        );
        let description: String = description.chars().take(900).collect();
        let purpose = format!(
            "1. What is wrong with v{}? {}. \
             2. What evidence supports this? {} skill-linked failures (events {}) with {} \
             linked successes considered, all newer than the v{} activation; learning \
             candidate {} (accepted, confidence {:.2}). \
             3. What should v{} change? {}. \
             4. Why should that change address the weakness? {}. \
             5. What evidence contradicts the hypothesis? {} linked success(es); contested \
             or outweighing successes block the proposal, and the skill health snapshot \
             ({} successes, {} failures) is carried for comparison.",
            skill.current_version,
            weakness.description,
            supporting_ids.len(),
            evidence_sample
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            contradicting_ids.len(),
            skill.current_version,
            evaluated.candidate_id,
            evaluated.confidence,
            skill.current_version + 1,
            weakness.proposed_change,
            weakness.rationale,
            contradicting_ids.len(),
            skill.health.success_count,
            skill.health.failure_count,
        );
        let candidate = SkillCandidate {
            candidate_id: candidate_id.clone(),
            workspace_root: skill.workspace_root.clone(),
            task_id: None,
            scope: skill.scope.clone(),
            name: skill.name.clone(),
            description,
            purpose,
            applicability: skill.applicability.clone(),
            source_learning_candidates: vec![evaluated.candidate_id.clone()],
            supporting_evidence: supporting_ids.clone(),
            contradicting_evidence: contradicting_ids.clone(),
            proposed_content: v2_content.clone(),
            status: SkillCandidateStatus::Candidate.as_str().to_string(),
            confidence: evaluated.confidence,
            validation: None,
            eval_reason: Some(format!(
                "proposed by P13 skill-evolution from recurring '{}' weakness \
                 (learning candidate {})",
                weakness.kind, evaluated.candidate_id
            )),
            rejection_reason: None,
            supersedes_skill: Some(skill.skill_id.clone()),
            based_on_version: Some(skill.current_version),
            created_at: now,
            updated_at: now,
            expires_at: Some(now + SKILL_CANDIDATE_TTL_SECS),
        };
        if let Err(e) = self.insert_skill_candidate(&candidate) {
            let msg = e.to_string();
            let outcome = if msg.contains("already exists") {
                "duplicate_candidate_in_review"
            } else {
                "skill_proposal_failed"
            };
            let mut v = learning_view(
                outcome,
                &format!(
                    "Skill proposal was refused ({msg}); inspect the learning candidate and \
                     propose manually if warranted."
                ),
                &evaluated,
            );
            v.proposed_content_hash = Some(content_hash(&v2_content));
            return Ok(v);
        }
        let validated = match self.evaluate_candidate_content(&candidate_id, now) {
            Ok(done) => done,
            Err(e) => {
                let mut v = learning_view(
                    "validation_failed",
                    &format!(
                        "Automated validation errored ({e}); inspect the candidate and fix its \
                         content before requesting approval."
                    ),
                    &evaluated,
                );
                v.skill_candidate_id = Some(candidate_id.clone());
                v.proposed_content_hash = Some(content_hash(&v2_content));
                return Ok(v);
            }
        };
        if validated.status != SkillCandidateStatus::Validated.as_str() {
            let mut v = learning_view(
                "validation_failed",
                "Automated validation did not pass: evolution never bypasses validation. \
                 Fix the candidate content, then validate again.",
                &evaluated,
            );
            v.skill_candidate_id = Some(validated.candidate_id.clone());
            v.skill_status = Some(validated.status.clone());
            v.proposed_content_hash = Some(content_hash(&v2_content));
            return Ok(v);
        }
        let mut v = learning_view(
            "created_validated",
            "Validated and ready for human approval: call skill request_approval with this \
             candidate_id (pass its suggested_question as question), present the interaction \
             to the human, then call skill respond with the human answer.",
            &evaluated,
        );
        v.skill_candidate_id = Some(validated.candidate_id.clone());
        v.skill_status = Some(validated.status.clone());
        v.proposed_content_hash = Some(content_hash(&v2_content));
        v.suggested_question = Some(evolution_approval_question(
            &skill.name,
            skill.current_version,
        ));
        Ok(v)
    }
}

// ─── Evidence comparison ────────────────────────────────────────────────

/// Simple version-window evidence comparison for one skill: linked
/// failures/successes before vs since a version activation. No statistics
/// are claimed — the counts are exposed so the human can see them.
///
/// Returns `(before_failures, before_successes, since_failures,
/// since_successes)` for the named skill in this workspace.
pub fn evolution_evidence_comparison(
    store: &ContextStore,
    workspace_root: &str,
    skill: &Skill,
    since_created_at: u64,
) -> Result<(usize, usize, usize, usize), ContextError> {
    let ws = canonical_workspace_key(workspace_root);
    store.with_conn(|conn| {
        let linked = fetch_linked_events(conn, &ws, skill)?;
        let mut before_f = 0;
        let mut before_s = 0;
        let mut since_f = 0;
        let mut since_s = 0;
        for e in &linked {
            let failure = crate::learning::outcome_polarity(e.outcome.as_deref())
                == crate::learning::OutcomePolarity::Failure;
            let success = crate::learning::outcome_polarity(e.outcome.as_deref())
                == crate::learning::OutcomePolarity::Success;
            if e.created_at > since_created_at {
                if failure {
                    since_f += 1;
                } else if success {
                    since_s += 1;
                }
            } else {
                if failure {
                    before_f += 1;
                } else if success {
                    before_s += 1;
                }
            }
        }
        Ok((before_f, before_s, since_f, since_s))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, STATE_DB_FILE};
    use crate::history::{HistoryInput, HistoryKind, OpenSession};
    use crate::skills::SkillHealth;

    const WS: &str = "/repo";
    const NOW: u64 = 1_700_000_000;

    fn test_store(dir: &std::path::Path) -> ContextStore {
        let db_path = dir.join(STATE_DB_FILE);
        db::open_checked(&db_path).unwrap();
        ContextStore::new(db_path)
    }

    fn v1_content(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: A test skill for {name}\n---\n\n\
             # Purpose\n\nFollow inspect, modify, verify.\n\n\
             # Procedure\n\n1. Inspect.\n2. Modify.\n3. Verify.\n"
        )
    }

    /// Publish an active v1 skill through the real lifecycle (candidate →
    /// validated → approved), so version rows, file guards, and lineage all
    /// behave like production.
    fn publish_v1(
        store: &ContextStore,
        skills_dir: &std::path::Path,
        name: &str,
        ws: &str,
        at: u64,
    ) -> Skill {
        let content = v1_content(name);
        let scope = SkillScope::Project;
        let candidate_id = mint_skill_candidate_id(&scope, Some(ws), None, name, &content);
        let candidate = SkillCandidate {
            candidate_id: candidate_id.clone(),
            workspace_root: Some(ws.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: name.to_string(),
            description: format!("Skill {name}"),
            purpose: "Testing evolution".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: Vec::new(),
            supporting_evidence: vec![1, 2, 3],
            contradicting_evidence: Vec::new(),
            proposed_content: content.clone(),
            status: SkillCandidateStatus::Candidate.as_str().to_string(),
            confidence: 0.75,
            validation: None,
            eval_reason: None,
            rejection_reason: None,
            supersedes_skill: None,
            based_on_version: None,
            created_at: at,
            updated_at: at,
            expires_at: None,
        };
        store.insert_skill_candidate(&candidate).unwrap();
        store
            .evaluate_candidate_content(&candidate_id, at + 1)
            .unwrap();
        let (skill, _) = store
            .approve_skill_candidate(&candidate_id, Some(ws), skills_dir, at + 2)
            .unwrap();
        assert_eq!(skill.current_version, 1);
        skill
    }

    /// One skill-linked execution failure (tool=skill, summary names the
    /// skill), each in its own session for evidence diversity.
    fn seed_linked_failure(
        store: &ContextStore,
        ws: &str,
        skill_name: &str,
        outcome: &str,
        summary: &str,
        at: u64,
    ) -> i64 {
        let session = store.open_session(ws, &OpenSession::default(), at).unwrap();
        let mut input = HistoryInput::new(ws, HistoryKind::ToolExecution, summary);
        input.session_id = Some(session.id.clone());
        input.tool = Some("skill".to_string());
        input.outcome = Some(outcome.to_string());
        input.payload = Some(format!(
            "{{\"authority\":\"observed\",\"skill_name\":\"{skill_name}\"}}"
        ));
        input.created_at = Some(at);
        let (id, dup) = store.record_history(&input, at).unwrap();
        assert!(!dup);
        id
    }

    fn seed_linked_success(store: &ContextStore, ws: &str, skill_name: &str, at: u64) -> i64 {
        let session = store.open_session(ws, &OpenSession::default(), at).unwrap();
        let mut input = HistoryInput::new(
            ws,
            HistoryKind::ToolExecution,
            format!("skill '{skill_name}' execution recorded as success (v1)"),
        );
        input.session_id = Some(session.id.clone());
        input.tool = Some("skill".to_string());
        input.outcome = Some("success".to_string());
        input.payload = Some(format!(
            "{{\"authority\":\"observed\",\"skill_name\":\"{skill_name}\"}}"
        ));
        input.created_at = Some(at);
        let (id, dup) = store.record_history(&input, at).unwrap();
        assert!(!dup);
        id
    }

    fn detect(store: &ContextStore, ws: &str, sel: Option<&str>, now: u64) -> EvolutionReport {
        store
            .detect_skill_evolution(ws, sel, None, now)
            .expect("detection must not hard-error")
    }

    fn skills_dir(dir: &std::path::Path) -> std::path::PathBuf {
        let p = dir.join("skills");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    const VERIFY_SUMMARY: &str =
        "skill 'verify-demo' execution recorded as failure (v1): sandbox verify test run failed, checks not confirmed";

    #[test]
    fn detects_recurring_verification_weakness_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        let report = detect(&store, WS, None, NOW);
        assert_eq!(report.status, STATUS_CANDIDATE_FOUND, "{report:?}");
        assert_eq!(report.candidates.len(), 1, "{report:?}");
        let c = &report.candidates[0];
        assert_eq!(c.outcome, "created_validated", "{c:?}");
        assert_eq!(c.weakness_kind, WEAKNESS_VERIFICATION_GAP, "{c:?}");
        assert_eq!(c.source_version, 1);
        assert_eq!(c.proposed_version, 2);
        assert_eq!(c.observations, 3);
        assert_eq!(c.supporting_events, 3);
        assert_eq!(c.contradicting_events, 0);
        assert!(
            c.confidence >= SKILL_APPROVAL_MIN_CONFIDENCE,
            "{}",
            c.confidence
        );
        assert_eq!(c.evidence_sample.len(), 3);
        assert!(c
            .suggested_question
            .as_deref()
            .unwrap()
            .contains("verify-demo"));
        let _ = skill;

        // The evolution candidate anchors the v1 lineage.
        let sc_id = c.skill_candidate_id.as_deref().unwrap();
        let sc = store.get_skill_candidate(sc_id).unwrap().unwrap();
        assert_eq!(sc.name, "verify-demo");
        assert_eq!(
            sc.supersedes_skill.as_deref(),
            Some(c.source_skill_id.as_str())
        );
        assert_eq!(sc.based_on_version, Some(1));
        assert_eq!(sc.status, "validated");
        assert_eq!(sc.supporting_evidence.len(), 3);
        assert!(!sc.source_learning_candidates.is_empty());
        // Learning provenance: accepted failure-pattern inference.
        let lc = store
            .get_candidate(sc.source_learning_candidates[0].as_str())
            .unwrap()
            .unwrap();
        assert_eq!(lc.status, "accepted");
        assert_eq!(lc.kind, "failure_pattern");
        // V2 is a complete successor: v1 preserved plus hardening.
        let v1 = store
            .get_active_skill_version(&c.source_skill_id)
            .unwrap()
            .unwrap();
        assert!(sc.proposed_content.starts_with(v1.content.trim_end()));
        assert!(sc.proposed_content.contains("Evolution hardening (v1 → v2"));
        assert!(sc
            .proposed_content
            .contains("Mandatory read-back verification"));
        assert!(
            crate::skills::validate_skill_content(&sc.proposed_content, "verify-demo", Some(WS))
                .valid
        );
        // Purpose answers the five evolution questions.
        assert!(sc.purpose.contains("What is wrong"));
        assert!(sc.purpose.contains("What evidence"));
        assert!(sc.purpose.contains("contradicts"));
    }

    #[test]
    fn single_failure_is_an_incident_not_a_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        seed_linked_failure(
            &store,
            WS,
            "verify-demo",
            "test_failure",
            VERIFY_SUMMARY,
            NOW - 10,
        );
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(report.candidates[0].outcome, "insufficient_evidence");
    }

    #[test]
    fn two_failures_are_below_threshold() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..2 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 20 + i,
            );
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(report.candidates[0].outcome, "insufficient_evidence");
    }

    #[test]
    fn unrelated_failures_never_count() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        // Three failures with other tools and no skill linkage.
        for i in 0..3 {
            let session = store
                .open_session(WS, &OpenSession::default(), NOW - 30 + i)
                .unwrap();
            let mut input =
                HistoryInput::new(WS, HistoryKind::Validation, "sandbox checks failed badly");
            input.session_id = Some(session.id.clone());
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("test_failure".to_string());
            input.created_at = Some(NOW - 30 + i);
            store.record_history(&input, NOW - 30 + i).unwrap();
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(report.candidates[0].outcome, "insufficient_evidence");
        assert_eq!(report.candidates[0].observations, 0);
    }

    #[test]
    fn other_workspaces_never_leak_in() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", "/repo-a", NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                "/repo-a",
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        let foreign = detect(&store, "/repo-b", None, NOW);
        assert_eq!(foreign.status, STATUS_NO_CANDIDATES, "{foreign:?}");
        assert!(foreign.candidates.is_empty(), "{foreign:?}");
        let home = detect(&store, "/repo-a", None, NOW);
        assert_eq!(home.status, STATUS_CANDIDATE_FOUND, "{home:?}");
    }

    #[test]
    fn weak_confidence_stays_learning_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        // 3 failures + 1 success: not contested (1*2 < 3) but confidence dips
        // below the skill floor while staying above the learning floor.
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        seed_linked_success(&store, WS, "verify-demo", NOW - 90);
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_LEARNING_ONLY, "{report:?}");
        assert_eq!(
            report.candidates[0].outcome, "learning_only_weak_confidence",
            "{report:?}"
        );
        assert!(store
            .list_skills(Some(WS), None, 100)
            .unwrap()
            .iter()
            .all(|s| s.current_version == 1));
    }

    #[test]
    fn strong_contradiction_blocks_the_candidate() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        // More linked successes than failures: the evidence weighs against
        // a recurring weakness (P3 majority rule).
        for i in 0..4 {
            seed_linked_success(&store, WS, "verify-demo", NOW - 50 + i);
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(report.candidates[0].outcome, "contradicted", "{report:?}");
    }

    #[test]
    fn contested_evidence_is_preserved_not_surfaced() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        for i in 0..2 {
            seed_linked_success(&store, WS, "verify-demo", NOW - 50 + i);
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(report.candidates[0].outcome, "contested", "{report:?}");
    }

    #[test]
    fn healthy_skill_is_never_evolved() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        // Many recorded successes dilute the failures: the skill's own
        // counters say healthy, so the detector stands down.
        for _ in 0..10 {
            store.record_skill_use(&skill.skill_id, true, NOW).unwrap();
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(
            report.candidates[0].outcome, "healthy_no_evolution",
            "{report:?}"
        );
    }

    #[test]
    fn failures_without_a_shared_signal_are_unrelated_incidents() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        // Distinct outcome labels, disjoint vocabularies: no recurrence.
        seed_linked_failure(
            &store,
            WS,
            "verify-demo",
            "failure",
            "skill 'verify-demo' execution recorded as failure (v1): zebra quarry xenon",
            NOW - 100,
        );
        seed_linked_failure(
            &store,
            WS,
            "verify-demo",
            "error",
            "skill 'verify-demo' execution recorded as failure (v1): quartz jigsaw pixel",
            NOW - 99,
        );
        seed_linked_failure(
            &store,
            WS,
            "verify-demo",
            "abandoned",
            "skill 'verify-demo' execution recorded as failure (v1): mammoth vulcan orbit",
            NOW - 98,
        );
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert_eq!(
            report.candidates[0].outcome, "no_recurring_signal",
            "{report:?}"
        );
    }

    #[test]
    fn timeout_weakness_classifies_deterministically() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "timeout",
                "skill 'verify-demo' execution recorded as failure (v1): step exceeded deadline and hung past timeout",
                NOW - 100 + i,
            );
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_CANDIDATE_FOUND, "{report:?}");
        let c = &report.candidates[0];
        assert_eq!(c.weakness_kind, WEAKNESS_TIMEOUT_GAP, "{c:?}");
        let sc = store
            .get_skill_candidate(c.skill_candidate_id.as_deref().unwrap())
            .unwrap()
            .unwrap();
        assert!(sc.proposed_content.contains("Timeout handling"));
    }

    #[test]
    fn generic_failures_get_an_honest_reliability_gap() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        // Shared label + shared token, but neither verification- nor
        // timeout-shaped: an honest generic gap, not an invented cause.
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "failure",
                "skill 'verify-demo' execution recorded as failure (v1): deploy rollout stalled unexpectedly",
                NOW - 100 + i,
            );
        }
        let report = detect(&store, WS, Some("verify-demo"), NOW);
        assert_eq!(report.status, STATUS_CANDIDATE_FOUND, "{report:?}");
        let c = &report.candidates[0];
        assert_eq!(c.weakness_kind, WEAKNESS_RELIABILITY_GAP, "{c:?}");
        assert!(c
            .weakness
            .as_ref()
            .unwrap()
            .rationale
            .contains("No localized cause"));
    }

    #[test]
    fn redetection_converges_without_duplicating() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        let first = detect(&store, WS, None, NOW);
        assert_eq!(first.status, STATUS_CANDIDATE_FOUND);
        let first_id = first.candidates[0].skill_candidate_id.clone().unwrap();
        let count_before = store
            .list_skill_candidates(None, None, None, 50)
            .unwrap()
            .len();
        // Re-running with unchanged history converges on the same lineage.
        let second = detect(&store, WS, None, NOW + 1);
        assert_eq!(second.status, STATUS_ALREADY_EXISTS, "{second:?}");
        assert_eq!(
            second.candidates[0].outcome,
            "duplicate_candidate_in_review"
        );
        assert_eq!(
            second.candidates[0].skill_candidate_id.as_deref(),
            Some(first_id.as_str())
        );
        let count_after = store
            .list_skill_candidates(None, None, None, 50)
            .unwrap()
            .len();
        assert_eq!(
            count_before, count_after,
            "no duplicate lineage may be minted"
        );
    }

    #[test]
    fn old_lineage_failures_do_not_haunt_the_new_version() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        let report = detect(&store, WS, None, NOW).candidates.pop().unwrap();
        let sc_id = report.skill_candidate_id.clone().unwrap();
        // Human approves: v2 goes active.
        store
            .approve_skill_candidate(&sc_id, Some(WS), &skills, NOW + 10)
            .unwrap();
        // The pre-v2 failures are outside the v2 window: no immediate
        // re-proposal, no v2 → v3 auto-generation.
        let again = detect(&store, WS, None, NOW + 11);
        assert_eq!(again.status, STATUS_NO_CANDIDATES, "{again:?}");
    }

    #[test]
    fn stale_anchor_is_refused_at_approval() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        let report = detect(&store, WS, None, NOW).candidates.pop().unwrap();
        let sc_id = report.skill_candidate_id.clone().unwrap();
        store
            .approve_skill_candidate(&sc_id, Some(WS), &skills, NOW + 10)
            .unwrap();
        // A proposal anchored at v1 after the lineage reached v2 is stale.
        let stale_content = v1_content("verify-demo") + "\nStale edit.\n";
        let scope = SkillScope::Project;
        let stale_id =
            mint_skill_candidate_id(&scope, Some(WS), None, "verify-demo", &stale_content);
        store
            .insert_skill_candidate(&SkillCandidate {
                candidate_id: stale_id.clone(),
                workspace_root: Some(WS.to_string()),
                task_id: None,
                scope: "project".to_string(),
                name: "verify-demo".to_string(),
                description: "stale".to_string(),
                purpose: "stale".to_string(),
                applicability: SkillApplicability::default(),
                source_learning_candidates: Vec::new(),
                supporting_evidence: vec![1],
                contradicting_evidence: Vec::new(),
                proposed_content: stale_content,
                status: SkillCandidateStatus::Candidate.as_str().to_string(),
                confidence: 0.8,
                validation: None,
                eval_reason: None,
                rejection_reason: None,
                supersedes_skill: None,
                based_on_version: Some(1),
                created_at: NOW + 11,
                updated_at: NOW + 11,
                expires_at: None,
            })
            .unwrap();
        store
            .evaluate_candidate_content(&stale_id, NOW + 12)
            .unwrap();
        let err = store
            .approve_skill_candidate(&stale_id, Some(WS), &skills, NOW + 13)
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
    }

    #[test]
    fn invalid_successor_never_reaches_approval() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        let bad_id = "sc::invalid-successor";
        store
            .insert_skill_candidate(&SkillCandidate {
                candidate_id: bad_id.to_string(),
                workspace_root: Some(WS.to_string()),
                task_id: None,
                scope: "project".to_string(),
                name: "verify-demo".to_string(),
                description: "bad".to_string(),
                purpose: "bad".to_string(),
                applicability: SkillApplicability::default(),
                source_learning_candidates: Vec::new(),
                supporting_evidence: Vec::new(),
                contradicting_evidence: Vec::new(),
                proposed_content: "no frontmatter at all".to_string(),
                status: SkillCandidateStatus::Candidate.as_str().to_string(),
                confidence: 0.9,
                validation: None,
                eval_reason: None,
                rejection_reason: None,
                supersedes_skill: None,
                based_on_version: Some(1),
                created_at: NOW,
                updated_at: NOW,
                expires_at: None,
            })
            .unwrap();
        let evaluated = store.evaluate_candidate_content(bad_id, NOW + 1).unwrap();
        assert_ne!(evaluated.status, "validated");
        // The approval gate requires `validated`: an invalid successor is
        // refused, never published.
        assert!(store
            .create_skill_approval_request(bad_id, WS, None, "approve", None, NOW + 2)
            .is_err());
    }

    #[test]
    fn generated_v2_always_passes_validation() {
        for kind in [
            WEAKNESS_VERIFICATION_GAP,
            WEAKNESS_TIMEOUT_GAP,
            WEAKNESS_RELIABILITY_GAP,
        ] {
            let weakness = EvolutionWeakness {
                kind: kind.to_string(),
                label: "label".to_string(),
                description: "description".to_string(),
                proposed_change: "change".to_string(),
                rationale: "rationale".to_string(),
            };
            let v2 = evolution_skill_content(&v1_content("verify-demo"), 1, &weakness);
            let validation = validate_skill_content(&v2, "verify-demo", Some(WS));
            assert!(validation.valid, "{kind}: {:?}", validation.errors);
            // Deterministic: same inputs, same content.
            let again = evolution_skill_content(&v1_content("verify-demo"), 1, &weakness);
            assert_eq!(v2, again);
        }
    }

    #[test]
    fn evidence_comparison_splits_on_version_activation() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        // Pre-lineage failures (older than the v1 activation): linked by
        // name, but outside every version window.
        for i in 0..2 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "failure",
                "skill 'verify-demo' pre-window failure rollout stall",
                NOW - 700 + i,
            );
        }
        let skill = publish_v1(&store, &skills, "verify-demo", WS, NOW - 500);
        let active = store
            .get_active_skill_version(&skill.skill_id)
            .unwrap()
            .unwrap();
        let (bf, bs, sf, ss) =
            evolution_evidence_comparison(&store, WS, &skill, active.created_at).unwrap();
        assert_eq!((bf, bs, sf, ss), (2, 0, 0, 0));
        for i in 0..3 {
            seed_linked_failure(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                VERIFY_SUMMARY,
                NOW - 100 + i,
            );
        }
        seed_linked_success(&store, WS, "verify-demo", NOW - 90);
        let (bf, bs, sf, ss) =
            evolution_evidence_comparison(&store, WS, &skill, active.created_at).unwrap();
        assert_eq!((bf, bs, sf, ss), (2, 0, 3, 1));
    }

    #[test]
    fn unknown_skill_selector_is_a_caller_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let err = store
            .detect_skill_evolution(WS, Some("no-such-skill"), None, NOW)
            .unwrap_err();
        assert!(err.to_string().contains("skill not found"), "{err}");
    }
}

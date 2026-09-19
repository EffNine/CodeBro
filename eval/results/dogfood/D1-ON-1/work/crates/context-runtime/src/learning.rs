//! P3 learning + inference: cautious hypotheses from accumulated history.
//!
//! P2 answers "what happened during previous work". This module answers
//! "what should CodeBro learn from what happened" — and stops before skill
//! generation (P4).
//!
//! ```text
//!               P2 HISTORY (events, immutable)
//!                    │
//!                    ▼
//!               OBSERVATION (topic clustering, deterministic)
//!                    │
//!                    ▼
//!           LEARNING CANDIDATE (supporting + contradicting evidence)
//!                    │
//!           ┌────────┴────────┐
//!           ▼                 ▼
//!       SUPPORTING        CONTRADICTING
//!        EVIDENCE           EVIDENCE
//!           │                 │
//!           └────────┬────────┘
//!                    ▼
//!                EVALUATE (deterministic, inspectable)
//!                    │
//!           ┌────────┼────────┐
//!           ▼        ▼        ▼
//!        ACCEPT    DEFER    REJECT
//!           │
//!           ▼
//!       AI_INFERRED (ContextRecord, evidence-bound, decaying)
//!           │
//!      ┌────┴────┐
//!      ▼         ▼
//!   CONFIRM    DECAY/EXPIRE
//!      │
//!      ▼
//! USER_CONFIRMED (only via explicit user confirmation)
//! ```
//!
//! # What learning is (and is not)
//!
//! - **Observation**: something happened (one event).
//! - **Historical fact**: something happened previously (recallable events).
//! - **Learning candidate**: a possible recurring pattern (this module).
//! - **Inference**: a sufficiently supported conclusion (`AI_INFERRED`).
//! - **User-confirmed preference**: explicitly confirmed by the user
//!   (`USER_CONFIRMED`, only via the trusted caller-principal path).
//!
//! These are NOT interchangeable. In particular `confidence = 0.95` never
//! means `USER_CONFIRMED`: authority and confidence are separate dimensions.
//!
//! # Determinism rules
//!
//! - No LLM, no embeddings, no network. Topic clustering is token-pair
//!   overlap over redacted summaries/payloads; evaluation is arithmetic.
//! - Candidate identity is a deterministic hash of
//!   `(scope, workspace, task, kind, topic-pair)`: reprocessing the same
//!   history updates one row, never duplicates.
//! - Evidence is *derived* from the canonical event log at detection and
//!   re-derived at evaluation. Callers cannot inject evidence ids, so there
//!   is no forgery surface: fake ids can only exist in hand-built rows, and
//!   evaluation drops ids that do not resolve.
//! - Learning never writes history (`record_history` is untouched) and never
//!   touches facts/memory/identity. History is canonical; learning is
//!   secondary. If learning fails, OpenCode work continues.
//!
//! # Evidence weighting
//!
//! Not all evidence is equal:
//!
//! | Weight | Signal |
//! |--------|--------|
//! | strong (1.0) | `decision` events, `validation`/`error` events carrying an explicit outcome, explicit rejection observations |
//! | medium (0.6) | `change_applied`, `tool_result` |
//! | weak (0.3) | `observation` / `agent_observation`, `tool_execution`, anything else |
//!
//! Conversational kinds (`user_message`, `assistant_message`,
//! `session_started/ended`) never form candidates: a single conversational
//! statement is weak evidence, and learning from chatter manufactures
//! certainty.
//!
//! # Confidence
//!
//! Bounded `[0.05, 0.95]`, rounded to two decimals (no fake precision):
//!
//! ```text
//! base = min(support/5, 1) * 0.25      (quantity)
//!      + avg_weight * 0.20              (quality)
//!      + (s - c)/(s + c) * 0.35         (outcome consistency; c = 0 → +0.35)
//!      + recent_ratio * 0.10            (recency: evidence within 30 days)
//!      + min(sessions/3, 1) * 0.10      (diversity)
//! confidence = clamp(base, 0.05, 0.95)
//! ```
//!
//! Contradiction is voiced twice on purpose: it lowers confidence through
//! the consistency term, and — when contradicting evidence reaches half of
//! support — it blocks acceptance outright (contested ⇒ deferred, majority
//! against ⇒ rejected). A single contradictory event therefore dents a
//! hypothesis without flip-flopping established knowledge.
//!
//! # Evaluation thresholds
//!
//! | Condition | Verdict |
//! |-----------|---------|
//! | supporting < 3 | `deferred` (insufficient evidence) |
//! | supporting ≥ 3, contradicting > supporting | `rejected` (evidence weighs against) |
//! | supporting ≥ 3, contradicting × 2 ≥ supporting | `deferred` (contested; preserve, do not surface) |
//! | supporting ≥ 3, contradiction low, confidence ≥ 0.55 | `accepted` (persisted as `AI_INFERRED`) |
//! | supporting ≥ 3, contradiction low, confidence < 0.55 | `deferred` (weak) |
//!
//! A global inference additionally requires broader evidence: supporting
//! events from ≥ 2 workspaces, or ≥ 5 supporting events. A pattern seen in
//! one project stays project-scoped.
//!
//! # Scope discipline
//!
//! Detection runs inside one viewpoint (project / task / global). A
//! project-viewpoint run only reads that workspace's events, so another
//! project's evidence can never leak in — neither as support nor as
//! contradiction. Contradictions are only counted within the same scope:
//! project B preferring abstraction does not contradict project A preferring
//! simplicity.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use rusqlite::{params, OptionalExtension};
use sha2::{Digest, Sha256};

use crate::db;
use crate::history::HistoryKind;
use crate::retrieval::{query_tokens, ContextRetriever, RecordQuery};
use crate::store::{ContextError, ContextStore, EVENT_COLUMNS};
use crate::types::{
    Authority, ContextRecord, LifecycleStage, RecordKind, RecordScope, RecordStatus,
};
use crate::workspace::canonical_workspace_key;

/// Minimum supporting events for an `AI_INFERRED` conclusion.
pub const MIN_SUPPORTING_EVIDENCE: usize = 3;
/// Minimum supporting events for weak (`observation`) clusters, which carry
/// less signal per event.
pub const MIN_OBSERVATION_SUPPORTING: usize = 4;
/// Minimum confidence for acceptance.
pub const ACCEPT_MIN_CONFIDENCE: f64 = 0.55;
/// Candidate time-to-live: 90 days without refresh → `expired`.
pub const CANDIDATE_TTL_SECS: u64 = 90 * 86_400;
/// Inference time-to-live: accepted knowledge expires after 180 days unless
/// refreshed (historical evidence itself is never deleted).
pub const INFERENCE_TTL_SECS: u64 = 180 * 86_400;
/// Recency window: evidence newer than this counts as recent.
pub const RECENCY_WINDOW_SECS: u64 = 30 * 86_400;
/// Bounded event scan per detection pass (indexed, newest-first).
pub const MAX_DETECTION_EVENTS: usize = 500;
/// Topic tokens kept per event (alphabetical first N keeps pairs bounded).
pub const MAX_TOPIC_TOKENS: usize = 10;
/// Minimum shared tokens for two events to be topically related. A single
/// shared word is string frequency, not a semantic relation.
pub const MIN_SHARED_TOKENS: usize = 2;

/// Learning viewpoint: which history is even eligible, before clustering.
/// Mirrors [`crate::recall::RecallScope`] without depending on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LearnScope {
    /// Exactly one workspace (the default).
    #[default]
    Project,
    /// One workspace plus an exact task id (required).
    Task,
    /// Every workspace. Explicit opt-in; global inference additionally
    /// requires broader evidence (see module docs).
    Global,
}

impl std::fmt::Display for LearnScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LearnScope::Project => write!(f, "project"),
            LearnScope::Task => write!(f, "task"),
            LearnScope::Global => write!(f, "global"),
        }
    }
}

impl std::str::FromStr for LearnScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "project" => Ok(LearnScope::Project),
            "task" => Ok(LearnScope::Task),
            "global" => Ok(LearnScope::Global),
            other => Err(format!(
                "unknown learn scope '{other}': use project, task, or global"
            )),
        }
    }
}

/// Small, extensible candidate taxonomy. Deliberately few kinds: a candidate
/// is a hypothesis about recurring engineering behaviour, nothing more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CandidateKind {
    UserPreference,
    EngineeringPattern,
    ProjectPattern,
    FailurePattern,
    SuccessPattern,
    WorkflowPattern,
    DecisionPattern,
}

impl CandidateKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateKind::UserPreference => "user_preference",
            CandidateKind::EngineeringPattern => "engineering_pattern",
            CandidateKind::ProjectPattern => "project_pattern",
            CandidateKind::FailurePattern => "failure_pattern",
            CandidateKind::SuccessPattern => "success_pattern",
            CandidateKind::WorkflowPattern => "workflow_pattern",
            CandidateKind::DecisionPattern => "decision_pattern",
        }
    }

    /// Short slug for namespace derivation.
    pub fn slug(&self) -> &'static str {
        match self {
            CandidateKind::UserPreference => "user-preference",
            CandidateKind::EngineeringPattern => "engineering-pattern",
            CandidateKind::ProjectPattern => "project-pattern",
            CandidateKind::FailurePattern => "failure-pattern",
            CandidateKind::SuccessPattern => "success-pattern",
            CandidateKind::WorkflowPattern => "workflow-pattern",
            CandidateKind::DecisionPattern => "decision-pattern",
        }
    }

    /// Which [`RecordKind`] an accepted inference of this candidate kind
    /// persists as. `UserPreference` persists as `Preference` (fingerprint
    /// lane); outcome patterns persist as `Experience` (passthrough lane,
    /// queryable, never silently ranked above confirmed preferences);
    /// structural patterns persist as `Pattern` (fingerprint lane).
    pub fn record_kind(&self) -> RecordKind {
        match self {
            CandidateKind::UserPreference => RecordKind::Preference,
            CandidateKind::FailurePattern | CandidateKind::SuccessPattern => RecordKind::Experience,
            CandidateKind::EngineeringPattern
            | CandidateKind::ProjectPattern
            | CandidateKind::WorkflowPattern
            | CandidateKind::DecisionPattern => RecordKind::Pattern,
        }
    }
}

impl std::fmt::Display for CandidateKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for CandidateKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "user_preference" => Ok(CandidateKind::UserPreference),
            "engineering_pattern" => Ok(CandidateKind::EngineeringPattern),
            "project_pattern" => Ok(CandidateKind::ProjectPattern),
            "failure_pattern" => Ok(CandidateKind::FailurePattern),
            "success_pattern" => Ok(CandidateKind::SuccessPattern),
            "workflow_pattern" => Ok(CandidateKind::WorkflowPattern),
            "decision_pattern" => Ok(CandidateKind::DecisionPattern),
            other => Err(format!("unknown candidate kind: {other}")),
        }
    }
}

/// Candidate lifecycle: `candidate → evaluating → accepted | rejected`,
/// with `deferred` meaning "insufficient or contested evidence; preserved
/// but never surfaced as trusted context". Rejected and expired candidates
/// stay auditable; nothing is erased.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateStatus {
    Candidate,
    Evaluating,
    Accepted,
    Rejected,
    Deferred,
    Superseded,
    Expired,
}

impl CandidateStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            CandidateStatus::Candidate => "candidate",
            CandidateStatus::Evaluating => "evaluating",
            CandidateStatus::Accepted => "accepted",
            CandidateStatus::Rejected => "rejected",
            CandidateStatus::Deferred => "deferred",
            CandidateStatus::Superseded => "superseded",
            CandidateStatus::Expired => "expired",
        }
    }

    /// Terminal states accept no further evaluation transitions (a new
    /// detection pass creates or updates a `candidate`, never rewrites a
    /// terminal row — except `deferred`, which re-evaluation may revive).
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            CandidateStatus::Accepted
                | CandidateStatus::Rejected
                | CandidateStatus::Superseded
                | CandidateStatus::Expired
        )
    }
}

impl std::fmt::Display for CandidateStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

impl std::str::FromStr for CandidateStatus {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "candidate" => Ok(CandidateStatus::Candidate),
            "evaluating" => Ok(CandidateStatus::Evaluating),
            "accepted" => Ok(CandidateStatus::Accepted),
            "rejected" => Ok(CandidateStatus::Rejected),
            "deferred" => Ok(CandidateStatus::Deferred),
            "superseded" => Ok(CandidateStatus::Superseded),
            "expired" => Ok(CandidateStatus::Expired),
            other => Err(format!("unknown candidate status: {other}")),
        }
    }
}

/// Outcome polarity of an event outcome label. `Neutral` events neither
/// support outcome patterns nor contradict them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomePolarity {
    Success,
    Failure,
    Neutral,
}

/// Normalize a passive-capture outcome / verification classification into a
/// polarity. Unknown labels are neutral: learning must not guess.
pub fn outcome_polarity(outcome: Option<&str>) -> OutcomePolarity {
    let raw = outcome.unwrap_or("").trim().to_ascii_lowercase();
    match raw.as_str() {
        "passed" | "success" | "successful" | "verified" | "applied" | "created" | "recorded"
        | "decided" | "completed" | "superseded" | "retired" | "ok" => OutcomePolarity::Success,
        "failed" | "failure" | "test_failure" | "compile_error" | "unknown_failure" | "error"
        | "rejected" | "removed" | "abandoned" | "timeout" => OutcomePolarity::Failure,
        _ => OutcomePolarity::Neutral,
    }
}

/// Structural kind groups for clustering. Conversational and session-framing
/// kinds map to `Ignored`: they never form candidates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum KindGroup {
    Decision,
    Validation,
    Change,
    Error,
    Observation,
    Ignored,
}

pub(crate) fn kind_group(kind: &str) -> KindGroup {
    match kind {
        "decision" => KindGroup::Decision,
        "validation" => KindGroup::Validation,
        "change_applied" => KindGroup::Change,
        "error" => KindGroup::Error,
        "observation" | "agent_observation" | "tool_execution" | "tool_result" => {
            KindGroup::Observation
        }
        // P5 task lifecycle: completions and validation outcomes are
        // outcome-bearing evidence (a finished piece of engineering work
        // succeeded or failed); the structural transitions are plain
        // observations. Learning needs no P5 knowledge beyond this mapping.
        "task_completed" | "task_validation_passed" | "task_validation_failed" | "task_failed" => {
            KindGroup::Validation
        }
        // P9 engineering outcomes: OpenCode-reported structured evidence
        // bound to a durable task. Like P5 completions, these are
        // outcome-bearing (the classification travels in the outcome
        // label); interpretation stays with the existing evaluation
        // machinery — no P9-specific learning logic.
        "task_outcome" => KindGroup::Validation,
        "task_created"
        | "task_started"
        | "task_paused"
        | "task_resumed"
        | "task_checkpoint"
        | "task_validation_started"
        | "task_cancelled" => KindGroup::Observation,
        // P6 engineering intelligence: routine indexing/analysis events
        // must never become learning candidates (that would pollute P3
        // with "index completed" hypotheses). Explicitly ignored — only
        // repeated engineering *outcomes* (task completions, validation
        // failures) feed learning, never index runs.
        "index_completed"
        | "index_failed"
        | "impact_analyzed"
        | "health_analyzed"
        | "repository_discovered" => KindGroup::Ignored,
        _ => KindGroup::Ignored,
    }
}

/// Evidence weight per event (see module-docs table).
fn evidence_weight(kind: &str, polarity: OutcomePolarity) -> f64 {
    match kind_group(kind) {
        KindGroup::Decision => 1.0,
        KindGroup::Validation | KindGroup::Error => {
            if polarity == OutcomePolarity::Neutral {
                0.6
            } else {
                1.0
            }
        }
        KindGroup::Change => 0.6,
        KindGroup::Observation => 0.3,
        KindGroup::Ignored => 0.0,
    }
}

/// Topic tokens that are structural noise rather than signal. Domain words
/// (test, build, dependency, …) are deliberately NOT here: only generic
/// glue words are removed so "mere string frequency" needs at least two
/// *meaningful* shared tokens.
const TOPIC_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "was", "were", "have", "has", "had",
    "will", "would", "can", "could", "should", "about", "into", "over", "after", "before",
    "during", "again", "what", "why", "how", "when", "there", "their", "they", "them", "then",
    "than", "our", "your", "you", "are", "but", "not", "all", "any", "per", "via", "just", "like",
    "also", "more", "most", "some", "such", "only", "onto", "within", "without", "between", "both",
    "each", "other", "its",
];

/// Avoidance/preference signal lexicon: a decision/change cluster whose
/// topic intersects this set reads as a preference ("avoid unnecessary
/// dependencies") rather than a neutral recurrence ("used SQLite").
const AVOIDANCE_LEXICON: &[&str] = &[
    "avoid",
    "avoids",
    "avoided",
    "avoiding",
    "reject",
    "rejects",
    "rejected",
    "rejecting",
    "refuse",
    "refuses",
    "refused",
    "minimal",
    "minimize",
    "minimise",
    "minimalist",
    "unnecessary",
    "simple",
    "simpler",
    "simplest",
    "simplicity",
    "simplify",
    "tiny",
    "small",
    "concise",
    "direct",
    "lightweight",
    "inline",
    "reuse",
];

/// Sensitive-topic blocklist: CodeBro learns engineering collaboration and
/// project behaviour only. No psychological profiling, no sensitive personal
/// attributes, no medical/political/religious/sexuality/ethnicity inference.
/// A topic intersecting this set never becomes a candidate.
const SENSITIVE_TOPICS: &[&str] = &[
    "personality",
    "disorder",
    "depressed",
    "depression",
    "anxious",
    "anxiety",
    "adhd",
    "autism",
    "autistic",
    "bipolar",
    "therapy",
    "therapist",
    "medical",
    "diagnosis",
    "diagnosed",
    "disease",
    "illness",
    "health",
    "religion",
    "religious",
    "christian",
    "muslim",
    "islam",
    "jewish",
    "hindu",
    "buddhist",
    "atheist",
    "politics",
    "political",
    "democrat",
    "republican",
    "vote",
    "voting",
    "election",
    "sexual",
    "sexuality",
    "gender",
    "transgender",
    "race",
    "racial",
    "ethnic",
    "ethnicity",
    "pregnant",
    "pregnancy",
    "divorce",
    "married",
    "dating",
];

/// Extract the topic token set of an event: redacted summary + bounded
/// payload excerpt, tokenized like retrieval queries, stopwords removed,
/// capped at [`MAX_TOPIC_TOKENS`] (alphabetical first N — deterministic).
fn topic_tokens(summary: Option<&str>, payload: Option<&str>) -> BTreeSet<String> {
    let mut inputs = Vec::new();
    if let Some(s) = summary {
        inputs.push(s.to_string());
    }
    if let Some(p) = payload {
        inputs.push(p.chars().take(500).collect::<String>());
    }
    let tokens = query_tokens(&inputs);
    tokens
        .into_iter()
        .filter(|t| !TOPIC_STOPWORDS.contains(&t.as_str()))
        .take(MAX_TOPIC_TOKENS)
        .collect()
}

/// Whether a topic may become a candidate. Sensitive topics are refused;
/// single-token topics are string frequency, not a semantic relation.
fn topic_allowed(topic: &BTreeSet<String>) -> bool {
    if topic.len() < MIN_SHARED_TOKENS {
        return false;
    }
    !topic.iter().any(|t| SENSITIVE_TOPICS.contains(&t.as_str()))
}

/// Whether a decision/change topic reads as a preference signal.
fn is_preference_topic(topic: &BTreeSet<String>) -> bool {
    topic
        .iter()
        .any(|t| AVOIDANCE_LEXICON.contains(&t.as_str()))
}

/// A topic token pair in alphabetical order — the clustering atom. Two
/// shared meaningful tokens are the minimum semantic relation (a single
/// shared word is string frequency, not evidence).
type TopicPair = (String, String);

/// One pair-group: (kind-group, polarity, pair) with member indexes into
/// the detection set.
type PairGroup = ((KindGroup, OutcomePolarity, TopicPair), Vec<usize>);

/// Voiced evidence after scope verification: (supporting, contradicting,
/// dropped) event ids.
type VoicedEvidence = (Vec<i64>, Vec<i64>, Vec<i64>);

/// One observed change→validation-success cycle: (change id, validation id).
type WorkflowCycle = (i64, i64);

/// A first-class learning candidate: a possible recurring pattern with its
/// evidence, confidence, lifecycle state, and evaluation metadata.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LearningCandidate {
    /// Deterministic id (`lc::<16 hex>`): hash of
    /// `(scope, workspace, task, kind, topic-pair)`. Same evidence ⇒ same
    /// id ⇒ reprocessing updates, never duplicates.
    pub candidate_id: String,
    /// Canonical workspace key for project/task scope; `None` for global.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// Task binding for task scope; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub scope: String,
    pub kind: String,
    /// The testable proposition (not a bare historical fact).
    pub proposition: String,
    /// Fingerprint namespace the accepted inference persists under.
    pub namespace: String,
    /// Canonical event ids backing the pattern.
    #[serde(default)]
    pub supporting_evidence: Vec<i64>,
    /// Canonical event ids weighing against it (never deleted).
    #[serde(default)]
    pub contradicting_evidence: Vec<i64>,
    pub confidence: f64,
    pub status: String,
    pub created_at: u64,
    pub updated_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
    /// Human-readable evaluation verdict (why accept/defer/reject).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_reason: Option<String>,
    /// Id of the persisted `AI_INFERRED` record once accepted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inference_record_id: Option<String>,
}

impl LearningCandidate {
    /// Explainable summary: what CodeBro believes, why, how confident, and
    /// on what authority. No chain-of-thought — concise evidence counts.
    pub fn explain(&self) -> serde_json::Value {
        serde_json::json!({
            "what": self.proposition,
            "why": format!(
                "Supported by {} historical event(s) across the {} scope{}.",
                self.supporting_evidence.len(),
                self.scope,
                if self.contradicting_evidence.is_empty() {
                    " with no contradicting evidence".to_string()
                } else {
                    format!(
                        " with {} contradicting event(s) considered",
                        self.contradicting_evidence.len()
                    )
                }
            ),
            "confidence": self.confidence,
            "authority": Authority::AiInferred.as_str(),
            "note": "AI_INFERRED is a strongly-supported hypothesis, not user-confirmed truth. Only explicit user confirmation creates USER_CONFIRMED knowledge.",
            "status": self.status,
            "scope": self.scope,
            "kind": self.kind,
            "namespace": self.namespace,
            "supporting_evidence": self.supporting_evidence,
            "contradicting_evidence": self.contradicting_evidence,
            "inference_record_id": self.inference_record_id,
            "eval_reason": self.eval_reason,
        })
    }
}

/// Deterministic candidate id from the clustering identity.
fn mint_candidate_id(
    scope: &LearnScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
    kind: &CandidateKind,
    pair: (&str, &str),
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        format!(
            "{}|{}|{}|{}|{}+{}",
            scope,
            workspace_root.unwrap_or(""),
            task_id.unwrap_or(""),
            kind.as_str(),
            pair.0,
            pair.1
        )
        .as_bytes(),
    );
    format!("lc::{:016x}", {
        let digest = hasher.finalize();
        let mut bytes = [0u8; 8];
        bytes.copy_from_slice(&digest[..8]);
        u64::from_be_bytes(bytes)
    })
}

/// Namespace for the accepted inference: stable per (kind, topic).
fn candidate_namespace(kind: &CandidateKind, pair: (&str, &str)) -> String {
    let ns = format!("learn.{}.{}-{}", kind.slug(), pair.0, pair.1);
    ns.chars().take(200).collect()
}

/// Human-readable topic phrase from a token pair.
fn topic_phrase(pair: (&str, &str)) -> String {
    format!("{} {}", pair.0, pair.1)
}

/// Build the testable proposition for a candidate kind.
fn build_proposition(
    kind: &CandidateKind,
    scope: &LearnScope,
    workspace_root: Option<&str>,
    support: usize,
    sessions: usize,
    pair: (&str, &str),
) -> String {
    let topic = topic_phrase(pair);
    let scope_phrase = match scope {
        LearnScope::Global => "across projects".to_string(),
        LearnScope::Project => format!("in project {}", workspace_root.unwrap_or("this project")),
        LearnScope::Task => "in this task".to_string(),
    };
    match kind {
        CandidateKind::FailurePattern => format!(
            "The approach '{topic}' has repeatedly failed validation {scope_phrase} \
             ({support} failures across {sessions} sessions); treat it as unreliable here \
             until counter-evidence appears."
        ),
        CandidateKind::SuccessPattern => format!(
            "The workflow '{topic}' has repeatedly succeeded {scope_phrase} \
             ({support} successes across {sessions} sessions); it appears reliable here."
        ),
        CandidateKind::UserPreference => {
            let subject = match scope {
                LearnScope::Global => "The user".to_string(),
                LearnScope::Project | LearnScope::Task => "This project".to_string(),
            };
            format!(
                "{subject} appears to prefer minimizing '{topic}' \
                 (supported by {support} historical decisions across {sessions} sessions)."
            )
        }
        CandidateKind::DecisionPattern => format!(
            "Decisions {scope_phrase} repeatedly converge on '{topic}' \
             ({support} decisions across {sessions} sessions)."
        ),
        CandidateKind::ProjectPattern => format!(
            "Project {} consistently exhibits '{topic}' \
             ({support} occurrences across {sessions} sessions).",
            workspace_root.unwrap_or("this project")
        ),
        CandidateKind::EngineeringPattern => format!(
            "Engineering work across projects repeatedly exhibits '{topic}' \
             ({support} occurrences across {sessions} sessions)."
        ),
        CandidateKind::WorkflowPattern => format!(
            "The change-then-validate workflow around '{topic}' repeatedly succeeds \
             {scope_phrase} ({support} successful cycles across {sessions} sessions)."
        ),
    }
}

/// One clustered observation: a topic pair with its member events.
struct Cluster {
    pair: (String, String),
    members: Vec<ClusterMember>,
}

struct ClusterMember {
    id: i64,
    kind: String,
    polarity: OutcomePolarity,
    weight: f64,
    created_at: u64,
    session_id: Option<String>,
    workspace_root: String,
}

/// Scan-scope event fetch: bounded, newest-first, indexed.
fn fetch_scope_events(
    conn: &rusqlite::Connection,
    scope: &LearnScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
) -> Result<Vec<ClusterMember>, ContextError> {
    let scope_sql: &str = match scope {
        LearnScope::Project => "workspace_root = ?1",
        LearnScope::Task => "workspace_root = ?1 AND task_id = ?2",
        LearnScope::Global => "1 = 1",
    };
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM events WHERE {scope_sql} \
         ORDER BY created_at DESC, id DESC LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![workspace_root, task_id, MAX_DETECTION_EVENTS as i64],
            crate::store::row_to_event_pub,
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ContextError::Decode(e.to_string()))?;
    let mut out = Vec::new();
    for event in rows {
        // Defense in depth: re-apply scope predicates in Rust.
        match scope {
            LearnScope::Project => {
                if Some(event.workspace_root.as_str()) != workspace_root {
                    continue;
                }
            }
            LearnScope::Task => {
                if Some(event.workspace_root.as_str()) != workspace_root {
                    continue;
                }
                if event.task_id.as_deref() != task_id {
                    continue;
                }
            }
            LearnScope::Global => {}
        }
        if kind_group(&event.kind) == KindGroup::Ignored {
            continue;
        }
        let polarity = outcome_polarity(event.outcome.as_deref());
        let weight = evidence_weight(&event.kind, polarity);
        if weight <= 0.0 {
            continue;
        }
        let id = event.id.unwrap_or(-1);
        if id < 0 {
            continue;
        }
        out.push(ClusterMember {
            id,
            kind: event.kind.clone(),
            polarity,
            weight,
            created_at: event.created_at,
            session_id: event.session_id.clone(),
            workspace_root: event.workspace_root.clone(),
        });
    }
    Ok(out)
}

/// Topic tokens per event id (second pass over the fetched rows).
fn fetch_topic_map(
    conn: &rusqlite::Connection,
    scope: &LearnScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
) -> Result<HashMap<i64, BTreeSet<String>>, ContextError> {
    let scope_sql: &str = match scope {
        LearnScope::Project => "workspace_root = ?1",
        LearnScope::Task => "workspace_root = ?1 AND task_id = ?2",
        LearnScope::Global => "1 = 1",
    };
    let sql = format!(
        "SELECT id, summary, payload_json FROM events WHERE {scope_sql} \
         ORDER BY created_at DESC, id DESC LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            params![workspace_root, task_id, MAX_DETECTION_EVENTS as i64],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| ContextError::Decode(e.to_string()))?;
    let mut map = HashMap::new();
    for (id, summary, payload) in rows {
        let tokens = topic_tokens(summary.as_deref(), payload.as_deref());
        if !tokens.is_empty() {
            map.insert(id, tokens);
        }
    }
    Ok(map)
}

/// Group events by `(kind-group, polarity, token-pair)`. A pair-group with
/// enough distinct events becomes one candidate hypothesis.
fn cluster_by_pair(
    members: &[ClusterMember],
    topics: &HashMap<i64, BTreeSet<String>>,
) -> Vec<PairGroup> {
    // (group, polarity, pair) → member indexes. Pairs per event are bounded
    // (≤10 tokens → ≤45 pairs).
    let mut groups: BTreeMap<(String, String, String), Vec<usize>> = BTreeMap::new();
    for (idx, member) in members.iter().enumerate() {
        let group = kind_group(&member.kind);
        let tokens = match topics.get(&member.id) {
            Some(t) if t.len() >= MIN_SHARED_TOKENS => t,
            _ => continue,
        };
        let token_vec: Vec<&String> = tokens.iter().collect();
        for a in 0..token_vec.len() {
            for b in (a + 1)..token_vec.len() {
                let key = (
                    format!("{:?}", group),
                    format!("{:?}", member.polarity),
                    format!("{}+{}", token_vec[a], token_vec[b]),
                );
                groups.entry(key).or_default().push(idx);
            }
        }
    }
    // Rehydrate typed keys; keep groups with ≥2 members here (the
    // per-kind minimum applies at candidate selection).
    let mut out = Vec::new();
    for ((g, p, pair), idxs) in groups {
        if idxs.len() < 2 {
            continue;
        }
        let group = match g.as_str() {
            "Decision" => KindGroup::Decision,
            "Validation" => KindGroup::Validation,
            "Change" => KindGroup::Change,
            "Error" => KindGroup::Error,
            "Observation" => KindGroup::Observation,
            _ => continue,
        };
        let polarity = match p.as_str() {
            "Success" => OutcomePolarity::Success,
            "Failure" => OutcomePolarity::Failure,
            "Neutral" => OutcomePolarity::Neutral,
            _ => continue,
        };
        let mut split = pair.splitn(2, '+');
        let (a, b) = match (split.next(), split.next()) {
            (Some(a), Some(b)) => (a.to_string(), b.to_string()),
            _ => continue,
        };
        out.push(((group, polarity, (a, b)), idxs));
    }
    out
}

/// Decide the candidate kind for a cluster.
fn kind_for_cluster(
    group: KindGroup,
    polarity: OutcomePolarity,
    pair: &(String, String),
    scope: &LearnScope,
) -> CandidateKind {
    match (group, polarity) {
        (KindGroup::Validation, OutcomePolarity::Success)
        | (KindGroup::Observation, OutcomePolarity::Success) => CandidateKind::SuccessPattern,
        (KindGroup::Validation, OutcomePolarity::Failure) | (KindGroup::Error, _) => {
            CandidateKind::FailurePattern
        }
        (KindGroup::Observation, OutcomePolarity::Failure) => CandidateKind::FailurePattern,
        (KindGroup::Decision, _) => {
            let mut topic = BTreeSet::new();
            topic.insert(pair.0.clone());
            topic.insert(pair.1.clone());
            if is_preference_topic(&topic) {
                CandidateKind::UserPreference
            } else {
                CandidateKind::DecisionPattern
            }
        }
        (KindGroup::Change, _) => {
            let mut topic = BTreeSet::new();
            topic.insert(pair.0.clone());
            topic.insert(pair.1.clone());
            if is_preference_topic(&topic) && matches!(scope, LearnScope::Global) {
                CandidateKind::UserPreference
            } else if matches!(scope, LearnScope::Global) {
                CandidateKind::EngineeringPattern
            } else {
                CandidateKind::ProjectPattern
            }
        }
        _ => {
            // Neutral validations / observations without a polarity read as
            // recurring engineering behaviour, scoped like changes.
            if matches!(scope, LearnScope::Global) {
                CandidateKind::EngineeringPattern
            } else {
                CandidateKind::ProjectPattern
            }
        }
    }
}

/// Minimum support for a (kind-group, kind) combination.
fn min_support_for(group: KindGroup) -> usize {
    match group {
        KindGroup::Observation => MIN_OBSERVATION_SUPPORTING,
        _ => MIN_SUPPORTING_EVIDENCE,
    }
}

/// Compute confidence from supporting/contradicting members (module-docs
/// formula). Rounded to two decimals — no fake precision.
fn compute_confidence(
    supporting: &[&ClusterMember],
    contradicting: &[&ClusterMember],
    now: u64,
) -> f64 {
    let s = supporting.len() as f64;
    let c = contradicting.len() as f64;
    if supporting.is_empty() {
        return 0.05;
    }
    let quantity = (s / 5.0).min(1.0) * 0.25;
    let avg_weight = supporting.iter().map(|m| m.weight).sum::<f64>() / s.max(1.0);
    let quality = avg_weight * 0.20;
    let consistency = if s + c > 0.0 {
        (s - c) / (s + c) * 0.35
    } else {
        0.0
    };
    let recent = supporting
        .iter()
        .filter(|m| now.saturating_sub(m.created_at) <= RECENCY_WINDOW_SECS)
        .count() as f64
        / s.max(1.0);
    let recency = recent * 0.10;
    let mut sessions = HashSet::new();
    let mut unlinked = false;
    for m in supporting {
        match &m.session_id {
            Some(sid) => {
                sessions.insert(sid.clone());
            }
            None => unlinked = true,
        }
    }
    let diversity = (((sessions.len() + usize::from(unlinked)) as f64) / 3.0).min(1.0) * 0.10;
    let base = quantity + quality + consistency + recency + diversity;
    ((base.clamp(0.05, 0.95)) * 100.0).round() / 100.0
}

/// Count distinct sessions in a member set.
fn count_sessions(members: &[&ClusterMember]) -> usize {
    let mut sessions = HashSet::new();
    let mut unlinked = false;
    for m in members {
        match &m.session_id {
            Some(sid) => {
                sessions.insert(sid.clone());
            }
            None => unlinked = true,
        }
    }
    sessions.len() + usize::from(unlinked)
}

/// Row mapping for `learning_candidates`.
fn row_to_candidate(row: &rusqlite::Row<'_>) -> rusqlite::Result<LearningCandidate> {
    let supporting_json: String = row.get(7)?;
    let contradicting_json: String = row.get(8)?;
    let supporting: Vec<i64> = serde_json::from_str(&supporting_json).unwrap_or_default();
    let contradicting: Vec<i64> = serde_json::from_str(&contradicting_json).unwrap_or_default();
    Ok(LearningCandidate {
        candidate_id: row.get(0)?,
        workspace_root: row.get(1)?,
        task_id: row.get(2)?,
        scope: row.get(3)?,
        kind: row.get(4)?,
        proposition: row.get(5)?,
        namespace: row.get(6)?,
        supporting_evidence: supporting,
        contradicting_evidence: contradicting,
        confidence: row.get(9)?,
        status: row.get(10)?,
        created_at: row.get::<_, i64>(11)? as u64,
        updated_at: row.get::<_, i64>(12)? as u64,
        expires_at: row.get::<_, Option<i64>>(13)?.map(|t| t as u64),
        eval_reason: row.get(14)?,
        inference_record_id: row.get(15)?,
    })
}

const CANDIDATE_COLUMNS: &str = "candidate_id, workspace_root, task_id, scope, \
    candidate_kind, proposition, namespace, supporting_json, contradicting_json, \
    confidence, status, created_at, updated_at, expires_at, eval_reason, \
    inference_record_id";

impl ContextStore {
    // ── Candidate persistence ──────────────────────────────────────────

    /// Insert or update a candidate row (deterministic id ⇒ idempotent).
    /// `created_at` survives an upsert; user verdicts and expiry
    /// (`rejected`, `superseded`, `expired`) are never rewritten by
    /// re-detection — a user's "no" sticks. Already-`accepted` rows refresh
    /// their evidence and confidence (knowledge evolves) while the verdict,
    /// reason, and inference backlink stand until re-evaluation.
    pub(crate) fn upsert_candidate(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<(), ContextError> {
        self.with_conn(|conn| {
            let supporting = serde_json::to_string(&candidate.supporting_evidence)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            let contradicting = serde_json::to_string(&candidate.contradicting_evidence)
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            // Refresh path for accepted rows: evidence/confidence evolve,
            // the verdict and its audit trail stand.
            let refreshed = conn.execute(
                "UPDATE learning_candidates SET
                    supporting_json = ?2, contradicting_json = ?3,
                    confidence = ?4, updated_at = ?5, expires_at = ?6
                 WHERE candidate_id = ?1 AND status = 'accepted'",
                params![
                    candidate.candidate_id,
                    supporting,
                    contradicting,
                    candidate.confidence,
                    candidate.updated_at as i64,
                    candidate.expires_at.map(|t| t as i64),
                ],
            )?;
            if refreshed > 0 {
                return Ok(());
            }
            let updated = conn.execute(
                "UPDATE learning_candidates SET
                    workspace_root = ?2, task_id = ?3, scope = ?4,
                    candidate_kind = ?5, proposition = ?6, namespace = ?7,
                    supporting_json = ?8, contradicting_json = ?9,
                    confidence = ?10, status = ?11, updated_at = ?12,
                    expires_at = ?13, eval_reason = ?14, inference_record_id = ?15
                 WHERE candidate_id = ?1
                   AND status IN ('candidate', 'evaluating', 'deferred')",
                params![
                    candidate.candidate_id,
                    candidate.workspace_root.as_deref(),
                    candidate.task_id.as_deref(),
                    candidate.scope,
                    candidate.kind,
                    candidate.proposition,
                    candidate.namespace,
                    supporting,
                    contradicting,
                    candidate.confidence,
                    candidate.status,
                    candidate.updated_at as i64,
                    candidate.expires_at.map(|t| t as i64),
                    candidate.eval_reason.as_deref(),
                    candidate.inference_record_id.as_deref(),
                ],
            )?;
            if updated == 0 {
                conn.execute(
                    "INSERT OR IGNORE INTO learning_candidates (
                        candidate_id, workspace_root, task_id, scope,
                        candidate_kind, proposition, namespace, supporting_json,
                        contradicting_json, confidence, status, created_at,
                        updated_at, expires_at, eval_reason, inference_record_id
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
                               ?12, ?13, ?14, ?15, ?16)",
                    params![
                        candidate.candidate_id,
                        candidate.workspace_root.as_deref(),
                        candidate.task_id.as_deref(),
                        candidate.scope,
                        candidate.kind,
                        candidate.proposition,
                        candidate.namespace,
                        supporting,
                        contradicting,
                        candidate.confidence,
                        candidate.status,
                        candidate.created_at as i64,
                        candidate.updated_at as i64,
                        candidate.expires_at.map(|t| t as i64),
                        candidate.eval_reason.as_deref(),
                        candidate.inference_record_id.as_deref(),
                    ],
                )?;
            }
            Ok(())
        })
    }

    /// Fetch a candidate by id (any status).
    pub fn get_candidate(&self, id: &str) -> Result<Option<LearningCandidate>, ContextError> {
        self.with_conn(|conn| {
            conn.query_row(
                &format!(
                    "SELECT {CANDIDATE_COLUMNS} FROM learning_candidates WHERE candidate_id = ?1"
                ),
                [id],
                row_to_candidate,
            )
            .optional()
            .map_err(|e| ContextError::Decode(e.to_string()))
        })
    }

    /// List candidates visible from a workspace, newest first. Global rows
    /// are visible everywhere; project/task rows only from their workspace
    /// (task rows additionally need the exact task).
    pub fn list_candidates(
        &self,
        workspace_root: Option<&str>,
        status: Option<CandidateStatus>,
        task_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LearningCandidate>, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        let task = task_id.map(|t| t.trim().to_string());
        let status_str = status.map(|s| s.as_str().to_string());
        let limit = limit.clamp(1, 100) as i64;
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {CANDIDATE_COLUMNS} FROM learning_candidates
                 WHERE (?1 IS NULL OR status = ?1)
                   AND (scope = 'global'
                        OR (scope = 'project' AND workspace_root = ?2)
                        OR (scope = 'task' AND workspace_root = ?2 AND task_id = ?3))
                 ORDER BY updated_at DESC, candidate_id ASC LIMIT ?4"
            ))?;
            let rows = stmt
                .query_map(
                    params![status_str.as_deref(), ws.as_deref(), task.as_deref(), limit],
                    row_to_candidate,
                )?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;
            Ok(rows)
        })
    }

    /// Set a candidate's status (lifecycle transitions outside evaluation,
    /// e.g. user rejection or expiry sweeps).
    fn set_candidate_status(
        &self,
        id: &str,
        status: CandidateStatus,
        reason: Option<&str>,
        inference_record_id: Option<&str>,
        now: u64,
    ) -> Result<bool, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE learning_candidates SET status = ?1, updated_at = ?2,
                        eval_reason = COALESCE(?3, eval_reason),
                        inference_record_id = COALESCE(?4, inference_record_id)
                 WHERE candidate_id = ?5",
                params![status.as_str(), now as i64, reason, inference_record_id, id],
            )?;
            Ok(updated > 0)
        })
    }

    // ── Detection ──────────────────────────────────────────────────────

    /// Detect learning candidates from in-scope history and upsert them.
    /// Pure observation: nothing is evaluated or persisted as knowledge
    /// here. Returns the touched candidates (new or refreshed).
    pub fn propose_candidates(
        &self,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
        scope: LearnScope,
        now: u64,
    ) -> Result<Vec<LearningCandidate>, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        if matches!(scope, LearnScope::Project | LearnScope::Task)
            && ws.as_deref().unwrap_or("").is_empty()
        {
            return Err(ContextError::Validation(
                "project/task learning requires a workspace_root".to_string(),
            ));
        }
        let task = task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        if matches!(scope, LearnScope::Task) && task.as_deref().unwrap_or("").is_empty() {
            return Err(ContextError::Validation(
                "task learning requires a task_id".to_string(),
            ));
        }
        // One connection section: fetch events + topics, then cluster in
        // memory. Upserts happen afterwards (each its own short txn).
        let (members, topics) = self.with_conn(|conn| {
            let members = fetch_scope_events(conn, &scope, ws.as_deref(), task.as_deref())?;
            let topics = fetch_topic_map(conn, &scope, ws.as_deref(), task.as_deref())?;
            Ok::<_, ContextError>((members, topics))
        })?;

        // Index members by id for evidence-voicing later.
        let by_id: HashMap<i64, &ClusterMember> = members.iter().map(|m| (m.id, m)).collect();

        let mut touched = Vec::new();
        let pair_groups = cluster_by_pair(&members, &topics);
        // One candidate per (kind-group, polarity): keep the largest
        // pair-group (ties → lexicographically smallest pair). Richer topic
        // modeling is P4+; P3 stays deterministic and inspectable.
        let mut best: BTreeMap<(String, String), (TopicPair, Vec<usize>)> = BTreeMap::new();
        for ((group, polarity, pair), idxs) in pair_groups {
            // Sensitive topics never become candidates.
            let mut topic = BTreeSet::new();
            topic.insert(pair.0.clone());
            topic.insert(pair.1.clone());
            if !topic_allowed(&topic) {
                continue;
            }
            let key = (format!("{:?}", group), format!("{:?}", polarity));
            let entry = best
                .entry(key)
                .or_insert_with(|| (pair.clone(), Vec::new()));
            if idxs.len() > entry.1.len() || (idxs.len() == entry.1.len() && pair < entry.0) {
                *entry = (pair, idxs);
            }
        }

        for ((group_s, polarity_s), (pair, idxs)) in best {
            let group = match group_s.as_str() {
                "Decision" => KindGroup::Decision,
                "Validation" => KindGroup::Validation,
                "Change" => KindGroup::Change,
                "Error" => KindGroup::Error,
                "Observation" => KindGroup::Observation,
                _ => continue,
            };
            let polarity = match polarity_s.as_str() {
                "Success" => OutcomePolarity::Success,
                "Failure" => OutcomePolarity::Failure,
                "Neutral" => OutcomePolarity::Neutral,
                _ => continue,
            };
            if idxs.len() < min_support_for(group) {
                continue;
            }
            let kind = kind_for_cluster(group, polarity, &pair, &scope);
            let candidate_id = mint_candidate_id(
                &scope,
                ws.as_deref(),
                task.as_deref(),
                &kind,
                (&pair.0, &pair.1),
            );
            // Voice evidence: supporting = cluster members; contradicting =
            // same-scope events sharing the pair with opposite polarity
            // (outcome-bearing kinds for polarized candidates; any failure
            // for neutral ones — see module docs).
            let member_ids: HashSet<i64> = idxs.iter().map(|&i| members[i].id).collect();
            let mut supporting: Vec<i64> = member_ids.iter().copied().collect();
            supporting.sort_unstable();
            let mut contradicting = Vec::new();
            for member in &members {
                if member_ids.contains(&member.id) {
                    continue;
                }
                let tokens = match topics.get(&member.id) {
                    Some(t) => t,
                    None => continue,
                };
                if !(tokens.contains(&pair.0) && tokens.contains(&pair.1)) {
                    continue;
                }
                let contradicts = match polarity {
                    OutcomePolarity::Success => {
                        member.polarity == OutcomePolarity::Failure
                            && matches!(
                                kind_group(&member.kind),
                                KindGroup::Validation | KindGroup::Error
                            )
                    }
                    OutcomePolarity::Failure => {
                        member.polarity == OutcomePolarity::Success
                            && matches!(
                                kind_group(&member.kind),
                                KindGroup::Validation | KindGroup::Error
                            )
                    }
                    OutcomePolarity::Neutral => member.polarity == OutcomePolarity::Failure,
                };
                if contradicts {
                    contradicting.push(member.id);
                }
            }
            contradicting.sort_unstable();
            contradicting.dedup();

            let sup_refs: Vec<&ClusterMember> = supporting
                .iter()
                .filter_map(|id| by_id.get(id).copied())
                .collect();
            let con_refs: Vec<&ClusterMember> = contradicting
                .iter()
                .filter_map(|id| by_id.get(id).copied())
                .collect();
            let sessions = count_sessions(&sup_refs).max(1);
            let confidence = compute_confidence(&sup_refs, &con_refs, now);
            let namespace = candidate_namespace(&kind, (&pair.0, &pair.1));
            let proposition = build_proposition(
                &kind,
                &scope,
                ws.as_deref(),
                supporting.len(),
                sessions,
                (&pair.0, &pair.1),
            );
            let record_scope = match scope {
                LearnScope::Global => RecordScope::Global,
                LearnScope::Project => RecordScope::Project,
                LearnScope::Task => RecordScope::Task,
            };
            let candidate = LearningCandidate {
                candidate_id: candidate_id.clone(),
                workspace_root: match record_scope {
                    RecordScope::Global => None,
                    _ => ws.clone(),
                },
                task_id: match record_scope {
                    RecordScope::Task => task.clone(),
                    _ => None,
                },
                scope: scope.to_string(),
                kind: kind.as_str().to_string(),
                proposition,
                namespace,
                supporting_evidence: supporting,
                contradicting_evidence: contradicting,
                confidence,
                status: CandidateStatus::Candidate.as_str().to_string(),
                created_at: now,
                updated_at: now,
                expires_at: Some(now.saturating_add(CANDIDATE_TTL_SECS)),
                eval_reason: Some("proposed from history; not yet evaluated".to_string()),
                inference_record_id: None,
            };
            self.upsert_candidate(&candidate)?;
            if let Some(stored) = self.get_candidate(&candidate_id)? {
                touched.push(stored);
            }
        }

        // Workflow detector: change→validation-success sequences in one
        // session sharing a token pair. ≥3 sequences ⇒ workflow candidate.
        if let Some(workflow) = self.detect_workflow_candidate(
            &members,
            &topics,
            &scope,
            ws.as_deref(),
            task.as_deref(),
            now,
        )? {
            self.upsert_candidate(&workflow)?;
            if let Some(stored) = self.get_candidate(&workflow.candidate_id)? {
                if !touched
                    .iter()
                    .any(|c| c.candidate_id == stored.candidate_id)
                {
                    touched.push(stored);
                }
            }
        }

        touched.sort_by(|a, b| a.candidate_id.cmp(&b.candidate_id));
        Ok(touched)
    }

    /// Detect change→validation-success cycles sharing a token pair.
    fn detect_workflow_candidate(
        &self,
        members: &[ClusterMember],
        topics: &HashMap<i64, BTreeSet<String>>,
        scope: &LearnScope,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
        now: u64,
    ) -> Result<Option<LearningCandidate>, ContextError> {
        // Per session: ordered changes and later successful validations.
        let mut by_session: BTreeMap<String, (Vec<usize>, Vec<usize>)> = BTreeMap::new();
        for (idx, m) in members.iter().enumerate() {
            let sid = match &m.session_id {
                Some(s) => s.clone(),
                None => continue,
            };
            let entry = by_session
                .entry(sid)
                .or_insert_with(|| (Vec::new(), Vec::new()));
            if kind_group(&m.kind) == KindGroup::Change {
                entry.0.push(idx);
            } else if kind_group(&m.kind) == KindGroup::Validation
                && m.polarity == OutcomePolarity::Success
            {
                entry.1.push(idx);
            }
        }
        // pair → successful cycles (change id, validation id).
        let mut cycles: BTreeMap<(String, String), Vec<(i64, i64)>> = BTreeMap::new();
        for (changes, validations) in by_session.values() {
            for &ci in changes {
                let c = &members[ci];
                let c_tokens = match topics.get(&c.id) {
                    Some(t) => t,
                    None => continue,
                };
                for &vi in validations {
                    let v = &members[vi];
                    if v.created_at < c.created_at {
                        continue;
                    }
                    let v_tokens = match topics.get(&v.id) {
                        Some(t) => t,
                        None => continue,
                    };
                    let shared: Vec<&String> = c_tokens.intersection(v_tokens).collect();
                    for a in 0..shared.len() {
                        for b in (a + 1)..shared.len() {
                            let (x, y) = if shared[a] < shared[b] {
                                (shared[a].clone(), shared[b].clone())
                            } else {
                                (shared[b].clone(), shared[a].clone())
                            };
                            cycles.entry((x, y)).or_default().push((c.id, v.id));
                        }
                    }
                }
            }
        }
        // Strongest pair with ≥3 cycles; ties → smallest pair.
        let mut best: Option<(TopicPair, Vec<WorkflowCycle>)> = None;
        for (pair, list) in cycles {
            let mut topic = BTreeSet::new();
            topic.insert(pair.0.clone());
            topic.insert(pair.1.clone());
            if !topic_allowed(&topic) {
                continue;
            }
            // Distinct sessions carrying the cycle.
            let mut sessions = HashSet::new();
            for (cid, _) in &list {
                if let Some(m) = members.iter().find(|m| m.id == *cid) {
                    sessions.insert(m.session_id.clone().unwrap_or_default());
                }
            }
            if list.len() < MIN_SUPPORTING_EVIDENCE || sessions.len() < 2 {
                continue;
            }
            match &best {
                Some((bp, bl))
                    if bl.len() > list.len() || (bl.len() == list.len() && *bp < pair) =>
                {
                    continue;
                }
                _ => best = Some((pair, list)),
            }
        }
        let (pair, list) = match best {
            Some(b) => b,
            None => return Ok(None),
        };
        let kind = CandidateKind::WorkflowPattern;
        let candidate_id =
            mint_candidate_id(scope, workspace_root, task_id, &kind, (&pair.0, &pair.1));
        let mut supporting: Vec<i64> = list
            .iter()
            .flat_map(|(c, v)| [*c, *v])
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        supporting.sort_unstable();
        let by_id: HashMap<i64, &ClusterMember> = members.iter().map(|m| (m.id, m)).collect();
        let sup_refs: Vec<&ClusterMember> = supporting
            .iter()
            .filter_map(|id| by_id.get(id).copied())
            .collect();
        let sessions = count_sessions(&sup_refs).max(1);
        let confidence = compute_confidence(&sup_refs, &[], now);
        let record_scope = match scope {
            LearnScope::Global => RecordScope::Global,
            LearnScope::Project => RecordScope::Project,
            LearnScope::Task => RecordScope::Task,
        };
        Ok(Some(LearningCandidate {
            candidate_id,
            workspace_root: match record_scope {
                RecordScope::Global => None,
                _ => workspace_root.map(str::to_string),
            },
            task_id: match record_scope {
                RecordScope::Task => task_id.map(str::to_string),
                _ => None,
            },
            scope: scope.to_string(),
            kind: kind.as_str().to_string(),
            proposition: build_proposition(
                &kind,
                scope,
                workspace_root,
                list.len(),
                sessions,
                (&pair.0, &pair.1),
            ),
            namespace: candidate_namespace(&kind, (&pair.0, &pair.1)),
            supporting_evidence: supporting,
            contradicting_evidence: Vec::new(),
            confidence,
            status: CandidateStatus::Candidate.as_str().to_string(),
            created_at: now,
            updated_at: now,
            expires_at: Some(now.saturating_add(CANDIDATE_TTL_SECS)),
            eval_reason: Some("proposed from history; not yet evaluated".to_string()),
            inference_record_id: None,
        }))
    }

    // ── Evaluation ─────────────────────────────────────────────────────

    /// Evaluate one candidate: re-derive its evidence from canonical history
    /// (fake or vanished ids are dropped), recompute confidence, and
    /// transition `candidate|deferred → evaluating → accepted|rejected|
    /// deferred`. Acceptance persists the `AI_INFERRED` record.
    ///
    /// Already-`accepted` candidates may be re-evaluated as evidence
    /// accumulates: a still-passing hypothesis refreshes its inference via
    /// the supersede chain (evolution, never overwrite); a hypothesis the
    /// evidence no longer supports retires its inference (rejected, kept as
    /// negative knowledge) and steps back to `deferred`/`rejected`.
    /// `rejected` (user verdict), `superseded`, and `expired` rows are never
    /// re-evaluated: detection of new evidence creates a fresh candidate, it
    /// never rewrites those verdicts.
    pub fn evaluate_candidate(
        &self,
        candidate_id: &str,
        now: u64,
    ) -> Result<LearningCandidate, ContextError> {
        let candidate = self.get_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("no learning candidate '{candidate_id}'"))
        })?;
        let status: CandidateStatus = candidate.status.parse().map_err(ContextError::Validation)?;
        match status {
            CandidateStatus::Rejected | CandidateStatus::Superseded | CandidateStatus::Expired => {
                return Err(ContextError::Validation(format!(
                    "candidate {candidate_id} is {status}: \
                     detection of new evidence creates a fresh candidate, it never rewrites this verdict"
                )));
            }
            CandidateStatus::Candidate
            | CandidateStatus::Evaluating
            | CandidateStatus::Deferred
            | CandidateStatus::Accepted => {}
        }
        let was_accepted = status == CandidateStatus::Accepted;
        self.set_candidate_status(candidate_id, CandidateStatus::Evaluating, None, None, now)?;

        // Re-derive evidence: every cited id must still resolve to a real
        // event in the candidate's scope. Fiction and foreign-workspace ids
        // are dropped with the reason saying so.
        let scope: LearnScope = candidate.scope.parse().map_err(ContextError::Validation)?;
        let voiced = self.voice_evidence(&candidate)?;
        let (supporting, contradicting, dropped) = voiced;
        let _ = scope;

        let by_id = self.evidence_members(&supporting, &contradicting)?;
        let sup_refs: Vec<&ClusterMember> =
            supporting.iter().filter_map(|id| by_id.get(id)).collect();
        let con_refs: Vec<&ClusterMember> = contradicting
            .iter()
            .filter_map(|id| by_id.get(id))
            .collect();
        let confidence = compute_confidence(&sup_refs, &con_refs, now);

        // Global bar: broader evidence or it stays project-scoped.
        let mut workspaces = HashSet::new();
        for m in &sup_refs {
            workspaces.insert(m.workspace_root.clone());
        }
        let global_ok =
            candidate.scope != "global" || workspaces.len() >= 2 || supporting.len() >= 5;

        let s = supporting.len();
        let c = contradicting.len();
        let (verdict, reason) = if !dropped.is_empty() && s < MIN_SUPPORTING_EVIDENCE {
            (
                CandidateStatus::Deferred,
                format!(
                    "insufficient evidence after verification: {s} supporting \
                     ({} cited id(s) did not resolve to in-scope history and were dropped)",
                    dropped.len()
                ),
            )
        } else if s < MIN_SUPPORTING_EVIDENCE {
            (
                CandidateStatus::Deferred,
                format!(
                    "insufficient evidence: {s} supporting event(s), need at least \
                     {MIN_SUPPORTING_EVIDENCE} for an AI_INFERRED conclusion"
                ),
            )
        } else if !global_ok {
            (
                CandidateStatus::Deferred,
                format!(
                    "global inference requires broader evidence (≥2 workspaces or ≥5 events; \
                     have {} workspace(s), {s} events): keep project-scoped",
                    workspaces.len()
                ),
            )
        } else if c > s {
            (
                CandidateStatus::Rejected,
                format!(
                    "contradictory evidence outweighs support ({s} supporting vs {c} \
                     contradicting): hypothesis weighs against itself; preserved for audit"
                ),
            )
        } else if c * 2 >= s {
            (
                CandidateStatus::Deferred,
                format!(
                    "contested evidence ({s} supporting vs {c} contradicting): confidence \
                     {confidence:.2} reflects the split; preserved but not surfaced"
                ),
            )
        } else if confidence >= ACCEPT_MIN_CONFIDENCE {
            (CandidateStatus::Accepted, String::new())
        } else {
            (
                CandidateStatus::Deferred,
                format!(
                    "weak support (confidence {confidence:.2} < {ACCEPT_MIN_CONFIDENCE}): \
                     {s} supporting vs {c} contradicting; preserved but not surfaced"
                ),
            )
        };

        match verdict {
            CandidateStatus::Accepted => {
                // Persist the inference FIRST (knowledge is the valuable
                // artifact); then mark the candidate. If marking fails the
                // next pass supersedes the orphaned inference by namespace
                // instead of duplicating it.
                let record_id = self.persist_inference(&candidate, &supporting, confidence, now)?;
                let reason = format!(
                    "accepted with confidence {confidence:.2}: {s} supporting event(s) \
                     across {} session(s){}; persisted as AI_INFERRED record {record_id}",
                    count_sessions(&sup_refs).max(1),
                    if c == 0 {
                        ", no contradiction".to_string()
                    } else {
                        format!(", {c} contradicting considered")
                    },
                );
                self.with_conn(|conn| {
                    let supporting_json = serde_json::to_string(&supporting)
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    let contradicting_json = serde_json::to_string(&contradicting)
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    conn.execute(
                        "UPDATE learning_candidates SET supporting_json = ?1,
                                contradicting_json = ?2, confidence = ?3, status = ?4,
                                updated_at = ?5, eval_reason = ?6,
                                inference_record_id = ?7
                         WHERE candidate_id = ?8",
                        params![
                            supporting_json,
                            contradicting_json,
                            confidence,
                            CandidateStatus::Accepted.as_str(),
                            now as i64,
                            reason,
                            record_id,
                            candidate_id,
                        ],
                    )?;
                    Ok(())
                })?;
                self.get_candidate(candidate_id)?
                    .ok_or_else(|| ContextError::Decode("candidate vanished".to_string()))
            }
            CandidateStatus::Deferred | CandidateStatus::Rejected => {
                // A previously-accepted hypothesis the evidence no longer
                // supports: retire its inference (rejected, preserved as
                // negative knowledge) instead of leaving stale knowledge
                // active beside a lapsed hypothesis.
                let mut reason = reason;
                if was_accepted {
                    if let Some(inf_id) = candidate.inference_record_id.as_deref() {
                        let _ = self.reject_record(inf_id, now);
                        reason.push_str(&format!(
                            "; prior inference {inf_id} retired as no longer supported"
                        ));
                    }
                }
                self.with_conn(|conn| {
                    let supporting_json = serde_json::to_string(&supporting)
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    let contradicting_json = serde_json::to_string(&contradicting)
                        .map_err(|e| ContextError::Decode(e.to_string()))?;
                    conn.execute(
                        "UPDATE learning_candidates SET supporting_json = ?1,
                                contradicting_json = ?2, confidence = ?3, status = ?4,
                                updated_at = ?5, eval_reason = ?6
                         WHERE candidate_id = ?7",
                        params![
                            supporting_json,
                            contradicting_json,
                            confidence,
                            verdict.as_str(),
                            now as i64,
                            reason,
                            candidate_id,
                        ],
                    )?;
                    Ok(())
                })?;
                self.get_candidate(candidate_id)?
                    .ok_or_else(|| ContextError::Decode("candidate vanished".to_string()))
            }
            _ => Err(ContextError::Decode("unreachable verdict".to_string())),
        }
    }

    /// Resolve cited evidence against canonical history: keep ids that
    /// exist AND belong to the candidate's scope; report dropped ids.
    /// Returns (supporting, contradicting, dropped).
    fn voice_evidence(
        &self,
        candidate: &LearningCandidate,
    ) -> Result<VoicedEvidence, ContextError> {
        self.with_conn(|conn| {
            let mut supporting = Vec::new();
            let mut contradicting = Vec::new();
            let mut dropped = Vec::new();
            for id in candidate
                .supporting_evidence
                .iter()
                .chain(candidate.contradicting_evidence.iter())
            {
                let ws: Option<String> = conn
                    .query_row(
                        "SELECT workspace_root FROM events WHERE id = ?1",
                        [*id],
                        |row| row.get(0),
                    )
                    .optional()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                let ws = match ws {
                    Some(w) => w,
                    None => {
                        dropped.push(*id);
                        continue;
                    }
                };
                let in_scope = match candidate.scope.as_str() {
                    "global" => true,
                    "project" => Some(ws.as_str()) == candidate.workspace_root.as_deref(),
                    "task" => {
                        Some(ws.as_str()) == candidate.workspace_root.as_deref()
                            && conn
                                .query_row(
                                    "SELECT task_id FROM events WHERE id = ?1",
                                    [*id],
                                    |row| row.get::<_, Option<String>>(0),
                                )
                                .optional()
                                .map_err(|e| ContextError::Decode(e.to_string()))?
                                .flatten()
                                .as_deref()
                                == candidate.task_id.as_deref()
                    }
                    _ => false,
                };
                if !in_scope {
                    dropped.push(*id);
                    continue;
                }
                if candidate.supporting_evidence.contains(id) {
                    supporting.push(*id);
                } else {
                    contradicting.push(*id);
                }
            }
            supporting.sort_unstable();
            supporting.dedup();
            contradicting.sort_unstable();
            contradicting.dedup();
            Ok((supporting, contradicting, dropped))
        })
    }

    /// Load member details (weight, time, session) for evidence ids.
    fn evidence_members(
        &self,
        supporting: &[i64],
        contradicting: &[i64],
    ) -> Result<HashMap<i64, ClusterMember>, ContextError> {
        self.with_conn(|conn| {
            let mut map = HashMap::new();
            for id in supporting.iter().chain(contradicting.iter()) {
                let event = conn
                    .query_row(
                        &format!("SELECT {EVENT_COLUMNS} FROM events WHERE id = ?1"),
                        [*id],
                        crate::store::row_to_event_pub,
                    )
                    .optional()
                    .map_err(|e| ContextError::Decode(e.to_string()))?;
                if let Some(event) = event {
                    let polarity = outcome_polarity(event.outcome.as_deref());
                    map.insert(
                        *id,
                        ClusterMember {
                            id: *id,
                            kind: event.kind.clone(),
                            polarity,
                            weight: evidence_weight(&event.kind, polarity),
                            created_at: event.created_at,
                            session_id: event.session_id.clone(),
                            workspace_root: event.workspace_root.clone(),
                        },
                    );
                }
            }
            Ok(map)
        })
    }

    /// Persist an accepted candidate as an `AI_INFERRED` context record.
    /// Same-namespace active inferences are superseded (chain preserved),
    /// never overwritten — re-running learning converges instead of
    /// duplicating.
    fn persist_inference(
        &self,
        candidate: &LearningCandidate,
        supporting: &[i64],
        confidence: f64,
        now: u64,
    ) -> Result<String, ContextError> {
        let kind: CandidateKind = candidate.kind.parse().map_err(ContextError::Validation)?;
        let record_kind = kind.record_kind();
        let scope: RecordScope = candidate.scope.parse().map_err(ContextError::Validation)?;
        let evidence: Vec<String> = supporting.iter().map(|id| id.to_string()).collect();
        if evidence.is_empty() {
            return Err(ContextError::Validation(
                "cannot persist an inference without evidence".to_string(),
            ));
        }
        let extra = serde_json::json!({
            "learning_candidate": candidate.candidate_id,
            "candidate_kind": candidate.kind,
            "supporting_count": supporting.len(),
            "contradicting_count": candidate.contradicting_evidence.len(),
        });
        let mut record = ContextRecord::new(
            format!(
                "ctx::learn::{}::{now}",
                candidate.candidate_id.trim_start_matches("lc::")
            ),
            record_kind,
            candidate.namespace.clone(),
            candidate.proposition.clone(),
            Authority::AiInferred,
        );
        record.scope = scope;
        record.workspace_root = candidate.workspace_root.clone();
        record.task_id = candidate.task_id.clone();
        record.lifecycle = LifecycleStage::Inferred;
        record.confidence = confidence;
        record.importance = 0.5;
        record.evidence = evidence;
        record.source = Some(format!("learn:{}", candidate.candidate_id));
        record.extra_json = Some(extra.to_string());
        record.expires_at = Some(now.saturating_add(INFERENCE_TTL_SECS));

        // Same-namespace active inference ⇒ supersede (converge, no dupes).
        // Re-running without new evidence reuses the record instead of
        // churning the supersede chain.
        let clash = self.active_inference_in_namespace(
            record_kind,
            &candidate.namespace,
            scope,
            candidate.workspace_root.as_deref(),
            candidate.task_id.as_deref(),
        )?;
        if let Some(prev_id) = clash {
            let unchanged = Some(prev_id.as_str()) == candidate.inference_record_id.as_deref()
                && candidate.supporting_evidence == *supporting;
            if unchanged {
                // Already persisted by an earlier pass: reuse the record.
                return Ok(prev_id);
            }
            record.supersedes = Some(prev_id.clone());
            self.supersede_record(&prev_id, &record, now)?;
            return Ok(record.id.clone());
        }
        self.put_record(&record, now)?;
        Ok(record.id.clone())
    }

    /// Active `AI_INFERRED` record owning (kind, namespace, scope), if any.
    fn active_inference_in_namespace(
        &self,
        kind: RecordKind,
        namespace: &str,
        scope: RecordScope,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
    ) -> Result<Option<String>, ContextError> {
        let query = RecordQuery {
            workspace_root,
            task_id,
            kind: Some(kind),
            status: None,
            keywords: Vec::new(),
            // u64::MAX would not decay; use a large-but-sane now.
            limit: 50,
        };
        let ranked = self.search(&query, 4_000_000_000)?;
        for r in ranked {
            if r.record.namespace == namespace
                && r.record.scope == scope
                && r.record.authority == Authority::AiInferred
                && r.record.workspace_root.as_deref() == workspace_root
                && r.record.task_id.as_deref() == task_id
            {
                return Ok(Some(r.record.id.clone()));
            }
        }
        Ok(None)
    }

    // ── Confirmation / rejection / expiry ──────────────────────────────

    /// Explicit user confirmation: `AI_INFERRED` ⇒ `USER_CONFIRMED` via
    /// supersede (audit trail kept). Requires `user_confirmed = true` —
    /// the caller-principal rule: only the user's explicit speech act (as
    /// reported by OpenCode) creates confirmed truth. The model can never
    /// self-confirm.
    pub fn confirm_candidate(
        &self,
        candidate_id: &str,
        user_confirmed: bool,
        now: u64,
    ) -> Result<LearningCandidate, ContextError> {
        if !user_confirmed {
            return Err(ContextError::Validation(
                "confirmation requires user_confirmed=true: only the user can \
                 create USER_CONFIRMED truth; an AI inference can never promote itself"
                    .to_string(),
            ));
        }
        let candidate = self.get_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("no learning candidate '{candidate_id}'"))
        })?;
        if candidate.status != CandidateStatus::Accepted.as_str() {
            return Err(ContextError::Validation(format!(
                "only an accepted candidate can be confirmed (candidate \
                 {candidate_id} is {})",
                candidate.status
            )));
        }
        let inference_id = candidate.inference_record_id.clone().ok_or_else(|| {
            ContextError::Validation(
                "accepted candidate has no persisted inference to confirm".to_string(),
            )
        })?;
        let inference = self.get_record(&inference_id)?.ok_or_else(|| {
            ContextError::Validation(format!("inference record {inference_id} no longer exists"))
        })?;
        if inference.status != RecordStatus::Active {
            return Err(ContextError::Validation(format!(
                "inference record {inference_id} is {} (not active)",
                inference.status
            )));
        }
        let mut confirmed = inference.clone();
        confirmed.id = format!("{inference_id}::confirmed::{now}");
        confirmed.authority = Authority::UserConfirmed;
        confirmed.lifecycle = LifecycleStage::Confirmed;
        confirmed.confidence = 0.9;
        confirmed.supersedes = Some(inference_id.clone());
        confirmed.source = Some(format!("learn-confirm:{candidate_id}"));
        self.supersede_record(&inference_id, &confirmed, now)?;
        self.set_candidate_status(
            candidate_id,
            CandidateStatus::Superseded,
            Some(
                "explicitly confirmed by the user; USER_CONFIRMED record supersedes the inference",
            ),
            Some(&confirmed.id),
            now,
        )?;
        self.get_candidate(candidate_id)?
            .ok_or_else(|| ContextError::Decode("candidate vanished".to_string()))
    }

    /// User rejection: the candidate is marked rejected and its inference
    /// (if still active) is rejected too — preserved as negative knowledge,
    /// never deleted. A rejected global may be followed by a project-scoped
    /// preference via `remember` ("only true for this project").
    pub fn reject_candidate(
        &self,
        candidate_id: &str,
        reason: Option<&str>,
        now: u64,
    ) -> Result<LearningCandidate, ContextError> {
        let candidate = self.get_candidate(candidate_id)?.ok_or_else(|| {
            ContextError::Validation(format!("no learning candidate '{candidate_id}'"))
        })?;
        if let Some(inference_id) = candidate.inference_record_id.as_deref() {
            // Best-effort: the inference may already be terminal (confirmed
            // or previously rejected) — the candidate verdict still lands.
            let _ = self.reject_record(inference_id, now);
        }
        let reason = reason
            .unwrap_or("rejected: evidence does not support this conclusion")
            .to_string();
        self.set_candidate_status(
            candidate_id,
            CandidateStatus::Rejected,
            Some(&reason),
            None,
            now,
        )?;
        self.get_candidate(candidate_id)?
            .ok_or_else(|| ContextError::Decode("candidate vanished".to_string()))
    }

    /// Expire candidates past their TTL. Historical evidence is untouched;
    /// only the unevaluated hypothesis lapses (re-detection revives it).
    pub fn expire_learning_sweep(&self, now: u64) -> Result<usize, ContextError> {
        self.with_conn(|conn| {
            let updated = conn.execute(
                "UPDATE learning_candidates SET status = ?1, updated_at = ?2
                 WHERE status IN ('candidate', 'deferred', 'evaluating')
                   AND expires_at IS NOT NULL AND expires_at < ?3",
                params![CandidateStatus::Expired.as_str(), now as i64, now as i64],
            )?;
            Ok(updated)
        })
    }

    /// Full learning pass: sweep expiry, detect, evaluate each touched
    /// candidate. Per-candidate failures are collected, never fatal:
    /// learning is secondary; history stays durable regardless.
    pub fn run_learning(
        &self,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
        scope: LearnScope,
        now: u64,
    ) -> Result<LearningRunOutcome, ContextError> {
        let _ = self.expire_learning_sweep(now);
        let proposed = self.propose_candidates(workspace_root, task_id, scope, now)?;
        let mut outcome = LearningRunOutcome {
            proposed: proposed.len(),
            ..Default::default()
        };
        for candidate in proposed {
            // User verdicts and expiry stand: re-detection refuses to
            // rewrite rejected / superseded / expired rows, so skip them.
            // Accepted rows re-evaluate (knowledge evolves with evidence).
            let status: CandidateStatus = candidate
                .status
                .parse()
                .unwrap_or(CandidateStatus::Candidate);
            match status {
                CandidateStatus::Rejected
                | CandidateStatus::Superseded
                | CandidateStatus::Expired => {
                    outcome.skipped_terminal += 1;
                    continue;
                }
                _ => {}
            }
            let was_accepted = status == CandidateStatus::Accepted;
            match self.evaluate_candidate(&candidate.candidate_id, now) {
                Ok(done) => match done.status.as_str() {
                    "accepted" => {
                        outcome.accepted += 1;
                        if was_accepted {
                            outcome.refreshed += 1;
                        }
                    }
                    "rejected" => outcome.rejected += 1,
                    _ => outcome.deferred += 1,
                },
                Err(e) => {
                    outcome
                        .failures
                        .push(format!("{}: {e}", candidate.candidate_id));
                }
            }
        }
        Ok(outcome)
    }
}

/// Summary of one [`ContextStore::run_learning`] pass.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LearningRunOutcome {
    pub proposed: usize,
    pub accepted: usize,
    pub deferred: usize,
    pub rejected: usize,
    pub skipped_terminal: usize,
    /// Accepted candidates that were already accepted and re-persisted
    /// (evidence evolved; inference refreshed via supersede).
    #[serde(default)]
    pub refreshed: usize,
    /// Per-candidate errors (collected, never fatal).
    #[serde(default)]
    pub failures: Vec<String>,
}

/// Refuse sensitive inference topics anywhere they appear (belt over the
/// suspenders beside [`topic_allowed`]: propositions and namespaces are
/// checked before any candidate row is written).
pub fn contains_sensitive_content(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    SENSITIVE_TOPICS.iter().any(|s| {
        lower
            .split(|c: char| !c.is_alphanumeric())
            .any(|tok| tok == *s)
    })
}

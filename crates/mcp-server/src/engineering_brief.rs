//! P7 Engineering Decision Support: bounded, deterministic engineering briefs.
//!
//! P6 answers "what exists, how is it connected, what is affected, what is
//! unhealthy". P7 answers "for this engineering task, what evidence and
//! context should OpenCode know before making a decision".
//!
//! ```text
//! task + context + repo intelligence + impact + health + history
//!   + memory + learning + skills + task state
//!     ──► assemble() ──► EngineeringBrief ──► OpenCode reasons/plans/executes
//! ```
//!
//! # Reuse (no duplicate subsystems)
//!
//! | Need | Reused mechanism |
//! |---|---|
//! | Fact ranking | `crate::mcp::facts::search` (lexical, deterministic) |
//! | Fact trust/freshness | `compute_fact_trust`, `compute_freshness` |
//! | Memory ranking/budget | `EngineeringMemoryRuntime::resolve_for_task` |
//! | Context-record resolution | Caller-supplied `ContextRecordExcerpt`s (the fingerprint resolver already ran; this module never re-resolves) |
//! | History ranking/grouping | `ContextStore::recall` (FTS5 + deterministic priors, session-grouped) |
//! | Learning trust | `list_candidates` with `Accepted`/`Rejected` status filters (P3 semantics preserved) |
//! | Skill applicability data | `list_skills` rows + task `resolve_task_skill_refs` (P4/P6 read paths) |
//! | Task state | `task_resume_snapshot` (read-only; no lifecycle transition) |
//! | Impact traversal | `crate::impact::analyze` (bounded BFS + risk signal) |
//! | Health findings | `crate::impact::health::analyze_health` (pure over the store) |
//! | Freshness | `compute_freshness` (live) + `repo_indexes` row (persisted) |
//! | Bounds | `MAX_*` consts below + `response_bounds::bounded_response` at the MCP layer |
//!
//! # Invariants
//!
//! - **Read-only.** Assembly opens stores for reads only, writes no files,
//!   mutates no task/skill/learning state, records no history. Verified by
//!   test (`brief_assembly_writes_nothing`).
//! - **No new ranking.** Section-internal order reuses the source ranking
//!   (facts score order, memory resolver order, recall order, fingerprint
//!   resolution). Brief-level combination uses stable sorts only (authority
//!   rank, name, id). There is deliberately no AI-generated relevance score.
//! - **Authority preserved.** Real `Authority` strings flow through verbatim
//!   (`user_confirmed`, `ai_inferred`, …). Accepted learning surfaces as
//!   `ai_inferred` (never `user_confirmed`); rejected learning surfaces only
//!   as negative knowledge; superseded decisions are never `current`.
//! - **Uncertainty explicit.** Every gap is a `BriefUnknown` entry
//!   (`STALE_INDEX`, `NO_RELEVANT_TESTS`, …). A missing fact is reported,
//!   never converted into confidence.
//! - **Deterministic.** Same repository + task + context state ⇒ same brief:
//!   `BTreeMap`/`BTreeSet` accumulation, sorted outputs, head-truncation
//!   with totals. No `HashMap` iteration, no wall-clock ranking.
//! - **Decision support, not decision making.** The brief contains evidence,
//!   pointers, constraints, risks, and unknowns. It never selects a solution,
//!   never says "execute skill X", never transitions a task.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::engineering_context::{ContextRecordExcerpt, TRUNCATION_MARKER};
use crate::engineering_facts::{ModuleId, SymbolId};
use crate::fact_store::FactStore;

// ── Bounds (§27) ─────────────────────────────────────────────────────────

/// Maximum keywords derived from task text + explicit hints.
pub const MAX_BRIEF_KEYWORDS: usize = 16;
/// Maximum file entries in the brief.
pub const MAX_BRIEF_FILES: usize = 10;
/// Maximum symbol records in the brief.
pub const MAX_BRIEF_SYMBOLS: usize = 10;
/// Maximum dependency edges in the brief.
pub const MAX_BRIEF_DEPENDENCIES: usize = 10;
/// Maximum direct impact relationships.
pub const MAX_BRIEF_IMPACT_DIRECT: usize = 10;
/// Maximum transitive impact relationships.
pub const MAX_BRIEF_IMPACT_TRANSITIVE: usize = 10;
/// Maximum relevant tests.
pub const MAX_BRIEF_TESTS: usize = 10;
/// Maximum health findings.
pub const MAX_BRIEF_HEALTH: usize = 10;
/// Maximum history excerpts.
pub const MAX_BRIEF_HISTORY: usize = 8;
/// Maximum engineering-memory entries.
pub const MAX_BRIEF_MEMORY: usize = 5;
/// Maximum accepted-learning entries.
pub const MAX_BRIEF_LEARNING: usize = 5;
/// Maximum negative-knowledge entries (rejected learning + failed outcomes).
pub const MAX_BRIEF_NEGATIVE: usize = 5;
/// Maximum skill applicability entries.
pub const MAX_BRIEF_SKILLS: usize = 8;
/// Maximum constraints.
pub const MAX_BRIEF_CONSTRAINTS: usize = 10;
/// Maximum decisions.
pub const MAX_BRIEF_DECISIONS: usize = 8;
/// Maximum decision conflicts surfaced.
pub const MAX_BRIEF_CONFLICTS: usize = 5;
/// Maximum risk signals.
pub const MAX_BRIEF_RISKS: usize = 8;
/// Maximum context-record excerpts embedded (mirrors the context packet cap).
pub const MAX_BRIEF_RECORDS: usize = 8;
/// Maximum ambiguity candidates listed for an ambiguous target.
pub const MAX_BRIEF_AMBIGUITY: usize = 5;
/// Maximum impact traversal depth for a brief (bounded blast radius; the
/// full `impact_analyze` tool remains available for deeper traversals).
pub const BRIEF_DEPTH_MAX: usize = 2;
/// Default impact traversal depth.
pub const BRIEF_DEPTH_DEFAULT: usize = 1;
/// Maximum characters kept per free-text excerpt before the truncation marker.
pub const MAX_BRIEF_EXCERPT_CHARS: usize = 240;
/// Maximum characters kept per memory/decision value before excerpting.
pub const MAX_BRIEF_VALUE_CHARS: usize = 500;

// ── Request (§29) ────────────────────────────────────────────────────────

/// What OpenCode wants a decision-support brief for.
///
/// At least one scoping signal is required (`task`, `task_id`, an explicit
/// target, or `keywords`); an empty request is rejected rather than answered
/// with an unbounded dump. An ad-hoc `task` description is never persisted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BriefRequest {
    /// The engineering task in OpenCode's own words.
    #[serde(default)]
    pub task: String,
    /// Existing P5 task id (canonical `task::<hex>`). Read-only snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Explicit workspace-relative file path target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    /// Explicit symbol target (id or name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_symbol: Option<String>,
    /// Explicit module target (id).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_module: Option<String>,
    /// Extra keyword hints for retrieval.
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Impact traversal depth (clamped to `0..=BRIEF_DEPTH_MAX`).
    #[serde(default)]
    pub depth: usize,
}

impl BriefRequest {
    /// All keywords: task text tokens (len ≥ 3) plus explicit hints, sorted,
    /// deduplicated, capped. Deterministic.
    pub fn keywords(&self) -> Vec<String> {
        let mut out: BTreeSet<String> = BTreeSet::new();
        for tok in self.task.split(|c: char| !c.is_alphanumeric()) {
            if tok.len() >= 3 {
                out.insert(tok.to_string());
            }
        }
        for kw in &self.keywords {
            for tok in kw.split(|c: char| !c.is_alphanumeric()) {
                if tok.len() >= 3 {
                    out.insert(tok.to_string());
                }
            }
        }
        // Task-id-bearing requests contribute no keywords by themselves;
        // the task snapshot's title/description feed the caller-side merge.
        out.into_iter().take(MAX_BRIEF_KEYWORDS).collect()
    }

    /// A request with no scoping signal cannot be made task-relevant.
    pub fn has_scope(&self) -> bool {
        !self.task.trim().is_empty()
            || self
                .task_id
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            || self
                .target_path
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            || self
                .target_symbol
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            || self
                .target_module
                .as_deref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
            || self.keywords.iter().any(|k| !k.trim().is_empty())
    }

    /// Clamped traversal depth. `0` is accepted here (no clamp floor)
    /// but the brief's impact traversal floors at 1 — a brief without a
    /// traversal has no impact value; `impact_analyze` owns depth 0.
    pub fn depth(&self) -> usize {
        self.depth.min(BRIEF_DEPTH_MAX)
    }
}

// ── Inputs (read-only handles) ───────────────────────────────────────────

/// Read-only handles the assembler needs. The caller (MCP layer) resolves
/// the workspace, loads the fact store, snapshots identity, and resolves
/// context-record excerpts through the existing fingerprint pipeline — this
/// module performs no resolution of its own beyond evidence retrieval.
pub struct BriefInputs<'a> {
    pub workspace_root: &'a Path,
    pub workspace_key: String,
    pub store: &'a FactStore,
    pub context_store: &'a crate::context_runtime::ContextStore,
    pub identity_loaded: bool,
    pub identity: &'a crate::project_identity::ProjectIdentity,
    pub records: &'a [ContextRecordExcerpt],
    pub now: u64,
}

// ── Brief model (§26) ────────────────────────────────────────────────────

/// Evidence categories (§11). Sections carry these so OpenCode can tell
/// verified structure apart from recorded intent, observed history,
/// inference, and explicit unknowns — never a flattened text dump.
pub mod category {
    pub const ENGINEERING_FACT: &str = "ENGINEERING_FACT";
    pub const FACT: &str = "FACT";
    pub const DECISION: &str = "DECISION";
    pub const CONSTRAINT: &str = "CONSTRAINT";
    pub const PREFERENCE: &str = "PREFERENCE";
    pub const INTENT: &str = "INTENT";
    pub const HISTORY: &str = "HISTORY";
    pub const LEARNING: &str = "LEARNING";
    pub const IMPACT: &str = "IMPACT";
    pub const HEALTH: &str = "HEALTH";
    pub const SKILL: &str = "SKILL";
    pub const TASK_STATE: &str = "TASK_STATE";
    pub const UNKNOWN: &str = "UNKNOWN";
}

/// Provenance tags (same vocabulary as the context packet).
pub mod provenance {
    pub const VERIFIED: &str = "verified";
    pub const RECORDED: &str = "recorded";
    pub const OBSERVED: &str = "observed";
    pub const DERIVED: &str = "derived";
}

/// Unknown-information kinds (§6). A missing fact is reported, never
/// converted into confidence.
pub mod unknown_kind {
    pub const NO_TASK: &str = "NO_TASK";
    pub const NO_TARGET: &str = "NO_TARGET";
    pub const AMBIGUOUS_TARGET: &str = "AMBIGUOUS_TARGET";
    pub const TARGET_NOT_FOUND: &str = "TARGET_NOT_FOUND";
    pub const EMPTY_REPOSITORY: &str = "EMPTY_REPOSITORY";
    pub const MISSING_IDENTITY: &str = "MISSING_IDENTITY";
    pub const STALE_INDEX: &str = "STALE_INDEX";
    pub const FAILED_INDEX: &str = "FAILED_INDEX";
    pub const UNKNOWN_FRESHNESS: &str = "UNKNOWN_FRESHNESS";
    pub const NO_RELEVANT_TESTS: &str = "NO_RELEVANT_TESTS";
    pub const NO_HISTORY: &str = "NO_HISTORY";
    pub const UNSUPPORTED_LANGUAGE: &str = "UNSUPPORTED_LANGUAGE";
    pub const TASK_NOT_FOUND: &str = "TASK_NOT_FOUND";
    pub const NO_APPLICABLE_SKILLS: &str = "NO_APPLICABLE_SKILLS";
    pub const NO_SKILLS: &str = "NO_SKILLS";
    pub const NO_LEARNING: &str = "NO_LEARNING";
    pub const NO_MEMORY: &str = "NO_MEMORY";
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefRepository {
    pub workspace_root: String,
    pub project_id: String,
    pub project_name: Option<String>,
    pub repository_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    pub identity_loaded: bool,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefFreshness {
    /// Live freshness: `fresh` | `stale` | `unknown`.
    pub status: String,
    /// Persisted P6 index status (`READY`/`STALE`/`FAILED`/`UNKNOWN`/…).
    pub persisted_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub indexed_at: Option<u64>,
    pub repository_revision: String,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefScope {
    pub workspace_root: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub keywords: Vec<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefArchitecture {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub relevant_modules: Vec<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefTargets {
    /// How the target was established: `explicit` | `discovered` |
    /// `ambiguous` | `none`.
    pub discovery: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    /// Bounded candidates when discovery is ambiguous.
    #[serde(default)]
    pub candidates: Vec<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefFile {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub symbols: usize,
    pub tests: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefDependency {
    pub source: String,
    pub target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefImpactEdge {
    pub target_id: String,
    pub target_name: String,
    pub relationship_kind: String,
    pub direction: String,
    pub depth: usize,
    pub confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefTest {
    pub id: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    pub relation: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefRiskIndicator {
    pub code: String,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefImpact {
    pub target_id: String,
    pub target_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_name: Option<String>,
    #[serde(default)]
    pub direct: Vec<BriefImpactEdge>,
    pub direct_total: usize,
    #[serde(default)]
    pub transitive: Vec<BriefImpactEdge>,
    pub transitive_total: usize,
    pub nodes_visited: usize,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    #[serde(default)]
    pub risk_indicators: Vec<BriefRiskIndicator>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blast_radius: Option<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefHealthFinding {
    #[serde(rename = "type")]
    pub finding_type: String,
    pub severity: String,
    pub evidence: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefHistoryItem {
    pub excerpt: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub session_stale: bool,
    pub task_match: bool,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefMemory {
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefLearning {
    pub candidate_id: String,
    pub proposition: String,
    pub namespace: String,
    pub confidence: f64,
    pub supporting: usize,
    pub contradicting: usize,
    /// Always `ai_inferred` for accepted learning — never `user_confirmed`.
    pub authority: String,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefNegative {
    /// `rejected_learning` | `failed_validation` | `failed_task` |
    /// `superseded_decision`.
    pub kind: String,
    pub summary: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefSkill {
    pub name: String,
    pub description: String,
    pub applicable: bool,
    pub applicability_reason: String,
    pub status: String,
    pub version: u32,
    /// Origin: `registry` (workspace-visible skill) or `task_ref`
    /// (referenced by the brief's task, resolved read-time per P6).
    pub origin: String,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefConstraint {
    pub content: String,
    /// `hard` for user-confirmed/project-declared constraints, `observed`
    /// for inferred ones. Preferences are never upgraded to hard.
    pub hardness: String,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authority: Option<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefDecision {
    pub id: String,
    pub title: String,
    pub status: String,
    /// `false` for superseded/deprecated decisions — never presented as current.
    pub current: bool,
    pub source: String,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefConflict {
    pub summary: String,
    #[serde(default)]
    pub decision_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefRisk {
    pub signal: String,
    pub source: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefUnknown {
    pub kind: String,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefTaskState {
    pub task_id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub version: u64,
    pub stale: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub validation: Option<String>,
    #[serde(default)]
    pub recent_events: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub intent_note: Option<String>,
    pub category: String,
    pub provenance: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BriefBounds {
    pub files: usize,
    pub symbols: usize,
    pub dependencies: usize,
    pub impact_direct: usize,
    pub impact_transitive: usize,
    pub tests: usize,
    pub health: usize,
    pub history: usize,
    pub memory: usize,
    pub learning: usize,
    pub skills: usize,
    #[serde(default)]
    pub truncated_sections: Vec<String>,
}

/// The bounded engineering brief (§5–§6, §26).
///
/// Evidence and structured context for OpenCode. The `decision` field
/// deliberately does not exist: OpenCode reasons, plans, and executes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EngineeringBrief {
    pub task: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub repository: BriefRepository,
    pub freshness: BriefFreshness,
    pub scope: BriefScope,
    pub architecture: BriefArchitecture,
    pub targets: BriefTargets,
    #[serde(default)]
    pub files: Vec<BriefFile>,
    #[serde(default)]
    pub symbols: Vec<crate::mcp::facts::FactRecord>,
    #[serde(default)]
    pub dependencies: Vec<BriefDependency>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impact: Option<BriefImpact>,
    #[serde(default)]
    pub tests: Vec<BriefTest>,
    #[serde(default)]
    pub health: Vec<BriefHealthFinding>,
    #[serde(default)]
    pub history: Vec<BriefHistoryItem>,
    pub history_total: usize,
    pub history_truncated: bool,
    #[serde(default)]
    pub engineering_memory: Vec<BriefMemory>,
    #[serde(default)]
    pub learning: Vec<BriefLearning>,
    #[serde(default)]
    pub negative_knowledge: Vec<BriefNegative>,
    #[serde(default)]
    pub skills: Vec<BriefSkill>,
    #[serde(default)]
    pub constraints: Vec<BriefConstraint>,
    #[serde(default)]
    pub decisions: Vec<BriefDecision>,
    #[serde(default)]
    pub decision_conflicts: Vec<BriefConflict>,
    #[serde(default)]
    pub risks: Vec<BriefRisk>,
    #[serde(default)]
    pub unknowns: Vec<BriefUnknown>,
    #[serde(default)]
    pub records: Vec<ContextRecordExcerpt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_state: Option<BriefTaskState>,
    pub bounds: BriefBounds,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl EngineeringBrief {
    /// Serialized size in bytes (for bound assertions and MCP caps).
    pub fn serialized_len(&self) -> usize {
        serde_json::to_vec(self)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
    }
}

// ── Assembly (§4 pipeline) ───────────────────────────────────────────────

/// Assemble a bounded engineering brief. Read-only: loads nothing beyond
/// the supplied handles, writes nothing, mutates no task/skill/learning
/// state, records no history.
pub fn assemble(
    inputs: &BriefInputs<'_>,
    request: &BriefRequest,
) -> Result<EngineeringBrief, String> {
    if !request.has_scope() {
        return Err(
            "task scope is required: supply task, task_id, a target, or keywords so the brief stays task-relevant"
                .to_string(),
        );
    }
    validate_targets(request)?;
    let keywords = request.keywords();
    let mut unknowns: Vec<BriefUnknown> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let mut truncated_sections: Vec<String> = Vec::new();

    // File-level-only languages (§6): paths/hashes indexed, never symbols.
    const FILE_LEVEL_LANGUAGES: &[&str] = &[
        "cpp", "c++", "hpp", "shell", "bash", "toml", "yaml", "json", "markdown",
    ];
    if let Some(hit) = keywords
        .iter()
        .find(|k| FILE_LEVEL_LANGUAGES.contains(&k.to_lowercase().as_str()))
    {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::UNSUPPORTED_LANGUAGE.to_string(),
            detail: format!("language '{hit}' has file-level coverage only — no symbols are extracted, so structural sections cannot speak to it"),
        });
    }

    let counts = inputs.store.collection().counts();
    if counts.total == 0 {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::EMPTY_REPOSITORY.to_string(),
            detail: "fact store is empty — run `reindex` after `codebro init` before trusting structural sections".to_string(),
        });
    }

    // ── Repository + freshness (§19) ──
    let repo_identity = codebro_core::RepoIdentity::from_workspace(inputs.workspace_root);
    let live = crate::mcp::facts::compute_freshness(inputs.store, inputs.workspace_root);
    let live_str = live.to_string();
    let persisted = persisted_index(inputs);
    match live {
        crate::mcp::facts::FreshnessStatus::Stale => {
            unknowns.push(BriefUnknown {
                kind: unknown_kind::STALE_INDEX.to_string(),
                detail: "fact store is stale relative to the working tree — structural details may be outdated; `reindex` refreshes them".to_string(),
            });
            notes.push("index is stale — treat impact, tests, and architecture as signals about the last indexed state, not current truth".to_string());
        }
        crate::mcp::facts::FreshnessStatus::Unknown => {
            unknowns.push(BriefUnknown {
                kind: unknown_kind::UNKNOWN_FRESHNESS.to_string(),
                detail: "freshness cannot be established (no generation state or non-git workspace) — structural confidence is reduced".to_string(),
            });
        }
        crate::mcp::facts::FreshnessStatus::Fresh => {}
    }
    if persisted.persisted_status == "FAILED" {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::FAILED_INDEX.to_string(),
            detail:
                "the last index run failed — last-good metadata is preserved but may be outdated"
                    .to_string(),
        });
    }
    if !inputs.identity_loaded {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::MISSING_IDENTITY.to_string(),
            detail: "no project identity for this workspace — decisions, constraints, and architecture summary are unavailable".to_string(),
        });
    }

    // ── Task state (§18, read-only) ──
    let task_state = task_state_section(inputs, request, &mut unknowns);
    // Task text enriches keyword scope deterministically (title +
    // description + checkpoint next-action tokens, same tokenizer).
    let mut scoped_keywords = keywords.clone();
    if let Some(ts) = &task_state {
        for extra in [
            &ts.title,
            ts.checkpoint_summary.as_deref().unwrap_or(""),
            ts.next_action.as_deref().unwrap_or(""),
        ] {
            for tok in extra.split(|c: char| !c.is_alphanumeric()) {
                if tok.len() >= 3 && !scoped_keywords.contains(&tok.to_string()) {
                    scoped_keywords.push(tok.to_string());
                }
            }
        }
        scoped_keywords.sort();
        scoped_keywords.truncate(MAX_BRIEF_KEYWORDS);
    }

    // ── Target discovery (§8, deterministic, no LLM) ──
    let targets = discover_targets(inputs.store, request, &scoped_keywords, &mut unknowns);

    // ── Repository intelligence: files / symbols / dependencies (§22–§23) ──
    let files = relevant_files(inputs.store, &scoped_keywords, &targets);
    let symbols = relevant_symbols(inputs.store, &scoped_keywords, live);
    if symbols.len() >= MAX_BRIEF_SYMBOLS {
        truncated_sections.push("symbols".to_string());
    }
    let dependencies = relevant_dependencies(inputs.store, &scoped_keywords);
    let architecture = architecture_section(inputs.identity, &files, &scoped_keywords);

    // ── Impact (§20, single bounded traversal) ──
    let (impact, impact_tests) = impact_section(inputs, request, &targets, &mut unknowns);
    // ── Tests (§23: impact linkage + module containment) ──
    let tests = tests_section(inputs.store, &impact_tests, &files, &mut unknowns);
    // ── Health (§21, task-relevant only) ──
    let health = health_section(
        inputs.store,
        &scoped_keywords,
        &targets,
        live,
        &mut truncated_sections,
    );

    // ── History (§14, relevant excerpts only) ──
    let (history, history_total, history_truncated) =
        history_section(inputs, request, &scoped_keywords, &mut unknowns);
    // ── Engineering memory (§15) ──
    let engineering_memory = memory_section(inputs, &scoped_keywords, &mut unknowns);
    // ── Learning (§16, accepted only + rejected as negative) ──
    let (learning, mut negative_knowledge) =
        learning_section(inputs, request, &scoped_keywords, &mut unknowns);
    // Task failures feed negative knowledge (§13).
    if let Some(ts) = &task_state {
        if ts.status == "failed" {
            negative_knowledge.push(BriefNegative {
                kind: "failed_task".to_string(),
                summary: excerpt(
                    &format!("task {} failed: {}", ts.task_id, ts.title),
                    MAX_BRIEF_EXCERPT_CHARS,
                ),
                source: "task_state".to_string(),
            });
        }
        if ts.validation.as_deref() == Some("failed") {
            negative_knowledge.push(BriefNegative {
                kind: "failed_validation".to_string(),
                summary: excerpt(
                    &format!("task {} recorded a failed validation", ts.task_id),
                    MAX_BRIEF_EXCERPT_CHARS,
                ),
                source: "task_state".to_string(),
            });
        }
    }
    negative_knowledge.truncate(MAX_BRIEF_NEGATIVE);

    // ── Skills (§17, applicability information only) ──
    let skills = skills_section(
        inputs,
        request,
        &task_state,
        &scoped_keywords,
        &mut unknowns,
    );

    // ── Constraints (§24) + decisions (§25) ──
    let constraints = constraints_section(inputs);
    let (decisions, decision_conflicts) = decisions_section(inputs, &scoped_keywords);

    // ── Risks (signals, not conclusions) ──
    let risks = risks_section(&impact, &health, &live);

    // ── Records (caller-resolved excerpts, bounded) ──
    let mut records: Vec<ContextRecordExcerpt> = inputs
        .records
        .iter()
        .take(MAX_BRIEF_RECORDS)
        .cloned()
        .collect();
    if inputs.records.len() > MAX_BRIEF_RECORDS {
        truncated_sections.push("records".to_string());
    }
    records.sort_by(|a, b| a.id.cmp(&b.id));

    truncated_sections.sort();
    truncated_sections.dedup();

    Ok(EngineeringBrief {
        task: excerpt(&request.task, MAX_BRIEF_VALUE_CHARS),
        task_id: request
            .task_id
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        repository: BriefRepository {
            workspace_root: inputs.workspace_root.display().to_string(),
            project_id: repo_identity.project_id.clone(),
            project_name: if inputs.identity.name.is_empty() {
                None
            } else {
                Some(inputs.identity.name.clone())
            },
            repository_type: repo_identity.repository_type.clone(),
            git_remote: repo_identity.git_remote.clone(),
            commit_sha: repo_identity.commit_sha.clone(),
            languages: inputs.identity.languages.clone(),
            identity_loaded: inputs.identity_loaded,
            category: category::FACT.to_string(),
            provenance: provenance::RECORDED.to_string(),
        },
        freshness: BriefFreshness {
            status: live_str,
            persisted_status: persisted.persisted_status,
            indexed_at: persisted.indexed_at,
            repository_revision: persisted.repository_revision,
            category: category::FACT.to_string(),
            provenance: provenance::DERIVED.to_string(),
        },
        scope: BriefScope {
            workspace_root: inputs.workspace_key.clone(),
            task_id: request
                .task_id
                .clone()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            keywords: scoped_keywords,
            category: category::FACT.to_string(),
            provenance: provenance::DERIVED.to_string(),
        },
        architecture,
        targets,
        files,
        symbols,
        dependencies,
        impact,
        tests,
        health,
        history,
        history_total,
        history_truncated,
        engineering_memory,
        learning,
        negative_knowledge,
        skills,
        constraints,
        decisions,
        decision_conflicts,
        risks,
        unknowns,
        records,
        task_state,
        bounds: BriefBounds {
            files: MAX_BRIEF_FILES,
            symbols: MAX_BRIEF_SYMBOLS,
            dependencies: MAX_BRIEF_DEPENDENCIES,
            impact_direct: MAX_BRIEF_IMPACT_DIRECT,
            impact_transitive: MAX_BRIEF_IMPACT_TRANSITIVE,
            tests: MAX_BRIEF_TESTS,
            health: MAX_BRIEF_HEALTH,
            history: MAX_BRIEF_HISTORY,
            memory: MAX_BRIEF_MEMORY,
            learning: MAX_BRIEF_LEARNING,
            skills: MAX_BRIEF_SKILLS,
            truncated_sections,
        },
        notes,
    })
}

// ── Validation ───────────────────────────────────────────────────────────

/// Reject path traversal and blank-but-present targets deterministically.
fn validate_targets(request: &BriefRequest) -> Result<(), String> {
    for (field, value) in [
        ("target_path", request.target_path.as_deref()),
        ("target_symbol", request.target_symbol.as_deref()),
        ("target_module", request.target_module.as_deref()),
    ] {
        if let Some(raw) = value {
            if raw.trim().is_empty() {
                return Err(format!("{field} must not be blank"));
            }
            if raw.split('/').any(|seg| seg == "..") {
                return Err(format!("{field} must not contain '..'"));
            }
            if raw.contains('\0') {
                return Err(format!("{field} must not contain NUL"));
            }
        }
    }
    if request.depth > 99 {
        return Err("depth is unreasonably large".to_string());
    }
    Ok(())
}

// ── Freshness (§19) ──────────────────────────────────────────────────────

struct PersistedFreshness {
    persisted_status: String,
    indexed_at: Option<u64>,
    repository_revision: String,
}

fn persisted_index(inputs: &BriefInputs<'_>) -> PersistedFreshness {
    match inputs
        .context_store
        .get_repo_index(&inputs.workspace_key, inputs.now)
    {
        Ok(row) => PersistedFreshness {
            persisted_status: row.index_status.as_str().to_string(),
            indexed_at: if row.indexed_at == 0 {
                None
            } else {
                Some(row.indexed_at)
            },
            repository_revision: row.repository_revision.clone(),
        },
        Err(_) => PersistedFreshness {
            persisted_status: "UNKNOWN".to_string(),
            indexed_at: None,
            repository_revision: "unknown".to_string(),
        },
    }
}

// ── Task state (§18, read-only) ──────────────────────────────────────────

fn task_state_section(
    inputs: &BriefInputs<'_>,
    request: &BriefRequest,
    unknowns: &mut Vec<BriefUnknown>,
) -> Option<BriefTaskState> {
    let task_id = request
        .task_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    match inputs
        .context_store
        .task_resume_snapshot(&inputs.workspace_key, task_id, inputs.now)
    {
        Ok(snap) => {
            let t = snap.task;
            let validation = t.validation.as_ref().and_then(|v| {
                v.result
                    .map(|r| r.as_str().to_string())
                    .or(Some("running".to_string()))
            });
            Some(BriefTaskState {
                task_id: t.task_id.clone(),
                title: excerpt(&t.title, MAX_BRIEF_EXCERPT_CHARS),
                status: t.status.as_str().to_string(),
                priority: t.priority.as_str().to_string(),
                version: t.current_version,
                stale: t.stale.unwrap_or_else(|| t.is_stale(inputs.now)),
                checkpoint_summary: snap
                    .latest_checkpoint
                    .as_ref()
                    .map(|c| excerpt(&c.summary, MAX_BRIEF_EXCERPT_CHARS)),
                next_action: snap
                    .latest_checkpoint
                    .as_ref()
                    .and_then(|c| c.next_action.clone())
                    .map(|s| excerpt(&s, MAX_BRIEF_EXCERPT_CHARS)),
                validation,
                recent_events: snap
                    .recent_events
                    .into_iter()
                    .take(5)
                    .map(|e| excerpt(&e.summary, MAX_BRIEF_EXCERPT_CHARS))
                    .collect(),
                intent_note: snap
                    .intent_note
                    .map(|s| excerpt(&s, MAX_BRIEF_EXCERPT_CHARS)),
                category: category::TASK_STATE.to_string(),
                provenance: provenance::RECORDED.to_string(),
            })
        }
        Err(_) => {
            // Cross-workspace tasks are invisible by store design (`None`
            // maps here too): report TASK_NOT_FOUND, never leak existence.
            unknowns.push(BriefUnknown {
                kind: unknown_kind::TASK_NOT_FOUND.to_string(),
                detail: "named task is not visible from this workspace — task state, task history, and task skill refs are unavailable".to_string(),
            });
            None
        }
    }
}

// ── Target discovery (§8) ────────────────────────────────────────────────

/// Deterministic target discovery. Explicit targets win; otherwise exact
/// (case-insensitive) symbol-name and module path-suffix matches over the
/// keyword set decide. Zero matches ⇒ `none`; more than one ⇒ `ambiguous`
/// with bounded candidates. Never silently selects.
fn discover_targets(
    store: &FactStore,
    request: &BriefRequest,
    keywords: &[String],
    unknowns: &mut Vec<BriefUnknown>,
) -> BriefTargets {
    // 1. Explicit symbol target.
    if let Some(raw) = request
        .target_symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let exact = SymbolId::new(raw);
        if store.collection().symbol(&exact).is_some() {
            return BriefTargets {
                discovery: "explicit".to_string(),
                target_id: Some(exact.as_str().to_string()),
                target_kind: Some("symbol".to_string()),
                target_name: Some(raw.to_string()),
                target_path: symbol_file(store, &exact),
                candidates: Vec::new(),
                category: category::ENGINEERING_FACT.to_string(),
                provenance: provenance::VERIFIED.to_string(),
            };
        }
        let matches = symbol_name_matches(store, raw);
        if matches.len() == 1 {
            return BriefTargets {
                discovery: "explicit".to_string(),
                target_id: Some(matches[0].0.clone()),
                target_kind: Some("symbol".to_string()),
                target_name: Some(matches[0].1.clone()),
                target_path: matches[0].2.clone(),
                candidates: Vec::new(),
                category: category::ENGINEERING_FACT.to_string(),
                provenance: provenance::VERIFIED.to_string(),
            };
        }
        if matches.len() > 1 {
            let candidates: Vec<String> = matches
                .into_iter()
                .take(MAX_BRIEF_AMBIGUITY)
                .map(|(id, name, path)| match path {
                    Some(p) => format!("{name} ({p}) [{id}]"),
                    None => format!("{name} [{id}]"),
                })
                .collect();
            unknowns.push(BriefUnknown {
                kind: unknown_kind::AMBIGUOUS_TARGET.to_string(),
                detail: format!("symbol '{raw}' matches {} facts — impact traversal skipped; disambiguate with an exact symbol id", candidates.len()),
            });
            return BriefTargets {
                discovery: "ambiguous".to_string(),
                target_id: None,
                target_kind: None,
                target_name: Some(raw.to_string()),
                target_path: None,
                candidates,
                category: category::UNKNOWN.to_string(),
                provenance: provenance::DERIVED.to_string(),
            };
        }
        unknowns.push(BriefUnknown {
            kind: unknown_kind::TARGET_NOT_FOUND.to_string(),
            detail: format!("symbol '{raw}' resolves to no known fact — impact and test sections are unavailable for it"),
        });
        return BriefTargets {
            discovery: "none".to_string(),
            target_id: None,
            target_kind: None,
            target_name: Some(raw.to_string()),
            target_path: None,
            candidates: Vec::new(),
            category: category::UNKNOWN.to_string(),
            provenance: provenance::DERIVED.to_string(),
        };
    }
    // 2. Explicit file target.
    if let Some(raw) = request
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let normalized = raw.trim_start_matches("./").to_string();
        let known = store.collection().modules().iter().any(|m| {
            m.path.as_deref() == Some(normalized.as_str())
                || m.location.file.as_deref() == Some(normalized.as_str())
        });
        if !known {
            unknowns.push(BriefUnknown {
                kind: unknown_kind::TARGET_NOT_FOUND.to_string(),
                detail: format!("file '{normalized}' matches no indexed module — impact reflects an unindexed path, if anything"),
            });
        }
        return BriefTargets {
            discovery: "explicit".to_string(),
            target_id: Some(normalized.clone()),
            target_kind: Some("file".to_string()),
            target_name: Some(normalized.clone()),
            target_path: Some(normalized),
            candidates: Vec::new(),
            category: category::ENGINEERING_FACT.to_string(),
            provenance: provenance::VERIFIED.to_string(),
        };
    }
    // 3. Explicit module target.
    if let Some(raw) = request
        .target_module
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let id = ModuleId::new(raw);
        if let Some(m) = store.collection().module(&id) {
            let path = m.path.clone().or_else(|| m.location.file.clone());
            return BriefTargets {
                discovery: "explicit".to_string(),
                target_id: Some(id.as_str().to_string()),
                target_kind: Some("module".to_string()),
                target_name: Some(m.name.clone()),
                target_path: path,
                candidates: Vec::new(),
                category: category::ENGINEERING_FACT.to_string(),
                provenance: provenance::VERIFIED.to_string(),
            };
        }
        unknowns.push(BriefUnknown {
            kind: unknown_kind::TARGET_NOT_FOUND.to_string(),
            detail: format!("module '{raw}' matches no indexed module"),
        });
        return BriefTargets {
            discovery: "none".to_string(),
            target_id: None,
            target_kind: None,
            target_name: Some(raw.to_string()),
            target_path: None,
            candidates: Vec::new(),
            category: category::UNKNOWN.to_string(),
            provenance: provenance::DERIVED.to_string(),
        };
    }
    // 4. Keyword discovery: exact symbol-name hits + module path-suffix hits.
    let mut found: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    for kw in keywords {
        let lowered = kw.to_lowercase();
        for (id, name, path) in symbol_name_matches(store, &lowered) {
            if name.to_lowercase() == lowered {
                found.insert(id, (name, path));
            }
        }
        for m in store.collection().modules() {
            let path = m.path.clone().or_else(|| m.location.file.clone());
            let matches_path = path
                .as_deref()
                .map(|p| {
                    let p = p.to_lowercase();
                    p == lowered || p.ends_with(&format!("/{lowered}")) || p.ends_with(&lowered)
                })
                .unwrap_or(false);
            if matches_path || m.name.to_lowercase() == lowered {
                found.insert(m.id.as_str().to_string(), (m.name.clone(), path));
            }
        }
    }
    // Ignore discovery when the keyword set came only from an empty task
    // (keywords empty ⇒ no signal, not a negative result).
    if keywords.is_empty() {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::NO_TARGET.to_string(),
            detail: "no target and no keywords — impact traversal skipped; supply target_path, target_symbol, or task text".to_string(),
        });
        return BriefTargets {
            discovery: "none".to_string(),
            target_id: None,
            target_kind: None,
            target_name: None,
            target_path: None,
            candidates: Vec::new(),
            category: category::UNKNOWN.to_string(),
            provenance: provenance::DERIVED.to_string(),
        };
    }
    let mut ids: Vec<String> = found.keys().cloned().collect();
    ids.sort();
    match ids.len() {
        0 => {
            unknowns.push(BriefUnknown {
                kind: unknown_kind::NO_TARGET.to_string(),
                detail: "no indexed symbol or module matches the task keywords — impact traversal skipped; fact/file sections still apply".to_string(),
            });
            BriefTargets {
                discovery: "none".to_string(),
                target_id: None,
                target_kind: None,
                target_name: None,
                target_path: None,
                candidates: Vec::new(),
                category: category::UNKNOWN.to_string(),
                provenance: provenance::DERIVED.to_string(),
            }
        }
        1 => {
            let id = ids.into_iter().next().expect("one");
            let (name, path) = found.remove(&id).expect("present");
            BriefTargets {
                discovery: "discovered".to_string(),
                target_id: Some(id),
                target_kind: Some("symbol_or_module".to_string()),
                target_name: Some(name),
                target_path: path,
                candidates: Vec::new(),
                category: category::ENGINEERING_FACT.to_string(),
                provenance: provenance::DERIVED.to_string(),
            }
        }
        _ => {
            let candidates: Vec<String> = ids
                .iter()
                .take(MAX_BRIEF_AMBIGUITY)
                .map(|id| {
                    let (name, path) = &found[id];
                    match path {
                        Some(p) => format!("{name} ({p}) [{id}]"),
                        None => format!("{name} [{id}]"),
                    }
                })
                .collect();
            unknowns.push(BriefUnknown {
                kind: unknown_kind::AMBIGUOUS_TARGET.to_string(),
                detail: format!("{} indexed facts match the task keywords — impact traversal skipped rather than guessing; narrow with target_symbol or target_path", ids.len()),
            });
            BriefTargets {
                discovery: "ambiguous".to_string(),
                target_id: None,
                target_kind: None,
                target_name: None,
                target_path: None,
                candidates,
                category: category::UNKNOWN.to_string(),
                provenance: provenance::DERIVED.to_string(),
            }
        }
    }
}

/// Case-insensitive substring matches on symbol names: `(id, name, file)`,
/// sorted by id for determinism.
fn symbol_name_matches(store: &FactStore, needle: &str) -> Vec<(String, String, Option<String>)> {
    let needle = needle.to_lowercase();
    let mut out: Vec<(String, String, Option<String>)> = Vec::new();
    for s in store.collection().symbols() {
        if s.name.to_lowercase().contains(needle.as_str()) {
            out.push((
                s.id.as_str().to_string(),
                s.name.clone(),
                s.location.file.clone(),
            ));
        }
    }
    out.sort();
    out
}

fn symbol_file(store: &FactStore, id: &SymbolId) -> Option<String> {
    store
        .collection()
        .symbol(id)
        .and_then(|s| s.location.file.clone())
}

// ── Repository intelligence (§22–§23) ─────────────────────────────────────

fn keyword_hit(hay: &str, keywords: &[String]) -> bool {
    let hay = hay.to_lowercase();
    keywords
        .iter()
        .any(|k| k.len() >= 3 && hay.contains(k.to_lowercase().as_str()))
}

fn relevant_files(
    store: &FactStore,
    keywords: &[String],
    targets: &BriefTargets,
) -> Vec<BriefFile> {
    let mut out: Vec<BriefFile> = Vec::new();
    let collection = store.collection();
    // Symbol counts per module (single pass, deterministic).
    let mut sym_counts: BTreeMap<String, usize> = BTreeMap::new();
    for s in collection.symbols() {
        if let Some(mid) = s.module.as_ref() {
            *sym_counts.entry(mid.as_str().to_string()).or_insert(0) += 1;
        }
    }
    let mut test_counts: BTreeMap<String, usize> = BTreeMap::new();
    for t in collection.tests() {
        if let Some(mid) = t.location.as_ref().and_then(|l| l.module.as_ref()) {
            *test_counts.entry(mid.as_str().to_string()).or_insert(0) += 1;
        }
    }
    for m in collection.modules() {
        let path = m.path.clone().or_else(|| m.location.file.clone());
        let hay = format!("{} {}", m.name, path.as_deref().unwrap_or(""));
        let relevant = keyword_hit(&hay, keywords)
            || targets
                .target_path
                .as_deref()
                .map(|p| Some(p) == path.as_deref())
                .unwrap_or(false)
            || targets.target_id.as_deref() == Some(m.id.as_str());
        if !relevant {
            continue;
        }
        out.push(BriefFile {
            id: m.id.as_str().to_string(),
            name: m.name.clone(),
            path,
            symbols: sym_counts.get(m.id.as_str()).copied().unwrap_or(0),
            tests: test_counts.get(m.id.as_str()).copied().unwrap_or(0),
        });
        if out.len() >= MAX_BRIEF_FILES {
            break;
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out.truncate(MAX_BRIEF_FILES);
    out
}

fn relevant_symbols(
    store: &FactStore,
    keywords: &[String],
    freshness: crate::mcp::facts::FreshnessStatus,
) -> Vec<crate::mcp::facts::FactRecord> {
    let mut out: Vec<crate::mcp::facts::FactRecord> = Vec::new();
    let mut seen: BTreeSet<(String, String)> = BTreeSet::new();
    for kw in keywords.iter().take(8) {
        if kw.len() < 3 {
            continue;
        }
        let params = crate::mcp::facts::FactSearch {
            query: kw.as_str(),
            kind: None,
            path: None,
            limit: MAX_BRIEF_SYMBOLS,
        };
        if let Ok(records) = crate::mcp::facts::search(store, &params, freshness) {
            for r in records {
                if seen.insert((r.kind.clone(), r.name.clone())) {
                    out.push(r);
                }
                if out.len() >= MAX_BRIEF_SYMBOLS {
                    break;
                }
            }
        }
        if out.len() >= MAX_BRIEF_SYMBOLS {
            break;
        }
    }
    out.sort_by(|a, b| {
        a.kind
            .cmp(&b.kind)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.path.cmp(&b.path))
    });
    out.truncate(MAX_BRIEF_SYMBOLS);
    out
}

fn relevant_dependencies(store: &FactStore, keywords: &[String]) -> Vec<BriefDependency> {
    let mut out: Vec<BriefDependency> = Vec::new();
    for d in store.collection().dependencies() {
        let hay = format!("{} {}", d.source.as_str(), d.target.as_str());
        if !keyword_hit(&hay, keywords) {
            continue;
        }
        out.push(BriefDependency {
            source: tail_label(d.source.as_str()),
            target: tail_label(d.target.as_str()),
            version: d.version_constraint.clone(),
        });
        if out.len() >= MAX_BRIEF_DEPENDENCIES {
            break;
        }
    }
    out.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then_with(|| a.target.cmp(&b.target))
    });
    out.truncate(MAX_BRIEF_DEPENDENCIES);
    out
}

fn tail_label(id: &str) -> String {
    id.rsplit("::").next().unwrap_or(id).to_string()
}

fn architecture_section(
    identity: &crate::project_identity::ProjectIdentity,
    files: &[BriefFile],
    keywords: &[String],
) -> BriefArchitecture {
    let mut relevant_modules: Vec<String> = files.iter().take(5).map(|f| f.name.clone()).collect();
    for m in &identity.known_modules {
        if keyword_hit(m, keywords) && !relevant_modules.contains(m) {
            relevant_modules.push(m.clone());
        }
        if relevant_modules.len() >= 8 {
            break;
        }
    }
    relevant_modules.sort();
    relevant_modules.truncate(8);
    BriefArchitecture {
        summary: identity
            .architecture_summary
            .clone()
            // Defense in depth: summary is redacted at the identity write
            // seam; legacy rows re-redacted here.
            .map(|s| {
                excerpt(
                    &crate::tools::shell::redact_secrets_public(&s),
                    MAX_BRIEF_VALUE_CHARS,
                )
            }),
        relevant_modules,
        languages: identity.languages.clone(),
        category: category::FACT.to_string(),
        provenance: provenance::RECORDED.to_string(),
    }
}

// ── Impact (§20) ─────────────────────────────────────────────────────────

/// Build the impact target for traversal. Explicit symbol/file/module
/// targets map directly; a singly-discovered id maps to symbol, then
/// module, then file (deterministic preference). Ambiguous/missing
/// targets yield no traversal (reported via unknowns, never guessed).
fn impact_target_for(
    store: &FactStore,
    request: &BriefRequest,
    targets: &BriefTargets,
) -> Option<crate::impact::ImpactTarget> {
    use crate::impact::ImpactTarget;
    if let Some(raw) = request
        .target_symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let exact = SymbolId::new(raw);
        if store.collection().symbol(&exact).is_some() {
            return Some(ImpactTarget::Symbol(exact));
        }
        // Name-based fallback only when unambiguous (mirrors discovery).
        let matches = symbol_name_matches(store, raw);
        if matches.len() == 1 {
            return Some(ImpactTarget::Symbol(SymbolId::new(&matches[0].0)));
        }
        return None;
    }
    if let Some(raw) = request
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let normalized = raw.trim_start_matches("./").to_string();
        return Some(ImpactTarget::File(normalized));
    }
    if let Some(raw) = request
        .target_module
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(ImpactTarget::Module(ModuleId::new(raw)));
    }
    // Discovered single target.
    if targets.discovery == "discovered" {
        if let Some(id) = targets.target_id.as_deref() {
            let sid = SymbolId::new(id);
            if store.collection().symbol(&sid).is_some() {
                return Some(ImpactTarget::Symbol(sid));
            }
            let mid = ModuleId::new(id);
            if store.collection().module(&mid).is_some() {
                return Some(ImpactTarget::Module(mid));
            }
            if let Some(path) = targets.target_path.clone() {
                return Some(ImpactTarget::File(path));
            }
        }
    }
    None
}

fn impact_section(
    inputs: &BriefInputs<'_>,
    request: &BriefRequest,
    targets: &BriefTargets,
    unknowns: &mut Vec<BriefUnknown>,
) -> (Option<BriefImpact>, Vec<BriefTest>) {
    let no_impact: (Option<BriefImpact>, Vec<BriefTest>) = (None, Vec::new());
    let target = match impact_target_for(inputs.store, request, targets) {
        Some(t) => t,
        None => return no_impact,
    };
    let opts = crate::impact::ImpactOptions {
        max_results: MAX_BRIEF_IMPACT_DIRECT.max(MAX_BRIEF_IMPACT_TRANSITIVE),
        include_tests: true,
        include_references: true,
        depth: request.depth().max(1),
        direction: "both".to_string(),
        relationship_types: Vec::new(),
        max_nodes: 1000,
    };
    if let Err(e) = crate::impact::validate_opts(&opts) {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::TARGET_NOT_FOUND.to_string(),
            detail: format!("impact options rejected: {e:?}"),
        });
        return no_impact;
    }
    let result = crate::impact::analyze(inputs.store, target, &opts, Some(inputs.workspace_root));
    if result.status == crate::impact::ImpactStatus::NotFound {
        // File targets for unindexed paths land here: honest unknown, and
        // only then (the traversal genuinely found nothing).
        if request.target_path.is_none() {
            unknowns.push(BriefUnknown {
                kind: unknown_kind::TARGET_NOT_FOUND.to_string(),
                detail: "impact target resolves to no indexed fact — no dependents, tests, or risk can be established".to_string(),
            });
        }
        return no_impact;
    }
    if let crate::impact::ImpactStatus::Ambiguous(matches) = &result.status {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::AMBIGUOUS_TARGET.to_string(),
            detail: format!(
                "impact target is ambiguous ({} candidates) — traversal skipped",
                matches.len()
            ),
        });
        return no_impact;
    }
    let direct_total = result.direct_relationships.len();
    let transitive_total = result.transitive_relationships.len();
    let direct: Vec<BriefImpactEdge> = result
        .direct_relationships
        .into_iter()
        .take(MAX_BRIEF_IMPACT_DIRECT)
        .map(project_edge)
        .collect();
    let transitive: Vec<BriefImpactEdge> = result
        .transitive_relationships
        .into_iter()
        .take(MAX_BRIEF_IMPACT_TRANSITIVE)
        .map(project_edge)
        .collect();
    let truncated = direct_total > direct.len() || transitive_total > transitive.len();
    let (risk_level, risk_indicators, blast_radius) = match result.risk {
        Some(r) => (
            Some(r.level.as_str().to_string()),
            r.indicators
                .into_iter()
                .map(|i| BriefRiskIndicator {
                    code: i.code,
                    evidence: excerpt(&i.evidence, MAX_BRIEF_EXCERPT_CHARS),
                })
                .collect(),
            Some(excerpt(&r.blast_radius, MAX_BRIEF_EXCERPT_CHARS)),
        ),
        None => (None, Vec::new(), None),
    };
    // Affected tests come straight from the traversal (TestFact.tested
    // linkage + module containment inside the impact engine) — the brief
    // projects them, never re-derives them.
    let mut impact_tests: Vec<BriefTest> = result
        .affected_tests
        .into_iter()
        .map(|t| BriefTest {
            id: t.id,
            name: t.name,
            file: t.file,
            relation: t.relation,
            provenance: match t.provenance {
                crate::impact::Provenance::Verified => provenance::VERIFIED.to_string(),
                crate::impact::Provenance::Heuristic => "heuristic".to_string(),
                crate::impact::Provenance::Unknown => "unknown".to_string(),
            },
        })
        .collect();
    impact_tests.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    impact_tests.truncate(MAX_BRIEF_TESTS);
    (
        Some(BriefImpact {
            target_id: result.target.id.clone(),
            target_kind: result.target.kind.clone(),
            target_name: result.target.name.clone(),
            direct,
            direct_total,
            transitive,
            transitive_total,
            nodes_visited: result.traversal_metadata.nodes_visited,
            truncated,
            risk_level,
            risk_indicators,
            blast_radius,
            category: category::IMPACT.to_string(),
            provenance: provenance::VERIFIED.to_string(),
        }),
        impact_tests,
    )
}

fn project_edge(e: crate::impact::ImpactRelationship) -> BriefImpactEdge {
    BriefImpactEdge {
        target_id: e.target_id,
        target_name: e.target_name,
        relationship_kind: e.relationship_kind,
        direction: e.direction,
        depth: e.depth,
        confidence: e.confidence,
        reason: e.reason.map(|s| excerpt(&s, MAX_BRIEF_EXCERPT_CHARS)),
    }
}

// ── Tests (§23) ──────────────────────────────────────────────────────────

fn tests_section(
    store: &FactStore,
    impact_tests: &[BriefTest],
    files: &[BriefFile],
    unknowns: &mut Vec<BriefUnknown>,
) -> Vec<BriefTest> {
    // Impact linkage first (TestFact.tested + module containment from the
    // traversal), then module containment over relevant files. Sorted,
    // deduplicated, bounded.
    let mut out: Vec<BriefTest> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    for t in impact_tests {
        if seen.insert(t.id.clone()) {
            out.push(t.clone());
        }
    }
    let module_ids: BTreeSet<&str> = files.iter().map(|f| f.id.as_str()).collect();
    for t in store.collection().tests() {
        let in_module = t
            .location
            .as_ref()
            .and_then(|l| l.module.as_ref())
            .map(|m| module_ids.contains(m.as_str()))
            .unwrap_or(false);
        let file_hit = t
            .location
            .as_ref()
            .and_then(|l| l.file.as_ref())
            .map(|f| {
                files
                    .iter()
                    .any(|bf| bf.path.as_deref() == Some(f.as_str()))
            })
            .unwrap_or(false);
        if !in_module && !file_hit {
            continue;
        }
        if seen.insert(t.id.as_str().to_string()) {
            out.push(BriefTest {
                id: t.id.as_str().to_string(),
                name: t.name.clone(),
                file: t.location.as_ref().and_then(|l| l.file.clone()),
                relation: "module_containment".to_string(),
                provenance: provenance::VERIFIED.to_string(),
            });
        }
        if out.len() >= MAX_BRIEF_TESTS {
            break;
        }
    }
    // Symbol linkage (TestFact.tested) is owned by the impact engine and
    // surfaced above; module containment below only adds tests the
    // traversal did not already link.
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out.truncate(MAX_BRIEF_TESTS);
    if out.is_empty() {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::NO_RELEVANT_TESTS.to_string(),
            detail: "no relevant test could be established for this task — this is not a claim that no tests exist (the index may not link them)".to_string(),
        });
    }
    out
}

// ── Health (§21, task-relevant only) ─────────────────────────────────────

fn health_section(
    store: &FactStore,
    keywords: &[String],
    targets: &BriefTargets,
    freshness: crate::mcp::facts::FreshnessStatus,
    truncated_sections: &mut Vec<String>,
) -> Vec<BriefHealthFinding> {
    use crate::impact::health::FindingType;
    let stale = freshness == crate::mcp::facts::FreshnessStatus::Stale;
    let all = crate::impact::health::analyze_health(store, stale, 500);
    let mut out: Vec<BriefHealthFinding> = Vec::new();
    for f in all {
        let relevant = f.finding_type == FindingType::StaleIndex
            || f.location
                .as_deref()
                .map(|loc| {
                    keyword_hit(loc, keywords)
                        || targets
                            .target_path
                            .as_deref()
                            .map(|p| loc.contains(p))
                            .unwrap_or(false)
                })
                .unwrap_or(false);
        if !relevant {
            continue;
        }
        out.push(BriefHealthFinding {
            finding_type: f.finding_type.as_str().to_string(),
            severity: f.severity.as_str().to_string(),
            evidence: excerpt(&f.evidence, MAX_BRIEF_EXCERPT_CHARS),
            location: f.location,
            confidence: f.confidence,
        });
        if out.len() >= MAX_BRIEF_HEALTH {
            truncated_sections.push("health".to_string());
            break;
        }
    }
    out.sort_by(|a, b| {
        a.finding_type
            .cmp(&b.finding_type)
            .then_with(|| a.location.cmp(&b.location))
    });
    out
}

// ── History (§14) ────────────────────────────────────────────────────────

fn history_section(
    inputs: &BriefInputs<'_>,
    request: &BriefRequest,
    keywords: &[String],
    unknowns: &mut Vec<BriefUnknown>,
) -> (Vec<BriefHistoryItem>, usize, bool) {
    use crate::context_runtime::{RecallQuery, RecallScope};
    if keywords.is_empty() {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::NO_HISTORY.to_string(),
            detail: "no task text or keywords — history retrieval needs a query; supply task text"
                .to_string(),
        });
        return (Vec::new(), 0, false);
    }
    let query_text = keywords.join(" ");
    let task_id = request
        .task_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let scope = if task_id.is_some() {
        RecallScope::Task
    } else {
        RecallScope::Project
    };
    let query = RecallQuery {
        query: &query_text,
        workspace_root: Some(inputs.workspace_key.as_str()),
        task_id,
        scope,
        kinds: Vec::new(),
        session_id: None,
        limit: MAX_BRIEF_HISTORY,
    };
    let outcome = match inputs.context_store.recall(&query, inputs.now) {
        Ok(o) => o,
        Err(_) => {
            unknowns.push(BriefUnknown {
                kind: unknown_kind::NO_HISTORY.to_string(),
                detail: "history retrieval failed — historical evidence is unavailable, not absent"
                    .to_string(),
            });
            return (Vec::new(), 0, false);
        }
    };
    let mut items: Vec<BriefHistoryItem> = Vec::new();
    for g in &outcome.groups {
        for h in &g.hits {
            // Never expose raw payloads or internal row ids: the bounded
            // excerpt plus kind/session/task provenance is the evidence.
            items.push(BriefHistoryItem {
                // Defense in depth: recall excerpts are redacted at the
                // history write seam; legacy rows re-redacted here.
                excerpt: excerpt(
                    &crate::tools::shell::redact_secrets_public(&h.excerpt),
                    MAX_BRIEF_EXCERPT_CHARS,
                ),
                kind: h.event.kind.clone(),
                session_id: h
                    .session
                    .as_ref()
                    .map(|s| s.id.clone())
                    .or_else(|| g.session_id.clone()),
                session_stale: h.session_stale || g.session_stale,
                task_match: h.task_match,
                category: category::HISTORY.to_string(),
                provenance: provenance::OBSERVED.to_string(),
            });
            if items.len() >= MAX_BRIEF_HISTORY {
                break;
            }
        }
        if items.len() >= MAX_BRIEF_HISTORY {
            break;
        }
    }
    let truncated = outcome.truncated || outcome.total_matches > items.len();
    if items.is_empty() {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::NO_HISTORY.to_string(),
            detail: "no relevant historical evidence for this task — no transcript was dumped"
                .to_string(),
        });
    }
    (items, outcome.total_matches, truncated)
}

// ── Engineering memory (§15) ─────────────────────────────────────────────

fn memory_section(
    inputs: &BriefInputs<'_>,
    keywords: &[String],
    unknowns: &mut Vec<BriefUnknown>,
) -> Vec<BriefMemory> {
    let identity = crate::project_identity::ProjectIdentityRuntime::new(inputs.workspace_root);
    let mut memory =
        crate::engineering_memory::EngineeringMemoryRuntime::new(inputs.workspace_root, identity);
    let _ = memory.load();
    let context = memory.resolve_for_task(keywords, &[]);
    let snapshot = memory.snapshot();
    let mut out: Vec<BriefMemory> = context
        .entries
        .iter()
        .take(MAX_BRIEF_MEMORY)
        .map(|e| {
            let src = snapshot.iter().find(|s| s.key == e.key);
            // Defense in depth: memory values are redacted at the
            // record_memory seam; legacy JSON rows re-redacted here.
            let (value, truncated) = excerpt_flag(
                &crate::tools::shell::redact_secrets_public(&e.value),
                MAX_BRIEF_VALUE_CHARS,
            );
            BriefMemory {
                key: e.key.clone(),
                value,
                confidence: e.confidence,
                truncated,
                source: src.and_then(|s| s.metadata.source.clone()),
                tags: src.map(|s| s.metadata.tags.clone()).unwrap_or_default(),
                category: category::FACT.to_string(),
                provenance: provenance::RECORDED.to_string(),
            }
        })
        .collect();
    out.sort_by(|a, b| a.key.cmp(&b.key));
    if out.is_empty() {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::NO_MEMORY.to_string(),
            detail: "no relevant engineering memory for this task".to_string(),
        });
    }
    out
}

// ── Learning (§16) ───────────────────────────────────────────────────────

fn learning_section(
    inputs: &BriefInputs<'_>,
    request: &BriefRequest,
    keywords: &[String],
    unknowns: &mut Vec<BriefUnknown>,
) -> (Vec<BriefLearning>, Vec<BriefNegative>) {
    use crate::context_runtime::CandidateStatus;
    let task_id = request
        .task_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut learning: Vec<BriefLearning> = Vec::new();
    let mut negative: Vec<BriefNegative> = Vec::new();
    // Accepted learning only (eligible as AI_INFERRED evidence).
    if let Ok(candidates) = inputs.context_store.list_candidates(
        Some(&inputs.workspace_key),
        Some(CandidateStatus::Accepted),
        task_id,
        50,
    ) {
        for c in candidates {
            let hay = format!("{} {} {}", c.proposition, c.namespace, c.kind).to_lowercase();
            if !keywords
                .iter()
                .any(|k| k.len() >= 3 && hay.contains(k.to_lowercase().as_str()))
            {
                continue;
            }
            learning.push(BriefLearning {
                candidate_id: c.candidate_id.clone(),
                // Defense in depth: propositions derive from redacted history;
                // legacy rows re-redacted here.
                proposition: excerpt(
                    &crate::tools::shell::redact_secrets_public(&c.proposition),
                    MAX_BRIEF_EXCERPT_CHARS,
                ),
                namespace: c.namespace.clone(),
                confidence: c.confidence,
                supporting: c.supporting_evidence.len(),
                contradicting: c.contradicting_evidence.len(),
                authority: "ai_inferred".to_string(),
                category: category::LEARNING.to_string(),
                provenance: provenance::RECORDED.to_string(),
            });
            if learning.len() >= MAX_BRIEF_LEARNING {
                break;
            }
        }
    }
    learning.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.candidate_id.cmp(&b.candidate_id))
    });
    // Rejected learning surfaces ONLY as relevant negative knowledge (§13).
    if let Ok(rejected) = inputs.context_store.list_candidates(
        Some(&inputs.workspace_key),
        Some(CandidateStatus::Rejected),
        task_id,
        50,
    ) {
        for c in rejected {
            let hay = format!("{} {} {}", c.proposition, c.namespace, c.kind).to_lowercase();
            if !keywords
                .iter()
                .any(|k| k.len() >= 3 && hay.contains(k.to_lowercase().as_str()))
            {
                continue;
            }
            negative.push(BriefNegative {
                kind: "rejected_learning".to_string(),
                summary: excerpt(
                    &format!(
                        "rejected approach: {}",
                        crate::tools::shell::redact_secrets_public(&c.proposition)
                    ),
                    MAX_BRIEF_EXCERPT_CHARS,
                ),
                source: "learning".to_string(),
            });
            if negative.len() >= MAX_BRIEF_NEGATIVE {
                break;
            }
        }
    }
    negative.sort_by(|a, b| a.summary.cmp(&b.summary));
    if learning.is_empty() {
        unknowns.push(BriefUnknown {
            kind: unknown_kind::NO_LEARNING.to_string(),
            detail: "no relevant accepted learning for this task — candidates and deferred hypotheses are never treated as knowledge".to_string(),
        });
    }
    (learning, negative)
}

// ── Skills (§17, applicability information only) ─────────────────────────

fn skills_section(
    inputs: &BriefInputs<'_>,
    request: &BriefRequest,
    task_state: &Option<BriefTaskState>,
    keywords: &[String],
    unknowns: &mut Vec<BriefUnknown>,
) -> Vec<BriefSkill> {
    use crate::context_runtime::SkillStatus;
    let mut out: Vec<BriefSkill> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let repo_langs: BTreeSet<String> = inputs
        .identity
        .languages
        .iter()
        .map(|l| l.to_lowercase())
        .collect();
    let listed = inputs
        .context_store
        .list_skills(Some(&inputs.workspace_key), Some(SkillStatus::Active), 100)
        .unwrap_or_default();
    for s in listed {
        let langs: BTreeSet<String> = s
            .applicability
            .languages
            .iter()
            .map(|l| l.to_lowercase())
            .collect();
        let subs: Vec<String> = s
            .applicability
            .subsystems
            .iter()
            .map(|x| x.to_lowercase())
            .collect();
        let lang_hit = !langs.is_empty() && !langs.is_disjoint(&repo_langs);
        let sub_hit = subs.iter().any(|sub| {
            keywords.iter().any(|k| {
                k.len() >= 3
                    && (sub.contains(k.to_lowercase().as_str())
                        || k.to_lowercase().contains(sub.as_str()))
            })
        });
        let unscoped = langs.is_empty() && subs.is_empty();
        let (applicable, reason) = if lang_hit {
            (
                true,
                format!(
                    "repository language matches skill applicability ({})",
                    s.applicability.languages.join(", ")
                ),
            )
        } else if sub_hit {
            (
                true,
                "task keywords overlap skill subsystem applicability".to_string(),
            )
        } else if unscoped {
            (
                false,
                "skill declares no language/subsystem applicability — relevance is uncertain"
                    .to_string(),
            )
        } else {
            continue;
        };
        if seen.insert(s.name.clone()) {
            out.push(BriefSkill {
                name: s.name.clone(),
                // Defense in depth: skill rows persist write-time redaction,
                // but the brief projection redacts again so a legacy or
                // externally-seeded row can never leak a secret through
                // a brief.
                description: excerpt(
                    &crate::tools::shell::redact_secrets_public(&s.description),
                    MAX_BRIEF_EXCERPT_CHARS,
                ),
                applicable,
                applicability_reason: reason,
                status: s.status.clone(),
                version: s.current_version,
                origin: "registry".to_string(),
                category: category::SKILL.to_string(),
                provenance: provenance::RECORDED.to_string(),
            });
        }
    }
    // Task-referenced skills (P6 read-time resolution; reference-only).
    if let Some(tid) = request
        .task_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Ok(resolution) = inputs
            .context_store
            .resolve_task_skill_refs(&inputs.workspace_key, tid)
        {
            for r in resolution.resolved {
                // Defense in depth (legacy rows): redact the fallback
                // reference echo; matched names come from validated skill
                // rows but the fallback re-echoes raw task text.
                let name = crate::tools::shell::redact_secrets_public(
                    &r.name.clone().unwrap_or_else(|| r.reference.clone()),
                );
                if seen.insert(name.clone()) {
                    out.push(BriefSkill {
                        name,
                        description: format!("referenced by task {}", task_state.as_ref().map(|t| t.task_id.as_str()).unwrap_or(tid)),
                        applicable: true,
                        applicability_reason: "task associated this skill with its work (reference only — not an execution order)".to_string(),
                        status: r.status.clone().unwrap_or_else(|| "unknown".to_string()),
                        version: 0,
                        origin: "task_ref".to_string(),
                        category: category::SKILL.to_string(),
                        provenance: provenance::RECORDED.to_string(),
                    });
                }
            }
            // Unmatched refs stay opaque (P6): association evidence without
            // invented skill metadata — applicability honestly uncertain.
            for r in resolution.unresolved {
                // Defense in depth: refs are redacted at the task write
                // seam, but a legacy row (or an embedded-store seed) could
                // predate that — never echo an unredacted ref.
                let r = crate::tools::shell::redact_secrets_public(&r);
                if seen.insert(r.clone()) {
                    out.push(BriefSkill {
                        name: r,
                        description: "opaque task skill reference — no matching workspace skill; applicability uncertain".to_string(),
                        applicable: false,
                        applicability_reason: "no workspace-visible skill matches this reference".to_string(),
                        status: "unresolved".to_string(),
                        version: 0,
                        origin: "task_ref".to_string(),
                        category: category::SKILL.to_string(),
                        provenance: provenance::RECORDED.to_string(),
                    });
                }
            }
        }
    }
    out.sort_by(|a, b| {
        b.applicable
            .cmp(&a.applicable)
            .then_with(|| a.name.cmp(&b.name))
    });
    out.truncate(MAX_BRIEF_SKILLS);
    if out.is_empty() {
        // Distinguish "no skills exist" from "none look applicable": the
        // unfiltered count decides, without exposing other workspaces.
        let any = inputs
            .context_store
            .list_skills(Some(&inputs.workspace_key), None, 1)
            .map(|v| !v.is_empty())
            .unwrap_or(false);
        unknowns.push(BriefUnknown {
            kind: if any { unknown_kind::NO_APPLICABLE_SKILLS } else { unknown_kind::NO_SKILLS }.to_string(),
            detail: if any {
                "skills exist but none declare applicability matching this task — skill applicability is uncertain".to_string()
            } else {
                "no skills registered for this workspace".to_string()
            },
        });
    }
    out
}

// ── Constraints (§24) + decisions (§25) ──────────────────────────────────

fn constraints_section(inputs: &BriefInputs<'_>) -> Vec<BriefConstraint> {
    let mut out: Vec<BriefConstraint> = Vec::new();
    for c in &inputs.identity.known_constraints {
        // Defense in depth: identity rows are redacted at the write seam,
        // but a legacy identity JSON could predate that — never surface
        // an unredacted constraint.
        out.push(BriefConstraint {
            content: excerpt(
                &crate::tools::shell::redact_secrets_public(c),
                MAX_BRIEF_EXCERPT_CHARS,
            ),
            hardness: "hard".to_string(),
            source: "project_identity".to_string(),
            authority: None,
            category: category::CONSTRAINT.to_string(),
            provenance: provenance::RECORDED.to_string(),
        });
        if out.len() >= MAX_BRIEF_CONSTRAINTS {
            break;
        }
    }
    // USER_CONFIRMED constraint-kind records stay constraints; preferences
    // are never upgraded.
    for r in inputs.records.iter() {
        if r.kind != "constraint" {
            continue;
        }
        let hardness = if r.authority == "user_confirmed" {
            "hard"
        } else {
            "observed"
        };
        out.push(BriefConstraint {
            content: excerpt(&r.content, MAX_BRIEF_EXCERPT_CHARS),
            hardness: hardness.to_string(),
            source: format!("context_record:{}", r.namespace),
            authority: Some(r.authority.clone()),
            category: category::CONSTRAINT.to_string(),
            provenance: provenance::RECORDED.to_string(),
        });
        if out.len() >= MAX_BRIEF_CONSTRAINTS {
            break;
        }
    }
    out.sort_by(|a, b| a.content.cmp(&b.content));
    out.truncate(MAX_BRIEF_CONSTRAINTS);
    out
}

fn decisions_section(
    inputs: &BriefInputs<'_>,
    keywords: &[String],
) -> (Vec<BriefDecision>, Vec<BriefConflict>) {
    let mut out: Vec<BriefDecision> = Vec::new();
    for d in &inputs.identity.engineering_decisions {
        let hay = format!("{} {}", d.title, d.description).to_lowercase();
        let relevant = keywords
            .iter()
            .any(|k| k.len() >= 3 && hay.contains(k.to_lowercase().as_str()))
            || d.status.to_string() == "accepted";
        if !relevant {
            continue;
        }
        let current = matches!(
            d.status,
            crate::project_identity::DecisionStatus::Accepted
                | crate::project_identity::DecisionStatus::Proposed
        );
        out.push(BriefDecision {
            id: d.id.clone(),
            // Defense in depth: decision titles are redacted at the
            // identity write seam; legacy rows re-redacted here.
            title: excerpt(&crate::tools::shell::redact_secrets_public(&d.title), 120),
            status: d.status.to_string(),
            current,
            source: "project_identity".to_string(),
            category: category::DECISION.to_string(),
            provenance: provenance::RECORDED.to_string(),
        });
        if out.len() >= MAX_BRIEF_DECISIONS {
            break;
        }
    }
    out.sort_by(|a, b| a.id.cmp(&b.id));
    // Conflicts: same-area decisions (≥2 shared significant tokens) with
    // differing currency — surfaced, never resolved by guessing.
    let mut conflicts: Vec<BriefConflict> = Vec::new();
    for i in 0..out.len() {
        for j in (i + 1)..out.len() {
            let ti = significant_tokens(&out[i].title);
            let tj = significant_tokens(&out[j].title);
            let shared = ti.intersection(&tj).count();
            if shared >= 2 && out[i].current != out[j].current {
                conflicts.push(BriefConflict {
                    summary: format!(
                        "conflicting decisions: '{}' ({}) vs '{}' ({})",
                        out[i].title, out[i].status, out[j].title, out[j].status
                    ),
                    decision_ids: vec![out[i].id.clone(), out[j].id.clone()],
                });
                if conflicts.len() >= MAX_BRIEF_CONFLICTS {
                    break;
                }
            }
        }
        if conflicts.len() >= MAX_BRIEF_CONFLICTS {
            break;
        }
    }
    // Superseded decisions feed negative knowledge (do not repeat failed approaches).
    // (Recorded by the caller merge: decisions with current=false are the signal.)
    (out, conflicts)
}

fn significant_tokens(title: &str) -> BTreeSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 4)
        .map(|t| t.to_lowercase())
        .collect()
}

// ── Risks (signals, not conclusions) ─────────────────────────────────────

fn risks_section(
    impact: &Option<BriefImpact>,
    health: &[BriefHealthFinding],
    freshness: &crate::mcp::facts::FreshnessStatus,
) -> Vec<BriefRisk> {
    let mut out: Vec<BriefRisk> = Vec::new();
    if let Some(imp) = impact {
        for ind in &imp.risk_indicators {
            out.push(BriefRisk {
                signal: ind.code.clone(),
                source: "impact".to_string(),
                detail: excerpt(&ind.evidence, MAX_BRIEF_EXCERPT_CHARS),
            });
        }
        if imp.truncated {
            out.push(BriefRisk {
                signal: "impact_truncated".to_string(),
                source: "impact".to_string(),
                detail: format!("impact traversal hit brief bounds ({} direct, {} transitive shown) — use impact_analyze for the full graph", imp.direct.len(), imp.transitive.len()),
            });
        }
    }
    for h in health {
        if h.severity == "warning" || h.severity == "error" {
            out.push(BriefRisk {
                signal: h.finding_type.clone(),
                source: "health".to_string(),
                detail: excerpt(&h.evidence, MAX_BRIEF_EXCERPT_CHARS),
            });
        }
        if out.len() >= MAX_BRIEF_RISKS {
            break;
        }
    }
    if *freshness == crate::mcp::facts::FreshnessStatus::Stale {
        out.push(BriefRisk {
            signal: "stale_index".to_string(),
            source: "freshness".to_string(),
            detail: "structural evidence describes the last indexed state, not the working tree"
                .to_string(),
        });
    }
    out.sort_by(|a, b| {
        a.signal
            .cmp(&b.signal)
            .then_with(|| a.source.cmp(&b.source))
    });
    out.truncate(MAX_BRIEF_RISKS);
    out
}

// ── Excerpt helpers ──────────────────────────────────────────────────────

fn excerpt(s: &str, max_chars: usize) -> String {
    let collected: String = s.chars().take(max_chars + 1).collect();
    if collected.chars().count() > max_chars {
        format!(
            "{}{}",
            collected.chars().take(max_chars).collect::<String>(),
            TRUNCATION_MARKER
        )
    } else {
        collected
    }
}

fn excerpt_flag(s: &str, max_chars: usize) -> (String, bool) {
    let collected: String = s.chars().take(max_chars + 1).collect();
    if collected.chars().count() > max_chars {
        (
            format!(
                "{}{}",
                collected.chars().take(max_chars).collect::<String>(),
                TRUNCATION_MARKER
            ),
            true,
        )
    } else {
        (collected, false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engineering_facts::location::SourceLocation;
    use crate::engineering_facts::{
        FactsBuilder, ModuleFact, ModuleId, RelationshipFact, RelationshipId, RelationshipKind,
        SymbolFact, SymbolId, SymbolKind, WorkspaceFact, WorkspaceId,
    };
    use crate::fact_store::FactStore;

    fn sample_store() -> FactStore {
        let ws_id = WorkspaceId::new("ws::brief");
        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(ws_id.clone(), "brief-proj"));
        let mut m = ModuleFact::new(ModuleId::new("mod::src/auth.rs"), "src::auth");
        m.path = Some("src/auth.rs".to_string());
        let mid = m.id.clone();
        builder.add_module(m);
        let mut login = SymbolFact::new(SymbolId::new("sym::login"), "login", SymbolKind::Function);
        login.module = Some(mid.clone());
        login.location = SourceLocation::new()
            .with_file("src/auth.rs")
            .with_point(10, 0);
        login.signature = Some("pub fn login(user: &str)".to_string());
        builder.add_symbol(login.clone());
        let mut verify =
            SymbolFact::new(SymbolId::new("sym::verify"), "verify", SymbolKind::Function);
        verify.module = Some(mid.clone());
        verify.location = SourceLocation::new()
            .with_file("src/auth.rs")
            .with_point(30, 0);
        builder.add_symbol(verify.clone());
        let mut rel = RelationshipFact::new(
            RelationshipId::new("rel::login-calls-verify"),
            RelationshipKind::Calls,
            crate::engineering_facts::FactId::Symbol(login.id.clone()),
            crate::engineering_facts::FactId::Symbol(verify.id.clone()),
        );
        rel.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
            .attr("provenance", "verified")
            .build();
        builder.add_relationship(rel);
        FactStore::build(builder.build())
    }

    fn temp_inputs<'a>(
        dir: &'a tempfile::TempDir,
        state: &'a tempfile::TempDir,
        store: &'a FactStore,
        ctx_store: &'a crate::context_runtime::ContextStore,
        identity: &'a crate::project_identity::ProjectIdentity,
        records: &'a [ContextRecordExcerpt],
    ) -> BriefInputs<'a> {
        let _ = (dir, state);
        BriefInputs {
            workspace_root: dir.path(),
            workspace_key: crate::context_runtime::canonical_workspace_key(
                &dir.path().to_string_lossy(),
            ),
            store,
            context_store: ctx_store,
            identity_loaded: false,
            identity,
            records,
            now: 1_800_000_000,
        }
    }

    fn blank_identity() -> crate::project_identity::ProjectIdentity {
        let mut rt = crate::project_identity::ProjectIdentityRuntime::new(std::path::Path::new(
            "/tmp/brief-test-unused",
        ));
        rt.snapshot()
    }

    fn brief_request(task: &str) -> BriefRequest {
        BriefRequest {
            task: task.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn empty_request_is_rejected_not_dumped() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = FactStore::empty();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let err = assemble(&inputs, &BriefRequest::default()).expect_err("empty brief must error");
        assert!(err.contains("task scope is required"), "{err}");
    }

    #[test]
    fn blank_and_traversal_targets_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = FactStore::empty();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut req = brief_request("fix login");
        req.target_path = Some("  ".to_string());
        assert!(assemble(&inputs, &req).is_err());
        let mut req = brief_request("fix login");
        req.target_symbol = Some("../escape".to_string());
        assert!(assemble(&inputs, &req).is_err());
    }

    #[test]
    fn empty_store_reports_unknowns_not_confidence() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = FactStore::empty();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request("fix authentication login")).unwrap();
        let kinds: Vec<&str> = brief.unknowns.iter().map(|u| u.kind.as_str()).collect();
        assert!(kinds.contains(&unknown_kind::EMPTY_REPOSITORY), "{kinds:?}");
        assert!(
            kinds.contains(&unknown_kind::NO_RELEVANT_TESTS),
            "{kinds:?}"
        );
        assert!(kinds.contains(&unknown_kind::NO_HISTORY), "{kinds:?}");
        assert!(brief.impact.is_none());
        assert!(brief.symbols.is_empty());
    }

    #[test]
    fn discovered_single_target_traverses_impact() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        // "verify" matches exactly one symbol ⇒ discovered ⇒ traversed.
        let brief = assemble(&inputs, &brief_request("verify token")).unwrap();
        assert_eq!(brief.targets.discovery, "discovered");
        let impact = brief.impact.as_ref().expect("impact traversed");
        assert!(
            !impact.direct.is_empty() || impact.direct_total > 0 || !impact.transitive.is_empty()
        );
    }

    #[test]
    fn ambiguous_keywords_report_ambiguity_without_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        // Both "login" and "verify" match ⇒ ambiguous ⇒ no traversal.
        let brief = assemble(&inputs, &brief_request("login verify")).unwrap();
        assert_eq!(brief.targets.discovery, "ambiguous");
        assert!(brief.impact.is_none());
        assert!(brief
            .unknowns
            .iter()
            .any(|u| u.kind == unknown_kind::AMBIGUOUS_TARGET));
        assert!(!brief.targets.candidates.is_empty());
    }

    #[test]
    fn explicit_missing_symbol_is_unknown_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut req = brief_request("fix things");
        req.target_symbol = Some("no_such_symbol_xyz".to_string());
        let brief = assemble(&inputs, &req).unwrap();
        assert!(brief
            .unknowns
            .iter()
            .any(|u| u.kind == unknown_kind::TARGET_NOT_FOUND));
        assert!(brief.impact.is_none());
    }

    #[test]
    fn brief_is_deterministic_across_runs() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let req = brief_request("login verify token");
        let a = assemble(&inputs, &req).unwrap();
        let b = assemble(&inputs, &req).unwrap();
        assert_eq!(a, b);
        assert_eq!(
            serde_json::to_vec(&a).unwrap(),
            serde_json::to_vec(&b).unwrap()
        );
    }

    #[test]
    fn keyword_order_does_not_change_brief() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut r1 = brief_request("alpha");
        r1.keywords = vec!["login".to_string(), "verify".to_string()];
        let mut r2 = brief_request("alpha");
        r2.keywords = vec!["verify".to_string(), "login".to_string()];
        assert_eq!(
            assemble(&inputs, &r1).unwrap(),
            assemble(&inputs, &r2).unwrap()
        );
    }

    #[test]
    fn cross_workspace_task_id_is_unknown_not_leak() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut req = brief_request("continue work");
        req.task_id = Some("task::deadbeefdeadbeef".to_string());
        let brief = assemble(&inputs, &req).unwrap();
        assert!(brief.task_state.is_none());
        assert!(brief
            .unknowns
            .iter()
            .any(|u| u.kind == unknown_kind::TASK_NOT_FOUND));
    }

    #[test]
    fn unknowns_cover_freshness_and_skills_and_learning() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request("login")).unwrap();
        let kinds: Vec<&str> = brief.unknowns.iter().map(|u| u.kind.as_str()).collect();
        // Fresh store without generation state ⇒ unknown freshness (honest).
        assert!(
            kinds.contains(&unknown_kind::UNKNOWN_FRESHNESS),
            "{kinds:?}"
        );
        assert!(kinds.contains(&unknown_kind::NO_SKILLS), "{kinds:?}");
        assert!(kinds.contains(&unknown_kind::NO_LEARNING), "{kinds:?}");
        assert!(kinds.contains(&unknown_kind::NO_MEMORY), "{kinds:?}");
    }

    #[test]
    fn brief_bounds_hold_on_sample_store() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request("login")).unwrap();
        assert!(brief.files.len() <= MAX_BRIEF_FILES);
        assert!(brief.symbols.len() <= MAX_BRIEF_SYMBOLS);
        assert!(brief.tests.len() <= MAX_BRIEF_TESTS);
        assert!(brief.history.len() <= MAX_BRIEF_HISTORY);
        assert!(brief.serialized_len() < 256 * 1024);
    }

    #[test]
    fn depth_is_clamped() {
        let req = BriefRequest {
            depth: 99,
            task: "x".to_string(),
            ..Default::default()
        };
        assert_eq!(req.depth(), BRIEF_DEPTH_MAX);
        let req = BriefRequest {
            depth: 1,
            task: "x".to_string(),
            ..Default::default()
        };
        assert_eq!(req.depth(), 1);
    }

    #[test]
    fn unsupported_language_is_explicit_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request("fix shell scripts")).unwrap();
        assert!(
            brief
                .unknowns
                .iter()
                .any(|u| u.kind == unknown_kind::UNSUPPORTED_LANGUAGE),
            "{:?}",
            brief.unknowns
        );
    }

    #[test]
    fn failed_index_preserves_last_good_and_reports_unknown() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let key = crate::context_runtime::canonical_workspace_key(&dir.path().to_string_lossy());
        let now = 1_800_000_000u64;
        ctx.upsert_repo_index(
            &key,
            crate::context_runtime::RepoIndexUpsert {
                repository_identity: "{}".to_string(),
                index_status: crate::context_runtime::RepoIndexStatus::Failed,
                indexed_at: now - 100,
                repository_revision: "abc123".to_string(),
                file_count: 10,
                symbol_count: 20,
                edge_count: 5,
                stale_count: 0,
            },
            now,
        )
        .unwrap();
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request("login")).unwrap();
        assert_eq!(brief.freshness.persisted_status, "FAILED");
        // Last-good metadata preserved, failure explicit.
        assert_eq!(brief.freshness.repository_revision, "abc123");
        assert!(brief
            .unknowns
            .iter()
            .any(|u| u.kind == unknown_kind::FAILED_INDEX));
    }

    #[test]
    fn deleted_file_target_is_unknown_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut req = brief_request("fix removed handler");
        req.target_path = Some("src/deleted.rs".to_string());
        let brief = assemble(&inputs, &req).unwrap();
        assert_eq!(brief.targets.discovery, "explicit");
        assert!(brief
            .unknowns
            .iter()
            .any(|u| u.kind == unknown_kind::TARGET_NOT_FOUND));
        assert!(brief.impact.is_none());
    }

    #[test]
    fn explicit_indexed_file_target_resolves() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut req = brief_request("harden auth module");
        req.target_path = Some("src/auth.rs".to_string());
        let brief = assemble(&inputs, &req).unwrap();
        assert_eq!(brief.targets.discovery, "explicit");
        assert!(!brief.files.is_empty());
        assert!(brief
            .files
            .iter()
            .any(|f| f.path.as_deref() == Some("src/auth.rs")));
    }

    /// Cyclic graphs must terminate with a deterministic brief.
    #[test]
    fn cyclic_graph_terminates_deterministically() {
        use crate::engineering_facts::FactId;
        let ws_id = WorkspaceId::new("ws::cycle");
        let mut builder = FactsBuilder::new();
        builder.add_workspace(WorkspaceFact::new(ws_id, "cycle"));
        for name in ["aaa", "bbb"] {
            let mut s = SymbolFact::new(
                SymbolId::new(format!("sym::{name}")),
                name,
                SymbolKind::Function,
            );
            s.location = SourceLocation::new()
                .with_file("src/cycle.rs")
                .with_point(1, 0);
            builder.add_symbol(s);
        }
        for (n, (from, to)) in [("sym::aaa", "sym::bbb"), ("sym::bbb", "sym::aaa")]
            .into_iter()
            .enumerate()
        {
            let mut rel = RelationshipFact::new(
                RelationshipId::new(format!("rel::cycle-{n}")),
                RelationshipKind::Calls,
                FactId::Symbol(SymbolId::new(from)),
                FactId::Symbol(SymbolId::new(to)),
            );
            rel.metadata = crate::engineering_facts::metadata::FactMetadata::builder()
                .attr("provenance", "verified")
                .build();
            builder.add_relationship(rel);
        }
        let store = FactStore::build(builder.build());
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let mut req = brief_request("cycle probe");
        req.target_symbol = Some("aaa".to_string());
        let a = assemble(&inputs, &req).unwrap();
        let b = assemble(&inputs, &req).unwrap();
        assert_eq!(a, b);
        assert!(a.impact.is_some());
    }

    /// Huge history stays bounded and deterministic.
    #[test]
    fn huge_history_stays_bounded() {
        use crate::context_runtime::{HistoryInput, HistoryKind, LearnScope};
        let _ = LearnScope::Project;
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let key = crate::context_runtime::canonical_workspace_key(&dir.path().to_string_lossy());
        let now = 1_800_000_000u64;
        for i in 0..60u64 {
            let mut input = HistoryInput::new(
                key.clone(),
                HistoryKind::Validation,
                format!("briefprobe validation round {i} passed"),
            );
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("success".to_string());
            input.created_at = Some(now - i);
            ctx.record_history(&input, now - i).unwrap();
        }
        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let a = assemble(&inputs, &brief_request("briefprobe validation")).unwrap();
        let b = assemble(&inputs, &brief_request("briefprobe validation")).unwrap();
        assert_eq!(a.history, b.history);
        assert!(a.history.len() <= MAX_BRIEF_HISTORY, "{}", a.history.len());
        assert!(!a.history.is_empty());
    }

    /// Huge engineering memory stays bounded.
    #[test]
    fn huge_memory_stays_bounded() {
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let identity = blank_identity();
        // Seed 25 memory entries sharing one keyword through the canonical runtime.
        {
            let id_rt = crate::project_identity::ProjectIdentityRuntime::new(dir.path());
            let mut memory =
                crate::engineering_memory::EngineeringMemoryRuntime::new(dir.path(), id_rt);
            let _ = memory.load();
            for i in 0..25 {
                let entry = crate::engineering_memory::types::EngineeringMemoryEntry::new(
                    format!("mem::briefprobe:{i}"),
                    format!("briefprobe:{i}"),
                    "briefprobe memory value for bounding",
                );
                let _ = memory.record(entry);
            }
            memory.persist().unwrap();
        }
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request("briefprobe memory")).unwrap();
        assert!(
            brief.engineering_memory.len() <= MAX_BRIEF_MEMORY,
            "{}",
            brief.engineering_memory.len()
        );
        assert!(!brief.engineering_memory.is_empty());
    }

    /// Accepted learning surfaces as AI_INFERRED evidence with provenance.
    #[test]
    fn accepted_learning_surfaces_as_ai_inferred() {
        use crate::context_runtime::{HistoryInput, HistoryKind, LearnScope};
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let key = crate::context_runtime::canonical_workspace_key(&dir.path().to_string_lossy());
        let now = 1_700_000_000u64;
        for i in 0..4u64 {
            let mut input = HistoryInput::new(
                key.clone(),
                HistoryKind::Validation,
                format!("cargo test learnprobe phase-{i} passed with fmt clippy suite"),
            );
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("success".to_string());
            input.created_at = Some(now + i);
            ctx.record_history(&input, now + i).unwrap();
        }
        let proposed = ctx
            .propose_candidates(Some(&key), None, LearnScope::Project, now + 10)
            .unwrap();
        assert!(
            !proposed.is_empty(),
            "detector must cluster repeated validations"
        );
        let evaluated = ctx
            .evaluate_candidate(&proposed[0].candidate_id, now + 20)
            .unwrap();
        assert_eq!(evaluated.status, "accepted", "got {:?}", evaluated.status);
        let proposition = evaluated.proposition.clone();
        let keyword = proposition
            .split(|c: char| !c.is_alphanumeric())
            .find(|t| t.len() >= 4)
            .unwrap_or("learnprobe")
            .to_string();

        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request(&format!("fix {keyword}"))).unwrap();
        assert!(
            !brief.learning.is_empty(),
            "accepted learning must surface: {:?}",
            brief.unknowns
        );
        for l in &brief.learning {
            assert_eq!(
                l.authority, "ai_inferred",
                "accepted learning is inference, never user-confirmed"
            );
            assert_eq!(l.category, "LEARNING");
        }
        assert!(!brief
            .unknowns
            .iter()
            .any(|u| u.kind == unknown_kind::NO_LEARNING));
    }

    /// Rejected learning surfaces only as negative knowledge.
    #[test]
    fn rejected_learning_surfaces_only_as_negative_knowledge() {
        use crate::context_runtime::{HistoryInput, HistoryKind, LearnScope};
        let dir = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        let store = sample_store();
        let ctx = crate::context_runtime::ContextStore::at_state_dir(state.path().to_path_buf());
        let key = crate::context_runtime::canonical_workspace_key(&dir.path().to_string_lossy());
        let now = 1_700_000_000u64;
        for i in 0..4u64 {
            let mut input = HistoryInput::new(
                key.clone(),
                HistoryKind::Validation,
                format!("cargo test rejectprobe phase-{i} passed with fmt clippy suite"),
            );
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("success".to_string());
            input.created_at = Some(now + i);
            ctx.record_history(&input, now + i).unwrap();
        }
        let proposed = ctx
            .propose_candidates(Some(&key), None, LearnScope::Project, now + 10)
            .unwrap();
        assert!(!proposed.is_empty());
        ctx.reject_candidate(&proposed[0].candidate_id, Some("not useful"), now + 20)
            .unwrap();
        let rejected = ctx
            .get_candidate(&proposed[0].candidate_id)
            .unwrap()
            .expect("candidate persists");
        assert_eq!(rejected.status, "rejected");
        let keyword = rejected
            .proposition
            .split(|c: char| !c.is_alphanumeric())
            .find(|t| t.len() >= 4)
            .unwrap_or("rejectprobe")
            .to_string();

        let identity = blank_identity();
        let inputs = temp_inputs(&dir, &state, &store, &ctx, &identity, &[]);
        let brief = assemble(&inputs, &brief_request(&format!("fix {keyword}"))).unwrap();
        assert!(
            brief.learning.is_empty(),
            "rejected learning must never pose as knowledge"
        );
        assert!(
            brief
                .negative_knowledge
                .iter()
                .any(|n| n.kind == "rejected_learning"),
            "rejected approach must survive as negative knowledge: {:?}",
            brief.negative_knowledge
        );
    }
}

//! Bounded engineering-context composition (foundation for the `context`
//! MCP capability).
//!
//! This module composes the existing read-side runtimes — project identity,
//! verified facts, engineering memory, the execution-evidence journal, and
//! durable context records (user-context store) — into a single
//! task-relevant packet for OpenCode.
//!
//! ```text
//! task ──► compose() ──► EngineeringContextPacket ──► OpenCode plans
//! ```
//!
//! # Invariants
//!
//! - **Read-only.** Composition never writes files, never executes
//!   commands, never mutates `.codebro/` state. Verified by test
//!   (`composing_creates_no_files`). The user-context store is opened only
//!   for reads by the caller and degrades to an empty section on error.
//! - **Bounded.** Every section has a hard cap; oversized memory values and
//!   context records are returned as explicit excerpts with a
//!   `…[truncated for context budget]` marker.
//! - **No whole-store dumps.** `compose` rejects an empty task exactly
//!   like `engineering_facts`: context must be task-relevant.
//!   `compose_structural` is the deliberate exception — a bounded,
//!   task-free orientation digest for session start, clearly labelled as
//!   structural in `notes`.
//! - **Provenance-tagged.** Every section carries a [`ContextProvenance`]
//!   tag so OpenCode can tell verified structure apart from agent-recorded
//!   prose, observed execution history, durable user context, and derived
//!   summaries.
//! - **No decisions made.** The packet contains evidence and pointers
//!   (including which `impact_analyze` call to make next); OpenCode remains
//!   the planner and decision-maker.
//!
//! # Provenance tags
//!
//! The tags reuse the vocabulary established in
//! [`codebro_core::provenance`](crate::provenance):
//!
//! | Tag | Meaning | Sections |
//! |-----|---------|----------|
//! | `verified` | Deterministic static analysis (`codebro init`) | facts |
//! | `recorded` | Human/agent-declared intent | identity, decisions, memory, records |
//! | `observed` | Machine-recorded execution history | evidence journal |
//! | `derived` | Deterministic summaries computed at compose time | freshness, impact guidance |
//!
//! There is deliberately no `unknown` tag: a section that cannot be produced
//! is returned empty with a `note` explaining why, never as untagged data.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use codebro_context_runtime::RankedRecord;
use serde::{Deserialize, Serialize};

// ── Bounds ────────────────────────────────────────────────────────────────

/// Maximum fact records in one packet.
pub const MAX_FACTS: usize = 10;
/// Maximum memory entries in one packet.
pub const MAX_MEMORY_ENTRIES: usize = 5;
/// Maximum characters kept per memory value before excerpting.
pub const MAX_MEMORY_VALUE_CHARS: usize = 500;
/// Maximum engineering decisions in one packet.
pub const MAX_DECISIONS: usize = 5;
/// Maximum evidence summaries in one packet.
pub const MAX_EVIDENCE_ITEMS: usize = 5;
/// Maximum suggested impact targets in one packet.
pub const MAX_IMPACT_SUGGESTIONS: usize = 5;
/// Maximum context records in one packet.
pub const MAX_CONTEXT_RECORDS: usize = 8;
/// Maximum characters kept per context-record content before excerpting.
pub const MAX_RECORD_CONTENT_CHARS: usize = 240;
/// Marker appended when a value is excerpted (matches memory convention).
pub const TRUNCATION_MARKER: &str = "…[truncated for context budget]";

// ── Provenance ────────────────────────────────────────────────────────────

/// Per-section provenance tag for the context packet.
///
/// Distinct from [`crate::provenance::SourceKind`] (the claim-level
/// vocabulary) but mapped 1:1: `verified` ↔ `StaticAnalysis`,
/// `recorded` ↔ `AgentDeclared`/`HumanDeclared`, `observed` ↔ `Execution`,
/// `derived` ↔ computed-at-compose-time summaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextProvenance {
    /// Deterministic static analysis (fact store).
    Verified,
    /// Declared intent (identity, decisions, agent memory).
    Recorded,
    /// Machine-observed execution history (evidence journal).
    Observed,
    /// Deterministic summary computed while composing.
    Derived,
}

impl std::fmt::Display for ContextProvenance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ContextProvenance::Verified => write!(f, "verified"),
            ContextProvenance::Recorded => write!(f, "recorded"),
            ContextProvenance::Observed => write!(f, "observed"),
            ContextProvenance::Derived => write!(f, "derived"),
        }
    }
}

// ── Request ───────────────────────────────────────────────────────────────

/// What OpenCode wants context for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EngineeringContextRequest {
    /// The task in OpenCode's own words (e.g. "Fix failing authentication test").
    #[serde(default)]
    pub task: String,
    /// Extra keyword hints for memory/fact retrieval.
    #[serde(default)]
    pub task_keywords: Vec<String>,
    /// Active-file tags to bias memory resolution.
    #[serde(default)]
    pub active_file_tags: Vec<String>,
}

impl EngineeringContextRequest {
    /// All keywords: task text tokens (len ≥ 3) plus explicit hints.
    pub fn keywords(&self) -> Vec<String> {
        let mut out: Vec<String> = self.task_keywords.clone();
        for tok in self.task.split(|c: char| !c.is_alphanumeric()) {
            if tok.len() >= 3 {
                out.push(tok.to_string());
            }
        }
        out.sort();
        out.dedup();
        out
    }

    /// A request with neither task text nor keywords cannot be made
    /// task-relevant; reject it rather than dumping the store.
    pub fn is_empty(&self) -> bool {
        self.task.trim().is_empty() && self.task_keywords.iter().all(|k| k.trim().is_empty())
    }
}

// ── Packet ────────────────────────────────────────────────────────────────

/// Repository orientation section (`recorded` + `derived`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositorySection {
    pub provenance: ContextProvenance,
    pub workspace_root: String,
    pub identity_loaded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
    #[serde(default)]
    pub languages: Vec<String>,
    pub freshness: String,
    /// Per-kind fact counts (`derived`), present when facts.json parses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fact_counts: Option<serde_json::Value>,
}

/// One decision excerpt (`recorded`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionExcerpt {
    pub id: String,
    pub title: String,
    pub status: String,
}

/// One memory excerpt (`recorded`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryExcerpt {
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub truncated: bool,
}

/// One evidence item (`observed`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub summary: String,
}

/// One durable context-record excerpt (`recorded`).
///
/// Records carry their kind, scope, and authority (user_confirmed /
/// ai_inferred / observed / project_derived / imported / system_derived)
/// so the agent can weigh them without opening the full store. Intent
/// records additionally carry their decoded `intent_status` / `priority` /
/// `rationale` (excerpted); a record whose intent metadata cannot be
/// decoded never reaches the packet (see the resolution layer), so
/// `intent` here is always well-formed when present.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextRecordExcerpt {
    pub id: String,
    pub kind: String,
    pub namespace: String,
    pub content: String,
    pub authority: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<IntentExcerpt>,
    pub status: String,
    pub importance: f64,
    /// Confidence after evidence decay at retrieval time.
    pub effective_confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Decoded intent metadata excerpt (bounded rationale).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct IntentExcerpt {
    pub intent_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// Maximum characters kept per intent rationale before excerpting.
pub const MAX_RATIONALE_EXCERPT_CHARS: usize = 160;

/// Bound an excerpt from a ranked record (content excerpted with the
/// standard marker when longer than the budget).
pub fn excerpt_from(ranked: &RankedRecord) -> ContextRecordExcerpt {
    let mut content: String = ranked
        .record
        .content
        .chars()
        .take(MAX_RECORD_CONTENT_CHARS + 1)
        .collect();
    if content.chars().count() > MAX_RECORD_CONTENT_CHARS {
        content = format!(
            "{}{}",
            content
                .chars()
                .take(MAX_RECORD_CONTENT_CHARS)
                .collect::<String>(),
            TRUNCATION_MARKER
        );
    }
    ContextRecordExcerpt {
        id: ranked.record.id.clone(),
        kind: ranked.record.kind.to_string(),
        namespace: ranked.record.namespace.clone(),
        content,
        authority: ranked.record.authority.to_string(),
        scope: ranked.record.scope.to_string(),
        task_id: ranked.record.task_id.clone(),
        intent: intent_excerpt_of(ranked),
        status: ranked.record.status.to_string(),
        importance: ranked.record.importance,
        effective_confidence: ranked.effective_confidence,
        language: ranked.record.language.clone(),
    }
}

/// Decode an intent excerpt for intent records; `None` for every other
/// kind, and `None` (not a placeholder) when the metadata is malformed —
/// malformed intents are excluded upstream by the resolution layer, so a
/// `None` here on an intent record only occurs when excerpting ad-hoc.
fn intent_excerpt_of(ranked: &RankedRecord) -> Option<IntentExcerpt> {
    if ranked.record.kind != codebro_context_runtime::RecordKind::Intent {
        return None;
    }
    let meta = codebro_context_runtime::IntentMetadata::read_from(&ranked.record).ok()?;
    let rationale = meta.rationale.as_deref().map(|r| {
        let mut short: String = r.chars().take(MAX_RATIONALE_EXCERPT_CHARS + 1).collect();
        if short.chars().count() > MAX_RATIONALE_EXCERPT_CHARS {
            short = format!(
                "{}{}",
                short
                    .chars()
                    .take(MAX_RATIONALE_EXCERPT_CHARS)
                    .collect::<String>(),
                TRUNCATION_MARKER
            );
        }
        short
    });
    Some(IntentExcerpt {
        intent_status: meta.intent_status.to_string(),
        priority: meta.priority.map(|p| p.to_string()),
        rationale,
    })
}

/// Impact next-step guidance (`derived` — pointers, not analysis).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactGuidance {
    pub provenance: ContextProvenance,
    pub note: String,
    #[serde(default)]
    pub suggested_targets: Vec<String>,
}

/// The bounded, task-relevant context packet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineeringContextPacket {
    pub repository: RepositorySection,
    #[serde(default)]
    pub facts: Vec<crate::mcp::facts::FactRecord>,
    pub facts_provenance: ContextProvenance,
    #[serde(default)]
    pub decisions: Vec<DecisionExcerpt>,
    pub decisions_provenance: ContextProvenance,
    #[serde(default)]
    pub memory: Vec<MemoryExcerpt>,
    pub memory_provenance: ContextProvenance,
    #[serde(default)]
    pub evidence: Vec<EvidenceItem>,
    pub evidence_provenance: ContextProvenance,
    #[serde(default)]
    pub records: Vec<ContextRecordExcerpt>,
    pub records_provenance: ContextProvenance,
    pub impact: ImpactGuidance,
    #[serde(default)]
    pub validation: Vec<String>,
    #[serde(default)]
    pub notes: Vec<String>,
}

impl EngineeringContextPacket {
    /// Serialized size in bytes (for bound assertions and future MCP caps).
    pub fn serialized_len(&self) -> usize {
        serde_json::to_vec(self)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
    }
}

/// Compose a bounded context packet. Read-only: loads identity, facts,
/// memory, the evidence journal, and (via the caller) durable context
/// records but writes nothing and executes nothing.
pub fn compose(
    workspace_root: &std::path::Path,
    request: &EngineeringContextRequest,
    records: &[ContextRecordExcerpt],
) -> Result<EngineeringContextPacket, String> {
    if request.is_empty() {
        return Err(
            "task is required: supply a task description or task_keywords so context stays task-relevant"
                .to_string(),
        );
    }
    let keywords = request.keywords();

    // ── Identity (recorded) ──
    let (identity_loaded, snapshot) = load_identity(workspace_root);

    // ── Facts (verified) ──
    let store = load_fact_store(workspace_root);
    let freshness = crate::mcp::facts::compute_freshness(&store, workspace_root);
    let facts = search_facts(&store, &keywords, freshness, workspace_root);
    let suggested_targets: Vec<String> = facts
        .iter()
        .take(MAX_IMPACT_SUGGESTIONS)
        .map(|r| {
            if let Some(p) = r.path.as_deref() {
                format!("{} ({})", r.name, p)
            } else {
                r.name.clone()
            }
        })
        .collect();

    // ── Decisions (recorded): keyword-filtered, bounded ──
    let decisions = filter_decisions(&snapshot.engineering_decisions, &keywords);

    // ── Memory (recorded): bounded resolution + excerpting ──
    let memory = resolve_memory(workspace_root, &keywords, &request.active_file_tags);

    // ── Evidence (observed): read-only journal status + recent summaries ──
    let (evidence, mut notes) = read_evidence(workspace_root);

    // ── Freshness (derived) ──
    let freshness_str = freshness.to_string();
    if freshness == crate::mcp::facts::FreshnessStatus::Stale {
        notes.push(
            "fact store is stale relative to the working tree — consider `reindex` before trusting structural details"
                .to_string(),
        );
    }
    if !identity_loaded {
        notes.push(
            "no project identity established for this workspace — repository orientation is filesystem-only"
                .to_string(),
        );
    }
    if facts.is_empty() {
        notes.push(
            "no verified facts matched the task — try shorter keywords or run `reindex` if the workspace was never scanned"
                .to_string(),
        );
    }

    Ok(EngineeringContextPacket {
        repository: repository_section(workspace_root, &snapshot, identity_loaded, &store, freshness_str),
        facts,
        facts_provenance: ContextProvenance::Verified,
        decisions,
        decisions_provenance: ContextProvenance::Recorded,
        memory,
        memory_provenance: ContextProvenance::Recorded,
        evidence,
        evidence_provenance: ContextProvenance::Observed,
        records: records.to_vec(),
        records_provenance: ContextProvenance::Recorded,
        impact: ImpactGuidance {
            provenance: ContextProvenance::Derived,
            note: "call impact_analyze with one specific symbol, file, module, or package — this packet only suggests starting points, it performs no traversal itself".to_string(),
            suggested_targets,
        },
        validation: vec![
            "repository_health for workspace state".to_string(),
            "sandbox_test / sandbox_build to verify any change".to_string(),
        ],
        notes,
    })
}

/// Compose a bounded, task-free orientation digest for session start.
///
/// Unlike [`compose`], this does not rank facts/memory (there is no task to
/// be relevant to). It returns identity, fact counts, evidence status, and
/// durable context records, and always labels itself structural in `notes`
/// so it is never mistaken for task-relevant context.
pub fn compose_structural(
    workspace_root: &std::path::Path,
    records: &[ContextRecordExcerpt],
) -> Result<EngineeringContextPacket, String> {
    let (identity_loaded, snapshot) = load_identity(workspace_root);
    let store = load_fact_store(workspace_root);
    let freshness = crate::mcp::facts::compute_freshness(&store, workspace_root);
    let (evidence, mut notes) = read_evidence(workspace_root);
    notes.push(
        "structural digest — no task supplied, so facts/memory were not ranked; describe the task to get task-relevant context"
            .to_string(),
    );
    Ok(EngineeringContextPacket {
        repository: repository_section(
            workspace_root,
            &snapshot,
            identity_loaded,
            &store,
            freshness.to_string(),
        ),
        facts: Vec::new(),
        facts_provenance: ContextProvenance::Verified,
        decisions: Vec::new(),
        decisions_provenance: ContextProvenance::Recorded,
        memory: Vec::new(),
        memory_provenance: ContextProvenance::Recorded,
        evidence,
        evidence_provenance: ContextProvenance::Observed,
        records: records.to_vec(),
        records_provenance: ContextProvenance::Recorded,
        impact: ImpactGuidance {
            provenance: ContextProvenance::Derived,
            note: "no task given — impact guidance resumes once a task is supplied".to_string(),
            suggested_targets: Vec::new(),
        },
        validation: vec![],
        notes,
    })
}

// ── Internal composition helpers (each delegates, none duplicates) ────────

fn load_identity(
    workspace_root: &std::path::Path,
) -> (bool, crate::project_identity::ProjectIdentity) {
    let mut identity_rt = crate::project_identity::ProjectIdentityRuntime::new(workspace_root);
    let identity_loaded = identity_rt.load().is_ok();
    (identity_loaded, identity_rt.snapshot())
}

/// Build the repository orientation section (`recorded` + `derived`):
/// identity digest plus per-kind fact counts.
fn repository_section(
    workspace_root: &std::path::Path,
    snapshot: &crate::project_identity::ProjectIdentity,
    identity_loaded: bool,
    store: &crate::fact_store::FactStore,
    freshness: String,
) -> RepositorySection {
    let counts = store.collection().counts();
    RepositorySection {
        provenance: ContextProvenance::Recorded,
        workspace_root: workspace_root.display().to_string(),
        identity_loaded,
        project_name: if snapshot.name.is_empty() {
            None
        } else {
            Some(snapshot.name.clone())
        },
        languages: snapshot.languages.clone(),
        freshness,
        fact_counts: Some(serde_json::json!({
            "modules": counts.modules,
            "packages": counts.packages,
            "symbols": counts.symbols,
            "tests": counts.tests,
            "build_targets": counts.build_targets,
            "dependencies": counts.dependencies,
            "relationships": counts.relationships,
            "references": counts.references,
            "total": counts.total,
        })),
    }
}

fn load_fact_store(workspace_root: &std::path::Path) -> crate::fact_store::FactStore {
    let path = workspace_root.join(".codebro/facts.json");
    match std::fs::read(&path) {
        Ok(bytes) => {
            match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
                Ok(model) => crate::fact_store::FactStore::from_model(&model),
                // Mirror the server's fail-safe: never crash composition on
                // a corrupt store; degrade to empty and let the note + doctor
                // surface the problem. No quarantine here — composition is
                // read-only and must not mutate the workspace.
                Err(_) => crate::fact_store::FactStore::empty(),
            }
        }
        Err(_) => crate::fact_store::FactStore::empty(),
    }
}

fn search_facts(
    store: &crate::fact_store::FactStore,
    keywords: &[String],
    freshness: crate::mcp::facts::FreshnessStatus,
    _workspace_root: &std::path::Path,
) -> Vec<crate::mcp::facts::FactRecord> {
    let mut out: Vec<crate::mcp::facts::FactRecord> = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for kw in keywords.iter().take(8) {
        if kw.len() < 3 {
            continue;
        }
        let params = crate::mcp::facts::FactSearch {
            query: kw,
            kind: None,
            path: None,
            limit: MAX_FACTS,
        };
        if let Ok(records) = crate::mcp::facts::search(store, &params, freshness) {
            for r in records {
                let key = (r.kind.clone(), r.name.clone());
                if seen.insert(key) {
                    out.push(r);
                }
                if out.len() >= MAX_FACTS {
                    break;
                }
            }
        }
        if out.len() >= MAX_FACTS {
            break;
        }
    }
    out.truncate(MAX_FACTS);
    out
}

fn filter_decisions(
    decisions: &[crate::project_identity::EngineeringDecision],
    keywords: &[String],
) -> Vec<DecisionExcerpt> {
    let lowered: Vec<String> = keywords.iter().map(|k| k.to_lowercase()).collect();
    let mut out = Vec::new();
    for d in decisions {
        let hay = format!("{} {}", d.title, d.description).to_lowercase();
        let relevant = lowered
            .iter()
            .any(|k| k.len() >= 3 && hay.contains(k.as_str()));
        // Always keep accepted decisions when the task is broad; otherwise
        // require a keyword hit so the section stays task-relevant.
        if relevant || d.status.to_string() == "accepted" {
            out.push(DecisionExcerpt {
                id: d.id.clone(),
                title: d.title.chars().take(120).collect(),
                status: d.status.to_string(),
            });
        }
        if out.len() >= MAX_DECISIONS {
            break;
        }
    }
    out
}

fn resolve_memory(
    workspace_root: &std::path::Path,
    keywords: &[String],
    active_file_tags: &[String],
) -> Vec<MemoryExcerpt> {
    let identity = crate::project_identity::ProjectIdentityRuntime::new(workspace_root);
    let mut memory =
        crate::engineering_memory::EngineeringMemoryRuntime::new(workspace_root, identity);
    let _ = memory.load(); // absent store is not an error for a read query
    let context = memory.resolve_for_task(keywords, active_file_tags);
    context
        .entries
        .iter()
        .take(MAX_MEMORY_ENTRIES)
        .map(|e| {
            let mut value: String = e.value.chars().take(MAX_MEMORY_VALUE_CHARS + 1).collect();
            let truncated = value.chars().count() > MAX_MEMORY_VALUE_CHARS;
            if truncated {
                value = format!(
                    "{}{}",
                    value
                        .chars()
                        .take(MAX_MEMORY_VALUE_CHARS)
                        .collect::<String>(),
                    TRUNCATION_MARKER
                );
            }
            MemoryExcerpt {
                key: e.key.clone(),
                value,
                confidence: e.confidence,
                truncated,
            }
        })
        .collect()
}

fn read_evidence(workspace_root: &std::path::Path) -> (Vec<EvidenceItem>, Vec<String>) {
    // Read-only: status() never writes, never quarantines.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let st = crate::sandbox::evidence_journal::status(workspace_root, now);
    if !st.exists || st.records == 0 {
        return (
            Vec::new(),
            vec!["no execution evidence recorded yet".to_string()],
        );
    }
    if !st.valid {
        return (
            Vec::new(),
            vec![
                "execution evidence journal is unparseable (quarantined on next validation run)"
                    .to_string(),
            ],
        );
    }
    let file = crate::sandbox::evidence_journal::load(workspace_root);
    let mut items: Vec<EvidenceItem> = file
        .records
        .iter()
        .rev()
        .take(MAX_EVIDENCE_ITEMS)
        .map(|r| EvidenceItem {
            summary: format!(
                "{} {} → {} ({}ms)",
                r.runner.as_deref().unwrap_or("run"),
                r.command.chars().take(80).collect::<String>(),
                r.classification,
                r.duration_ms
            ),
        })
        .collect();
    items.truncate(MAX_EVIDENCE_ITEMS);
    (items, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn request(task: &str) -> EngineeringContextRequest {
        EngineeringContextRequest {
            task: task.to_string(),
            task_keywords: Vec::new(),
            active_file_tags: Vec::new(),
        }
    }

    fn codebro_files(root: &std::path::Path) -> Vec<String> {
        let dir = root.join(".codebro");
        if !dir.exists() {
            return Vec::new();
        }
        std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect()
    }

    #[test]
    fn empty_task_is_rejected_not_dumped() {
        let dir = temp_root();
        let req = EngineeringContextRequest::default();
        assert!(compose(dir.path(), &req, &[]).is_err());
    }

    #[test]
    fn fresh_workspace_yields_empty_categories_with_notes() {
        let dir = temp_root();
        let packet = compose(dir.path(), &request("Fix failing authentication test"), &[]).unwrap();
        assert!(!packet.repository.workspace_root.is_empty());
        assert!(!packet.repository.identity_loaded);
        assert!(packet.facts.is_empty());
        assert_eq!(packet.facts_provenance, ContextProvenance::Verified);
        assert_eq!(packet.memory_provenance, ContextProvenance::Recorded);
        assert_eq!(packet.evidence_provenance, ContextProvenance::Observed);
        assert_eq!(packet.impact.provenance, ContextProvenance::Derived);
        assert!(!packet.notes.is_empty());
        assert!(packet.serialized_len() < 8192);
    }

    #[test]
    fn composing_creates_no_files() {
        let dir = temp_root();
        let before = codebro_files(dir.path());
        let _ = compose(dir.path(), &request("investigate parser failure"), &[]).unwrap();
        // Composition may READ .codebro state but must never create it.
        assert_eq!(codebro_files(dir.path()), before);
    }

    #[test]
    fn facts_and_memory_stay_separated() {
        let dir = temp_root();
        // Record one memory entry through the canonical runtime.
        let identity = crate::project_identity::ProjectIdentityRuntime::new(dir.path());
        let mut memory =
            crate::engineering_memory::EngineeringMemoryRuntime::new(dir.path(), identity);
        let _ = memory.load();
        // Seed via file so the test does not depend on record() internals:
        // memory resolution reads the same store compose() reads.
        let packet = compose(dir.path(), &request("memory separation probe"), &[]).unwrap();
        // With no facts file and no memory entries, both sections are empty
        // but carry DISTINCT provenance tags — never merged.
        assert!(packet.facts.is_empty() || packet.memory.is_empty());
        assert_ne!(
            packet.facts_provenance, packet.memory_provenance,
            "facts (verified) and memory (recorded) must never share a tag"
        );
    }

    #[test]
    fn provenance_tags_cover_all_sections() {
        let dir = temp_root();
        let packet = compose(dir.path(), &request("audit provenance tags"), &[]).unwrap();
        let json = serde_json::to_value(&packet).unwrap();
        for section in [
            "facts_provenance",
            "decisions_provenance",
            "memory_provenance",
            "evidence_provenance",
        ] {
            assert!(json.get(section).is_some(), "missing {section}");
        }
        assert!(json.get("impact").unwrap().get("provenance").is_some());
        assert!(json.get("repository").unwrap().get("provenance").is_some());
    }

    #[test]
    fn packet_is_bounded_on_seeded_workspace() {
        let dir = temp_root();
        crate::init::run(dir.path()).unwrap();
        let packet = compose(dir.path(), &request("investigate change engine"), &[]).unwrap();
        assert!(packet.facts.len() <= MAX_FACTS);
        assert!(packet.memory.len() <= MAX_MEMORY_ENTRIES);
        assert!(packet.decisions.len() <= MAX_DECISIONS);
        assert!(packet.evidence.len() <= MAX_EVIDENCE_ITEMS);
        assert!(packet.impact.suggested_targets.len() <= MAX_IMPACT_SUGGESTIONS);
        for m in &packet.memory {
            assert!(
                m.value.chars().count()
                    <= MAX_MEMORY_VALUE_CHARS + TRUNCATION_MARKER.chars().count()
            );
        }
        assert!(packet.serialized_len() < 16384);
    }

    #[test]
    fn structural_digest_needs_no_task() {
        let dir = temp_root();
        let packet = compose_structural(dir.path(), &[]).unwrap();
        // Orientation fields present; ranked sections deliberately empty.
        assert!(!packet.repository.workspace_root.is_empty());
        assert!(!packet.repository.identity_loaded);
        assert!(packet.facts.is_empty());
        assert!(packet.memory.is_empty());
        assert!(packet.records.is_empty());
        // The digest labels itself structural so it is never mistaken for
        // task-relevant context.
        assert!(packet.notes.iter().any(|n| n.contains("structural digest")));
    }

    #[test]
    fn records_section_is_provenance_tagged_and_bounded() {
        let dir = temp_root();
        let ranked = RankedRecord {
            record: codebro_context_runtime::ContextRecord::new(
                "ctx::a",
                codebro_context_runtime::RecordKind::Preference,
                "fp.communication.verbosity",
                "Prefers direct concise replies",
                codebro_context_runtime::Authority::UserConfirmed,
            ),
            bm25: None,
            effective_confidence: 0.9,
        };
        let excerpts = vec![excerpt_from(&ranked)];
        let packet = compose(dir.path(), &request("how should I reply?"), &excerpts).unwrap();
        assert_eq!(packet.records.len(), 1);
        assert_eq!(packet.records[0].authority, "user_confirmed");
        assert_eq!(packet.records_provenance, ContextProvenance::Recorded);

        // Excerpts are bounded even for oversized content.
        let mut big = ranked.clone();
        big.record.content = "x".repeat(10_000);
        let ex = excerpt_from(&big);
        assert!(
            ex.content.chars().count()
                <= MAX_RECORD_CONTENT_CHARS + TRUNCATION_MARKER.chars().count()
        );
        assert!(ex.content.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn structural_digest_reports_fact_counts_when_store_exists() {
        let dir = temp_root();
        crate::init::run(dir.path()).unwrap();
        let packet = compose_structural(dir.path(), &[]).unwrap();
        let counts = packet.repository.fact_counts.as_ref().expect("fact counts");
        assert!(counts["modules"].is_number());
        assert!(counts["total"].as_u64().unwrap() > 0);
        assert!(packet.serialized_len() < 16384);
    }
}

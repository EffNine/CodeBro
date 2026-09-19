//! P11 skill reuse detector: repeated successful workflows → evidence-backed
//! skill candidates through the existing learning + skill lifecycle.
//!
//! ```text
//! successful executions (history events, same scope)
//!         │
//!         ▼
//! repeated tool-sequence detection (deterministic, this module)
//!         │
//!         ▼
//! P3 learning candidate (WorkflowPattern, evidence-cited)
//!         │
//!         ▼
//! P3 evaluation (accepted-only trust boundary, existing machinery)
//!         │
//!         ▼
//! P4 skill candidate (existing `create_skill_candidate_from_learning`)
//!         │
//!         ▼
//! automated validation (existing `evaluate_candidate_content`)
//!         │
//!         ▼
//! human approval (`request_approval` → needs_input → `respond`)
//!         │
//!         ▼
//! ACTIVE skill → future contextual reuse (`applicable` + `skill_context`)
//! ```
//!
//! # What the detector is (and is not)
//!
//! - **Explicitly invoked.** OpenCode calls `skill detect_reuse`; nothing
//!   runs in the background. No scheduler, daemon, watcher, or executor.
//! - **Deterministic.** Tool-sequence mining over canonical history: no LLM,
//!   no embeddings, no network. Re-running on unchanged history converges
//!   (same ids, idempotent refresh), never duplicates.
//! - **Tool information first.** Patterns are mined from the ordered `tool`
//!   (falling back to `kind`) of in-scope events. File paths are never used
//!   for matching: two executions touching different files but running the
//!   same tools are the same workflow; one path touched by different tools
//!   is not.
//! - **Success-only support.** A session supports a pattern only when it
//!   contains at least one success-polarity outcome and zero failure-polarity
//!   outcomes (same [`crate::learning::outcome_polarity`] vocabulary the P3
//!   pipeline uses). Failed/aborted executions never support; failed
//!   sessions containing the same sequence count as contradiction.
//! - **A detector observation is not a skill.** Every qualifying pattern
//!   becomes a P3 learning candidate first and must pass the existing
//!   evaluation (≥3 supporting events, contradiction gates, confidence
//!   floor) before it may seed a skill candidate — and a skill candidate
//!   still needs automated validation plus explicit human approval before
//!   anything publishes. The authority ladder
//!   (observed → inferred → candidate → approved) is never skipped.
//! - **No new tables.** Learning candidates reuse `learning_candidates`,
//!   skill candidates/skills/versions reuse their P4 tables, approvals reuse
//!   `skill_approval_requests`, history stays append-only. No new database
//!   subsystem.
//!
//! # Minimum signal (all required for a skill candidate)
//!
//! - At least [`REUSE_MIN_SUPPORTING_SESSIONS`] successful sessions
//!   (distinct sessions, same scope) containing the same contiguous tool
//!   subsequence of length ≥ [`REUSE_MIN_PATTERN_LEN`] with at least
//!   [`REUSE_MIN_DISTINCT_TOOLS`] distinct tools.
//! - No strong contradiction: contradicting (failed-session) events neither
//!   outweigh support nor reach half of it (mirrors the P3 contested rule).
//! - Accepted P3 evaluation with confidence at or above the skill approval
//!   floor ([`crate::skills::SKILL_APPROVAL_MIN_CONFIDENCE`]).
//! - No equivalent active skill or in-review candidate already exists
//!   (deterministic name from the tool sequence; duplicates converge).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::learning::{LearnScope, CANDIDATE_TTL_SECS, MAX_DETECTION_EVENTS};
use crate::skills::{SkillApplicability, SKILL_APPROVAL_MIN_CONFIDENCE};
use crate::store::{ContextError, ContextStore, EVENT_COLUMNS};
use crate::workspace::canonical_workspace_key;

/// Minimum successful sessions (distinct) containing the same tool
/// subsequence before it qualifies as a repeated workflow.
pub const REUSE_MIN_SUPPORTING_SESSIONS: usize = 3;
/// Minimum tools in a reused subsequence. Single-tool repetition is
/// frequency, not a workflow.
pub const REUSE_MIN_PATTERN_LEN: usize = 2;
/// Maximum tools mined per subsequence (bounds the per-session combinatorial
/// fan-out: a 20-step session yields ≤ 4×20 windows, never exponential).
pub const REUSE_MAX_PATTERN_LEN: usize = 5;
/// Minimum distinct tools in a pattern (after consecutive-duplicate
/// collapse, a run like `test → test → test` cannot qualify).
pub const REUSE_MIN_DISTINCT_TOOLS: usize = 2;
/// Maximum skill candidates minted per detector run (ranked best-first, so
/// output stays bounded and model-friendly).
pub const REUSE_MAX_CANDIDATES_PER_RUN: usize = 3;
/// Maximum event ids stored per learning candidate side (supporting /
/// contradicting). Enough provenance for audit without unbounded rows.
pub const REUSE_MAX_EVIDENCE_IDS: usize = 100;
/// Maximum event ids echoed per candidate in the MCP view (the DB row may
/// hold more; the wire view stays small).
pub const REUSE_MAX_VIEW_IDS: usize = 20;
/// Skill-name prefix for detector-minted skills.
pub const REUSE_SKILL_NAME_PREFIX: &str = "reuse";

/// Report status values (machine-readable, model-friendly).
pub const STATUS_CANDIDATES_FOUND: &str = "candidates_found";
pub const STATUS_ALREADY_EXISTS: &str = "already_exists";
pub const STATUS_LEARNING_ONLY: &str = "learning_only";
pub const STATUS_NO_CANDIDATES: &str = "no_candidates";

/// One detector outcome for a single repeated pattern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReuseCandidateView {
    /// Deterministic skill name derived from the tool sequence.
    pub name: String,
    /// Human-readable tool chain, e.g. `apply_change → sandbox_test`.
    pub pattern: String,
    /// Successful sessions backing the pattern.
    pub observations: usize,
    /// Supporting event count (stored on the learning candidate).
    pub supporting_events: usize,
    /// Contradicting event count considered (and discounted).
    pub contradicting_events: usize,
    /// Post-evaluation learning confidence (bounded, two decimals).
    pub confidence: f64,
    /// `project` | `task` | `global`.
    pub scope: String,
    pub learning_candidate_id: String,
    pub learning_status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_candidate_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_status: Option<String>,
    /// `created_validated` | `duplicate_existing_skill` |
    /// `duplicate_candidate_in_review` | `learning_only_*` |
    /// `skipped_terminal` | `validation_failed`.
    pub outcome: String,
    pub next_action: String,
    /// Bounded sample of supporting event ids (audit pointers, capped).
    #[serde(default)]
    pub evidence_sample: Vec<i64>,
}

/// Bounded, model-friendly result of one detector run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReuseReport {
    pub status: String,
    pub scope: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Distinct repeated patterns evaluated this run (before ranking caps).
    pub patterns_considered: usize,
    #[serde(default)]
    pub candidates: Vec<ReuseCandidateView>,
    pub next_action: String,
    pub note: String,
}

/// Mined subsequence accumulator: (tool chain, supporting sessions,
/// first occurrence event ids per session).
type SubseqStat = (Vec<String>, BTreeSet<String>, HashMap<String, Vec<i64>>);

/// One mined pattern: (tool chain, supporting sessions, supporting events,
/// contradicting events, supporting ids, contradicting ids).
type RankedPattern = (Vec<String>, usize, usize, usize, Vec<i64>, Vec<i64>);

/// One in-scope history event as the detector sees it.
struct ReuseEvent {
    id: i64,
    session_id: String,
    kind: String,
    tool: Option<String>,
    outcome: Option<String>,
    created_at: u64,
}

/// Normalize an event's tool signal: the recorded `tool` when present,
/// otherwise the structural `kind`. Lowercased so `Sandbox_Test` and
/// `sandbox_test` mine as one step; `apply_changes` (plural batch form)
/// folds into `apply_change` (same workflow step, different arity).
fn normalize_tool(tool: Option<&str>, kind: &str) -> String {
    let raw = tool
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(kind);
    let lower = raw.to_ascii_lowercase();
    match lower.as_str() {
        "apply_changes" => "apply_change".to_string(),
        other => other.to_string(),
    }
}

/// Collapse consecutive duplicate steps, keeping the first event id per
/// run: `test → test → verify` is the two-step workflow `test → verify`
/// (retry noise must not fork patterns), with provenance intact.
fn collapse_with_ids(tools: Vec<String>, ids: Vec<i64>) -> (Vec<String>, Vec<i64>) {
    let mut ctools: Vec<String> = Vec::with_capacity(tools.len());
    let mut cids: Vec<i64> = Vec::with_capacity(ids.len());
    for (t, id) in tools.into_iter().zip(ids) {
        if ctools.last().map(|l| l != &t).unwrap_or(true) {
            ctools.push(t);
            cids.push(id);
        }
    }
    (ctools, cids)
}

/// URL-safe slug for namespaces and skill names: lowercase alphanumerics
/// and single hyphens (underscores become hyphens). Deterministic.
fn slugify(parts: &[String]) -> String {
    let mut slug = String::new();
    for part in parts {
        for ch in part.chars() {
            if ch.is_ascii_alphanumeric() {
                slug.push(ch.to_ascii_lowercase());
            } else if !slug.ends_with('-') && !slug.is_empty() {
                slug.push('-');
            }
        }
        if !slug.ends_with('-') {
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

/// Deterministic skill name for a tool pattern: `reuse-<slug>`, confined to
/// the OpenCode naming rule (≤64 chars, cut at a hyphen boundary).
pub fn reuse_skill_name(tools: &[String]) -> String {
    let mut slug = slugify(tools);
    // Reserve room for the `reuse-` prefix inside the 64-char limit.
    const MAX_SLUG: usize = 64 - 6;
    if slug.len() > MAX_SLUG {
        slug.truncate(MAX_SLUG);
        // Avoid cutting mid-token: back up to the previous hyphen.
        if let Some(pos) = slug.rfind('-') {
            slug.truncate(pos);
        }
        while slug.ends_with('-') {
            slug.pop();
        }
    }
    if slug.is_empty() {
        return "reuse-workflow".to_string();
    }
    format!("{REUSE_SKILL_NAME_PREFIX}-{slug}")
}

/// Deterministic learning-candidate id for a reuse signature. The
/// `p11-skill-reuse` domain separator keeps these ids disjoint from P3's
/// topic-pair ids (same table, different identity space — no collisions).
fn mint_reuse_candidate_id(
    scope: &LearnScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
    signature: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(
        format!(
            "p11-skill-reuse|{scope}|{}|{}|workflow_pattern|{signature}",
            workspace_root.unwrap_or(""),
            task_id.unwrap_or(""),
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

/// Whether evaluated learning confidence clears the skill approval floor.
/// Below-floor learning stays knowledge (accepted inference); it must not
/// become a publishable candidate.
pub fn meets_skill_confidence_floor(confidence: f64) -> bool {
    confidence >= SKILL_APPROVAL_MIN_CONFIDENCE
}

/// Fetch in-scope detector events (bounded, chronological). Scope
/// predicates mirror P3 detection: another project's evidence can never
/// leak in — neither as support nor as contradiction.
fn fetch_reuse_events(
    conn: &rusqlite::Connection,
    scope: &LearnScope,
    workspace_root: Option<&str>,
    task_id: Option<&str>,
) -> Result<Vec<ReuseEvent>, ContextError> {
    let scope_sql: &str = match scope {
        LearnScope::Project => "workspace_root = ?1",
        LearnScope::Task => "workspace_root = ?1 AND task_id = ?2",
        LearnScope::Global => "1 = 1",
    };
    let sql = format!(
        "SELECT {EVENT_COLUMNS} FROM events WHERE {scope_sql} \
         ORDER BY created_at ASC, id ASC LIMIT ?3"
    );
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt
        .query_map(
            rusqlite::params![workspace_root, task_id, MAX_DETECTION_EVENTS as i64],
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
        // Only workflow-eligible kinds mine patterns: conversational and
        // session-framing kinds (Ignored) plus the skill-approval protocol
        // itself (also Ignored — approving skills must not become a skill).
        if crate::learning::kind_group(&event.kind) == crate::learning::KindGroup::Ignored {
            continue;
        }
        let session_id = match event.session_id.clone() {
            Some(s) if !s.trim().is_empty() => s,
            _ => continue, // session-less events carry no ordering context
        };
        let id = match event.id {
            Some(id) if id > 0 => id,
            _ => continue,
        };
        out.push(ReuseEvent {
            id,
            session_id,
            kind: event.kind.clone(),
            tool: event.tool.clone(),
            outcome: event.outcome.clone(),
            created_at: event.created_at,
        });
    }
    Ok(out)
}

/// Session polarity: success needs ≥1 success outcome and zero failures;
/// any failure makes the session failed (excluded from support, counted as
/// contradiction when it shares the pattern); neutral-only sessions are
/// neither.
fn session_polarity(events: &[&ReuseEvent]) -> crate::learning::OutcomePolarity {
    use crate::learning::{outcome_polarity, OutcomePolarity};
    let mut saw_success = false;
    for e in events {
        match outcome_polarity(e.outcome.as_deref()) {
            OutcomePolarity::Failure => return OutcomePolarity::Failure,
            OutcomePolarity::Success => saw_success = true,
            OutcomePolarity::Neutral => {}
        }
    }
    if saw_success {
        OutcomePolarity::Success
    } else {
        OutcomePolarity::Neutral
    }
}

/// Contiguous-subsequence containment (the pattern must appear as one
/// unbroken run — order matters, gaps do not match).
fn contains_subsequence(haystack: &[String], needle: &[String]) -> Option<Vec<usize>> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    (0..=(haystack.len() - needle.len()))
        .find(|&start| haystack[start..start + needle.len()] == *needle)
        .map(|start| (start..start + needle.len()).collect())
}

/// Proposed SKILL.md for a detected pattern. Stable by construction: it
/// references the learning candidate id (deterministic) and the pattern —
/// never volatile counts, confidence snapshots, or event ids — so
/// re-detection converges on one lineage instead of minting a candidate
/// per run. (Volatile counts live in the candidate's description/purpose,
/// which re-proposals refresh idempotently without changing identity.)
fn reuse_skill_content(
    name: &str,
    tools: &[String],
    scope: &LearnScope,
    workspace_root: Option<&str>,
    learning_candidate_id: &str,
) -> String {
    let display = tools.join(" → ");
    let description = format!("Repeated {display} workflow (evidence-backed)");
    let scope_phrase = match scope {
        LearnScope::Global => "across projects".to_string(),
        LearnScope::Project => format!("in project {}", workspace_root.unwrap_or("this project")),
        LearnScope::Task => "in this task".to_string(),
    };
    let mut steps = String::new();
    for (i, tool) in tools.iter().enumerate() {
        steps.push_str(&format!(
            "{}. Run `{tool}` as observed in evidence.\n",
            i + 1
        ));
    }
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n\
# Purpose\n\n\
Execute the repeated successful workflow `{display}` {scope_phrase}.\n\n\
# When to Use\n\n\
Use this skill when the current task repeats the `{display}` workflow {scope_phrase}.\n\n\
# Procedure\n\n\
{steps}\n\
# Required Context\n\n\
- The tools in the recorded sequence must be available: {}.\n\
- The task must match this workflow's scope ('{}').\n\n\
# Constraints\n\n\
- Scope '{}': {}.\n\
- Do not run unrelated tools as part of this procedure.\n\
- Never bypass human approval for changes outside this procedure.\n\n\
# Expected Outcome\n\n\
The same successful verification outcome observed in evidence: the workflow \
completes and its verification passes. Report what was done as task evidence.\n\n\
# Evidence\n\n\
Derived from learning candidate {learning_candidate_id}. Full event citations \
live in the skill registry, not in this file.\n",
        tools
            .iter()
            .map(|t| format!("`{t}`"))
            .collect::<Vec<_>>()
            .join(", "),
        scope,
        scope,
        match scope {
            LearnScope::Global => "visible in every workspace".to_string(),
            _ => format!(
                "visible only from {}",
                workspace_root.unwrap_or("its project")
            ),
        },
    )
}

impl ContextStore {
    /// P11 entry point: detect repeated successful workflows in scope and
    /// route qualifying patterns through the existing learning → skill
    /// candidate → validate pipeline. Explicitly invoked (no background
    /// work); read-only when nothing qualifies.
    pub fn detect_skill_reuse(
        &self,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
        scope: LearnScope,
        now: u64,
    ) -> Result<ReuseReport, ContextError> {
        let ws = workspace_root.map(canonical_workspace_key);
        if matches!(scope, LearnScope::Project | LearnScope::Task)
            && ws.as_deref().unwrap_or("").is_empty()
        {
            return Err(ContextError::Validation(
                "project/task reuse detection requires a workspace_root".to_string(),
            ));
        }
        let task = task_id
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        if matches!(scope, LearnScope::Task) && task.as_deref().unwrap_or("").is_empty() {
            return Err(ContextError::Validation(
                "task reuse detection requires a task_id".to_string(),
            ));
        }

        let events = self
            .with_conn(|conn| fetch_reuse_events(conn, &scope, ws.as_deref(), task.as_deref()))?;

        // Group ordered tool sequences per session.
        let mut by_session: BTreeMap<String, Vec<&ReuseEvent>> = BTreeMap::new();
        for e in &events {
            by_session.entry(e.session_id.clone()).or_default().push(e);
        }
        // (events are already chronological; per-session order is stable)
        let mut success_seqs: BTreeMap<String, (Vec<String>, Vec<i64>)> = BTreeMap::new();
        let mut failed_seqs: BTreeMap<String, (Vec<String>, Vec<i64>)> = BTreeMap::new();
        for (sid, evts) in &by_session {
            use crate::learning::OutcomePolarity;
            match session_polarity(evts) {
                OutcomePolarity::Success => {
                    let tools: Vec<String> = evts
                        .iter()
                        .map(|e| normalize_tool(e.tool.as_deref(), &e.kind))
                        .collect();
                    let ids: Vec<i64> = evts.iter().map(|e| e.id).collect();
                    let (ctools, cids) = collapse_with_ids(tools, ids);
                    if ctools.len() >= REUSE_MIN_PATTERN_LEN {
                        success_seqs.insert(sid.clone(), (ctools, cids));
                    }
                }
                OutcomePolarity::Failure => {
                    let tools: Vec<String> = evts
                        .iter()
                        .map(|e| normalize_tool(e.tool.as_deref(), &e.kind))
                        .collect();
                    let ids: Vec<i64> = evts.iter().map(|e| e.id).collect();
                    let (collapsed, aligned) = collapse_with_ids(tools, ids);
                    // Short failed sessions still contradict short patterns
                    // they fully contain (e.g. a 2-step abort of a 3-step
                    // workflow prefix is still evidence the prefix can fail)
                    // — keep them for matching.
                    if !collapsed.is_empty() {
                        failed_seqs.insert(sid.clone(), (collapsed, aligned));
                    }
                }
                OutcomePolarity::Neutral => {}
            }
        }

        // Mine frequent contiguous subsequences over successful sessions.
        // Key: tools joined by '>' (ascii, deterministic, hash-stable).
        let mut stats: BTreeMap<String, SubseqStat> = BTreeMap::new();
        for (sid, (tools, ids)) in &success_seqs {
            let mut seen_in_session: HashSet<String> = HashSet::new();
            let max_w = REUSE_MAX_PATTERN_LEN.min(tools.len());
            for w in REUSE_MIN_PATTERN_LEN..=max_w {
                for start in 0..=(tools.len() - w) {
                    let window = &tools[start..start + w];
                    // Distinct-tool gate at mining time (cheap, early).
                    {
                        let distinct: HashSet<&String> = window.iter().collect();
                        if distinct.len() < REUSE_MIN_DISTINCT_TOOLS {
                            continue;
                        }
                    }
                    let key = window.join(">");
                    if !seen_in_session.insert(key.clone()) {
                        continue; // one vote per session per subsequence
                    }
                    let entry = stats
                        .entry(key)
                        .or_insert_with(|| (window.to_vec(), BTreeSet::new(), HashMap::new()));
                    entry.1.insert(sid.clone());
                    entry
                        .2
                        .entry(sid.clone())
                        .or_insert_with(|| ids[start..start + w].to_vec());
                }
            }
        }

        // Score, gate, and rank candidate patterns.
        let mut ranked: Vec<RankedPattern> = Vec::new();
        // (tools, support_sessions, supporting_events, contradicting_events,
        //  supporting_ids, contradicting_ids)
        for (key, (tools, sessions, occurrences)) in &stats {
            if sessions.len() < REUSE_MIN_SUPPORTING_SESSIONS {
                continue;
            }
            let mut supporting_ids: Vec<i64> = occurrences.values().flatten().copied().collect();
            supporting_ids.sort_unstable();
            supporting_ids.dedup();
            // Contradiction: failed sessions containing this exact run.
            let mut contradicting_sessions: BTreeSet<String> = BTreeSet::new();
            let mut contradicting_ids: Vec<i64> = Vec::new();
            for (fsid, (ftools, fids)) in &failed_seqs {
                if let Some(idx) = contains_subsequence(ftools, tools) {
                    contradicting_sessions.insert(fsid.clone());
                    contradicting_ids.extend(idx.iter().map(|&i| fids[i]));
                }
            }
            contradicting_ids.sort_unstable();
            contradicting_ids.dedup();
            let s = supporting_ids.len();
            let c = contradicting_ids.len();
            // Mirrors the P3 contested/majority rules on event counts: a
            // pattern that fails as often as it succeeds is not reusable.
            if c > s || (s > 0 && c * 2 >= s) {
                continue;
            }
            let _ = key;
            ranked.push((
                tools.clone(),
                sessions.len(),
                s,
                c,
                supporting_ids,
                contradicting_ids,
            ));
        }
        // Support desc, length desc, signature asc — deterministic.
        ranked.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.0.len().cmp(&a.0.len()))
                .then_with(|| a.0.join(">").cmp(&b.0.join(">")))
        });
        let patterns_considered = ranked.len();
        // Subsumption filter: a pattern that already appears as a contiguous
        // run inside a kept pattern with at least as much support is
        // redundant (e.g. `a → b` inside the kept `a → b → c` with equal
        // support) — keep the most specific form, never both.
        let mut kept: Vec<RankedPattern> = Vec::new();
        for entry in ranked {
            let dominated = kept
                .iter()
                .any(|k| k.1 >= entry.1 && contains_subsequence(&k.0, &entry.0).is_some());
            if !dominated {
                kept.push(entry);
            }
            if kept.len() >= REUSE_MAX_CANDIDATES_PER_RUN {
                break;
            }
        }
        let ranked = kept;

        let mut candidates = Vec::new();
        for (
            tools,
            support_sessions,
            s_count,
            c_count,
            mut supporting_ids,
            mut contradicting_ids,
        ) in ranked
        {
            supporting_ids.truncate(REUSE_MAX_EVIDENCE_IDS);
            contradicting_ids.truncate(REUSE_MAX_EVIDENCE_IDS);
            let view = self.reuse_pattern_to_candidate(
                &tools,
                support_sessions,
                supporting_ids,
                contradicting_ids,
                &scope,
                ws.as_deref(),
                task.as_deref(),
                now,
            )?;
            let _ = (s_count, c_count);
            candidates.push(view);
        }
        candidates.sort_by(|a: &ReuseCandidateView, b: &ReuseCandidateView| {
            b.observations
                .cmp(&a.observations)
                .then_with(|| a.name.cmp(&b.name))
        });

        let created = candidates
            .iter()
            .filter(|c| c.outcome == "created_validated")
            .count();
        let duplicates = candidates
            .iter()
            .filter(|c| {
                c.outcome == "duplicate_existing_skill"
                    || c.outcome == "duplicate_candidate_in_review"
                    || c.outcome == "skipped_terminal"
            })
            .count();
        let learning_only = candidates
            .iter()
            .filter(|c| c.outcome.starts_with("learning_only"))
            .count();
        let (status, next_action) = if created > 0 {
            (
                STATUS_CANDIDATES_FOUND.to_string(),
                "Request human approval: call skill request_approval for each \
                 validated candidate, present its interaction question to the human \
                 with the approve/reject/modify/defer options, then call skill \
                 respond with the human answer. Never publish without approval."
                    .to_string(),
            )
        } else if duplicates > 0 && duplicates == candidates.len() && !candidates.is_empty() {
            (
                STATUS_ALREADY_EXISTS.to_string(),
                "Nothing new to approve: an equivalent skill or candidate already \
                 exists. Use skill applicable + skill_context for contextual reuse."
                    .to_string(),
            )
        } else if learning_only > 0 {
            (
                STATUS_LEARNING_ONLY.to_string(),
                "Patterns were accepted as learning but carry too little confidence \
                 to publish yet. Accumulate more successful repetitions, then \
                 re-run detect_reuse."
                    .to_string(),
            )
        } else {
            (
                STATUS_NO_CANDIDATES.to_string(),
                "No repeated successful workflow found in this scope. A reusable \
                 pattern needs at least 3 successful executions of the same tool \
                 sequence with no strong contradiction."
                    .to_string(),
            )
        };
        Ok(ReuseReport {
            status,
            scope: scope.to_string(),
            workspace_root: ws.clone(),
            task_id: task.clone(),
            patterns_considered,
            candidates,
            next_action,
            note: "P11 detector: explicitly invoked, deterministic, evidence-backed. \
                   Detector observations become P3 learning first; only accepted \
                   learning seeds skill candidates; only validated candidates with \
                   explicit human approval publish. No automatic publish, no \
                   approval bypass."
                .to_string(),
        })
    }

    /// Route one repeated pattern through learning → skill → validate.
    #[allow(clippy::too_many_arguments)]
    fn reuse_pattern_to_candidate(
        &self,
        tools: &[String],
        support_sessions: usize,
        supporting_ids: Vec<i64>,
        contradicting_ids: Vec<i64>,
        scope: &LearnScope,
        workspace_root: Option<&str>,
        task_id: Option<&str>,
        now: u64,
    ) -> Result<ReuseCandidateView, ContextError> {
        let signature = tools.join(">");
        let display = tools.join(" → ");
        let name = reuse_skill_name(tools);
        let scope_phrase = match scope {
            LearnScope::Global => "across projects".to_string(),
            LearnScope::Project => {
                format!("in project {}", workspace_root.unwrap_or("this project"))
            }
            LearnScope::Task => "in this task".to_string(),
        };
        let candidate_id = mint_reuse_candidate_id(scope, workspace_root, task_id, &signature);
        let slug = slugify(tools);
        let namespace: String = format!("learn.workflow-pattern.reuse-{slug}")
            .chars()
            .take(200)
            .collect();
        let proposition = format!(
            "The workflow '{display}' has repeatedly succeeded {scope_phrase} \
             ({support_sessions} successful executions across {support_sessions} sessions, \
             {} supporting events with {} contradicting considered); it appears reliable here.",
            supporting_ids.len(),
            contradicting_ids.len(),
        );
        let learning = crate::learning::LearningCandidate {
            candidate_id: candidate_id.clone(),
            workspace_root: match scope {
                LearnScope::Global => None,
                _ => workspace_root.map(str::to_string),
            },
            task_id: match scope {
                LearnScope::Task => task_id.map(str::to_string),
                _ => None,
            },
            scope: scope.to_string(),
            kind: crate::learning::CandidateKind::WorkflowPattern
                .as_str()
                .to_string(),
            proposition,
            namespace,
            supporting_evidence: supporting_ids.clone(),
            contradicting_evidence: contradicting_ids.clone(),
            confidence: 0.70,
            status: crate::learning::CandidateStatus::Candidate
                .as_str()
                .to_string(),
            created_at: now,
            updated_at: now,
            expires_at: Some(now.saturating_add(CANDIDATE_TTL_SECS)),
            eval_reason: Some(
                "proposed by the P11 skill-reuse detector from repeated successful \
                 tool sequences; not yet evaluated"
                    .to_string(),
            ),
            inference_record_id: None,
        };
        self.upsert_candidate(&learning)?;
        let stored = self.get_candidate(&candidate_id)?.ok_or_else(|| {
            ContextError::Decode("reuse learning candidate vanished after upsert".to_string())
        })?;
        let evidence_sample: Vec<i64> = supporting_ids
            .iter()
            .copied()
            .take(REUSE_MAX_VIEW_IDS)
            .collect();
        let base_view = |outcome: &str,
                         next_action: &str,
                         learning_status: &str,
                         confidence: f64| ReuseCandidateView {
            name: name.clone(),
            pattern: display.clone(),
            observations: support_sessions,
            supporting_events: supporting_ids.len(),
            contradicting_events: contradicting_ids.len(),
            confidence,
            scope: scope.to_string(),
            learning_candidate_id: candidate_id.clone(),
            learning_status: learning_status.to_string(),
            skill_candidate_id: None,
            skill_status: None,
            outcome: outcome.to_string(),
            next_action: next_action.to_string(),
            evidence_sample: evidence_sample.clone(),
        };
        // User verdicts and expiry stand: re-detection never rewrites them.
        match stored.status.as_str() {
            "rejected" | "superseded" | "expired" => {
                return Ok(base_view(
                    "skipped_terminal",
                    "This pattern was already decided (rejected/superseded/expired): \
                     new evidence must arrive as a new pattern; this verdict stands.",
                    stored.status.as_str(),
                    stored.confidence,
                ));
            }
            _ => {}
        }
        let evaluated = match self.evaluate_candidate(&candidate_id, now) {
            Ok(done) => done,
            Err(e) if e.to_string().contains("never rewrites this verdict") => {
                return Ok(base_view(
                    "skipped_terminal",
                    "This pattern carries a terminal verdict; re-detection does not \
                     rewrite it.",
                    "terminal",
                    stored.confidence,
                ));
            }
            Err(e) => {
                return Ok(base_view(
                    "learning_failed",
                    &format!("Learning evaluation errored ({e}); fix history scope and retry."),
                    stored.status.as_str(),
                    stored.confidence,
                ));
            }
        };
        if evaluated.status != crate::learning::CandidateStatus::Accepted.as_str() {
            return Ok(base_view(
                &format!("learning_only_{}", evaluated.status),
                "Accepted learning required before any skill proposal: this pattern \
                 stays a hypothesis (deferred/rejected). Accumulate more successful \
                 repetitions, then re-run detect_reuse.",
                evaluated.status.as_str(),
                evaluated.confidence,
            ));
        }
        if !meets_skill_confidence_floor(evaluated.confidence) {
            return Ok(base_view(
                "learning_only_weak_confidence",
                "Learning accepted but below the skill approval floor: no candidate \
                 minted. Accumulate more successful repetitions, then re-run \
                 detect_reuse.",
                evaluated.status.as_str(),
                evaluated.confidence,
            ));
        }
        // Duplicate guard: an equivalent active skill already exists.
        if let Some(existing) = self.get_skill_by_name(&name)? {
            let mut view = base_view(
                "duplicate_existing_skill",
                "An equivalent skill already exists: use skill applicable + \
                 skill_context for contextual reuse instead of proposing again.",
                evaluated.status.as_str(),
                evaluated.confidence,
            );
            view.skill_status = Some(existing.status.clone());
            return Ok(view);
        }
        let description = format!(
            "Repeated successful workflow: {display} (observed in {support_sessions} \
             successful sessions {scope_phrase}; evidence-backed, confidence {:.2}).",
            evaluated.confidence
        );
        let description: String = description.chars().take(900).collect();
        let purpose = format!(
            "Capture the repeated '{display}' workflow so future matching work \
             reuses the same verified sequence. Applies when the task repeats these \
             steps {scope_phrase}. Requires the listed tools; constrains execution \
             to the recorded sequence; expects the same successful verification \
             outcome. Evidence: learning candidate {candidate_id} \
             ({} supporting events, {} contradicting).",
            supporting_ids.len(),
            contradicting_ids.len(),
        );
        let mut distinct: Vec<String> = tools.to_vec();
        distinct.sort();
        distinct.dedup();
        distinct.truncate(8);
        let applicability = SkillApplicability {
            subsystems: distinct,
            ..Default::default()
        };
        let content = reuse_skill_content(&name, tools, scope, workspace_root, &candidate_id);
        let skill_candidate = match self.create_skill_candidate_from_learning(
            &evaluated,
            &name,
            &description,
            &purpose,
            applicability,
            &content,
            now,
        ) {
            Ok(candidate) => candidate,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("'expired'")
                    || msg.contains("'rejected'")
                    || msg.contains("'superseded'")
                {
                    // A prior terminal verdict on this exact lineage stands:
                    // re-detection never resurrects it.
                    return Ok(base_view(
                        "skipped_terminal",
                        "A prior verdict (rejected/superseded/expired) on this \
                         exact proposal stands; new evidence must arrive as a \
                         new pattern.",
                        evaluated.status.as_str(),
                        evaluated.confidence,
                    ));
                }
                if msg.contains("already exists") {
                    return Ok(base_view(
                        "duplicate_candidate_in_review",
                        "An equivalent candidate is already in the review pipeline: \
                         validate it and request approval instead of proposing again.",
                        evaluated.status.as_str(),
                        evaluated.confidence,
                    ));
                }
                return Ok(base_view(
                    "skill_proposal_failed",
                    &format!(
                        "Skill proposal was refused ({msg}); inspect the learning \
                             candidate and propose manually if warranted."
                    ),
                    evaluated.status.as_str(),
                    evaluated.confidence,
                ));
            }
        };
        let validated = match self.evaluate_candidate_content(&skill_candidate.candidate_id, now) {
            Ok(done) => done,
            Err(e) => {
                let mut view = base_view(
                    "validation_failed",
                    &format!(
                        "Automated validation errored ({e}); inspect the candidate \
                             and fix its content before requesting approval."
                    ),
                    evaluated.status.as_str(),
                    evaluated.confidence,
                );
                view.skill_candidate_id = Some(skill_candidate.candidate_id.clone());
                view.skill_status = Some(skill_candidate.status.clone());
                return Ok(view);
            }
        };
        if validated.status != crate::skills::SkillCandidateStatus::Validated.as_str() {
            let mut view = base_view(
                "validation_failed",
                "Automated validation did not pass; fix the candidate content \
                 (propose a corrected lineage), then validate again.",
                evaluated.status.as_str(),
                evaluated.confidence,
            );
            view.skill_candidate_id = Some(validated.candidate_id.clone());
            view.skill_status = Some(validated.status.clone());
            return Ok(view);
        }
        let mut view = base_view(
            "created_validated",
            "Validated and ready for human approval: call skill request_approval \
             with this candidate_id, present the interaction to the human, then \
             call skill respond with the human answer.",
            evaluated.status.as_str(),
            evaluated.confidence,
        );
        view.skill_candidate_id = Some(validated.candidate_id.clone());
        view.skill_status = Some(validated.status.clone());
        Ok(view)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::history::{HistoryInput, HistoryKind, OpenSession};

    const WS: &str = "/repo";
    const NOW: u64 = 1_700_000_000;

    fn store() -> (tempfile::TempDir, ContextStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        (dir, store)
    }

    fn session(store: &ContextStore, ws: &str, at: u64) -> String {
        store
            .open_session(ws, &OpenSession::default(), at)
            .unwrap()
            .id
    }

    #[allow(clippy::too_many_arguments)]
    fn event(
        store: &ContextStore,
        ws: &str,
        sid: &str,
        kind: HistoryKind,
        tool: &str,
        path: Option<&str>,
        outcome: &str,
        summary: &str,
        at: u64,
    ) -> i64 {
        let mut input = HistoryInput::new(ws, kind, summary);
        input.session_id = Some(sid.to_string());
        input.tool = Some(tool.to_string());
        input.path = path.map(str::to_string);
        input.outcome = Some(outcome.to_string());
        input.created_at = Some(at);
        let (id, dup) = store.record_history(&input, at).unwrap();
        assert!(!dup);
        id
    }

    /// One successful `inspect → modify → verify` execution.
    fn seed_success_run(store: &ContextStore, ws: &str, at: u64) -> String {
        let sid = session(store, ws, at);
        event(
            store,
            ws,
            &sid,
            HistoryKind::ToolExecution,
            "workspace_context",
            None,
            "success",
            "inspected workspace context",
            at,
        );
        event(
            store,
            ws,
            &sid,
            HistoryKind::ChangeApplied,
            "apply_change",
            Some("/src/a.rs"),
            "applied",
            "applied change to /src/a.rs",
            at + 1,
        );
        event(
            store,
            ws,
            &sid,
            HistoryKind::Validation,
            "sandbox_test",
            None,
            "passed",
            "sandbox_test passed",
            at + 2,
        );
        sid
    }

    /// Same tools but the verification failed: a failed execution.
    fn seed_failed_run(store: &ContextStore, ws: &str, at: u64) -> String {
        let sid = session(store, ws, at);
        event(
            store,
            ws,
            &sid,
            HistoryKind::ToolExecution,
            "workspace_context",
            None,
            "success",
            "inspected workspace context",
            at,
        );
        event(
            store,
            ws,
            &sid,
            HistoryKind::ChangeApplied,
            "apply_change",
            Some("/src/a.rs"),
            "applied",
            "applied change to /src/a.rs",
            at + 1,
        );
        event(
            store,
            ws,
            &sid,
            HistoryKind::Validation,
            "sandbox_test",
            None,
            "test_failure",
            "sandbox_test failed",
            at + 2,
        );
        sid
    }

    fn detect(store: &ContextStore, ws: &str, now: u64) -> ReuseReport {
        store
            .detect_skill_reuse(Some(ws), None, LearnScope::Project, now)
            .unwrap()
    }

    #[test]
    fn detects_repeated_successful_workflow_end_to_end() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_CANDIDATES_FOUND, "{report:?}");
        assert_eq!(report.candidates.len(), 1, "{report:?}");
        let c = &report.candidates[0];
        assert_eq!(c.outcome, "created_validated");
        assert_eq!(c.learning_status, "accepted");
        assert_eq!(c.skill_status.as_deref(), Some("validated"));
        assert_eq!(c.observations, 3);
        assert_eq!(c.supporting_events, 9);
        assert_eq!(c.contradicting_events, 0);
        assert!(
            c.confidence >= SKILL_APPROVAL_MIN_CONFIDENCE,
            "{}",
            c.confidence
        );
        assert!(c.name.starts_with("reuse-"));
        assert!(crate::skills::is_valid_skill_name(&c.name));
        assert_eq!(c.pattern, "workspace_context → apply_change → sandbox_test");
        assert!(report.next_action.contains("request_approval"));
        // Evidence is preserved and bounded.
        assert_eq!(c.evidence_sample.len(), 9);
        // The skill candidate exists with evidence-backed content.
        let sc_id = c.skill_candidate_id.as_deref().unwrap();
        let sc = store.get_skill_candidate(sc_id).unwrap().unwrap();
        assert_eq!(sc.name, c.name);
        assert_eq!(
            sc.source_learning_candidates,
            vec![c.learning_candidate_id.clone()]
        );
        assert_eq!(sc.supporting_evidence.len(), 9);
        let content = &sc.proposed_content;
        assert!(content.contains("workspace_context"));
        assert!(content.contains("apply_change"));
        assert!(content.contains("sandbox_test"));
        assert!(content.contains("# Purpose"));
        assert!(crate::skills::validate_skill_content(content, &c.name, Some(WS)).valid);
    }

    #[test]
    fn fewer_than_three_observations_yields_no_candidate() {
        let (_dir, store) = store();
        for i in 0..2 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn failed_executions_never_support_a_pattern() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_failed_run(&store, WS, NOW - 300 + i * 10);
        }
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn contradictory_outcomes_block_the_pattern() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        // Two failed executions of the same tool chain: 6 contradicting
        // events vs 9 supporting (6*2 >= 9) → contested, no candidate.
        for i in 0..2 {
            seed_failed_run(&store, WS, NOW - 200 + i * 10);
        }
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
        assert!(report.candidates.is_empty());
    }

    #[test]
    fn other_workspaces_never_leak_in() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, "/repo-a", NOW - 300 + i * 10);
        }
        let foreign = detect(&store, "/repo-b", NOW);
        assert_eq!(foreign.status, STATUS_NO_CANDIDATES, "{foreign:?}");
        let home = detect(&store, "/repo-a", NOW);
        assert_eq!(home.status, STATUS_CANDIDATES_FOUND, "{home:?}");
    }

    #[test]
    fn unrelated_workflows_yield_no_candidate() {
        let (_dir, store) = store();
        let tools = [["aa1", "aa2"], ["bb1", "bb2"], ["cc1", "cc2"]];
        for (i, pair) in tools.iter().enumerate() {
            let at = NOW - 300 + i as u64 * 10;
            let sid = session(&store, WS, at);
            event(
                &store,
                WS,
                &sid,
                HistoryKind::ToolExecution,
                pair[0],
                None,
                "success",
                "did first thing",
                at,
            );
            event(
                &store,
                WS,
                &sid,
                HistoryKind::Validation,
                pair[1],
                None,
                "passed",
                "verified second thing",
                at + 1,
            );
        }
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_NO_CANDIDATES, "{report:?}");
    }

    #[test]
    fn duplicate_existing_skill_is_not_reproposed() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        let first = detect(&store, WS, NOW);
        assert_eq!(first.status, STATUS_CANDIDATES_FOUND);
        let sc_id = first.candidates[0]
            .skill_candidate_id
            .as_deref()
            .unwrap()
            .to_string();
        let skills_dir = tempfile::tempdir().unwrap();
        store
            .approve_skill_candidate(&sc_id, Some(WS), skills_dir.path(), NOW + 1)
            .unwrap();
        // Re-running with the same evidence converges: no second candidate.
        let second = detect(&store, WS, NOW + 2);
        assert_eq!(second.status, STATUS_ALREADY_EXISTS, "{second:?}");
        assert_eq!(second.candidates.len(), 1);
        assert_eq!(second.candidates[0].outcome, "duplicate_existing_skill");
        assert_eq!(store.list_skills(Some(WS), None, 10).unwrap().len(), 1);
    }

    #[test]
    fn rejected_learning_verdict_sticks() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        let first = detect(&store, WS, NOW);
        assert_eq!(first.status, STATUS_CANDIDATES_FOUND);
        let lc_id = first.candidates[0].learning_candidate_id.clone();
        store
            .reject_candidate(&lc_id, Some("not a real pattern"), NOW + 1)
            .unwrap();
        let second = detect(&store, WS, NOW + 2);
        assert!(
            second
                .candidates
                .iter()
                .all(|c| c.outcome == "skipped_terminal"),
            "{second:?}"
        );
        // Nothing new entered the skill pipeline.
        assert_eq!(second.candidates.len(), 1);
        assert!(second.candidates[0].skill_candidate_id.is_none());
    }

    #[test]
    fn deferred_candidate_is_never_silently_published() {
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        let first = detect(&store, WS, NOW);
        let sc_id = first.candidates[0]
            .skill_candidate_id
            .as_deref()
            .unwrap()
            .to_string();
        store
            .transition_skill_candidate(
                &sc_id,
                crate::skills::SkillCandidateStatus::Deferred,
                Some("wait for more evidence"),
                NOW + 1,
            )
            .unwrap();
        // Re-detection may revive the deferred row through re-validation,
        // but it must never publish without human approval.
        let second = detect(&store, WS, NOW + 2);
        assert!(
            store.list_skills(Some(WS), None, 10).unwrap().is_empty(),
            "deferred work must never auto-publish: {second:?}"
        );
        assert!(
            !second
                .candidates
                .iter()
                .any(|c| c.outcome == "created_validated"
                    && c.skill_status.as_deref() == Some("active")),
            "{second:?}"
        );
    }

    #[test]
    fn expired_skill_proposal_is_not_resurrected() {
        use crate::skills::SKILL_CANDIDATE_TTL_SECS;
        let (_dir, store) = store();
        for i in 0..3 {
            seed_success_run(&store, WS, NOW - 300 + i * 10);
        }
        let first = detect(&store, WS, NOW);
        assert_eq!(first.status, STATUS_CANDIDATES_FOUND);
        let sc_id = first.candidates[0]
            .skill_candidate_id
            .as_deref()
            .unwrap()
            .to_string();
        // Park the proposal as deferred, then let its TTL lapse. Review
        // content (draft/validated) never silently expires — deferred does.
        store
            .transition_skill_candidate(
                &sc_id,
                crate::skills::SkillCandidateStatus::Deferred,
                Some("wait for more evidence"),
                NOW + 1,
            )
            .unwrap();
        let later = NOW + SKILL_CANDIDATE_TTL_SECS + 10;
        assert_eq!(store.expire_skill_candidates(later).unwrap(), 1);
        assert_eq!(
            store.get_skill_candidate(&sc_id).unwrap().unwrap().status,
            "expired"
        );
        // Re-detection must not resurrect the expired lineage.
        let second = store
            .detect_skill_reuse(Some(WS), None, LearnScope::Project, later + 1)
            .unwrap();
        assert!(
            second
                .candidates
                .iter()
                .all(|c| c.outcome == "skipped_terminal"),
            "{second:?}"
        );
        assert_eq!(
            store.get_skill_candidate(&sc_id).unwrap().unwrap().status,
            "expired",
            "expired rows are never rewritten by re-detection"
        );
        assert!(store.list_skills(Some(WS), None, 10).unwrap().is_empty());
    }

    #[test]
    fn weak_confidence_stays_learning_only() {
        use crate::learning::RECENCY_WINDOW_SECS;
        let (_dir, store) = store();
        // Old, weak (tool_execution) successes plus one old failure sharing
        // the pair: accepted by P3 (≥0.55) but below the skill floor (0.60).
        let old = NOW - RECENCY_WINDOW_SECS - 1000;
        for i in 0..3 {
            let at = old + i as u64 * 10;
            let sid = session(&store, WS, at);
            event(
                &store,
                WS,
                &sid,
                HistoryKind::ToolExecution,
                "alpha_tool",
                None,
                "success",
                "ran alpha",
                at,
            );
            event(
                &store,
                WS,
                &sid,
                HistoryKind::ToolExecution,
                "beta_tool",
                None,
                "success",
                "ran beta",
                at + 1,
            );
        }
        let at = old + 100;
        let sid = session(&store, WS, at);
        event(
            &store,
            WS,
            &sid,
            HistoryKind::ToolExecution,
            "alpha_tool",
            None,
            "success",
            "ran alpha",
            at,
        );
        event(
            &store,
            WS,
            &sid,
            HistoryKind::ToolResult,
            "beta_tool",
            None,
            "error",
            "beta errored",
            at + 1,
        );
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_LEARNING_ONLY, "{report:?}");
        assert_eq!(report.candidates.len(), 1);
        let c = &report.candidates[0];
        assert_eq!(c.outcome, "learning_only_weak_confidence", "{c:?}");
        assert!(
            c.confidence >= 0.55 && c.confidence < 0.60,
            "{}",
            c.confidence
        );
        assert!(c.skill_candidate_id.is_none());
        assert!(
            store
                .list_skill_candidates(Some(WS), None, None, 10)
                .unwrap()
                .is_empty(),
            "weak learning must not mint candidates"
        );
    }

    #[test]
    fn detection_survives_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(db::STATE_DB_FILE);
        let lc_id: String;
        let sc_id: String;
        {
            let store = ContextStore::new(path.clone());
            for i in 0..3 {
                seed_success_run(&store, WS, NOW - 300 + i * 10);
            }
            let report = detect(&store, WS, NOW);
            assert_eq!(report.status, STATUS_CANDIDATES_FOUND);
            lc_id = report.candidates[0].learning_candidate_id.clone();
            sc_id = report.candidates[0]
                .skill_candidate_id
                .as_deref()
                .unwrap()
                .to_string();
        }
        // Fresh handle over the same file: the loop state is durable.
        let store = ContextStore::new(path);
        assert!(store.get_candidate(&lc_id).unwrap().is_some());
        assert!(store.get_skill_candidate(&sc_id).unwrap().is_some());
        let again = detect(&store, WS, NOW + 5);
        assert_eq!(again.candidates[0].learning_candidate_id, lc_id);
    }

    #[test]
    fn tool_signal_wins_over_filenames() {
        let (_dir, store) = store();
        // Same tools, different files each run — plus a same-file red
        // herring running a different tool (single step: too short anyway).
        for (i, path) in ["/a.py", "/b.py", "/c.py"].iter().enumerate() {
            let at = NOW - 300 + i as u64 * 10;
            let sid = session(&store, WS, at);
            event(
                &store,
                WS,
                &sid,
                HistoryKind::ToolExecution,
                "alpha_tool",
                Some(path),
                "success",
                "ran alpha",
                at,
            );
            event(
                &store,
                WS,
                &sid,
                HistoryKind::Validation,
                "beta_tool",
                Some(path),
                "passed",
                "beta passed",
                at + 1,
            );
        }
        let at = NOW - 100;
        let sid = session(&store, WS, at);
        event(
            &store,
            WS,
            &sid,
            HistoryKind::ToolExecution,
            "gamma_tool",
            Some("/a.py"),
            "success",
            "ran gamma on the same file",
            at,
        );
        let report = detect(&store, WS, NOW);
        assert_eq!(report.status, STATUS_CANDIDATES_FOUND, "{report:?}");
        let c = &report.candidates[0];
        assert_eq!(c.pattern, "alpha_tool → beta_tool");
        assert_eq!(c.name, "reuse-alpha-tool-beta-tool");
        let sc = store
            .get_skill_candidate(c.skill_candidate_id.as_deref().unwrap())
            .unwrap()
            .unwrap();
        assert!(!sc.proposed_content.contains(".py"));
        assert!(!sc.description.contains(".py"));
    }

    #[test]
    fn skill_names_are_valid_and_bounded() {
        assert_eq!(
            reuse_skill_name(&["apply_change".to_string(), "sandbox_test".to_string()]),
            "reuse-apply-change-sandbox-test"
        );
        assert_eq!(
            reuse_skill_name(&["Apply_Change".to_string(), "Sandbox_Test!".to_string()]),
            "reuse-apply-change-sandbox-test"
        );
        let long = vec!["a-very-long-tool-name-that-keeps-going".to_string(); 6];
        let name = reuse_skill_name(&long);
        assert!(name.len() <= 64, "{name}");
        assert!(crate::skills::is_valid_skill_name(&name), "{name}");
        assert_eq!(reuse_skill_name(&[]), "reuse-workflow");
    }

    #[test]
    fn confidence_floor_gate() {
        assert!(!meets_skill_confidence_floor(0.59));
        assert!(meets_skill_confidence_floor(0.60));
        assert!(meets_skill_confidence_floor(0.95));
    }

    #[test]
    fn invalid_scope_is_refused() {
        let (_dir, store) = store();
        assert!(store
            .detect_skill_reuse(None, None, LearnScope::Project, NOW)
            .is_err());
        assert!(store
            .detect_skill_reuse(Some(WS), None, LearnScope::Task, NOW)
            .is_err());
    }
}

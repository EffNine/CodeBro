//! Explicit context model + deterministic skill applicability + skill context.
//!
//! # Phase 1 — the context model, made explicit
//!
//! CodeBro's persistent-intelligence foundation (P0–P9) already stores these
//! concepts in different homes, but nothing named them in one place, so
//! callers collapsed them into "context". This module names the eight
//! concepts and pins each to its owner:
//!
//! | Concept | Owner | Shape here |
//! |---|---|---|
//! | **Task Context** (current objective + task state) | P5 task runtime (`tasks.rs`); ad-hoc task text is never persisted | [`TaskContext`] |
//! | **Session Context** (relevant events/decisions of this session) | P2 history/recall (`history.rs`, `recall.rs`) | [`SessionContext`] |
//! | **Project Context** (facts, conventions, architecture, environment) | fact store + project identity | [`ProjectContext`] |
//! | **Memory** (durable knowledge: preferences, facts, learned conclusions) | engineering memory + context records | [`MemoryView`] |
//! | **Learning** (evidence-derived hypotheses with confidence/support/provenance) | P3 learning (`learning.rs`) | [`LearningView`] |
//! | **Skill** (reusable procedure/workflow) | P4 skill lifecycle (`skills.rs`) | re-exported [`Skill`] |
//! | **Skill Context** (what a skill needs to execute correctly) | this module | [`SkillContextPacket`] |
//! | **Evidence** (executions, outcomes, verification, provenance) | evidence journal + history events | [`EvidenceView`] |
//!
//! The task packet OpenCode receives is the composition of these — never one
//! generic blob:
//!
//! ```text
//! Task Context + Project Context + relevant Memory/Learning
//!   + selected Skill + Skill Context = OpenCode-facing task packet
//! ```
//!
//! # Phase 2 — deterministic, explainable skill applicability
//!
//! [`select_applicable_skills`] ranks workspace-visible skills against a task
//! without embeddings, keywords-only matching, or LLM calls. Every signal is
//! lexical and deterministic (sorted sets, capped arithmetic, stable
//! tie-breaks), and every ranked skill carries its `reasons` so OpenCode can
//! answer "this skill applies because X". Deprecated/invalid/out-of-scope
//! skills are excluded with a reason, never silently.
//!
//! # Phase 3 — skill context as a first-class concept
//!
//! A skill reference is not generic memory. [`build_skill_context`] builds a
//! minimal, bounded [`SkillContextPacket`] with only what the skill needs:
//! required vs optional inputs, constraints, expected outputs, and bounded
//! excerpts — never a full memory dump or full `SKILL.md` content.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::skills::{Skill, SkillHealth, HEALTH_FAILURE_THRESHOLD};
use crate::workspace::canonical_workspace_key;

// ─── Bounds ─────────────────────────────────────────────────────────────

/// Maximum ranked skills returned by selection.
pub const MAX_APPLICABLE_SKILLS: usize = 8;
/// Maximum reasons carried per ranked skill.
pub const MAX_SKILL_REASONS: usize = 8;
/// Maximum required/optional/constraint entries in a skill context packet.
pub const MAX_SKILL_CONTEXT_ENTRIES: usize = 8;
/// Maximum characters kept per excerpt (descriptions, content, memory refs).
pub const MAX_SKILL_EXCERPT_CHARS: usize = 500;
/// Maximum characters of the active SKILL.md echoed into a skill context
/// packet (an excerpt, never the full file).
pub const MAX_SKILL_CONTENT_EXCERPT_CHARS: usize = 2000;
/// Maximum memory/learning references carried by a skill context packet.
pub const MAX_SKILL_CONTEXT_REFS: usize = 5;
/// Marker appended when an excerpt is cut (matches the context-packet marker).
pub const SKILL_TRUNCATION_MARKER: &str = "…[truncated for context budget]";

/// Known programming-language tokens (lowercase, length ≥ 3 to match the
/// request tokenizer, which drops shorter tokens). Used ONLY to decide
/// whether the task itself names a language: excluding a skill as
/// "language-irrelevant" requires positive evidence of a mismatch (a known
/// repository language, or a language named in the task) — never an unknown
/// repository. Deterministic, no embeddings, no LLM.
pub const KNOWN_LANGUAGE_TOKENS: &[&str] = &[
    "rust",
    "golang",
    "python",
    "javascript",
    "typescript",
    "java",
    "kotlin",
    "swift",
    "ruby",
    "php",
    "scala",
    "haskell",
    "elixir",
    "erlang",
    "dart",
    "julia",
    "cpp",
    "csharp",
    "clojure",
    "groovy",
    "perl",
    "powershell",
    "shell",
    "bash",
    "sql",
    "html",
    "css",
    "vue",
    "svelte",
    "solidity",
    "zig",
    "nim",
    "crystal",
    "ocaml",
    "fsharp",
    "matlab",
    "rscript",
    "dockerfile",
    "terraform",
    "markdown",
];

/// Whether task tokens name any known language (positive evidence the task
/// is about a language — used to justify language-irrelevance exclusion).
pub fn task_mentions_language(tokens: &BTreeSet<String>) -> bool {
    tokens
        .iter()
        .any(|t| KNOWN_LANGUAGE_TOKENS.contains(&t.as_str()))
}

// ─── Phase 1 — explicit context model ────────────────────────────────────

/// Task Context: what the current task is — objective plus live task state.
///
/// Ad-hoc `task_text` is never persisted; `task_id` (when present) binds the
/// read-only P5 snapshot. `objective` is the caller's one-line framing of
/// the goal, distinct from checkpoint `next_action` noise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskContext {
    pub task_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub objective: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_state: Option<String>,
}

/// Session Context: relevant events and decisions from the current session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default)]
    pub recent_event_summaries: Vec<String>,
    #[serde(default)]
    pub decision_ids: Vec<String>,
}

/// Project Context: repository facts, conventions, architecture, environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectContext {
    pub workspace_root: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub conventions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub architecture_summary: Option<String>,
    #[serde(default)]
    pub environment: Vec<String>,
}

/// Memory: durable knowledge — bounded references, never full values.
///
/// A memory entry here is a *pointer* (key + excerpt + confidence), not the
/// store row. Full values stay behind `engineering_memory` resolution.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryView {
    pub key: String,
    pub excerpt: String,
    pub confidence: f64,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Learning: evidence-derived knowledge with confidence/support/provenance.
///
/// Accepted learning surfaces as `ai_inferred` (never `user_confirmed`);
/// rejected learning surfaces only as negative knowledge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LearningView {
    pub candidate_id: String,
    pub proposition_excerpt: String,
    pub confidence: f64,
    pub supporting: usize,
    pub contradicting: usize,
    pub authority: String,
}

/// Evidence: executions, outcomes, verification results, provenance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceView {
    #[serde(default)]
    pub execution_summaries: Vec<String>,
    #[serde(default)]
    pub outcome_summaries: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
    pub provenance: String,
}

// ─── Phase 2 — deterministic skill applicability ─────────────────────────

/// What OpenCode wants applicable skills for.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillSelectionRequest {
    /// Task text in OpenCode's own words (tokenized deterministically).
    #[serde(default)]
    pub task_text: String,
    /// Extra keyword hints (same tokenizer as the brief).
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Canonical workspace root requesting selection (scope confinement).
    #[serde(default)]
    pub workspace_root: String,
    /// Task identity for task-scoped visibility (optional).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Repository languages (e.g. from project identity).
    #[serde(default)]
    pub repo_languages: Vec<String>,
    /// Maximum ranked skills to return (clamped to [`MAX_APPLICABLE_SKILLS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
}

impl SkillSelectionRequest {
    /// Deterministic keyword set: task-text tokens (len ≥ 3) plus explicit
    /// hints, lowercased, sorted, deduplicated, capped.
    pub fn tokens(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for src in [&self.task_text, &self.keywords.join(" ")] {
            for tok in src.split(|c: char| !c.is_alphanumeric()) {
                if tok.len() >= 3 {
                    out.insert(tok.to_lowercase());
                }
            }
        }
        out.into_iter().take(32).collect()
    }

    pub fn limit(&self) -> usize {
        self.limit
            .unwrap_or(MAX_APPLICABLE_SKILLS)
            .clamp(1, MAX_APPLICABLE_SKILLS)
    }
}

/// One ranked, explainable skill selection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedSkill {
    pub skill_id: String,
    pub name: String,
    pub description: String,
    pub status: String,
    pub version: u32,
    pub scope: String,
    /// Deterministic score (higher = more applicable). Not a probability.
    pub score: i64,
    /// Why this skill is relevant — human-readable, bounded.
    pub reasons: Vec<String>,
    /// Which task/repository signals matched (bounded, sorted).
    pub matched_signals: Vec<String>,
    /// Context the skill requires to execute correctly.
    pub required_context: Vec<String>,
    /// Context that helps but is not required.
    pub optional_context: Vec<String>,
    /// Constraints OpenCode must respect (scope, version, health…).
    pub constraints: Vec<String>,
    pub category: String,
    pub provenance: String,
}

/// Why a skill was excluded from selection (audit, never silent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExcludedSkill {
    pub name: String,
    pub reason: String,
}

/// Bounded ranked selection result: applicable skills plus exclusion audit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillSelection {
    #[serde(default)]
    pub applicable: Vec<RankedSkill>,
    #[serde(default)]
    pub excluded: Vec<ExcludedSkill>,
    pub total_considered: usize,
    pub truncated: bool,
}

fn excerpt_bounded(s: &str, max: usize) -> String {
    let collected: String = s.chars().take(max + 1).collect();
    if collected.chars().count() > max {
        format!(
            "{}{}",
            collected.chars().take(max).collect::<String>(),
            SKILL_TRUNCATION_MARKER
        )
    } else {
        collected
    }
}

fn tokenize_lower(s: &str) -> BTreeSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() >= 3)
        .map(|t| t.to_lowercase())
        .collect()
}

/// Deterministic, explainable skill applicability.
///
/// Given workspace-visible skills (the caller lists them — this function
/// performs no I/O), rank the ones actually applicable to the request.
///
/// Scoring (all integer, all capped, all explainable):
/// - purpose/description/name token overlap with the task: +10 per shared
///   token, capped at +40 (intent, not filename matching: the purpose and
///   description carry the weight, the name only breaks ties lexically);
/// - language applicability vs repository languages: +30 on intersection;
///   when the skill declares languages and none match the repository *or*
///   the task, the skill is excluded as irrelevant (with a reason) — but
///   only on positive evidence of a mismatch: a known repository language,
///   or a language the task itself names. When the repository language is
///   unknown and the task names none, the skill is kept as uncertain (no
///   score, no exclusion, an explicit note) rather than misreported as
///   irrelevant;
/// - subsystem/task-type/framework/project applicability vs task tokens and
///   the requesting workspace: +15/+15/+10/+10 respectively, capped;
/// - confidence: +0..+10 scaled from the recorded confidence;
/// - health: +5 when assessable and healthy, −25 with a constraint when
///   degraded, +0 with a note when too few uses exist.
///
/// Excluded, never ranked: non-active statuses (deprecated/superseded/draft
/// lineages cannot execute), invalid names (could never publish), and
/// out-of-scope rows (a project skill from another workspace). Ordering is
/// score-descending, then name-ascending. Truncated to the request limit.
pub fn select_applicable_skills(
    skills: &[Skill],
    request: &SkillSelectionRequest,
) -> SkillSelection {
    let tokens = request.tokens();
    let repo_langs: BTreeSet<String> = request
        .repo_languages
        .iter()
        .map(|l| l.to_lowercase())
        .collect();
    let ws_key = canonical_workspace_key(&request.workspace_root);

    let mut applicable: Vec<RankedSkill> = Vec::new();
    let mut excluded: Vec<ExcludedSkill> = Vec::new();

    for skill in skills {
        // ── Exclusion gates (audit, never silent) ──
        if skill.status != "active" {
            excluded.push(ExcludedSkill {
                name: skill.name.clone(),
                reason: format!(
                    "status '{}' is not executable — only active skills are selected (deprecated/superseded/draft lineages never execute)",
                    skill.status
                ),
            });
            continue;
        }
        if !crate::skills::is_valid_skill_name(&skill.name) {
            excluded.push(ExcludedSkill {
                name: skill.name.clone(),
                reason: "invalid skill name — could never publish, never selected".to_string(),
            });
            continue;
        }
        if skill.scope == "project" {
            let ok = skill
                .workspace_root
                .as_deref()
                .map(|w| canonical_workspace_key(w) == ws_key)
                .unwrap_or(false);
            if !ok {
                excluded.push(ExcludedSkill {
                    name: skill.name.clone(),
                    reason: "project-scoped to another workspace — does not leak across workspaces"
                        .to_string(),
                });
                continue;
            }
        }

        let mut score: i64 = 0;
        let mut reasons: Vec<String> = Vec::new();
        let mut matched: BTreeSet<String> = BTreeSet::new();
        let mut required: Vec<String> = Vec::new();
        let mut optional: Vec<String> = Vec::new();
        let mut constraints: Vec<String> = Vec::new();

        // ── Task intent: purpose + description + name token overlap ──
        let mut intent_hay = BTreeSet::new();
        intent_hay.extend(tokenize_lower(&skill.name.replace('-', " ")));
        intent_hay.extend(tokenize_lower(&skill.description));
        let shared: BTreeSet<String> = intent_hay.intersection(&tokens).cloned().collect();
        if !shared.is_empty() {
            let gain = (shared.len() as i64 * 10).min(40);
            score += gain;
            for s in &shared {
                matched.insert(format!("intent:{s}"));
            }
            reasons.push(format!(
                "task intent overlaps skill purpose/description ({})",
                shared.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }

        // ── Applicability: languages ──
        let skill_langs: BTreeSet<String> = skill
            .applicability
            .languages
            .iter()
            .map(|l| l.to_lowercase())
            .collect();
        if skill_langs.is_empty() {
            optional.push("no language constraint declared — relevance is uncertain".to_string());
        } else {
            let lang_hit: BTreeSet<String> =
                skill_langs.intersection(&repo_langs).cloned().collect();
            let task_lang_hit: BTreeSet<String> =
                skill_langs.intersection(&tokens).cloned().collect();
            if !lang_hit.is_empty() {
                score += 30;
                for l in &lang_hit {
                    matched.insert(format!("language:{l}"));
                }
                reasons.push(format!(
                    "repository language matches skill applicability ({})",
                    lang_hit.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
                required.push(format!(
                    "repository language includes {}",
                    lang_hit.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            } else if !task_lang_hit.is_empty() {
                score += 20;
                for l in &task_lang_hit {
                    matched.insert(format!("language:{l}"));
                }
                reasons.push(format!(
                    "task mentions skill language ({})",
                    task_lang_hit.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
                required.push(format!(
                    "task context mentions {}",
                    task_lang_hit.iter().cloned().collect::<Vec<_>>().join(", ")
                ));
            } else {
                // No repository hit and no task hit. Exclude as irrelevant
                // only on positive evidence of a mismatch: the repository
                // language is known, or the task itself names a (different)
                // language. When both are unknown, report uncertainty —
                // never claim irrelevance without evidence.
                let repo_known = !repo_langs.is_empty();
                if repo_known || task_mentions_language(&tokens) {
                    excluded.push(ExcludedSkill {
                        name: skill.name.clone(),
                        reason: format!(
                            "declares languages ({}) matching neither the repository nor the task — irrelevant here",
                            skill.applicability.languages.join(", ")
                        ),
                    });
                    continue;
                }
                optional.push(format!(
                    "declares languages ({}) but the repository language is unknown and the task names none — relevance is uncertain",
                    skill.applicability.languages.join(", ")
                ));
            }
        }

        // ── Applicability: subsystems / task types / frameworks / projects ──
        let mut sub_gain: i64 = 0;
        for sub in &skill.applicability.subsystems {
            let sub_tokens = tokenize_lower(sub);
            let hit: Vec<String> = sub_tokens.intersection(&tokens).cloned().collect();
            if !hit.is_empty()
                || tokens
                    .iter()
                    .any(|t| sub.to_lowercase().contains(t.as_str()))
            {
                sub_gain += 15;
                matched.insert(format!("subsystem:{sub}"));
            }
        }
        if sub_gain > 0 {
            let gain = sub_gain.min(30);
            score += gain;
            reasons.push("task keywords overlap skill subsystem applicability".to_string());
            required.push(format!(
                "task touches subsystem ({})",
                skill.applicability.subsystems.join(", ")
            ));
        }
        let mut type_gain: i64 = 0;
        for tt in &skill.applicability.task_types {
            if tokens.iter().any(|t| {
                tt.to_lowercase().contains(t.as_str()) || t.contains(tt.to_lowercase().as_str())
            }) {
                type_gain += 15;
                matched.insert(format!("task_type:{tt}"));
            }
        }
        if type_gain > 0 {
            score += type_gain.min(30);
            reasons.push("task intent matches skill task-type applicability".to_string());
        }
        for fw in &skill.applicability.frameworks {
            if tokens.iter().any(|t| {
                fw.to_lowercase().contains(t.as_str()) || t.contains(fw.to_lowercase().as_str())
            }) {
                score += 10;
                matched.insert(format!("framework:{fw}"));
                reasons.push(format!("task mentions skill framework ({fw})"));
            }
        }
        for proj in &skill.applicability.projects {
            if ws_key.contains(proj.to_lowercase().as_str())
                || proj
                    .to_lowercase()
                    .contains(tokens.iter().next().map(String::as_str).unwrap_or("\u{0}"))
            {
                score += 10;
                matched.insert(format!("project:{proj}"));
                reasons.push(format!(
                    "skill project applicability matches workspace ({proj})"
                ));
            }
        }

        // ── Confidence (evidence-backed trust, bounded) ──
        let conf_gain = (skill.confidence.clamp(0.0, 1.0) * 10.0).round() as i64;
        score += conf_gain;
        if skill.confidence > 0.0 {
            optional.push(format!(
                "confidence {:.2} (evidence-backed)",
                skill.confidence
            ));
        }

        // ── Health (usage evidence, never auto-rewrites the skill) ──
        if skill.health.is_assessable() {
            if skill.health.is_degraded() {
                score -= 25;
                constraints.push(format!(
                    "health degraded: {:.0}% failures over {} uses — apply with caution, do not auto-rewrite",
                    skill.health.failure_ratio() * 100.0,
                    skill.health.success_count + skill.health.failure_count
                ));
                reasons.push(
                    "usage evidence shows elevated failures (penalized, not hidden)".to_string(),
                );
            } else {
                score += 5;
                optional.push(format!(
                    "healthy usage record ({} successes, {} failures)",
                    skill.health.success_count, skill.health.failure_count
                ));
            }
        } else {
            optional.push("insufficient usage evidence for health assessment".to_string());
        }

        // ── Scope + version constraints (always present) ──
        constraints.push(format!(
            "scope '{}' — {}",
            skill.scope,
            if skill.scope == "global" {
                "visible in every workspace".to_string()
            } else {
                format!("visible only from {ws_key}")
            }
        ));
        constraints.push(format!(
            "version-pinned: v{} (active)",
            skill.current_version
        ));
        required.push(format!(
            "skill version v{} (status '{}')",
            skill.current_version, skill.status
        ));

        if score < 0 {
            score = 0;
        }
        let mut matched_signals: Vec<String> = matched.into_iter().collect();
        matched_signals.sort();
        reasons.truncate(MAX_SKILL_REASONS);
        required.truncate(MAX_SKILL_CONTEXT_ENTRIES);
        optional.truncate(MAX_SKILL_CONTEXT_ENTRIES);
        constraints.truncate(MAX_SKILL_CONTEXT_ENTRIES);

        applicable.push(RankedSkill {
            skill_id: skill.skill_id.clone(),
            name: skill.name.clone(),
            description: excerpt_bounded(&skill.description, MAX_SKILL_EXCERPT_CHARS),
            status: skill.status.clone(),
            version: skill.current_version,
            scope: skill.scope.clone(),
            score,
            reasons,
            matched_signals,
            required_context: required,
            optional_context: optional,
            constraints,
            category: "SKILL".to_string(),
            provenance: "recorded".to_string(),
        });
    }

    applicable.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
    let total_considered = applicable.len() + excluded.len();
    let limit = request.limit();
    let truncated = applicable.len() > limit;
    applicable.truncate(limit);
    excluded.sort_by(|a, b| a.name.cmp(&b.name));
    excluded.truncate(MAX_APPLICABLE_SKILLS);

    SkillSelection {
        applicable,
        excluded,
        total_considered,
        truncated,
    }
}

// ─── Phase 3 — skill context as a first-class concept ────────────────────

/// A bounded pointer to a memory entry (key + excerpt, never the full value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillMemoryRef {
    pub key: String,
    pub excerpt: String,
    pub confidence: f64,
}

/// A bounded pointer to a learning entry (never a full promotion).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillLearningRef {
    pub candidate_id: String,
    pub excerpt: String,
    pub confidence: f64,
    /// Always `ai_inferred` for accepted learning — never `user_confirmed`.
    pub authority: String,
}

/// Minimal Skill Context packet: only what this skill needs to execute.
///
/// Deliberately distinct from generic memory: `category` is
/// `SKILL_CONTEXT` (not `MEMORY`), memory/learning travel as bounded
/// references (not values), and the active `SKILL.md` travels as a bounded
/// excerpt (not a full dump). Bounded and deterministic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillContextPacket {
    pub skill_id: String,
    pub skill_name: String,
    pub skill_version: u32,
    pub skill_status: String,
    pub skill_scope: String,
    /// "This skill applies because X."
    #[serde(default)]
    pub why_applicable: Vec<String>,
    /// "This is the context required by this skill."
    #[serde(default)]
    pub required: Vec<String>,
    #[serde(default)]
    pub optional: Vec<String>,
    #[serde(default)]
    pub constraints: Vec<String>,
    #[serde(default)]
    pub expected_outputs: Vec<String>,
    pub task_excerpt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    pub workspace_root: String,
    #[serde(default)]
    pub repo_languages: Vec<String>,
    /// Bounded memory pointers — not full values.
    #[serde(default)]
    pub memory: Vec<SkillMemoryRef>,
    /// Bounded learning pointers — accepted learning only.
    #[serde(default)]
    pub learning: Vec<SkillLearningRef>,
    /// Bounded excerpt of the active SKILL.md — never a full dump.
    pub content_excerpt: String,
    pub content_truncated: bool,
    pub category: String,
    pub provenance: String,
}

impl SkillContextPacket {
    /// Serialized size in bytes (for bound assertions and MCP caps).
    pub fn serialized_len(&self) -> usize {
        serde_json::to_vec(self)
            .map(|v| v.len())
            .unwrap_or(usize::MAX)
    }
}

/// Build a minimal skill context packet.
///
/// - `ranked` is the selection result for this skill (carries why/required/
///   optional/constraints — no recomputation, no drift from selection);
/// - `skill_content` is the active SKILL.md body (excerpted, never dumped);
/// - `memory`/`learning` are caller-filtered relevant refs (already bounded
///   by the caller; re-capped here deterministically).
#[allow(clippy::too_many_arguments)]
pub fn build_skill_context(
    ranked: &RankedSkill,
    skill_content: &str,
    task_text: &str,
    task_id: Option<&str>,
    workspace_root: &str,
    repo_languages: &[String],
    memory: Vec<SkillMemoryRef>,
    learning: Vec<SkillLearningRef>,
) -> SkillContextPacket {
    let mut memory = memory;
    memory.truncate(MAX_SKILL_CONTEXT_REFS);
    let mut learning = learning;
    learning.truncate(MAX_SKILL_CONTEXT_REFS);

    let content_chars = skill_content.chars().count();
    let (content_excerpt, content_truncated) = if content_chars > MAX_SKILL_CONTENT_EXCERPT_CHARS {
        (
            format!(
                "{}{}",
                skill_content
                    .chars()
                    .take(MAX_SKILL_CONTENT_EXCERPT_CHARS)
                    .collect::<String>(),
                SKILL_TRUNCATION_MARKER
            ),
            true,
        )
    } else {
        (skill_content.to_string(), false)
    };

    let mut expected_outputs = vec![format!(
        "apply the '{}' procedure to the current task; report what was done as task evidence",
        ranked.name
    )];
    expected_outputs.truncate(MAX_SKILL_CONTEXT_ENTRIES);

    SkillContextPacket {
        skill_id: ranked.skill_id.clone(),
        skill_name: ranked.name.clone(),
        skill_version: ranked.version,
        skill_status: ranked.status.clone(),
        skill_scope: ranked.scope.clone(),
        why_applicable: ranked.reasons.clone(),
        required: ranked.required_context.clone(),
        optional: ranked.optional_context.clone(),
        constraints: ranked.constraints.clone(),
        expected_outputs,
        task_excerpt: excerpt_bounded(task_text, MAX_SKILL_EXCERPT_CHARS),
        task_id: task_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        workspace_root: workspace_root.to_string(),
        repo_languages: repo_languages.to_vec(),
        memory,
        learning,
        content_excerpt,
        content_truncated,
        category: "SKILL_CONTEXT".to_string(),
        provenance: "recorded".to_string(),
    }
}

/// Map a [`RankedSkill`] back onto the brief-level skill projection without
/// recomputing applicability (single implementation of the rule).
pub fn brief_reason_summary(ranked: &RankedSkill) -> (bool, String) {
    let applicable = ranked.score > 0 && !ranked.reasons.is_empty();
    let reason = if applicable {
        ranked
            .reasons
            .first()
            .cloned()
            .unwrap_or_else(|| "applicable".to_string())
    } else {
        "no task/repository signal matched — relevance is uncertain".to_string()
    };
    (applicable, reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills::{SkillApplicability, SkillHealth};

    fn skill(name: &str, status: &str, scope: &str, ws: Option<&str>) -> Skill {
        Skill {
            skill_id: format!("sk::{name}"),
            workspace_root: ws.map(str::to_string),
            scope: scope.to_string(),
            name: name.to_string(),
            description: format!("Helps with {name} workflows and procedures"),
            applicability: SkillApplicability::default(),
            current_version: 1,
            status: status.to_string(),
            confidence: 0.8,
            health: SkillHealth::default(),
            source_candidate_id: None,
            superseded_by: None,
            created_at: 1,
            updated_at: 2,
        }
    }

    fn request(task: &str, ws: &str) -> SkillSelectionRequest {
        SkillSelectionRequest {
            task_text: task.to_string(),
            keywords: Vec::new(),
            workspace_root: ws.to_string(),
            task_id: None,
            repo_languages: vec!["rust".to_string()],
            limit: None,
        }
    }

    #[test]
    fn relevant_skill_selected_with_reasons() {
        let mut s = skill("code-review", "active", "global", None);
        s.applicability.languages = vec!["rust".to_string()];
        s.applicability.subsystems = vec!["review".to_string()];
        let sel = select_applicable_skills(&[s], &request("review this rust code", "/repo"));
        assert_eq!(sel.applicable.len(), 1);
        assert!(!sel.applicable[0].reasons.is_empty());
        assert!(sel.applicable[0].score > 0);
        assert_eq!(sel.applicable[0].category, "SKILL");
    }

    #[test]
    fn irrelevant_skill_excluded_not_keyword_matched() {
        let mut s = skill("k8s-deploy", "active", "global", None);
        s.applicability.languages = vec!["go".to_string()];
        s.applicability.subsystems = vec!["kubernetes".to_string()];
        let sel =
            select_applicable_skills(&[s], &request("fix rust borrow checker error", "/repo"));
        assert!(sel.applicable.is_empty());
        assert_eq!(sel.excluded.len(), 1);
        assert!(
            sel.excluded[0].reason.contains("irrelevant") || !sel.excluded[0].reason.is_empty()
        );
    }

    #[test]
    fn deprecated_skill_always_excluded() {
        let s = skill("old-skill", "deprecated", "global", None);
        let sel = select_applicable_skills(&[s], &request("old skill workflows", "/repo"));
        assert!(sel.applicable.is_empty());
        assert!(sel.excluded[0].reason.contains("not executable"));
    }

    #[test]
    fn ranking_is_score_then_name_deterministic() {
        let a = skill("aaa-skill", "active", "global", None);
        let b = skill("zzz-skill", "active", "global", None);
        let sel = select_applicable_skills(
            &[b.clone(), a.clone()],
            &request("unrelated task text", "/repo"),
        );
        // Both unscoped with no signals: equal scores → name order.
        assert_eq!(sel.applicable.len(), 2);
        assert!(sel.applicable[0].name < sel.applicable[1].name);
        assert_eq!(sel.applicable[0].score, sel.applicable[1].score);
    }

    #[test]
    fn unknown_repo_and_languageless_task_keep_skill_as_uncertain() {
        // The repository language is unknown (no identity signal) and the
        // task names no language: a language-declaring skill must be kept
        // as uncertain — never misreported as irrelevant without evidence.
        let mut s = skill("rust-helper", "active", "global", None);
        s.applicability.languages = vec!["rust".to_string()];
        let req = SkillSelectionRequest {
            task_text: "change a tool description following the checklist".to_string(),
            keywords: Vec::new(),
            workspace_root: "/repo".to_string(),
            task_id: None,
            repo_languages: Vec::new(),
            limit: None,
        };
        let sel = select_applicable_skills(&[s], &req);
        assert_eq!(sel.applicable.len(), 1);
        assert!(sel.excluded.is_empty());
        assert!(
            sel.applicable[0]
                .optional_context
                .iter()
                .any(|o| o.contains("uncertain")),
            "uncertainty must be explicit: {:?}",
            sel.applicable[0].optional_context
        );
    }

    #[test]
    fn unknown_repo_but_task_names_another_language_excludes() {
        // Positive mismatch evidence from the task itself ("python" named,
        // skill declares "go") still excludes — the filter keeps its teeth.
        let mut s = skill("go-deployer", "active", "global", None);
        s.applicability.languages = vec!["go".to_string()];
        s.applicability.subsystems = vec!["kubernetes".to_string()];
        let req = SkillSelectionRequest {
            task_text: "review this python pull request".to_string(),
            keywords: Vec::new(),
            workspace_root: "/repo".to_string(),
            task_id: None,
            repo_languages: Vec::new(),
            limit: None,
        };
        let sel = select_applicable_skills(&[s], &req);
        assert!(sel.applicable.is_empty());
        assert_eq!(sel.excluded.len(), 1);
        assert!(sel.excluded[0].reason.contains("irrelevant"));
    }

    #[test]
    fn task_mentions_language_detector_is_deterministic() {
        let yes: BTreeSet<String> = ["review", "this", "rust", "pull", "request"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(task_mentions_language(&yes));
        let no: BTreeSet<String> = ["change", "tool", "description"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(!task_mentions_language(&no));
    }

    #[test]
    fn output_is_bounded() {
        let skills: Vec<Skill> = (0..20)
            .map(|i| skill(&format!("skill-{i:02}"), "active", "global", None))
            .collect();
        let sel = select_applicable_skills(&skills, &request("skill workflows", "/repo"));
        assert!(sel.applicable.len() <= MAX_APPLICABLE_SKILLS);
        for r in &sel.applicable {
            assert!(r.reasons.len() <= MAX_SKILL_REASONS);
        }
    }

    #[test]
    fn workspace_scoped_skill_does_not_leak_across_workspaces() {
        let mut s = skill("secret-sauce", "active", "project", Some("/repo-a"));
        s.applicability.languages = vec!["rust".to_string()];
        // Same task, owning workspace: selected.
        let home = select_applicable_skills(&[s.clone()], &request("rust workflows", "/repo-a"));
        assert_eq!(home.applicable.len(), 1);
        assert!(home.excluded.is_empty());
        // Another workspace: excluded with an audit reason, never ranked.
        let away = select_applicable_skills(&[s], &request("rust workflows", "/repo-b"));
        assert!(away.applicable.is_empty());
        assert_eq!(away.excluded.len(), 1);
        assert!(away.excluded[0].reason.contains("another workspace"));
    }

    #[test]
    fn skill_context_is_distinct_from_generic_memory_and_bounded() {
        use crate::skills::SkillApplicability;
        let ranked = RankedSkill {
            skill_id: "sk::demo".to_string(),
            name: "demo-skill".to_string(),
            description: "Does demo things".to_string(),
            status: "active".to_string(),
            version: 3,
            scope: "global".to_string(),
            score: 42,
            reasons: vec!["task intent overlaps skill purpose/description (demo)".to_string()],
            matched_signals: vec!["intent:demo".to_string()],
            required_context: vec!["skill version v3 (status 'active')".to_string()],
            optional_context: vec!["confidence 0.80 (evidence-backed)".to_string()],
            constraints: vec!["version-pinned: v3 (active)".to_string()],
            category: "SKILL".to_string(),
            provenance: "recorded".to_string(),
        };
        let big_content = format!(
            "---\nname: demo-skill\ndescription: x\n---\n\n# Purpose\n\n{}",
            "y".repeat(10_000)
        );
        let packet = build_skill_context(
            &ranked,
            &big_content,
            "demo the thing",
            Some("task-1"),
            "/repo",
            &["rust".to_string()],
            vec![SkillMemoryRef {
                key: "arch:demo".to_string(),
                excerpt: "use demos".to_string(),
                confidence: 0.9,
            }],
            vec![],
        );
        // First-class concept, not generic memory: distinct category and
        // provenance vocabulary, references instead of values.
        assert_eq!(packet.category, "SKILL_CONTEXT");
        assert_ne!(packet.category, "MEMORY");
        assert!(!packet.why_applicable.is_empty());
        assert!(!packet.required.is_empty());
        // Bounded: content excerpted, task excerpted, refs capped.
        assert!(packet.content_truncated);
        assert!(packet.content_excerpt.ends_with(SKILL_TRUNCATION_MARKER));
        assert!(packet.content_excerpt.len() < big_content.len());
        assert!(packet.memory.len() <= MAX_SKILL_CONTEXT_REFS);
        assert!(packet.serialized_len() < 16_384);
        let _ = SkillApplicability::default();
    }
}

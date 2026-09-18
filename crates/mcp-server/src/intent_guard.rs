//! WS4 Intent Guard: deterministic wrong-project / wrong-context detection.
//!
//! The guard is a context-quality layer, not a judge. It never rewrites
//! knowledge, never promotes authority, and never writes anything: it
//! answers one bounded question at packet-assembly time — *does this
//! context plausibly belong to the project the session is actually working
//! in?* — using only signals CodeBro already holds:
//!
//! - the workspace root and the project identity declared for it,
//! - the user's confirmed intents (resolved per (kind, namespace)),
//! - the set of other workspaces the user-context store holds project
//!   knowledge for,
//! - record scope (global vs project vs task) and record ids.
//!
//! Three outcomes, all explainable from the bounded `signals` list:
//!
//! | Verdict | Meaning |
//! |---------|---------|
//! | `aligned` | Identity and/or an actionable intent anchor the viewpoint; no contradiction found |
//! | `review` | At least one contradiction: identity mismatch, a task naming another known workspace, or records referencing another project |
//! | `unverified` | No identity and no intent to align against — the guard cannot vouch either way |
//!
//! Rules (deterministic, lexical, no embeddings, no LLM):
//!
//! 1. A *global* record that names exactly one other known workspace and
//!    does not name the current project is **excluded** — knowledge scoped
//!    to another project must not silently leak into this project's packet.
//! 2. A record that names the current project *and* another known
//!    workspace (or several others) is **flagged**, never dropped: it may
//!    be deliberate cross-project knowledge.
//! 3. Intent records are never excluded, only flagged: a confirmed user
//!    goal is never silently hidden by a heuristic.
//! 4. Project/task-scoped records are never content-scanned: their scope
//!    already binds them to this workspace (and task), which is the
//!    stronger guarantee.
//!
//! Matching is on normalized alphanumeric identifiers (lowercased,
//! separators removed) derived from workspace basenames, the identity name,
//! and the repository URL. Bounds: [`MAX_GUARD_SIGNALS`] signals,
//! [`MAX_GUARD_EXCLUDED_RECORDS`] ids per list, [`MAX_GUARD_REASON_CHARS`]
//! characters per detail, [`MAX_GUARD_KNOWN_WORKSPACES`] other workspaces
//! considered. Output is a pure function of the input: no clocks, no
//! randomness, no I/O.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::collections::{BTreeMap, BTreeSet};

use codebro_context_runtime::{
    ContextRecord, IntentMetadata, RankedRecord, RecordKind, RecordScope, RecordStatus,
};
use serde::{Deserialize, Serialize};

use crate::engineering_context::RecordGuardNote;

/// Maximum signals carried by one report.
pub const MAX_GUARD_SIGNALS: usize = 8;
/// Maximum record ids listed per report list (excluded / flagged).
pub const MAX_GUARD_EXCLUDED_RECORDS: usize = 16;
/// Maximum characters per signal detail.
pub const MAX_GUARD_REASON_CHARS: usize = 160;
/// Maximum other known workspaces considered.
pub const MAX_GUARD_KNOWN_WORKSPACES: usize = 64;

/// Minimum length (normalized) of a *foreign* workspace identifier before it
/// is distinctive enough to match record content.
const MIN_FOREIGN_IDENT_CHARS: usize = 4;
/// Minimum length (normalized) of a current-project identifier.
const MIN_PROJECT_IDENT_CHARS: usize = 3;

/// Deterministic verdict of one guard evaluation.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardVerdict {
    /// Identity and/or an actionable intent anchor the viewpoint.
    Aligned,
    /// A contradiction was found; the signals explain it.
    Review,
    /// No identity and no intent to align against.
    #[default]
    Unverified,
}

impl GuardVerdict {
    pub fn as_str(&self) -> &'static str {
        match self {
            GuardVerdict::Aligned => "aligned",
            GuardVerdict::Review => "review",
            GuardVerdict::Unverified => "unverified",
        }
    }
}

impl std::fmt::Display for GuardVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Signal severity: `warn` drives the `review` verdict; `info` explains
/// without changing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardSeverity {
    Info,
    Warn,
}

fn severity_rank(s: GuardSeverity) -> u8 {
    match s {
        GuardSeverity::Warn => 0,
        GuardSeverity::Info => 1,
    }
}

/// One deterministic finding. `code` is a stable machine identifier; the
/// `detail` never contains record content (ids and workspace basenames
/// only).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardSignal {
    pub code: String,
    pub severity: GuardSeverity,
    pub detail: String,
}

/// Bounded, explainable intent-guard report attached to a context packet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentGuardReport {
    pub verdict: GuardVerdict,
    /// Whether a project identity was available to check.
    pub identity_checked: bool,
    /// Whether the identity matched the workspace (false when unchecked).
    pub identity_matched: bool,
    /// Actionable-intent coverage for this viewpoint:
    /// `task` | `project` | `global` | `none`.
    pub intent_coverage: String,
    /// Number of other known workspaces considered (bounded).
    pub known_workspaces: usize,
    /// Ids of records excluded as foreign to this project.
    pub excluded_records: Vec<String>,
    /// Ids of records kept but annotated as cross-project.
    pub flagged_records: Vec<String>,
    /// Deterministic findings, warnings first.
    pub signals: Vec<GuardSignal>,
}

impl Default for IntentGuardReport {
    fn default() -> Self {
        IntentGuardReport {
            verdict: GuardVerdict::Unverified,
            identity_checked: false,
            identity_matched: false,
            intent_coverage: "none".to_string(),
            known_workspaces: 0,
            excluded_records: Vec::new(),
            flagged_records: Vec::new(),
            signals: Vec::new(),
        }
    }
}

/// The retrieval viewpoint the guard evaluates: where the session is, who
/// the project says it is, what the task says, and which other workspaces
/// the store knows about.
#[derive(Debug, Clone)]
pub struct GuardView<'a> {
    /// Canonical workspace root of the current session.
    pub workspace_root: &'a str,
    /// Declared identity name, when an identity is loaded.
    pub project_name: Option<&'a str>,
    /// Declared repository URL, when present.
    pub repository_url: Option<&'a str>,
    /// Canonical roots of other workspaces with project-scoped records.
    pub known_workspace_roots: &'a [String],
    /// The task text in the caller's own words (may be empty).
    pub task_text: &'a str,
}

/// Guarded records plus the report explaining every decision.
#[derive(Debug, Default)]
pub struct GuardedRecords {
    /// Records that survive the guard, in their original order.
    pub records: Vec<RankedRecord>,
    /// Per-record annotations for kept records (id → note).
    pub notes: BTreeMap<String, RecordGuardNote>,
    /// The bounded, deterministic report.
    pub report: IntentGuardReport,
}

/// Evaluate the guard over resolved records. Pure: same input, same output.
pub fn apply_guard(view: &GuardView<'_>, ranked: Vec<RankedRecord>) -> GuardedRecords {
    let mut out = GuardedRecords::default();
    let current_idents = current_project_idents(view);
    let foreign = foreign_workspace_idents(view, &current_idents);
    let mut signals: Vec<GuardSignal> = Vec::new();

    // ── Identity consistency ──
    let (identity_checked, identity_matched) = check_identity(view);
    if identity_checked && !identity_matched {
        signals.push(warn(
            "identity_mismatch",
            format!(
                "project identity \"{}\" does not match workspace basename \"{}\" — verify this is the intended repository",
                view.project_name.unwrap_or("").trim(),
                basename_of(view.workspace_root)
            ),
        ));
    }

    // ── Task ↔ known-workspace consistency ──
    let task_norm = normalize_ident(view.task_text);
    if !task_norm.is_empty() {
        let mentions_current = current_idents
            .iter()
            .any(|c| task_norm.contains(c.as_str()));
        if !mentions_current {
            if let Some((root, _)) = foreign
                .iter()
                .find(|(_, ident)| task_norm.contains(ident.as_str()))
            {
                signals.push(warn(
                    "task_mentions_foreign_workspace",
                    format!(
                        "task text references another known workspace ({}); verify this session is in the right repository",
                        basename_of(root)
                    ),
                ));
            }
        }
    }

    // ── Record partition (global records only; scope is authoritative otherwise) ──
    let mut excluded: Vec<String> = Vec::new();
    let mut flagged: Vec<String> = Vec::new();
    let mut foreign_examples: Vec<String> = Vec::new();

    for ranked in ranked {
        let record = &ranked.record;
        if record.scope != RecordScope::Global {
            out.records.push(ranked);
            continue;
        }
        let content = normalize_ident(&record.content);
        let foreign_hits: Vec<&(String, String)> = foreign
            .iter()
            .filter(|(_, ident)| content.contains(ident.as_str()))
            .collect();
        if foreign_hits.is_empty() {
            out.records.push(ranked);
            continue;
        }
        let current_hit = current_idents.iter().any(|c| content.contains(c.as_str()));
        let is_intent = record.kind == RecordKind::Intent;
        if is_intent || current_hit || foreign_hits.len() >= 2 {
            out.notes.insert(
                record.id.clone(),
                RecordGuardNote {
                    status: "ambiguous_project_reference".to_string(),
                    reason: bounded_reason(
                        "record references more than one project; treat as cross-project knowledge",
                    ),
                },
            );
            flagged.push(record.id.clone());
            out.records.push(ranked);
        } else {
            foreign_examples.push(basename_of(&foreign_hits[0].0));
            excluded.push(record.id.clone());
        }
    }

    // ── Aggregate findings ──
    if !excluded.is_empty() {
        let example = foreign_examples.first().cloned().unwrap_or_default();
        signals.push(warn(
            "foreign_records_excluded",
            format!(
                "{} global record(s) reference another known workspace ({}) and were excluded from this packet",
                excluded.len(),
                example
            ),
        ));
    }
    if !flagged.is_empty() {
        signals.push(warn(
            "ambiguous_records_flagged",
            format!(
                "{} record(s) reference more than one known project and were flagged, not excluded",
                flagged.len()
            ),
        ));
    }

    // ── Intent coverage ──
    let coverage = intent_coverage(&out.records);
    if coverage == "none" {
        signals.push(info(
            "no_confirmed_intent",
            "no actionable confirmed intent is recorded for this workspace or globally — intent alignment could not be checked",
        ));
    }

    // ── Verdict ──
    let has_warn = signals.iter().any(|s| s.severity == GuardSeverity::Warn);
    let verdict = if has_warn {
        GuardVerdict::Review
    } else if (identity_checked && identity_matched) || coverage != "none" {
        GuardVerdict::Aligned
    } else {
        GuardVerdict::Unverified
    };

    // ── Deterministic, bounded output ──
    signals.sort_by(|a, b| {
        severity_rank(a.severity)
            .cmp(&severity_rank(b.severity))
            .then_with(|| a.code.cmp(&b.code))
            .then_with(|| a.detail.cmp(&b.detail))
    });
    signals.truncate(MAX_GUARD_SIGNALS);
    excluded.sort();
    excluded.truncate(MAX_GUARD_EXCLUDED_RECORDS);
    flagged.sort();
    flagged.truncate(MAX_GUARD_EXCLUDED_RECORDS);

    out.report = IntentGuardReport {
        verdict,
        identity_checked,
        identity_matched,
        intent_coverage: coverage.to_string(),
        known_workspaces: foreign.len(),
        excluded_records: excluded,
        flagged_records: flagged,
        signals,
    };
    out
}

// ── Internal helpers ─────────────────────────────────────────────────────

fn warn(code: &str, detail: impl Into<String>) -> GuardSignal {
    GuardSignal {
        code: code.to_string(),
        severity: GuardSeverity::Warn,
        detail: bounded_reason(&detail.into()),
    }
}

fn info(code: &str, detail: impl Into<String>) -> GuardSignal {
    GuardSignal {
        code: code.to_string(),
        severity: GuardSeverity::Info,
        detail: bounded_reason(&detail.into()),
    }
}

fn bounded_reason(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_GUARD_REASON_CHARS + 1).collect();
    if out.chars().count() > MAX_GUARD_REASON_CHARS {
        out = out.chars().take(MAX_GUARD_REASON_CHARS).collect();
        out.push('…');
    }
    out
}

/// Normalize an identifier for lexical matching: alphanumerics only,
/// lowercased. Separators are removed so "hermes-agent" and "hermes agent"
/// compare equal.
fn normalize_ident(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

fn basename_of(root: &str) -> String {
    std::path::Path::new(root)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn repo_last_segment(url: &str) -> String {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit('/').next().unwrap_or("");
    last.strip_suffix(".git").unwrap_or(last).to_string()
}

/// A declared identity name that is meaningful for matching. The default
/// identity carries the sentinel name `unknown`; placeholders are not
/// declarations.
fn usable_identity_name(name: &str) -> Option<String> {
    let normalized = normalize_ident(name);
    if normalized.len() < MIN_PROJECT_IDENT_CHARS || normalized == "unknown" {
        None
    } else {
        Some(normalized)
    }
}

/// Identifiers that mean "the current project" in record content or task
/// text: workspace basename, identity name, repository last segment.
fn current_project_idents(view: &GuardView<'_>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let base = normalize_ident(&basename_of(view.workspace_root));
    if base.len() >= MIN_PROJECT_IDENT_CHARS {
        out.insert(base);
    }
    if let Some(name) = view.project_name.and_then(usable_identity_name) {
        out.insert(name);
    }
    if let Some(url) = view.repository_url {
        let seg = normalize_ident(&repo_last_segment(url));
        if seg.len() >= MIN_PROJECT_IDENT_CHARS {
            out.insert(seg);
        }
    }
    out
}

/// Distinctive identifiers of *other* known workspaces, in deterministic
/// order. Aliases of the current project and identifiers too short to be
/// distinctive are skipped.
fn foreign_workspace_idents(
    view: &GuardView<'_>,
    current: &BTreeSet<String>,
) -> Vec<(String, String)> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut out: Vec<(String, String)> = Vec::new();
    for root in view.known_workspace_roots {
        if root == view.workspace_root {
            continue;
        }
        let ident = normalize_ident(&basename_of(root));
        if ident.len() < MIN_FOREIGN_IDENT_CHARS {
            continue;
        }
        if current
            .iter()
            .any(|c| c.contains(&ident) || ident.contains(c.as_str()))
        {
            continue;
        }
        if !seen.insert(ident.clone()) {
            continue;
        }
        out.push((root.clone(), ident));
        if out.len() >= MAX_GUARD_KNOWN_WORKSPACES {
            break;
        }
    }
    out
}

/// Whether the declared identity is consistent with the workspace basename.
/// `(checked, matched)`; `(false, false)` when there is nothing to check.
///
/// The identity default carries the sentinel name `unknown`; a placeholder
/// is not a declaration and must never produce a mismatch finding.
fn check_identity(view: &GuardView<'_>) -> (bool, bool) {
    let name = match view.project_name.and_then(usable_identity_name) {
        Some(n) => n,
        None => return (false, false),
    };
    let base = normalize_ident(&basename_of(view.workspace_root));
    if base.is_empty() {
        return (false, false);
    }
    (true, name.contains(&base) || base.contains(&name))
}

/// Strongest actionable-intent coverage among the kept records.
fn intent_coverage(records: &[RankedRecord]) -> &'static str {
    let mut coverage = "none";
    for ranked in records {
        let record = &ranked.record;
        if record.kind != RecordKind::Intent || record.status != RecordStatus::Active {
            continue;
        }
        let actionable = IntentMetadata::read_from(record)
            .map(|m| m.intent_status.is_actionable())
            .unwrap_or(false);
        if !actionable {
            continue;
        }
        match record.scope {
            RecordScope::Task => return "task",
            RecordScope::Project => {
                if coverage == "none" || coverage == "global" {
                    coverage = "project";
                }
            }
            RecordScope::Global => {
                if coverage == "none" {
                    coverage = "global";
                }
            }
        }
    }
    coverage
}

#[cfg(test)]
mod tests {
    use super::*;
    use codebro_context_runtime::{Authority, ContextRecord};

    const CURRENT: &str = "/home/dev/projects/codebro";
    const FOREIGN: &str = "/home/dev/projects/hermes-agent";

    fn ranked(id: &str, kind: RecordKind, scope: RecordScope, content: &str) -> RankedRecord {
        let mut record =
            ContextRecord::new(id, kind, format!("ns.{id}"), content, Authority::AiInferred);
        record.scope = scope;
        record.workspace_root = match scope {
            RecordScope::Global => None,
            _ => Some(CURRENT.to_string()),
        };
        record.updated_at = 1000;
        RankedRecord {
            record,
            bm25: None,
            effective_confidence: 0.8,
        }
    }

    fn view<'a>(task: &'a str, known: &'a [String]) -> GuardView<'a> {
        GuardView {
            workspace_root: CURRENT,
            project_name: Some("CodeBro"),
            repository_url: Some("https://github.com/EffNine/CodeBro.git"),
            known_workspace_roots: known,
            task_text: task,
        }
    }

    fn known() -> Vec<String> {
        vec![CURRENT.to_string(), FOREIGN.to_string()]
    }

    fn ids(records: &[RankedRecord]) -> Vec<&str> {
        records.iter().map(|r| r.record.id.as_str()).collect()
    }

    #[test]
    fn aligned_identity_and_project_intent() {
        let mut intent = ranked(
            "ctx::intent",
            RecordKind::Intent,
            RecordScope::Project,
            "Build CodeBro as the engineering context layer",
        );
        intent.record.authority = Authority::UserConfirmed;
        let records = vec![intent];
        let guarded = apply_guard(&view("improve the context packet", &known()), records);
        assert_eq!(guarded.report.verdict, GuardVerdict::Aligned);
        assert!(guarded.report.identity_checked);
        assert!(guarded.report.identity_matched);
        assert_eq!(guarded.report.intent_coverage, "project");
        assert!(guarded.report.excluded_records.is_empty());
        assert!(
            guarded
                .report
                .signals
                .iter()
                .all(|s| s.severity == GuardSeverity::Info),
            "aligned packet carries no warnings: {:?}",
            guarded.report.signals
        );
    }

    #[test]
    fn identity_mismatch_is_review() {
        let known = known();
        let v = GuardView {
            workspace_root: CURRENT,
            project_name: Some("Hermes Agent"),
            repository_url: None,
            known_workspace_roots: &known,
            task_text: "polish the parser",
        };
        let guarded = apply_guard(&v, Vec::new());
        assert_eq!(guarded.report.verdict, GuardVerdict::Review);
        assert!(guarded.report.identity_checked);
        assert!(!guarded.report.identity_matched);
        assert!(guarded
            .report
            .signals
            .iter()
            .any(|s| s.code == "identity_mismatch" && s.severity == GuardSeverity::Warn));
    }

    #[test]
    fn foreign_global_record_is_excluded() {
        let records = vec![
            ranked(
                "ctx::foreign",
                RecordKind::Preference,
                RecordScope::Global,
                "In hermes-agent always run the soak suite before merging",
            ),
            ranked(
                "ctx::neutral",
                RecordKind::Preference,
                RecordScope::Global,
                "Prefer the simplest reasonable implementation",
            ),
        ];
        let guarded = apply_guard(&view("implement a feature", &known()), records);
        assert_eq!(ids(&guarded.records), vec!["ctx::neutral"]);
        assert_eq!(guarded.report.excluded_records, vec!["ctx::foreign"]);
        assert_eq!(guarded.report.verdict, GuardVerdict::Review);
        assert!(guarded
            .report
            .signals
            .iter()
            .any(|s| s.code == "foreign_records_excluded"));
    }

    #[test]
    fn cross_project_record_is_flagged_not_excluded() {
        let records = vec![ranked(
            "ctx::cross",
            RecordKind::Experience,
            RecordScope::Global,
            "The parser pattern from hermes-agent also applies to codebro",
        )];
        let guarded = apply_guard(&view("improve the parser", &known()), records);
        assert_eq!(ids(&guarded.records), vec!["ctx::cross"]);
        assert!(guarded.report.excluded_records.is_empty());
        assert_eq!(guarded.report.flagged_records, vec!["ctx::cross"]);
        assert!(guarded.notes.contains_key("ctx::cross"));
        assert_eq!(
            guarded.notes["ctx::cross"].status,
            "ambiguous_project_reference"
        );
    }

    #[test]
    fn global_intent_is_flagged_never_excluded() {
        let mut intent = ranked(
            "ctx::foreign-intent",
            RecordKind::Intent,
            RecordScope::Global,
            "Finish the hermes-agent migration",
        );
        intent.record.authority = Authority::UserConfirmed;
        let guarded = apply_guard(&view("current work", &known()), vec![intent]);
        assert_eq!(ids(&guarded.records), vec!["ctx::foreign-intent"]);
        assert!(guarded.report.excluded_records.is_empty());
        assert_eq!(guarded.report.flagged_records, vec!["ctx::foreign-intent"]);
    }

    #[test]
    fn task_mentions_foreign_workspace_warns() {
        let guarded = apply_guard(
            &view("fix the failing hermes agent test", &known()),
            Vec::new(),
        );
        assert_eq!(guarded.report.verdict, GuardVerdict::Review);
        assert!(guarded
            .report
            .signals
            .iter()
            .any(|s| s.code == "task_mentions_foreign_workspace"));
    }

    #[test]
    fn task_mentioning_both_projects_does_not_warn() {
        let guarded = apply_guard(
            &view("port codebro patterns into hermes-agent", &known()),
            Vec::new(),
        );
        assert!(
            guarded
                .report
                .signals
                .iter()
                .all(|s| s.code != "task_mentions_foreign_workspace"),
            "cross-project task is not a mismatch: {:?}",
            guarded.report.signals
        );
    }

    #[test]
    fn unknown_sentinel_is_not_a_declared_identity() {
        let known = known();
        let v = GuardView {
            workspace_root: CURRENT,
            project_name: Some("unknown"),
            repository_url: None,
            known_workspace_roots: &known,
            task_text: "",
        };
        let guarded = apply_guard(&v, Vec::new());
        assert!(!guarded.report.identity_checked);
        assert!(
            guarded
                .report
                .signals
                .iter()
                .all(|s| s.code != "identity_mismatch"),
            "placeholder identity must not warn: {:?}",
            guarded.report.signals
        );
    }

    #[test]
    fn no_identity_no_intent_is_unverified() {
        let known = known();
        let v = GuardView {
            workspace_root: CURRENT,
            project_name: None,
            repository_url: None,
            known_workspace_roots: &known,
            task_text: "",
        };
        let guarded = apply_guard(&v, Vec::new());
        assert_eq!(guarded.report.verdict, GuardVerdict::Unverified);
        assert!(!guarded.report.identity_checked);
        assert_eq!(guarded.report.intent_coverage, "none");
        assert!(guarded
            .report
            .signals
            .iter()
            .any(|s| s.code == "no_confirmed_intent" && s.severity == GuardSeverity::Info));
    }

    #[test]
    fn report_is_deterministic_and_order_independent() {
        let build = || {
            vec![
                ranked(
                    "ctx::a",
                    RecordKind::Preference,
                    RecordScope::Global,
                    "hermes-agent prefers verbose logs",
                ),
                ranked(
                    "ctx::b",
                    RecordKind::Preference,
                    RecordScope::Global,
                    "hermes-agent and codebro share the parser",
                ),
                ranked(
                    "ctx::c",
                    RecordKind::Pattern,
                    RecordScope::Global,
                    "always run cargo fmt",
                ),
            ]
        };
        let first = apply_guard(&view("work", &known()), build());
        let mut shuffled = build();
        shuffled.reverse();
        let second = apply_guard(&view("work", &known()), shuffled);
        assert_eq!(first.report, second.report);
        assert_eq!(first.report.excluded_records, vec!["ctx::a"]);
        assert_eq!(first.report.flagged_records, vec!["ctx::b"]);
    }

    #[test]
    fn report_is_bounded() {
        let mut records: Vec<RankedRecord> = (0..100)
            .map(|i| {
                ranked(
                    &format!("ctx::foreign-{i:03}"),
                    RecordKind::Preference,
                    RecordScope::Global,
                    "hermes-agent convention",
                )
            })
            .collect();
        records.push(ranked(
            "ctx::ambig",
            RecordKind::Preference,
            RecordScope::Global,
            "hermes-agent and codebro both use tree-sitter",
        ));
        let guarded = apply_guard(&view("work", &known()), records);
        assert!(guarded.report.excluded_records.len() <= MAX_GUARD_EXCLUDED_RECORDS);
        assert!(guarded.report.flagged_records.len() <= MAX_GUARD_EXCLUDED_RECORDS);
        assert!(guarded.report.signals.len() <= MAX_GUARD_SIGNALS);
        for signal in &guarded.report.signals {
            assert!(
                signal.detail.chars().count() <= MAX_GUARD_REASON_CHARS + 1,
                "unbounded detail: {}",
                signal.detail
            );
        }
    }

    #[test]
    fn report_never_leaks_record_content() {
        let secret = "sk-live-supersecret-0123456789";
        let records = vec![ranked(
            "ctx::secret",
            RecordKind::Preference,
            RecordScope::Global,
            &format!("hermes-agent token {secret} must never leak"),
        )];
        let guarded = apply_guard(&view("work", &known()), records);
        let serialized = serde_json::to_string(&guarded.report).unwrap();
        assert!(!serialized.contains(secret), "report leaked content");
        assert!(!serialized.contains("must never leak"));
        assert!(serialized.contains("ctx::secret"), "ids remain for audit");
    }

    #[test]
    fn substring_of_current_project_is_not_foreign() {
        let mut known = known();
        known.push("/home/dev/projects/codebro-old".to_string());
        let records = vec![ranked(
            "ctx::old",
            RecordKind::Preference,
            RecordScope::Global,
            "codebro-old experiments are archived",
        )];
        let guarded = apply_guard(&view("work", &known), records);
        assert_eq!(
            ids(&guarded.records),
            vec!["ctx::old"],
            "an alias of the current project must not be treated as foreign"
        );
        assert!(guarded.report.excluded_records.is_empty());
    }

    #[test]
    fn project_scoped_records_are_not_content_scanned() {
        let records = vec![ranked(
            "ctx::proj",
            RecordKind::Experience,
            RecordScope::Project,
            "hermes-agent comparison notes kept in this project",
        )];
        let guarded = apply_guard(&view("work", &known()), records);
        assert_eq!(ids(&guarded.records), vec!["ctx::proj"]);
        assert!(guarded.report.excluded_records.is_empty());
        assert!(guarded.report.flagged_records.is_empty());
    }

    #[test]
    fn global_intent_coverage_is_reported() {
        let mut intent = ranked(
            "ctx::global-intent",
            RecordKind::Intent,
            RecordScope::Global,
            "Keep memory unified across sessions",
        );
        intent.record.authority = Authority::UserConfirmed;
        let guarded = apply_guard(&view("work", &known()), vec![intent]);
        assert_eq!(guarded.report.intent_coverage, "global");
        assert_eq!(guarded.report.verdict, GuardVerdict::Aligned);
    }

    #[test]
    fn task_scoped_intent_outranks_project_and_global() {
        let mut task = ranked(
            "ctx::task-intent",
            RecordKind::Intent,
            RecordScope::Task,
            "Finish the guard tests",
        );
        task.record.task_id = Some("task-1".to_string());
        let mut global = ranked(
            "ctx::global-intent",
            RecordKind::Intent,
            RecordScope::Global,
            "Keep memory unified",
        );
        global.record.authority = Authority::UserConfirmed;
        let guarded = apply_guard(&view("work", &known()), vec![task, global]);
        assert_eq!(guarded.report.intent_coverage, "task");
    }
}

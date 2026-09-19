//! P15 skill-evolution validation: determine whether a successor skill
//! version is demonstrably better than its predecessor, from evidence.
//!
//! ```text
//! ACTIVE skill v1
//!         │
//!         ▼
//! multiple v1 executions (skill-linked history, same workspace,
//!                         attributed to the v1 window)
//!         │
//!         ▼
//! P13 detect_evolution → validated candidate → human approval
//!         │
//!         ▼
//! ACTIVE skill v2 (v1 rows immutable)
//!         │
//!         ▼
//! multiple v2 executions (same workspace, attributed to the v2 window)
//!         │
//!         ▼
//! P15 validate_evolution (this module — deterministic, read-only)
//!         │
//!         ▼
//! IMPROVED | REGRESSED | NO_MEASURABLE_IMPROVEMENT |
//! INCONCLUSIVE | INSUFFICIENT_EVIDENCE (+ next action)
//! ```
//!
//! # What validation is (and is not)
//!
//! - **Validation, not self-improvement.** This module never mints
//!   candidates, never publishes versions, never rolls back, never triggers
//!   `v2 → v3`. It reads history + versions and reports a verdict. The
//!   human remains the final authority: `IMPROVED` suggests `keep_v2`,
//!   `REGRESSED` suggests `review_rollback` (the existing `rollback_skill`
//!   restores v1 on explicit human decision), anything else suggests
//!   collecting more evidence. No automatic publish, no automatic rollback,
//!   no automatic skill mutation.
//! - **Explicitly invoked.** OpenCode calls `skill validate_evolution`
//!   (alias `compare_versions`); nothing runs in the background. No
//!   scheduler, daemon, watcher, executor, or agent.
//! - **Deterministic and read-only.** Same history + same versions ⇒ same
//!   report, byte for byte (sorted sets, capped arithmetic, stable
//!   tie-breaks). Repeated comparisons converge without writing anything:
//!   duplicate comparison is a no-op by construction. No new tables, no new
//!   memory subsystem, no embeddings, no ML, no A/B framework, no
//!   statistical inference.
//! - **Conservative.** A newer version is never "better" for being newer,
//!   longer, human-approved, or higher-confidence. `IMPROVED` requires
//!   actual outcome evidence with enough comparable executions and a
//!   meaningful margin. Tiny samples, all-neutral windows, unrelated
//!   workflows, and mixed scopes all read as `INSUFFICIENT_EVIDENCE`, never
//!   as improvement. One successful execution never overpowers multiple
//!   failures (minimums enforce this structurally).
//! - **Version-attributed.** Every skill-linked execution belongs to exactly
//!   one version window — the latest version activated strictly before the
//!   execution — or to no window at all. The bound is strict (`>`, not
//!   `>=`): timestamps have second resolution, so an execution recorded in
//!   the same second as any activation is conservatively excluded rather
//!   than risk misattribution across the boundary. Pre-lineage executions
//!   (older than the first compared version) are excluded the same way.
//! - **Targeted.** When the caller names the weakness that caused v2
//!   (e.g. `verification_gap` from the P13 candidate), the comparison
//!   measures that failure pattern specifically instead of treating every
//!   unrelated success/failure equally. An overall rate gain that leaves
//!   the targeted pattern unfixed is `INCONCLUSIVE`, not `IMPROVED`.
//!
//! # Reused systems (no new subsystem)
//!
//! - Skill lifecycle + versions (`skills.rs`): identity, lineage,
//!   immutability, rollback, selection (active version).
//! - Execution evidence: skill-linked history events (`tool = 'skill'`,
//!   summary/payload names the skill) — the same linkage the P13 detector
//!   mines, reused verbatim via `skill_evolution::fetch_linked_events`
//!   (no drift between detection and validation).
//! - Outcome vocabulary (`learning::outcome_polarity`): success / failure /
//!   neutral. Unknown labels are neutral — validation never guesses.
//! - Weakness taxonomy (`skill_evolution`): the same verification/timeout
//!   token and outcome matching the detector used, so "the exact weakness
//!   that caused v2" means the same thing at validation time.
//! - Approval protocol + rollback + selection: unchanged. Validation only
//!   *recommends*; humans decide through the existing seams.
//!
//! # Verdicts (conservative, deterministic)
//!
//! - `insufficient_evidence` — too few comparable executions, all-neutral
//!   windows, unrelated workflows, or mixed scopes. Never an improvement
//!   claim.
//! - `improved` — v2's success rate exceeds v1's by at least the margin,
//!   with enough evidence on both sides, and (when a weakness is named)
//!   the targeted failure pattern actually decreased.
//! - `no_measurable_improvement` — enough evidence, rates within the
//!   margin, no conflicting targeted signal. v2 is different, not
//!   demonstrably better.
//! - `regressed` — v2's success rate trails v1's by at least the margin.
//!   Surfaces `review_rollback`; never rolls back automatically.
//! - `inconclusive` — enough evidence but conflicting signals (overall vs
//!   targeted disagree). Conservative by design: collects more evidence
//!   instead of claiming either direction.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};

use crate::learning::{outcome_polarity, OutcomePolarity};
use crate::skill_evolution::{fetch_linked_events, is_known_weakness_kind, is_targeted_failure};
use crate::skills::{Skill, SkillVersion};
use crate::store::{ContextError, ContextStore};
use crate::workspace::canonical_workspace_key;

// ─── Constants ──────────────────────────────────────────────────────────

/// Minimum skill-linked executions attributed to EACH compared version
/// before any directional verdict may be returned. Below this on either
/// side the report is `insufficient_evidence` — tiny samples never carry
/// an improvement or regression claim.
pub const VALIDATION_MIN_EXECUTIONS_PER_VERSION: usize = 5;
/// Minimum decisive (success + failure) executions per version. An
/// all-neutral window (unknown outcomes) is evidence of nothing, no matter
/// how many executions it holds.
pub const VALIDATION_MIN_DECISIVE_PER_VERSION: usize = 3;
/// Overall success-rate margin: |v2 − v1| must reach this to count as a
/// directional signal. Smaller deltas read as noise.
pub const VALIDATION_IMPROVEMENT_MARGIN: f64 = 0.15;
/// Targeted failure-rate improvement required (drop in the named weakness
/// pattern) before an overall gain may be called `improved`.
pub const VALIDATION_TARGET_IMPROVEMENT_MARGIN: f64 = 0.10;
/// Maximum event ids echoed per sample list in the wire view (the
/// comparison scans all attributed events; the view stays small).
pub const VALIDATION_MAX_VIEW_IDS: usize = 20;
/// Maximum distinct failure labels echoed per version (bounded pattern
/// surface, deterministic order).
pub const VALIDATION_MAX_LABELS: usize = 8;

/// Verdict: not enough comparable evidence to say anything directional.
pub const VERDICT_INSUFFICIENT_EVIDENCE: &str = "insufficient_evidence";
/// Verdict: v2 is demonstrably better than v1 on the evidence.
pub const VERDICT_IMPROVED: &str = "improved";
/// Verdict: enough evidence, no meaningful difference.
pub const VERDICT_NO_MEASURABLE_IMPROVEMENT: &str = "no_measurable_improvement";
/// Verdict: v2 is demonstrably worse than v1 on the evidence.
pub const VERDICT_REGRESSED: &str = "regressed";
/// Verdict: enough evidence but conflicting signals (overall vs targeted).
pub const VERDICT_INCONCLUSIVE: &str = "inconclusive";

/// One attributed execution: (event id, outcome label, summary).
type AttributedExecution = (i64, Option<String>, String);
/// Version number → attributed executions (ascending, deterministic).
type VersionBuckets = BTreeMap<u32, Vec<AttributedExecution>>;

/// Next actions (machine-readable, model-friendly).
pub const NEXT_KEEP_V2: &str = "keep_v2";
pub const NEXT_REVIEW_ROLLBACK: &str = "review_rollback";
pub const NEXT_COLLECT_MORE_EVIDENCE: &str = "collect_more_evidence";
pub const NEXT_KEEP_CURRENT_OBSERVE: &str = "keep_current_observe";
pub const NEXT_HUMAN_REVIEW_COLLECT_MORE: &str = "human_review_collect_more";

// ─── Types ──────────────────────────────────────────────────────────────

/// Per-version outcome evidence: counts, rates, patterns,bounded samples.
///
/// `success_rate` is `None` when the window holds no decisive executions
/// (all neutral) — a rate over nothing is never fabricated. Targeted
/// fields are `None` when no weakness kind was supplied for the comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VersionWindowStats {
    pub version_number: u32,
    pub version_id: String,
    pub content_hash: String,
    pub created_at: u64,
    pub executions: usize,
    pub successes: usize,
    pub failures: usize,
    pub neutrals: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_rate: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted_failures: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted_failure_rate: Option<f64>,
    /// Bounded sorted sample of attributed event ids (audit pointers).
    #[serde(default)]
    pub evidence_sample: Vec<i64>,
    /// Bounded sorted sample of attributed targeted-failure ids.
    #[serde(default)]
    pub targeted_sample: Vec<i64>,
    /// Bounded failure-label histogram (label → count, top entries,
    /// deterministic order) so the human sees *which* failures dominate.
    #[serde(default)]
    pub failure_labels: BTreeMap<String, usize>,
}

/// Bounded, model-friendly result of one validation pass.
///
/// Read-only by construction: building this report writes nothing (no
/// candidates, no versions, no learning, no history). Repeated validation
/// of unchanged state returns an equal report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ValidationReport {
    pub skill: String,
    pub skill_id: String,
    pub workspace_root: String,
    pub skill_status: String,
    pub from_version: u32,
    pub to_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weakness_kind: Option<String>,
    pub from_stats: VersionWindowStats,
    pub to_stats: VersionWindowStats,
    /// v2 rate − v1 rate (`None` when either side has no decisive evidence).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub success_rate_delta: Option<f64>,
    /// v2 targeted-failure rate − v1 targeted-failure rate (negative is
    /// good; `None` when no weakness was supplied or a side has no
    /// executions to rate over).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted_delta: Option<f64>,
    /// One of `insufficient_evidence` | `improved` |
    /// `no_measurable_improvement` | `regressed` | `inconclusive`.
    pub verdict: String,
    /// Conservative descriptive confidence (two decimals, never a
    /// significance claim). `0.0` when evidence is insufficient.
    pub confidence: f64,
    /// `keep_v2` | `review_rollback` | `collect_more_evidence` |
    /// `keep_current_observe` | `human_review_collect_more`.
    pub next_action: String,
    /// One plain sentence for OpenCode/the user (no internals).
    pub message: String,
    pub note: String,
}

// ─── Pure helpers (deterministic, unit-testable without a DB) ───────────

/// Success rate over decisive executions, rounded to four decimals for
/// deterministic wire output. `None` when there is nothing decisive.
fn success_rate(successes: usize, failures: usize) -> Option<f64> {
    let decisive = successes + failures;
    if decisive == 0 {
        return None;
    }
    let rate = successes as f64 / decisive as f64;
    Some((rate * 10_000.0).round() / 10_000.0)
}

/// Targeted failure rate over ALL attributed executions (the pattern's
/// share of everything the version did). `None` when the window is empty.
fn targeted_rate(targeted_failures: usize, executions: usize) -> Option<f64> {
    if executions == 0 {
        return None;
    }
    let rate = targeted_failures as f64 / executions as f64;
    Some((rate * 10_000.0).round() / 10_000.0)
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

/// Decide the verdict from two fully-computed windows. Pure: no I/O, no
/// writes, no guessing. Callers must have already attributed executions
/// conservatively (same-second excluded, pre-lineage excluded).
#[allow(clippy::too_many_arguments)]
fn decide_verdict(
    from_executions: usize,
    from_successes: usize,
    from_failures: usize,
    from_targeted: Option<usize>,
    to_executions: usize,
    to_successes: usize,
    to_failures: usize,
    to_targeted: Option<usize>,
    weakness_present: bool,
) -> (String, f64, String, String) {
    let from_decisive = from_successes + from_failures;
    let to_decisive = to_successes + to_failures;

    // Gate 0: minimums. Tiny samples, all-neutral windows, and unrelated
    // workflows (which attribute nothing) all land here.
    if from_executions < VALIDATION_MIN_EXECUTIONS_PER_VERSION
        || to_executions < VALIDATION_MIN_EXECUTIONS_PER_VERSION
        || from_decisive < VALIDATION_MIN_DECISIVE_PER_VERSION
        || to_decisive < VALIDATION_MIN_DECISIVE_PER_VERSION
    {
        return (
            VERDICT_INSUFFICIENT_EVIDENCE.to_string(),
            0.0,
            NEXT_COLLECT_MORE_EVIDENCE.to_string(),
            "v2 has insufficient evidence.".to_string(),
        );
    }
    let (Some(from_rate), Some(to_rate)) = (
        success_rate(from_successes, from_failures),
        success_rate(to_successes, to_failures),
    ) else {
        return (
            VERDICT_INSUFFICIENT_EVIDENCE.to_string(),
            0.0,
            NEXT_COLLECT_MORE_EVIDENCE.to_string(),
            "v2 has insufficient evidence.".to_string(),
        );
    };
    let delta = to_rate - from_rate;
    let total = from_executions + to_executions;

    // Confidence is descriptive volume + margin, never a significance
    // claim, and never high on minimum-only evidence.
    let confidence_for = |abs_delta: f64| -> f64 {
        if total >= 20 && abs_delta >= 0.30 {
            0.80
        } else if total >= 10 && abs_delta >= VALIDATION_IMPROVEMENT_MARGIN {
            0.65
        } else if abs_delta >= VALIDATION_IMPROVEMENT_MARGIN {
            0.60
        } else {
            0.55
        }
    };

    if !weakness_present {
        if delta >= VALIDATION_IMPROVEMENT_MARGIN {
            let c = confidence_for(delta.abs());
            return (
                VERDICT_IMPROVED.to_string(),
                c,
                NEXT_KEEP_V2.to_string(),
                "v2 improved outcomes over v1 based on evidence.".to_string(),
            );
        }
        if delta <= -VALIDATION_IMPROVEMENT_MARGIN {
            let c = confidence_for(delta.abs());
            return (
                VERDICT_REGRESSED.to_string(),
                c,
                NEXT_REVIEW_ROLLBACK.to_string(),
                "v2 regressed and rollback is recommended.".to_string(),
            );
        }
        return (
            VERDICT_NO_MEASURABLE_IMPROVEMENT.to_string(),
            0.55,
            NEXT_KEEP_CURRENT_OBSERVE.to_string(),
            "v2 shows no measurable improvement over v1 yet.".to_string(),
        );
    }

    // Weakness-targeted path: the exact pattern that caused v2 must move.
    let (Some(from_t), Some(to_t)) = (from_targeted, to_targeted) else {
        return (
            VERDICT_INSUFFICIENT_EVIDENCE.to_string(),
            0.0,
            NEXT_COLLECT_MORE_EVIDENCE.to_string(),
            "v2 has insufficient evidence.".to_string(),
        );
    };
    let from_tr = targeted_rate(from_t, from_executions).unwrap_or(0.0);
    let to_tr = targeted_rate(to_t, to_executions).unwrap_or(0.0);
    let t_delta = to_tr - from_tr;
    let from_had_targeted_signal = from_t > 0;

    if delta >= VALIDATION_IMPROVEMENT_MARGIN {
        // Overall gain — but it only counts as improved when the named
        // weakness actually decreased (or there was no targeted signal in
        // v1 to fix). Otherwise the gain came from elsewhere and the
        // creation reason stands unfixed: inconclusive, not improved.
        if !from_had_targeted_signal || t_delta <= -VALIDATION_TARGET_IMPROVEMENT_MARGIN {
            let c = confidence_for(delta.abs());
            return (
                VERDICT_IMPROVED.to_string(),
                c,
                NEXT_KEEP_V2.to_string(),
                "v2 improved the failure pattern it was created to address.".to_string(),
            );
        }
        return (
            VERDICT_INCONCLUSIVE.to_string(),
            0.45,
            NEXT_HUMAN_REVIEW_COLLECT_MORE.to_string(),
            "Evidence is inconclusive; more comparable executions are needed.".to_string(),
        );
    }
    if delta <= -VALIDATION_IMPROVEMENT_MARGIN {
        let c = confidence_for(delta.abs());
        return (
            VERDICT_REGRESSED.to_string(),
            c,
            NEXT_REVIEW_ROLLBACK.to_string(),
            "v2 regressed and rollback is recommended.".to_string(),
        );
    }
    // Overall flat. A strong targeted move under a flat overall still
    // conflicts (one pattern fixed, another appearing): inconclusive
    // rather than either directional claim.
    if from_had_targeted_signal && t_delta.abs() >= VALIDATION_TARGET_IMPROVEMENT_MARGIN + 0.10 {
        return (
            VERDICT_INCONCLUSIVE.to_string(),
            0.45,
            NEXT_HUMAN_REVIEW_COLLECT_MORE.to_string(),
            "Evidence is inconclusive; more comparable executions are needed.".to_string(),
        );
    }
    (
        VERDICT_NO_MEASURABLE_IMPROVEMENT.to_string(),
        0.55,
        NEXT_KEEP_CURRENT_OBSERVE.to_string(),
        "v2 shows no measurable improvement over v1 yet.".to_string(),
    )
}

// ─── Store entry point ──────────────────────────────────────────────────

impl ContextStore {
    /// P15 entry point: compare two versions of one skill on their
    /// attributed execution evidence and report whether the successor is
    /// demonstrably better. Explicitly invoked, read-only, deterministic.
    ///
    /// - `skill_selector` names one skill (id or name, as visible from
    ///   `workspace_root`). Cross-workspace, unknown, or empty selectors
    ///   are caller errors, never verdicts.
    /// - `from_version` / `to_version` pin the comparison window. When
    ///   absent, the two most recent versions compare. `from` must be
    ///   strictly older than `to`; equal, inverted, or unknown versions
    ///   are caller errors (stale references surface here, not as data).
    /// - `weakness_kind` optionally names the P13 weakness that caused the
    ///   successor (`verification_gap` | `timeout_gap` |
    ///   `reliability_gap`). Unknown kinds are caller errors; `None`
    ///   compares overall outcomes only.
    pub fn validate_skill_evolution(
        &self,
        workspace_root: &str,
        skill_selector: &str,
        from_version: Option<u32>,
        to_version: Option<u32>,
        weakness_kind: Option<&str>,
    ) -> Result<ValidationReport, ContextError> {
        let ws = canonical_workspace_key(workspace_root);
        if ws.is_empty() {
            return Err(ContextError::Validation(
                "evolution validation requires a workspace_root".to_string(),
            ));
        }
        let sel = skill_selector.trim();
        if sel.is_empty() {
            return Err(ContextError::Validation(
                "evolution validation requires a skill selector (skill id or name)".to_string(),
            ));
        }
        let weakness = weakness_kind
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        if let Some(ref k) = weakness {
            if !is_known_weakness_kind(k) {
                return Err(ContextError::Validation(format!(
                    "unknown weakness kind '{k}': use verification_gap, timeout_gap, or reliability_gap"
                )));
            }
        }

        // Resolve one skill, honoring visibility (project skills never leak
        // across workspaces; global skills are visible everywhere).
        let skill: Skill = self
            .get_skill(sel)?
            .or(self.get_skill_by_name(sel)?.filter(|s| {
                s.scope == "global"
                    || s.workspace_root.as_deref().map(canonical_workspace_key) == Some(ws.clone())
            }))
            .ok_or_else(|| {
                ContextError::Validation(format!(
                    "skill not found (by id or name, visible from this workspace): {sel}"
                ))
            })?;
        if skill.scope == "project" {
            let owns =
                skill.workspace_root.as_deref().map(canonical_workspace_key) == Some(ws.clone());
            if !owns {
                return Err(ContextError::Validation(format!(
                    "skill '{}' belongs to another workspace — validation does not leak across workspaces",
                    skill.name
                )));
            }
        }

        // Version identity: every compared version must exist. Rollback
        // rows are ordinary versions here (new number, old content) — the
        // attribution is by activation time, never by content equality.
        let mut versions: Vec<SkillVersion> = self.list_skill_versions(&skill.skill_id, 100)?;
        versions.sort_by_key(|v| v.version_number);
        if versions.is_empty() {
            return Err(ContextError::Validation(format!(
                "skill '{}' has no recorded versions: nothing to compare",
                skill.name
            )));
        }
        let (from_n, to_n) = match (from_version, to_version) {
            (Some(f), Some(t)) => (f, t),
            (Some(f), None) => {
                let latest = versions.last().map(|v| v.version_number).unwrap_or(f);
                (f, latest)
            }
            (None, Some(t)) => {
                let prev = versions
                    .iter()
                    .rev()
                    .find(|v| v.version_number < t)
                    .map(|v| v.version_number);
                match prev {
                    Some(p) => (p, t),
                    None => {
                        return Err(ContextError::Validation(format!(
                            "skill '{}' has no version older than v{t}: nothing to compare",
                            skill.name
                        )));
                    }
                }
            }
            (None, None) => {
                if versions.len() < 2 {
                    return Err(ContextError::Validation(format!(
                        "skill '{}' has only v{}: publish a successor before validating",
                        skill.name, versions[0].version_number
                    )));
                }
                let n = versions.len();
                (
                    versions[n - 2].version_number,
                    versions[n - 1].version_number,
                )
            }
        };
        if from_n == to_n {
            return Err(ContextError::Validation(
                "from_version and to_version must differ (duplicate comparison)".to_string(),
            ));
        }
        if from_n > to_n {
            return Err(ContextError::Validation(
                "from_version must be older than to_version".to_string(),
            ));
        }
        let from_v = versions
            .iter()
            .find(|v| v.version_number == from_n)
            .ok_or_else(|| {
                ContextError::Validation(format!(
                    "version v{from_n} not found for skill '{}' (stale version reference)",
                    skill.name
                ))
            })?;
        let to_v = versions
            .iter()
            .find(|v| v.version_number == to_n)
            .ok_or_else(|| {
                ContextError::Validation(format!(
                    "version v{to_n} not found for skill '{}' (stale version reference)",
                    skill.name
                ))
            })?;

        // Linked executions in this workspace (same linkage as detection),
        // minus any execution that also names another active skill (that
        // execution is evidence about the peer, not this skill).
        let linked = self.with_conn(|conn| fetch_linked_events(conn, &ws, &skill))?;
        let peer_names: Vec<String> = self
            .list_skills(Some(&ws), None, 100)?
            .into_iter()
            .filter(|s| s.skill_id != skill.skill_id && s.status == "active")
            .map(|s| s.name.to_ascii_lowercase())
            .collect();

        // Activation boundaries, ascending. Same-second executions are
        // unattributed (conservative): the set below implements the strict
        // `>` discipline without comparing floats or guessing.
        let activations: BTreeMap<u32, u64> = versions
            .iter()
            .map(|v| (v.version_number, v.created_at))
            .collect();
        let activation_instants: HashSet<u64> = activations.values().copied().collect();

        // Attribute each linked execution to its owning window.
        let mut buckets: VersionBuckets = BTreeMap::new();
        for v in versions.iter().map(|v| v.version_number) {
            buckets.insert(v, Vec::new());
        }
        for e in &linked {
            if activation_instants.contains(&e.created_at) {
                continue; // ambiguous same-second: attribute to nothing.
            }
            let haystack = format!("{}\n{}", e.summary, e.payload.as_deref().unwrap_or(""))
                .to_ascii_lowercase();
            if peer_names
                .iter()
                .any(|n| !n.is_empty() && haystack.contains(n))
            {
                continue; // context-pollution guard: peer evidence excluded.
            }
            // Latest activation strictly before the execution owns it.
            let owner = activations
                .iter()
                .filter(|(_, at)| **at < e.created_at)
                .max_by_key(|(_, at)| **at)
                .map(|(n, _)| *n);
            if let Some(n) = owner {
                if let Some(bucket) = buckets.get_mut(&n) {
                    bucket.push((e.id, e.outcome.clone(), e.summary.clone()));
                }
            }
            // Older than every activation ⇒ pre-lineage: unattributed.
        }

        let from_stats = window_stats(from_v, &buckets, skill.name.as_str(), weakness.as_deref());
        let to_stats = window_stats(to_v, &buckets, skill.name.as_str(), weakness.as_deref());

        let delta = match (from_stats.success_rate, to_stats.success_rate) {
            (Some(f), Some(t)) => Some(((t - f) * 10_000.0).round() / 10_000.0),
            _ => None,
        };
        let targeted_delta = match (
            weakness.as_deref(),
            from_stats.targeted_failure_rate,
            to_stats.targeted_failure_rate,
        ) {
            (Some(_), Some(f), Some(t)) => Some(((t - f) * 10_000.0).round() / 10_000.0),
            _ => None,
        };

        let (verdict, confidence, next_action, message) = decide_verdict(
            from_stats.executions,
            from_stats.successes,
            from_stats.failures,
            from_stats.targeted_failures,
            to_stats.executions,
            to_stats.successes,
            to_stats.failures,
            to_stats.targeted_failures,
            weakness.is_some(),
        );

        let note = if skill.status != "active" {
            format!(
                "P15 validation (read-only, deterministic): v{from_n} vs v{to_n} of '{}' \
                 (status '{}') compared on skill-linked executions in this workspace only \
                 (same-second boundary executions excluded, peer-skill and other-workspace \
                 evidence excluded). Verdict {verdict} with next action {next_action}. \
                 The skill is not active, so rollback is unavailable — human review decides. \
                 Validation never publishes, never rolls back, and never triggers a successor: \
                 a successful v2 does not auto-propose v3.",
                skill.name, skill.status
            )
        } else {
            format!(
                "P15 validation (read-only, deterministic): v{from_n} vs v{to_n} of '{}' \
                 compared on skill-linked executions in this workspace only (same-second \
                 boundary executions excluded, peer-skill and other-workspace evidence \
                 excluded). Verdict {verdict} with next action {next_action}. Approved is \
                 not improved: only outcome evidence moves the verdict. Validation never \
                 publishes, never rolls back, and never triggers a successor: a successful \
                 v2 does not auto-propose v3.",
                skill.name
            )
        };

        Ok(ValidationReport {
            skill: skill.name.clone(),
            skill_id: skill.skill_id.clone(),
            workspace_root: ws,
            skill_status: skill.status.clone(),
            from_version: from_n,
            to_version: to_n,
            weakness_kind: weakness,
            from_stats,
            to_stats,
            success_rate_delta: delta,
            targeted_delta,
            verdict,
            confidence: round2(confidence),
            next_action,
            message,
            note,
        })
    }
}

/// Build the per-version window statistics from attributed executions.
fn window_stats(
    version: &SkillVersion,
    buckets: &VersionBuckets,
    skill_name: &str,
    weakness_kind: Option<&str>,
) -> VersionWindowStats {
    let empty: Vec<AttributedExecution> = Vec::new();
    let rows = buckets.get(&version.version_number).unwrap_or(&empty);
    let mut successes = 0usize;
    let mut failures = 0usize;
    let mut neutrals = 0usize;
    let mut targeted = 0usize;
    let mut targeted_ids: Vec<i64> = Vec::new();
    let mut ids: Vec<i64> = Vec::new();
    let mut label_counts: BTreeMap<String, usize> = BTreeMap::new();
    for (id, outcome, summary) in rows {
        ids.push(*id);
        match outcome_polarity(outcome.as_deref()) {
            OutcomePolarity::Success => successes += 1,
            OutcomePolarity::Failure => {
                failures += 1;
                let label = outcome
                    .as_deref()
                    .unwrap_or("failure")
                    .trim()
                    .to_ascii_lowercase();
                *label_counts.entry(label).or_default() += 1;
                if let Some(kind) = weakness_kind {
                    if is_targeted_failure(kind, outcome.as_deref(), summary, skill_name) {
                        targeted += 1;
                        targeted_ids.push(*id);
                    }
                }
            }
            OutcomePolarity::Neutral => neutrals += 1,
        }
    }
    ids.sort_unstable();
    targeted_ids.sort_unstable();
    // Failure labels: most-frequent first, label-ascending ties, capped.
    let mut labels: Vec<(String, usize)> = label_counts.into_iter().collect();
    labels.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    labels.truncate(VALIDATION_MAX_LABELS);
    let failure_labels: BTreeMap<String, usize> = labels.into_iter().collect();

    let executions = rows.len();
    VersionWindowStats {
        version_number: version.version_number,
        version_id: version.version_id.clone(),
        content_hash: version.content_hash.clone(),
        created_at: version.created_at,
        executions,
        successes,
        failures,
        neutrals,
        success_rate: success_rate(successes, failures),
        targeted_failures: weakness_kind.map(|_| targeted),
        targeted_failure_rate: weakness_kind.and_then(|_| targeted_rate(targeted, executions)),
        evidence_sample: ids.into_iter().take(VALIDATION_MAX_VIEW_IDS).collect(),
        targeted_sample: targeted_ids
            .into_iter()
            .take(VALIDATION_MAX_VIEW_IDS)
            .collect(),
        failure_labels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{self, STATE_DB_FILE};
    use crate::history::{HistoryInput, HistoryKind, OpenSession};
    use crate::skills::{
        SkillApplicability, SkillCandidate, SkillCandidateStatus, SkillScope,
        SKILL_CANDIDATE_TTL_SECS,
    };

    const WS: &str = "/repo";
    const NOW: u64 = 1_700_000_000;

    fn test_store(dir: &std::path::Path) -> ContextStore {
        let db_path = dir.join(STATE_DB_FILE);
        db::open_checked(&db_path).unwrap();
        ContextStore::new(db_path)
    }

    fn skills_dir(dir: &std::path::Path) -> std::path::PathBuf {
        let p = dir.join("skills");
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn v1_content(name: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: A test skill for {name}\n---\n\n\
             # Purpose\n\nFollow inspect, modify, verify.\n\n\
             # Procedure\n\n1. Inspect.\n2. Modify.\n3. Verify.\n"
        )
    }

    fn v2_content(name: &str, marker: &str) -> String {
        format!(
            "---\nname: {name}\ndescription: A test skill for {name} ({marker})\n---\n\n\
             # Purpose\n\nTest procedure body for {marker}.\n\n\
             # Procedure\n\n1. Inspect.\n2. Modify.\n3. Verify.\n"
        )
    }

    /// Publish an active v1 skill through the real lifecycle.
    fn publish_v1(
        store: &ContextStore,
        skills: &std::path::Path,
        name: &str,
        ws: &str,
        at: u64,
    ) -> Skill {
        let content = v1_content(name);
        let scope = SkillScope::Project;
        let candidate_id =
            crate::skills::mint_skill_candidate_id(&scope, Some(ws), None, name, &content);
        let candidate = SkillCandidate {
            candidate_id: candidate_id.clone(),
            workspace_root: Some(ws.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: name.to_string(),
            description: format!("Skill {name}"),
            purpose: "Testing validation".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: Vec::new(),
            supporting_evidence: vec![1, 2, 3],
            contradicting_evidence: Vec::new(),
            proposed_content: content,
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
            .approve_skill_candidate(&candidate_id, Some(ws), skills, at + 2)
            .unwrap();
        assert_eq!(skill.current_version, 1);
        skill
    }

    /// Publish a manual successor (v_current+1) with distinct content.
    fn publish_successor(
        store: &ContextStore,
        skills: &std::path::Path,
        skill: &Skill,
        ws: &str,
        at: u64,
        marker: &str,
    ) -> Skill {
        let content = v2_content(&skill.name, marker);
        let scope = SkillScope::Project;
        let candidate_id =
            crate::skills::mint_skill_candidate_id(&scope, Some(ws), None, &skill.name, &content);
        let candidate = SkillCandidate {
            candidate_id: candidate_id.clone(),
            workspace_root: Some(ws.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: skill.name.clone(),
            description: format!("Successor of {}", skill.name),
            purpose: "Testing validation successor".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: Vec::new(),
            supporting_evidence: vec![10, 11, 12],
            contradicting_evidence: Vec::new(),
            proposed_content: content,
            status: SkillCandidateStatus::Candidate.as_str().to_string(),
            confidence: 0.75,
            validation: None,
            eval_reason: None,
            rejection_reason: None,
            supersedes_skill: Some(skill.skill_id.clone()),
            based_on_version: Some(skill.current_version),
            created_at: at,
            updated_at: at,
            expires_at: Some(at + SKILL_CANDIDATE_TTL_SECS),
        };
        store.insert_skill_candidate(&candidate).unwrap();
        store
            .evaluate_candidate_content(&candidate_id, at + 1)
            .unwrap();
        let (updated, version) = store
            .approve_skill_candidate(&candidate_id, Some(ws), skills, at + 2)
            .unwrap();
        assert_eq!(version.version_number, skill.current_version + 1);
        updated
    }

    /// One skill-linked execution event (tool=skill, summary names skill).
    fn seed_execution(
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

    fn success_summary(skill: &str, ver: u32, note: &str) -> String {
        format!("skill '{skill}' execution recorded as success (v{ver}): {note}")
    }

    fn failure_summary(skill: &str, ver: u32, note: &str) -> String {
        format!("skill '{skill}' execution recorded as failure (v{ver}): {note}")
    }

    const VERIFY_NOTE: &str = "sandbox verify test run failed, checks not confirmed";

    fn validate(
        store: &ContextStore,
        ws: &str,
        sel: &str,
        from: Option<u32>,
        to: Option<u32>,
        weakness: Option<&str>,
    ) -> ValidationReport {
        store
            .validate_skill_evolution(ws, sel, from, to, weakness)
            .expect("validation must not hard-error")
    }

    /// Seed N successes + M failures for one version window.
    #[allow(clippy::too_many_arguments)]
    fn seed_window(
        store: &ContextStore,
        ws: &str,
        skill: &str,
        ver: u32,
        successes: usize,
        failures: usize,
        failure_outcome: &str,
        failure_note: &str,
        base: u64,
    ) {
        for i in 0..successes {
            seed_execution(
                store,
                ws,
                skill,
                "success",
                &success_summary(skill, ver, "workflow completed cleanly"),
                base + i as u64,
            );
        }
        for i in 0..failures {
            seed_execution(
                store,
                ws,
                skill,
                failure_outcome,
                &failure_summary(skill, ver, failure_note),
                base + 100 + i as u64,
            );
        }
    }

    // ── CASE A: improved ─────────────────────────────────────────────

    #[test]
    fn improved_case_a_overall() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let v1at = NOW - 1000;
        let skill = publish_v1(&store, &skills, "mcp-tool-change", WS, v1at);
        // v1 window: 10 executions, 7 successful, 3 failed.
        seed_window(
            &store,
            WS,
            "mcp-tool-change",
            1,
            7,
            3,
            "failure",
            "deploy rollout stalled",
            NOW - 900,
        );
        // v2 approved, then v2 window: 12 executions, 11 successful, 1 failed.
        let v2at = NOW - 400;
        publish_successor(&store, &skills, &skill, WS, v2at, "v2");
        seed_window(
            &store,
            WS,
            "mcp-tool-change",
            2,
            11,
            1,
            "failure",
            "deploy rollout stalled",
            NOW - 300,
        );
        let r = validate(&store, WS, "mcp-tool-change", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_IMPROVED, "{r:?}");
        assert_eq!(r.next_action, NEXT_KEEP_V2);
        assert_eq!(r.from_stats.executions, 10);
        assert_eq!(r.from_stats.successes, 7);
        assert_eq!(r.from_stats.failures, 3);
        assert_eq!(r.to_stats.executions, 12);
        assert_eq!(r.to_stats.successes, 11);
        assert_eq!(r.to_stats.failures, 1);
        assert!(r.success_rate_delta.unwrap() > 0.15);
        assert!(r.message.contains("improved"));
        assert!(r.note.contains("never triggers a successor"));
    }

    #[test]
    fn improved_targeted_weakness_validation() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "verify-demo", WS, NOW - 1000);
        // v1: verification-shaped failures dominate.
        seed_window(
            &store,
            WS,
            "verify-demo",
            1,
            5,
            5,
            "test_failure",
            VERIFY_NOTE,
            NOW - 900,
        );
        let v2at = NOW - 400;
        publish_successor(&store, &skills, &skill, WS, v2at, "v2");
        // v2: verification failures gone, one unrelated failure remains.
        seed_window(
            &store,
            WS,
            "verify-demo",
            2,
            9,
            1,
            "failure",
            "deploy rollout stalled",
            NOW - 300,
        );
        let r = validate(
            &store,
            WS,
            "verify-demo",
            Some(1),
            Some(2),
            Some("verification_gap"),
        );
        assert_eq!(r.verdict, VERDICT_IMPROVED, "{r:?}");
        assert_eq!(r.next_action, NEXT_KEEP_V2);
        assert_eq!(
            r.message,
            "v2 improved the failure pattern it was created to address."
        );
        assert!(r.targeted_delta.unwrap() < 0.0, "{r:?}");
        assert_eq!(r.from_stats.targeted_failures.unwrap(), 5);
        assert_eq!(r.to_stats.targeted_failures.unwrap(), 0);
    }

    #[test]
    fn overall_gain_without_targeted_fix_is_inconclusive() {
        // Overall rate gain (+20%) while the named weakness pattern does
        // NOT decrease must read as inconclusive, never improved: the
        // creation reason stands unfixed and the gain came from elsewhere.
        let (verdict, _, next, _) = decide_verdict(10, 5, 5, Some(5), 10, 7, 3, Some(5), true);
        // from 50% → to 70% (+20%) but targeted 5/10=50% → 5/10=50%
        // (flat): must be inconclusive, never improved.
        assert_eq!(verdict, VERDICT_INCONCLUSIVE);
        assert_eq!(next, NEXT_HUMAN_REVIEW_COLLECT_MORE);

        // Same rule at the store level: v1 mixes generic + verification
        // failures (targeted 2/10), v2 is better overall but ALL its
        // remaining failures are verification-shaped (targeted 3/10, up).
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "verify-demo", WS, NOW - 1000);
        for i in 0..5 {
            seed_execution(
                &store,
                WS,
                "verify-demo",
                "success",
                &success_summary("verify-demo", 1, "clean"),
                NOW - 900 + i,
            );
        }
        for i in 0..3 {
            seed_execution(
                &store,
                WS,
                "verify-demo",
                "failure",
                &failure_summary("verify-demo", 1, "deploy rollout stalled"),
                NOW - 890 + i,
            );
        }
        for i in 0..2 {
            seed_execution(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                &failure_summary("verify-demo", 1, VERIFY_NOTE),
                NOW - 880 + i,
            );
        }
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        for i in 0..7 {
            seed_execution(
                &store,
                WS,
                "verify-demo",
                "success",
                &success_summary("verify-demo", 2, "clean"),
                NOW - 300 + i,
            );
        }
        for i in 0..3 {
            seed_execution(
                &store,
                WS,
                "verify-demo",
                "test_failure",
                &failure_summary("verify-demo", 2, VERIFY_NOTE),
                NOW - 290 + i,
            );
        }
        let r = validate(
            &store,
            WS,
            "verify-demo",
            Some(1),
            Some(2),
            Some("verification_gap"),
        );
        // v1: 50% overall, targeted 20%. v2: 70% overall (+20%), targeted
        // 30% (worse, certainly not a ≥10% drop) → inconclusive.
        assert_eq!(r.verdict, VERDICT_INCONCLUSIVE, "{r:?}");
        assert_eq!(r.next_action, NEXT_HUMAN_REVIEW_COLLECT_MORE);
    }

    // ── CASE B: regressed + rollback ─────────────────────────────────

    #[test]
    fn regressed_case_b_overall() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "mcp-tool-change", WS, NOW - 1000);
        // v1: 8 successes / 2 failures.
        seed_window(
            &store,
            WS,
            "mcp-tool-change",
            1,
            8,
            2,
            "failure",
            "deploy rollout stalled",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        // v2: 4 successes / 6 failures.
        seed_window(
            &store,
            WS,
            "mcp-tool-change",
            2,
            4,
            6,
            "failure",
            "deploy rollout stalled",
            NOW - 300,
        );
        let r = validate(&store, WS, "mcp-tool-change", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_REGRESSED, "{r:?}");
        assert_eq!(r.next_action, NEXT_REVIEW_ROLLBACK);
        assert_eq!(r.message, "v2 regressed and rollback is recommended.");
    }

    #[test]
    fn rollback_after_regressed_restores_v1() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "mcp-tool-change", WS, NOW - 1000);
        let v1_hash = store
            .get_active_skill_version(&skill.skill_id)
            .unwrap()
            .unwrap()
            .content_hash;
        seed_window(
            &store,
            WS,
            "mcp-tool-change",
            1,
            8,
            2,
            "failure",
            "deploy rollout stalled",
            NOW - 900,
        );
        let v2 = publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        assert_eq!(v2.current_version, 2);
        seed_window(
            &store,
            WS,
            "mcp-tool-change",
            2,
            4,
            6,
            "failure",
            "deploy rollout stalled",
            NOW - 300,
        );
        let r = validate(&store, WS, "mcp-tool-change", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_REGRESSED);
        // Human chooses rollback: existing mechanism restores v1 content
        // as a NEW version (history append-only).
        let (rolled, new_version) = store
            .rollback_skill(&skill.skill_id, Some(WS), 1, &skills, NOW + 10)
            .unwrap();
        assert_eq!(rolled.current_version, 3);
        assert_eq!(new_version.version_number, 3);
        assert_eq!(new_version.content_hash, v1_hash, "v3 carries v1 content");
        // v2 remains in history; lineage preserved; evidence preserved.
        let versions = store.list_skill_versions(&skill.skill_id, 10).unwrap();
        assert_eq!(versions.len(), 3);
        assert!(versions.iter().any(|v| v.version_number == 1));
        assert!(versions.iter().any(|v| v.version_number == 2));
        // Future selection points at the correct active version (v3).
        let current = store.get_skill(&skill.skill_id).unwrap().unwrap();
        assert_eq!(current.current_version, 3);
        let active = store
            .get_active_skill_version(&skill.skill_id)
            .unwrap()
            .unwrap();
        assert_eq!(active.version_number, 3);
        assert_eq!(active.content_hash, v1_hash);
        // Validation of v2 vs v3 still reads the preserved windows.
        let r2 = validate(&store, WS, "mcp-tool-change", Some(1), Some(2), None);
        assert_eq!(
            r2.verdict, VERDICT_REGRESSED,
            "evidence preserved across rollback"
        );
    }

    // ── Insufficient evidence ────────────────────────────────────────

    #[test]
    fn insufficient_on_tiny_samples() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "thin-skill", WS, NOW - 1000);
        // v1: 2 executions; v2: 1 execution.
        seed_window(
            &store,
            WS,
            "thin-skill",
            1,
            1,
            1,
            "failure",
            "deploy rollout stalled",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        seed_execution(
            &store,
            WS,
            "thin-skill",
            "success",
            &success_summary("thin-skill", 2, "lone run"),
            NOW - 300,
        );
        let r = validate(&store, WS, "thin-skill", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_INSUFFICIENT_EVIDENCE, "{r:?}");
        assert_eq!(r.next_action, NEXT_COLLECT_MORE_EVIDENCE);
        assert_eq!(r.message, "v2 has insufficient evidence.");
        assert_eq!(r.confidence, 0.0);
    }

    #[test]
    fn insufficient_when_v2_has_no_executions() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "fresh-v2", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "fresh-v2",
            1,
            7,
            3,
            "failure",
            "deploy rollout stalled",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        let r = validate(&store, WS, "fresh-v2", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_INSUFFICIENT_EVIDENCE, "{r:?}");
    }

    #[test]
    fn insufficient_when_all_neutral() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "neutral-skill", WS, NOW - 1000);
        for i in 0..6 {
            seed_execution(
                &store,
                WS,
                "neutral-skill",
                "partial",
                &format!("skill 'neutral-skill' execution recorded as partial (v1): step {i}"),
                NOW - 900 + i,
            );
        }
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        for i in 0..6 {
            seed_execution(
                &store,
                WS,
                "neutral-skill",
                "partial",
                &format!("skill 'neutral-skill' execution recorded as partial (v2): step {i}"),
                NOW - 300 + i,
            );
        }
        let r = validate(&store, WS, "neutral-skill", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_INSUFFICIENT_EVIDENCE, "{r:?}");
        assert_eq!(r.from_stats.success_rate, None);
    }

    #[test]
    fn insufficient_for_unrelated_workflows() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "lonely-skill", WS, NOW - 1000);
        // Failures with other tools and no skill linkage: unattributed.
        for i in 0..6 {
            let session = store
                .open_session(WS, &OpenSession::default(), NOW - 900 + i)
                .unwrap();
            let mut input =
                HistoryInput::new(WS, HistoryKind::Validation, "sandbox checks failed badly");
            input.session_id = Some(session.id.clone());
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("test_failure".to_string());
            input.created_at = Some(NOW - 900 + i);
            store.record_history(&input, NOW - 900 + i).unwrap();
        }
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        for i in 0..6 {
            let session = store
                .open_session(WS, &OpenSession::default(), NOW - 300 + i)
                .unwrap();
            let mut input =
                HistoryInput::new(WS, HistoryKind::Validation, "sandbox checks failed badly");
            input.session_id = Some(session.id.clone());
            input.tool = Some("sandbox_test".to_string());
            input.outcome = Some("test_failure".to_string());
            input.created_at = Some(NOW - 300 + i);
            store.record_history(&input, NOW - 300 + i).unwrap();
        }
        let r = validate(&store, WS, "lonely-skill", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_INSUFFICIENT_EVIDENCE, "{r:?}");
        assert_eq!(r.from_stats.executions, 0);
        assert_eq!(r.to_stats.executions, 0);
    }

    #[test]
    fn no_measurable_improvement_on_flat_rates() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "flat-skill", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "flat-skill",
            1,
            7,
            3,
            "failure",
            "deploy rollout stalled",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        seed_window(
            &store,
            WS,
            "flat-skill",
            2,
            7,
            3,
            "failure",
            "deploy rollout stalled",
            NOW - 300,
        );
        let r = validate(&store, WS, "flat-skill", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_NO_MEASURABLE_IMPROVEMENT, "{r:?}");
        assert_eq!(r.next_action, NEXT_KEEP_CURRENT_OBSERVE);
    }

    // ── Confidence / contradiction unit rules ────────────────────────

    #[test]
    fn verdict_units_are_conservative() {
        // Strong improvement signal.
        let (v, c, n, _) = decide_verdict(12, 8, 4, None, 12, 11, 1, None, false);
        assert_eq!(v, VERDICT_IMPROVED);
        assert_eq!(n, NEXT_KEEP_V2);
        assert!(c >= 0.60, "{c}");
        // Weak improvement signal (within margin): no measurable.
        let (v, _, _, _) = decide_verdict(10, 6, 4, None, 10, 7, 3, None, false);
        assert_eq!(v, VERDICT_NO_MEASURABLE_IMPROVEMENT);
        // Same failure recurring in v2 at the same rate: flat → no measurable.
        let (v, _, _, _) = decide_verdict(10, 5, 5, Some(5), 10, 5, 5, Some(5), true);
        assert_eq!(v, VERDICT_NO_MEASURABLE_IMPROVEMENT);
        // One success cannot overpower multiple failures: tiny v2 sample
        // is insufficient, never improved.
        let (v, _, _, _) = decide_verdict(10, 5, 5, None, 1, 1, 0, None, false);
        assert_eq!(v, VERDICT_INSUFFICIENT_EVIDENCE);
        // Contradictory: overall up but targeted flat → inconclusive.
        let (v, _, _, _) = decide_verdict(10, 5, 5, Some(5), 10, 7, 3, Some(5), true);
        assert_eq!(v, VERDICT_INCONCLUSIVE);
    }

    // ── Adversarial ──────────────────────────────────────────────────

    #[test]
    fn unknown_skill_is_caller_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let err = store
            .validate_skill_evolution(WS, "no-such-skill", None, None, None)
            .unwrap_err();
        assert!(err.to_string().contains("skill not found"), "{err}");
    }

    #[test]
    fn different_workspace_does_not_leak() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "ws-skill", "/repo-a", NOW - 1000);
        seed_window(
            &store,
            "/repo-a",
            "ws-skill",
            1,
            6,
            4,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, "/repo-a", NOW - 400, "v2");
        seed_window(
            &store,
            "/repo-a",
            "ws-skill",
            2,
            8,
            2,
            "failure",
            "deploy stall",
            NOW - 300,
        );
        // Foreign workspace: the skill is not visible there.
        let err = store
            .validate_skill_evolution("/repo-b", "ws-skill", Some(1), Some(2), None)
            .unwrap_err();
        assert!(
            err.to_string().contains("another workspace") || err.to_string().contains("not found"),
            "{err}"
        );
        // Events recorded in the foreign workspace never count at home.
        for i in 0..10 {
            seed_execution(
                &store,
                "/repo-b",
                "ws-skill",
                "failure",
                &failure_summary("ws-skill", 2, "foreign failure"),
                NOW - 250 + i,
            );
        }
        let r = validate(&store, "/repo-a", "ws-skill", Some(1), Some(2), None);
        assert_eq!(r.to_stats.executions, 10, "{r:?}");
    }

    #[test]
    fn different_skill_evidence_is_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let a = publish_v1(&store, &skills, "skill-alpha", WS, NOW - 1000);
        publish_v1(&store, &skills, "skill-beta", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "skill-alpha",
            1,
            6,
            4,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        publish_successor(&store, &skills, &a, WS, NOW - 400, "v2");
        seed_window(
            &store,
            WS,
            "skill-alpha",
            2,
            8,
            2,
            "failure",
            "deploy stall",
            NOW - 300,
        );
        // Peer executions naming skill-beta must not pollute alpha.
        for i in 0..10 {
            seed_execution(
                &store,
                WS,
                "skill-beta",
                "failure",
                &failure_summary("skill-beta", 1, "beta failure"),
                NOW - 290 + i,
            );
        }
        // An execution naming BOTH skills is excluded as ambiguous.
        seed_execution(
            &store,
            WS,
            "skill-alpha",
            "failure",
            "skill 'skill-alpha' execution with skill-beta mentioned as peer",
            NOW - 280,
        );
        let r = validate(&store, WS, "skill-alpha", Some(1), Some(2), None);
        assert_eq!(r.from_stats.executions, 10, "{r:?}");
        assert_eq!(r.to_stats.executions, 10, "{r:?}");
    }

    #[test]
    fn stale_version_is_caller_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "stale-skill", WS, NOW - 1000);
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        let err = store
            .validate_skill_evolution(WS, "stale-skill", Some(1), Some(99), None)
            .unwrap_err();
        assert!(err.to_string().contains("stale"), "{err}");
        let err = store
            .validate_skill_evolution(WS, "stale-skill", Some(2), Some(2), None)
            .unwrap_err();
        assert!(err.to_string().contains("must differ"), "{err}");
        let err = store
            .validate_skill_evolution(WS, "stale-skill", Some(2), Some(1), None)
            .unwrap_err();
        assert!(err.to_string().contains("must be older"), "{err}");
    }

    #[test]
    fn duplicate_comparison_is_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "dup-skill", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "dup-skill",
            1,
            7,
            3,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        seed_window(
            &store,
            WS,
            "dup-skill",
            2,
            11,
            1,
            "failure",
            "deploy stall",
            NOW - 300,
        );
        let first = validate(&store, WS, "dup-skill", Some(1), Some(2), None);
        let second = validate(&store, WS, "dup-skill", Some(1), Some(2), None);
        assert_eq!(first, second, "repeated comparison must converge");
    }

    #[test]
    fn validation_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "clean-skill", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "clean-skill",
            1,
            7,
            3,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        seed_window(
            &store,
            WS,
            "clean-skill",
            2,
            11,
            1,
            "failure",
            "deploy stall",
            NOW - 300,
        );
        let cands_before = store
            .list_skill_candidates(None, None, None, 100)
            .unwrap()
            .len();
        let vers_before = store
            .list_skill_versions(&skill.skill_id, 100)
            .unwrap()
            .len();
        let lc_before = store
            .list_candidates(Some(WS), None, None, 100)
            .unwrap()
            .len();
        let _ = validate(&store, WS, "clean-skill", Some(1), Some(2), None);
        let _ = validate(
            &store,
            WS,
            "clean-skill",
            Some(1),
            Some(2),
            Some("reliability_gap"),
        );
        let cands_after = store
            .list_skill_candidates(None, None, None, 100)
            .unwrap()
            .len();
        let vers_after = store
            .list_skill_versions(&skill.skill_id, 100)
            .unwrap()
            .len();
        let lc_after = store
            .list_candidates(Some(WS), None, None, 100)
            .unwrap()
            .len();
        assert_eq!(
            cands_before, cands_after,
            "validation must not mint candidates (no auto v3)"
        );
        assert_eq!(
            vers_before, vers_after,
            "validation must not publish versions"
        );
        assert_eq!(lc_before, lc_after, "validation must not write learning");
    }

    #[test]
    fn restarted_database_preserves_validation() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join(STATE_DB_FILE);
        db::open_checked(&db_path).unwrap();
        let skills = skills_dir(dir.path());
        let before = {
            let store = ContextStore::new(db_path.clone());
            let skill = publish_v1(&store, &skills, "restart-skill", WS, NOW - 1000);
            seed_window(
                &store,
                WS,
                "restart-skill",
                1,
                7,
                3,
                "failure",
                "deploy stall",
                NOW - 900,
            );
            publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
            seed_window(
                &store,
                WS,
                "restart-skill",
                2,
                11,
                1,
                "failure",
                "deploy stall",
                NOW - 300,
            );
            validate(&store, WS, "restart-skill", Some(1), Some(2), None)
        };
        assert_eq!(before.verdict, VERDICT_IMPROVED);
        // Drop the store (simulated restart) and revalidate from disk.
        let reopened = ContextStore::new(db_path);
        let after = validate(&reopened, WS, "restart-skill", Some(1), Some(2), None);
        assert_eq!(before, after, "evidence/version state must survive restart");
    }

    #[test]
    fn deprecated_skill_still_validates_with_note() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "old-skill", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "old-skill",
            1,
            8,
            2,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        seed_window(
            &store,
            WS,
            "old-skill",
            2,
            4,
            6,
            "failure",
            "deploy stall",
            NOW - 300,
        );
        store
            .deprecate_skill(&skill.skill_id, Some(WS), &skills, None, NOW + 5)
            .unwrap();
        let r = validate(&store, WS, "old-skill", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_REGRESSED, "{r:?}");
        assert_eq!(r.skill_status, "deprecated");
        assert!(r.note.contains("not active"), "{note}", note = r.note);
        // History preserved: versions still list v1 and v2.
        assert_eq!(
            store
                .list_skill_versions(&skill.skill_id, 10)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn boundary_same_second_executions_are_excluded() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let v1at = NOW - 1000;
        let skill = publish_v1(&store, &skills, "edge-skill", WS, v1at);
        let v1_active = store
            .get_active_skill_version(&skill.skill_id)
            .unwrap()
            .unwrap();
        // An execution stamped at exactly the v1 activation instant is
        // ambiguous: excluded, never attributed to v1.
        seed_execution(
            &store,
            WS,
            "edge-skill",
            "failure",
            &failure_summary("edge-skill", 1, "boundary failure"),
            v1_active.created_at,
        );
        seed_window(
            &store,
            WS,
            "edge-skill",
            1,
            6,
            4,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        let v2at = NOW - 400;
        publish_successor(&store, &skills, &skill, WS, v2at, "v2");
        let v2_active = store
            .list_skill_versions(&skill.skill_id, 10)
            .unwrap()
            .into_iter()
            .find(|v| v.version_number == 2)
            .unwrap();
        seed_execution(
            &store,
            WS,
            "edge-skill",
            "success",
            &success_summary("edge-skill", 2, "boundary success"),
            v2_active.created_at,
        );
        seed_window(
            &store,
            WS,
            "edge-skill",
            2,
            8,
            2,
            "failure",
            "deploy stall",
            NOW - 300,
        );
        let r = validate(&store, WS, "edge-skill", Some(1), Some(2), None);
        assert_eq!(r.from_stats.executions, 10, "{r:?}");
        assert_eq!(r.to_stats.executions, 10, "{r:?}");
        // Pre-lineage executions (older than v1) are excluded too.
        seed_execution(
            &store,
            WS,
            "edge-skill",
            "failure",
            &failure_summary("edge-skill", 0, "pre-lineage failure"),
            v1at - 500,
        );
        let r2 = validate(&store, WS, "edge-skill", Some(1), Some(2), None);
        assert_eq!(r, r2, "pre-lineage evidence must not move the comparison");
    }

    #[test]
    fn approved_is_not_improved_without_evidence() {
        // A freshly approved v2 with almost no executions: human approval
        // happened, but the verdict must stay insufficient — approved ≠
        // improved, newer ≠ better.
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "newbie-skill", WS, NOW - 1000);
        seed_window(
            &store,
            WS,
            "newbie-skill",
            1,
            6,
            4,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        seed_execution(
            &store,
            WS,
            "newbie-skill",
            "success",
            &success_summary("newbie-skill", 2, "single good run"),
            NOW - 300,
        );
        let r = validate(&store, WS, "newbie-skill", Some(1), Some(2), None);
        assert_eq!(r.verdict, VERDICT_INSUFFICIENT_EVIDENCE, "{r:?}");
        assert!(
            !matches!(r.verdict.as_str(), "improved"),
            "approval must not imply improvement"
        );
    }

    #[test]
    fn unknown_weakness_kind_is_caller_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        let skill = publish_v1(&store, &skills, "wk-skill", WS, NOW - 1000);
        publish_successor(&store, &skills, &skill, WS, NOW - 400, "v2");
        let err = store
            .validate_skill_evolution(WS, "wk-skill", Some(1), Some(2), Some("turbo_gap"))
            .unwrap_err();
        assert!(err.to_string().contains("unknown weakness kind"), "{err}");
    }

    #[test]
    fn single_version_skill_is_caller_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        publish_v1(&store, &skills, "solo-skill", WS, NOW - 1000);
        let err = store
            .validate_skill_evolution(WS, "solo-skill", None, None, None)
            .unwrap_err();
        assert!(err.to_string().contains("only v1"), "{err}");
    }

    // ── Long horizon ─────────────────────────────────────────────────

    #[test]
    fn long_horizon_improved_then_regressed_with_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let store = test_store(dir.path());
        let skills = skills_dir(dir.path());
        // Scenario 1: v1 active → weakness → v2 approved → v2 executes →
        // comparison → IMPROVED.
        let _skill = publish_v1(&store, &skills, "journey-skill", WS, NOW - 2000);
        for i in 0..5 {
            seed_execution(
                &store,
                WS,
                "journey-skill",
                "test_failure",
                &failure_summary("journey-skill", 1, VERIFY_NOTE),
                NOW - 1900 + i,
            );
        }
        let report = store
            .detect_skill_evolution(WS, Some("journey-skill"), None, NOW - 1800)
            .unwrap();
        assert_eq!(report.status, "candidate_found", "{report:?}");
        let sc_id = report.candidates[0].skill_candidate_id.clone().unwrap();
        store
            .approve_skill_candidate(&sc_id, Some(WS), &skills, NOW - 1700)
            .unwrap();
        // v1 window needs ≥5 executions for validation: top up with the
        // detector's 5 plus linked successes/failures to reach 10.
        seed_window(
            &store,
            WS,
            "journey-skill",
            1,
            5,
            0,
            "failure",
            "deploy stall",
            NOW - 1650,
        );
        for i in 0..11 {
            seed_execution(
                &store,
                WS,
                "journey-skill",
                "success",
                &success_summary("journey-skill", 2, "clean post-fix run"),
                NOW - 1600 + i,
            );
        }
        seed_execution(
            &store,
            WS,
            "journey-skill",
            "failure",
            &failure_summary("journey-skill", 2, "deploy rollout stalled"),
            NOW - 1500,
        );
        let improved = validate(
            &store,
            WS,
            "journey-skill",
            Some(1),
            Some(2),
            Some("verification_gap"),
        );
        // v1 window: detector's 5 verification failures + 5 successes =
        // 10 exec (50%); v2: 12 exec (91.7%) with zero targeted failures.
        assert_eq!(improved.verdict, VERDICT_IMPROVED, "{improved:?}");
        assert_eq!(improved.next_action, NEXT_KEEP_V2);
        // A successful v2 does NOT auto-trigger v2 → v3.
        let again = store
            .detect_skill_evolution(WS, Some("journey-skill"), None, NOW - 1400)
            .unwrap();
        assert_eq!(again.status, "no_candidates", "{again:?}");

        // Scenario 2 on a second lineage: v1 → v2 → REGRESSED → rollback.
        let skill2 = publish_v1(&store, &skills, "fragile-skill", WS, NOW - 1200);
        seed_window(
            &store,
            WS,
            "fragile-skill",
            1,
            8,
            2,
            "failure",
            "deploy stall",
            NOW - 1100,
        );
        publish_successor(&store, &skills, &skill2, WS, NOW - 1000, "v2");
        seed_window(
            &store,
            WS,
            "fragile-skill",
            2,
            4,
            6,
            "failure",
            "deploy stall",
            NOW - 900,
        );
        let regressed = validate(&store, WS, "fragile-skill", Some(1), Some(2), None);
        assert_eq!(regressed.verdict, VERDICT_REGRESSED, "{regressed:?}");
        // Rollback AFTER the v2 window closes (v2 failures sit at
        // NOW-800..NOW-795, so the v3 activation must be strictly newer
        // or the tail would attribute to v3 instead of v2).
        store
            .rollback_skill(&skill2.skill_id, Some(WS), 1, &skills, NOW - 700)
            .unwrap();
        let current = store.get_skill(&skill2.skill_id).unwrap().unwrap();
        assert_eq!(current.current_version, 3);
        // Restart: all evidence/version state survives.
        let db_path = dir.path().join(STATE_DB_FILE);
        drop(store);
        let reopened = ContextStore::new(db_path);
        let survived = validate(
            &reopened,
            WS,
            "journey-skill",
            Some(1),
            Some(2),
            Some("verification_gap"),
        );
        assert_eq!(survived.verdict, VERDICT_IMPROVED, "{survived:?}");
        let survived2 = validate(&reopened, WS, "fragile-skill", Some(1), Some(2), None);
        assert_eq!(survived2.verdict, VERDICT_REGRESSED, "{survived2:?}");
        let versions = reopened.list_skill_versions(&current.skill_id, 10).unwrap();
        assert_eq!(versions.len(), 3, "rollback lineage must survive restart");
    }
}

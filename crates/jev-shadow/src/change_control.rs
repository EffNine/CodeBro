//! Phase-9 change control: version locks + regression oracles for Jev advisory.
//! Phase-10 baseline lock: the validated baseline record (`baseline.json`,
//! [`BaselineRecord`]) is cross-checked against the live consts on EVERY
//! advisory evaluation ([`verify_baseline`]).
//!
//! Safety contract (read before touching this file):
//! - Jev remains ADVISORY-ONLY. This module adds NO authority surface: no
//!   tool handle, no task handle, no sandbox handle, no policy handle. It
//!   only answers "is advisory currently allowed?" and pins frozen
//!   regression expectations. The deterministic CodeBro policy remains the
//!   sole authority; the reference oracles below are TEST-ONLY frozen
//!   mirrors of the pre-existing v3 rubric used to pin regression fixtures.
//!   They are never called from any execution path.
//! - Locks: model (`LOCKED_MODEL`), question set (`LOCKED_QUESTION_SET_*`),
//!   state schema (`STATE_SCHEMA_VERSION`), confidence threshold
//!   (`LOCKED_CONFIDENCE_THRESHOLD`, cross-checked against the advisory
//!   module's own const), advisory scope (escalation-only, cross-checked),
//!   and authority owner. Any mismatch forces advisory OFF (the advisory
//!   choke point `evaluate_advisory` returns `None`) until revalidated.
//!   Flags default OFF.
//! - Do NOT modify the frozen v3 questions here: this module only READS them
//!   (via `wire_questions_for_version`) to compute the lock hash. Any edit
//!   to `questions.rs` v3 wording changes the computed hash, fails the lock,
//!   and disables advisory automatically.
//! - Do NOT "fix" a baseline trip by editing `baseline.json` to match drifted
//!   code. A trip means revalidation is required: fresh evidence, deliberate
//!   re-freeze, new baseline id. Silent re-baselining is a REJECT finding.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use sha2::{Digest, Sha256};
use std::sync::OnceLock;

/// Validated baseline id (`jev-baseline-1`, locked 2026-09-17 from Phase
/// 3–10 evidence). A new baseline requires a new id, never a silent edit.
pub const BASELINE_ID: &str = "jev-baseline-1";
/// Authority owner: the deterministic CodeBro policy (and the human) decide.
/// Jev never has authority. Changing this const trips the baseline gate.
pub const AUTHORITY_OWNER: &str = "deterministic-codebro-policy";

/// Validated Jev model. Advisory is allowed ONLY when both the requested
/// and the provider-resolved model equal this value. Any model change
/// (requested or resolved) forces advisory OFF until revalidated.
pub const LOCKED_MODEL: &str = "jev-1.13.0";

/// Validated question-set version. Advisory is escalation-only on v3.
pub const LOCKED_QUESTION_SET_VERSION: &str = "v3";
/// Validated advisory question id (escalation-only, unchanged from Phase 8).
pub const LOCKED_QUESTION_ID: &str = "escalation";

/// Expected sha256 of the canonical v3 question-set rendering (see
/// [`current_v3_question_set_hash`]). Recorded in
/// `/tmp/opencode/jev-phase9/question-lock.json`. A recompute mismatch
/// means the question set changed -> advisory OFF until revalidated.
///
/// Value computed 2026-09-17 from the frozen v3 set; pinned by
/// `question_lock_hash_matches_expected`.
pub const EXPECTED_V3_QUESTION_SET_HASH: &str =
    "595d8e90667cb351e4e91fa38ab2ac0c661fec27e024a958b2fd5aa4ccea0919";

/// Advisory confidence gate (frozen, same as Phase 8). Advisory output
/// requires Noul-derived confidence `|p-0.5|*2 >= 0.80`.
pub const LOCKED_CONFIDENCE_THRESHOLD: f64 = 0.80;

/// Normalized Jev input/state schema version currently used (builders in
/// `questions.rs`: `state_tool_selection`, `state_shell_risk`,
/// `state_escalation`, `state_routing`, `state_test_classification`).
/// Recorded in `/tmp/opencode/jev-phase9/state-schema.json`. A shape change
/// that fails [`validate_state_shape`] forces advisory OFF until revalidated.
pub const STATE_SCHEMA_VERSION: &str = "1";

// ── validated baseline record (Phase-10 lock) ──────────────────────────
// `baseline.json` (crate root, compiled in via `include_str!`) is the
// machine-readable validated baseline. `verify_baseline()` cross-checks the
// record against the LIVE consts (including the advisory module's own
// threshold/scope consts, which live in a different module precisely so a
// one-sided edit trips the gate). Any tripwire mismatch -> advisory OFF.
// Informational fields (hashes of audit files, paths, dates, notes) are
// recorded, never tripwires.

/// Advisory scope sub-record.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BaselineScope {
    pub question_id: String,
    pub question_set_version: String,
    #[serde(default)]
    pub kind: String,
}

/// Validated baseline record shape (mirrors `baseline.json`).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct BaselineRecord {
    pub baseline_id: String,
    pub model: String,
    pub question_set_version: String,
    pub question_set_hash: String,
    pub state_schema_version: String,
    #[serde(default)]
    pub state_schema_hash: String,
    pub confidence_threshold: f64,
    pub advisory_scope: BaselineScope,
    pub authority_owner: String,
    #[serde(default)]
    pub rollback_flag: String,
}

/// The shipped baseline record, parsed once. `None` when the record is
/// missing or malformed (fail-closed: [`verify_baseline`] is false then).
fn parsed_baseline() -> &'static Option<BaselineRecord> {
    static CELL: OnceLock<Option<BaselineRecord>> = OnceLock::new();
    CELL.get_or_init(|| {
        serde_json::from_str::<BaselineRecord>(include_str!("../baseline.json")).ok()
    })
}

/// Public accessor for tests and auditors. Returns `None` when the record
/// is missing or malformed.
pub fn baseline_record() -> Option<BaselineRecord> {
    parsed_baseline().clone()
}

/// Pure tripwire comparison: `true` only when EVERY lock field of `rec`
/// matches the live runtime. Each clause is an independent tripwire:
/// model/version drift, question edit, schema change, threshold change,
/// scope change, authority change, or baseline-id change all return false.
pub fn baseline_matches_runtime(rec: &BaselineRecord) -> bool {
    rec.baseline_id == BASELINE_ID
        && rec.model == LOCKED_MODEL
        && rec.question_set_version == LOCKED_QUESTION_SET_VERSION
        && rec.question_set_hash == current_v3_question_set_hash()
        && rec.state_schema_version == STATE_SCHEMA_VERSION
        && (rec.confidence_threshold - crate::advisory::ADVISORY_CONFIDENCE_THRESHOLD).abs()
            < f64::EPSILON
        && (LOCKED_CONFIDENCE_THRESHOLD - crate::advisory::ADVISORY_CONFIDENCE_THRESHOLD).abs()
            < f64::EPSILON
        && rec.advisory_scope.question_id == crate::advisory::ADVISORY_QUESTION_ID
        && rec.advisory_scope.question_set_version == crate::advisory::ADVISORY_QUESTION_SET_VERSION
        && rec.advisory_scope.question_id == LOCKED_QUESTION_ID
        && rec.authority_owner == AUTHORITY_OWNER
}

/// Runtime baseline gate: `true` only when the shipped record parses AND
/// matches the live runtime. Called on EVERY advisory evaluation; `false`
/// forces advisory OFF (shadow-only) until revalidated under a new baseline.
pub fn verify_baseline() -> bool {
    match parsed_baseline() {
        Some(rec) => baseline_matches_runtime(rec),
        None => false,
    }
}

/// Canonical rendering of the full v3 question set: the five v3 wire shapes
/// in fixed id order, joined by `\n`. The hash covers exact instructions +
/// criteria + option keys, so ANY wording/option change alters it.
pub fn canonical_v3_question_set() -> String {
    let ids = [
        "tool_selection",
        "shell_risk",
        "escalation",
        "routing",
        "test_classification",
    ];
    let mut parts = Vec::with_capacity(ids.len());
    for id in ids {
        let wire = crate::questions::wire_questions_for_version("v3", id)
            .unwrap_or(serde_json::Value::Null);
        parts.push(serde_json::to_string(&wire).unwrap_or_default());
    }
    parts.join("\n")
}

/// sha256 hex of [`canonical_v3_question_set`].
pub fn current_v3_question_set_hash() -> String {
    let mut h = Sha256::new();
    h.update(canonical_v3_question_set().as_bytes());
    hex_encode(h.finalize())
}

/// `true` only when the live v3 question set still matches the frozen lock.
pub fn question_set_matches_lock() -> bool {
    current_v3_question_set_hash() == EXPECTED_V3_QUESTION_SET_HASH
}

/// Model gate: advisory allowed only when requested AND resolved both equal
/// the locked model. Detects silent `jev-latest` drift and unexpected
/// provider-side model substitution.
pub fn model_matches_lock(requested: &str, resolved: &str) -> bool {
    requested == LOCKED_MODEL && resolved == LOCKED_MODEL
}

/// Validate one normalized state value against schema version "1".
/// Fail-closed: unknown question id, wrong JSON types, missing/extra keys,
/// or over-length free text all return `false` (advisory OFF for that
/// observation; the shadow record itself is still appended).
pub fn validate_state_shape(question_id: &str, state: &serde_json::Value) -> bool {
    let obj = match state.as_object() {
        Some(o) => o,
        None => return false,
    };
    match question_id {
        "escalation" => {
            if obj.len() != 2 {
                return false;
            }
            let (kind, summary) = match (obj.get("action_kind"), obj.get("summary")) {
                (Some(k), Some(s)) => (k, s),
                _ => return false,
            };
            let (k, s) = match (kind.as_str(), summary.as_str()) {
                (Some(k), Some(s)) => (k, s),
                _ => return false,
            };
            // Builders truncate to 200/1000 chars plus a short marker.
            !k.is_empty() && k.chars().count() <= 220 && !s.is_empty() && s.chars().count() <= 1100
        }
        "tool_selection" => {
            if obj.len() != 2 {
                return false;
            }
            check_string_list(obj.get("task_keywords"))
                && check_string_list(obj.get("active_file_tags"))
        }
        "shell_risk" => {
            if obj.len() != 4 {
                return false;
            }
            let cmd_ok = matches!(obj.get("command"), Some(v) if v.is_string());
            let cmd_len_ok = obj
                .get("command")
                .and_then(|v| v.as_str())
                .map(|s| !s.is_empty() && s.chars().count() <= 1100)
                .unwrap_or(false);
            cmd_ok
                && cmd_len_ok
                && matches!(obj.get("allow_network"), Some(v) if v.is_boolean())
                && matches!(obj.get("allow_writes"), Some(v) if v.is_boolean())
                && matches!(obj.get("timeout_secs"), Some(v) if v.is_number())
        }
        "routing" => {
            if obj.len() != 1 {
                return false;
            }
            matches!(obj.get("task_summary"), Some(v) if v.is_string())
        }
        "test_classification" => {
            if obj.len() != 4 {
                return false;
            }
            matches!(obj.get("classification"), Some(v) if v.is_string())
                && matches!(obj.get("exit_code"), Some(v) if v.is_number())
                && matches!(obj.get("command_hash"), Some(v) if v.is_string() || v.is_null())
                && matches!(obj.get("diagnostics"), Some(v) if v.is_array())
        }
        _ => false,
    }
}

fn check_string_list(v: Option<&serde_json::Value>) -> bool {
    match v {
        Some(serde_json::Value::Array(items)) => {
            items.len() <= 16
                && items.iter().all(|i| {
                    i.as_str()
                        .map(|s| s.chars().count() <= 220)
                        .unwrap_or(false)
                })
        }
        _ => false,
    }
}

// ── frozen reference oracles (TEST-ONLY regression mirrors) ────────────
// These mirror the pre-existing deterministic v3 escalation rubric (frozen in
// `/tmp/opencode/jev-phase7/v3-question-set.md` + the Phase-7 targeted
// fixtures) so regression fixtures have an executable expectation. They are
// NEVER called from any execution path: the authoritative verdict always
// comes from the pre-existing CodeBro policy/human at the call site. If the
// real policy ever changes, these oracles MUST be re-frozen deliberately
// (change-matrix case D), never silently tuned to fit Jev output.

/// Frozen mirror of the v3 escalation rubric: skill-approve is a gate BY
/// KIND (any `approv` token under `action_kind == "skill"`), destructive
/// kinds are gates by kind, task-complete is qualifier-SENSITIVE, everything
/// else (read-only, guarded edits, routine exec, checkpoints, propose,
/// confirm-as-substring) is not a gate.
///
/// Verified to reproduce all 80 Phase-7 targeted fixtures verbatim
/// (regression test `reference_oracle_reproduces_targeted_fixtures` pins the
/// curated Phase-9 subset; the full 80 were checked offline during freezing).
pub fn reference_escalation_verdict(action_kind: &str, summary: &str) -> bool {
    let k = action_kind.to_ascii_lowercase();
    let s = summary.to_ascii_lowercase();
    // 1. Skill-approve is a gate BY KIND, irrespective of confirm flags.
    if k == "skill" && s.contains("approv") {
        return true;
    }
    // 2. Destructive/publishing/risky kinds are gates by kind.
    if matches!(
        k.as_str(),
        "publish" | "delete" | "hyperdrive" | "execute-risky"
    ) {
        return true;
    }
    // 3. Task-complete is qualifier-SENSITIVE: approval IFF a failure or
    //    timeout qualifier from the frozen phrase list is present.
    if k == "task" && s.contains("complet") {
        const QUALIFIERS: &[&str] = &[
            "still unresolved",
            "still recorded",
            "still open",
            "still marked failed",
            "remain recorded",
            "still present",
            "recorded as failed",
            "marked failed",
            "after timeout",
            "with failing",
            "with failures",
            "with unresolved",
        ];
        if QUALIFIERS.iter().any(|q| s.contains(q)) {
            return true;
        }
        return false;
    }
    false
}

/// Frozen mirror of shell-risk intuition for regression pinning: `false`
/// (unsafe) when the command shows destructive, irreversible, exfiltration,
/// pipe-to-shell, or privilege-widening behavior; `true` (safe) otherwise.
/// Test-only; the real sandbox policy remains authoritative.
pub fn reference_shell_safe(command: &str) -> bool {
    let c = command.to_ascii_lowercase();
    const UNSAFE_MARKERS: &[&str] = &[
        "rm -rf",
        "clean -fd",
        "reset --hard",
        "| bash",
        "| sh",
        "curl",
        "wget",
        "chmod -r 777",
        "chmod 777",
        "mkfs",
        ":(){",
        "push",
    ];
    !UNSAFE_MARKERS.iter().any(|m| c.contains(m))
}

/// Frozen mirror of routing intuition for regression pinning. Test-only; the
/// real agent policy remains authoritative.
pub fn reference_routing_label(task_summary: &str) -> &'static str {
    let s = task_summary.to_ascii_lowercase();
    if s.contains("http")
        || s.contains("upstream")
        || s.contains("external docs")
        || s.contains("documentation for")
    {
        return "research";
    }
    if s.contains("fail")
        || s.contains("panic")
        || s.contains("diagnos")
        || s.contains("timeout")
        || s.contains("stack trace")
    {
        return "debug";
    }
    if s.contains("review") || s.contains("diff") {
        return "review";
    }
    if s.contains("write")
        || s.contains("edit")
        || s.contains("implement")
        || s.contains("rename")
        || s.contains("run ")
        || s.contains("build")
        || s.contains("verify")
    {
        return "implement";
    }
    "explore"
}

fn hex_encode(bytes: impl AsRef<[u8]>) -> String {
    const ALPHABET: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.as_ref().len() * 2);
    for b in bytes.as_ref() {
        out.push(ALPHABET[(b >> 4) as usize] as char);
        out.push(ALPHABET[(b & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recorded_hash_matches_recompute() {
        // Sanity used while recording the lock; the authoritative pin lives
        // in `tests/change_control.rs`.
        assert_eq!(
            current_v3_question_set_hash(),
            EXPECTED_V3_QUESTION_SET_HASH
        );
    }
}

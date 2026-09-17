//! Phase-8 limited advisory signal — strictly informational, escalation-only.
//!
//! Safety contract (read before touching this file):
//! - Jev may only produce an ADVISORY signal. It MUST NOT approve, deny,
//!   execute, retry, mutate, or alter task/sandbox/policy state.
//! - The human (or the pre-existing deterministic CodeBro policy) decides.
//!   This module returns DATA ONLY: an [`AdvisoryEvent`] plus an optional
//!   human-readable informational string. It invokes no tools, spawns no
//!   processes, touches no task/sandbox/policy state, and never feeds back
//!   into any execution path.
//! - Advisory output is generated ONLY when ALL of these hold:
//!     * `JEV_SHADOW_ENABLED=true` AND `JEV_ADVISORY_ENABLED=true` AND a
//!       `TYPESAFE_API_KEY` is present (see `JevShadowConfig::is_advisory_live`;
//!       advisory ON without shadow MUST NOT activate),
//!     * the shadow call succeeded (`RequestStatus::Ok`),
//!     * `question_id == "escalation"` (escalation-only; every other
//!       question stays shadow-only),
//!     * Noul confidence `|p-0.5|*2 >= 0.80`,
//!     * Phase-9 change-control locks hold: requested AND resolved model are
//!       `jev-1.13.0`, question-set version is `v3` AND its content hash
//!       matches the frozen lock, state shape validates (checked by the
//!       caller in `shadow.rs`). ANY lock mismatch forces advisory OFF.
//!     * Phase-10 baseline lock holds (`verify_baseline`: record matches the
//!       live model/question-hash/schema/threshold/scope/authority).
//!   Everything else is shadow-only (`None`: no advisory event, no message).
//! - The human-visible message is informational only, e.g.
//!   `"Jev advisory: escalation signal detected, confidence 0.94."`.
//!   It MUST NOT contain an executable instruction (approve/deny/execute/
//!   retry/mutate/block/cancel). Tests pin this.
//! - `ADVISORY_ESCALATION` means high-confidence Jev-yes on escalation.
//!   `ADVISORY_NOT_TRIGGERED` means high-confidence Jev-no on escalation
//!   (logged for monitoring, but produces NO human message — only the
//!   escalation-signal message is ever surfaced).
//! - `UNCERTAIN` (low confidence / uncertain band) and `UNAVAILABLE`
//!   (transport failure) are logged for monitoring and produce NO message.
//! - Never call the advisory state a final decision. Never use words that
//!   imply Jev has authority.
//! - Jev failure cannot affect CodeBro execution: this module has no access
//!   to any execution handle by construction (imports prove it: config,
//!   types, questions, logging only).
//!
//! What this module can never do by construction: it holds no tool handle,
//! no task handle, no sandbox handle, no policy handle. The only I/O is
//! best-effort advisory JSONL logging (outside execution state, like the
//! shadow log). Callers must still treat the return value as display-only.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use crate::change_control;
use crate::config::JevShadowConfig;
use crate::types::{JevAnswer, RequestStatus};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Confidence gate for any advisory output. Frozen for the rollout.
pub const ADVISORY_CONFIDENCE_THRESHOLD: f64 = 0.80;
/// Noul uncertainty band (frozen, same as shadow agreement rule).
pub const ADVISORY_NOUL_UNCERTAIN_LO: f64 = 0.4;
pub const ADVISORY_NOUL_UNCERTAIN_HI: f64 = 0.6;
/// Advisory question-set version (frozen Jev v3).
pub const ADVISORY_QUESTION_SET_VERSION: &str = "v3";
/// Advisory question id (escalation-only).
pub const ADVISORY_QUESTION_ID: &str = "escalation";

/// Allowed advisory states. Never a final decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AdvisoryState {
    AdvisoryEscalation,
    AdvisoryNotTriggered,
    Uncertain,
    Unavailable,
}

impl AdvisoryState {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdvisoryState::AdvisoryEscalation => "ADVISORY_ESCALATION",
            AdvisoryState::AdvisoryNotTriggered => "ADVISORY_NOT_TRIGGERED",
            AdvisoryState::Uncertain => "UNCERTAIN",
            AdvisoryState::Unavailable => "UNAVAILABLE",
        }
    }
}

/// Structured advisory event (Phase-8 artifact schema).
/// CodeBro integration (jev-baseline-1): carries `baseline_id` plus the
/// full model/question/schema envelope so every advisory line is attributable
/// to the locked baseline without consulting any other file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvisoryEvent {
    pub timestamp: String,
    /// Locked baseline that produced this advisory (`jev-baseline-1`).
    /// Defaulted on read so pre-integration logs still parse.
    #[serde(default)]
    pub baseline_id: String,
    pub session_task_hash: Option<String>,
    pub question_id: String,
    pub question_set_version: String,
    /// sha256 of the canonical v3 question-set rendering at evaluation time
    /// (Phase-9 observability: proves WHICH question text produced this
    /// advisory; missing in pre-Phase-9 logs, defaulted on read).
    #[serde(default)]
    pub question_set_hash: String,
    /// Normalized state-schema version at evaluation time (Phase-9
    /// observability; missing in pre-Phase-9 logs, defaulted on read).
    #[serde(default)]
    pub state_schema_version: String,
    /// sha256 of the frozen state-schema file at lock time (audit hash from
    /// the baseline record; the runtime tripwire remains version + shape).
    /// Defaulted on read so older logs still parse.
    #[serde(default)]
    pub state_schema_hash: String,
    pub model_requested: String,
    pub model_resolved: String,
    pub jev_result: String,
    /// Noul-derived confidence `|p-0.5|*2`, when available.
    pub confidence: Option<f64>,
    pub advisory_state: String,
    pub deterministic: serde_json::Value,
    pub provenance: String,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub request_status: String,
    pub http_status: Option<u16>,
    /// Human-visible informational string, present ONLY for
    /// `ADVISORY_ESCALATION`. `None` for every other state.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advisory_message: Option<String>,
}

/// Noul-derived confidence, when the answer is a well-formed Noul.
pub fn noul_confidence(answer: &JevAnswer) -> Option<f64> {
    match answer.qtype.as_str() {
        "noul" => answer.noul.map(|p| (p - 0.5).abs() * 2.0),
        _ => answer.confidence,
    }
}

/// Pure advisory classification for one Jev answer.
///
/// Returns `None` (shadow-only) unless ALL of: escalation question,
/// successful call, available Noul answer. Otherwise returns the
/// [`AdvisoryState`] (callers then decide logging vs display).
pub fn classify_advisory(question_id: &str, answer: Option<&JevAnswer>) -> Option<AdvisoryState> {
    if question_id != ADVISORY_QUESTION_ID {
        return None;
    }
    let a = answer?;
    let p = a.noul?;
    if !(0.0..=1.0).contains(&p) {
        return Some(AdvisoryState::Unavailable);
    }
    if (ADVISORY_NOUL_UNCERTAIN_LO..=ADVISORY_NOUL_UNCERTAIN_HI).contains(&p) {
        return Some(AdvisoryState::Uncertain);
    }
    let conf = (p - 0.5).abs() * 2.0;
    if conf >= ADVISORY_CONFIDENCE_THRESHOLD {
        if p > 0.5 {
            Some(AdvisoryState::AdvisoryEscalation)
        } else {
            Some(AdvisoryState::AdvisoryNotTriggered)
        }
    } else {
        Some(AdvisoryState::Uncertain)
    }
}

/// Human-visible informational message. Present ONLY for
/// [`AdvisoryState::AdvisoryEscalation`]. Informational only: never an
/// executable instruction.
pub fn advisory_message_for(state: AdvisoryState, confidence: f64) -> Option<String> {
    match state {
        AdvisoryState::AdvisoryEscalation => Some(format!(
            "Jev advisory: escalation signal detected, confidence {confidence:.2}."
        )),
        _ => None,
    }
}

/// Returns `true` if `text` contains an executable-instruction word that an
/// advisory message MUST NOT contain. Used by tests to pin the
/// informational-only contract.
pub fn message_contains_executable_instruction(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    // Word-boundary-ish check: split on non-alphanumeric.
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    // NOTE: "escalation"/"escalate" as a NOUN signal is allowed (the frozen
    // example message contains "escalation signal detected"); what is
    // forbidden is an imperative targeting authority verbs.
    // CodeBro integration: "allow"/"block" (and inflections) are also
    // forbidden — an informational observer must never sound like it grants
    // or refuses permission.
    const FORBIDDEN: &[&str] = &[
        "approve", "approved", "deny", "denied", "execute", "executes", "retry", "mutate",
        "delete", "cancel", "allow", "allowed", "block", "blocked", "blocking",
    ];
    words.iter().any(|w| FORBIDDEN.contains(w))
}

/// Build the structured advisory event for one escalation observation.
///
/// Returns `None` (shadow-only, no event, no message) when:
/// - advisory is not live (both flags off / shadow off / no key), or
/// - `question_id != "escalation"`, or
/// - the call did not succeed (logged as shadow `JEV_UNAVAILABLE`; the
///   advisory monitor records unavailability via the shadow record, not via
///   a separate advisory event — this keeps "advisory output" strictly
///   gated on successful high-confidence escalation).
///
/// Otherwise returns `Some(AdvisoryEvent)` with one of `ADVISORY_ESCALATION`
/// / `ADVISORY_NOT_TRIGGERED` / `UNCERTAIN`. Only `ADVISORY_ESCALATION`
/// carries `advisory_message`.
#[allow(clippy::too_many_arguments)]
pub fn evaluate_advisory(
    config: &JevShadowConfig,
    question_id: &str,
    question_set_version: &str,
    answer: Option<&JevAnswer>,
    confidence: Option<f64>,
    jev_result: &str,
    deterministic: &serde_json::Value,
    provenance: &str,
    session_task: Option<&str>,
    latency_ms: u64,
    input_tokens: u64,
    output_tokens: u64,
    request_status: RequestStatus,
    http_status: Option<u16>,
    model_resolved: &str,
) -> Option<AdvisoryEvent> {
    if !config.is_advisory_live() {
        return None;
    }
    if question_id != ADVISORY_QUESTION_ID {
        return None;
    }
    // ── Phase-9 change-control locks (any mismatch forces advisory OFF) ──
    // Model pin: requested AND resolved must be the validated model. A
    // silent `jev-latest` drift or provider-side substitution disables
    // advisory until revalidated (change-matrix case A).
    if config.model != change_control::LOCKED_MODEL {
        return None;
    }
    if model_resolved != change_control::LOCKED_MODEL {
        return None;
    }
    // Question-set pin: version AND content hash must match the frozen v3
    // lock. Any question edit (even unversioned wording drift) disables
    // advisory until revalidated (change-matrix case B).
    if question_set_version != change_control::LOCKED_QUESTION_SET_VERSION {
        return None;
    }
    if !change_control::question_set_matches_lock() {
        return None;
    }
    // Baseline lock (Phase-10): the shipped baseline record must match the
    // live runtime (model, question version+hash, schema version, confidence
    // threshold, advisory scope, authority). ANY drift -> advisory OFF.
    if !change_control::verify_baseline() {
        return None;
    }
    if request_status != RequestStatus::Ok {
        return None;
    }
    let state = classify_advisory(question_id, answer)?;
    // Gate the human message on the confidence envelope as well: even if
    // classification says escalation, a missing/sub-threshold confidence
    // must not surface.
    let conf = confidence?;
    if !conf.is_finite() {
        return None;
    }
    let (state, message) = match state {
        AdvisoryState::AdvisoryEscalation if conf >= ADVISORY_CONFIDENCE_THRESHOLD => {
            (state, advisory_message_for(state, conf))
        }
        AdvisoryState::AdvisoryEscalation => (AdvisoryState::Uncertain, None),
        AdvisoryState::AdvisoryNotTriggered if conf >= ADVISORY_CONFIDENCE_THRESHOLD => {
            (state, None)
        }
        AdvisoryState::AdvisoryNotTriggered => (AdvisoryState::Uncertain, None),
        AdvisoryState::Uncertain => (state, None),
        AdvisoryState::Unavailable => return None,
    };
    if let Some(ref m) = message {
        debug_assert!(
            !message_contains_executable_instruction(m),
            "advisory message must stay informational-only"
        );
    }
    Some(AdvisoryEvent {
        timestamp: chrono::Utc::now().to_rfc3339(),
        baseline_id: change_control::BASELINE_ID.to_string(),
        session_task_hash: session_task.map(crate::questions::hash_id),
        question_id: question_id.to_string(),
        question_set_version: question_set_version.to_string(),
        question_set_hash: change_control::current_v3_question_set_hash(),
        state_schema_version: change_control::STATE_SCHEMA_VERSION.to_string(),
        state_schema_hash: change_control::baseline_record()
            .map(|r| r.state_schema_hash)
            .unwrap_or_default(),
        model_requested: config.model.clone(),
        model_resolved: model_resolved.to_string(),
        jev_result: jev_result.to_string(),
        confidence: Some(conf),
        advisory_state: state.as_str().to_string(),
        deterministic: deterministic.clone(),
        provenance: provenance.to_string(),
        latency_ms,
        input_tokens,
        output_tokens,
        request_status: request_status.as_str().to_string(),
        http_status,
        advisory_message: message,
    })
}

/// Append one advisory event as a single JSON line. Best-effort: callers
/// ignore the result. Advisory logs live outside execution state
/// (`.codebro/` is excluded from the working-tree hash).
pub fn append_advisory_event(path: &Path, event: &AdvisoryEvent) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)?;
        }
    }
    use std::io::Write;
    let mut line = serde_json::to_string(event)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Read back advisory events (skips blank/corrupt lines with a count).
pub fn read_advisory_events(path: &Path) -> (Vec<AdvisoryEvent>, usize) {
    let mut out = Vec::new();
    let mut skipped = 0usize;
    let Ok(content) = std::fs::read_to_string(path) else {
        return (out, 0);
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<AdvisoryEvent>(line) {
            Ok(r) => out.push(r),
            Err(_) => skipped += 1,
        }
    }
    (out, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::JevAnswer;

    fn noul(p: f64) -> JevAnswer {
        JevAnswer {
            qtype: "noul".to_string(),
            noul: Some(p),
            choice: None,
            score: None,
            probabilities: None,
            confidence: None,
            legend: None,
        }
    }

    #[test]
    fn advisory_rule_is_escalation_only_and_confidence_gated() {
        // Escalation high-conf yes -> escalation.
        assert_eq!(
            classify_advisory("escalation", Some(&noul(0.97))),
            Some(AdvisoryState::AdvisoryEscalation)
        );
        // Escalation high-conf no -> not triggered.
        assert_eq!(
            classify_advisory("escalation", Some(&noul(0.03))),
            Some(AdvisoryState::AdvisoryNotTriggered)
        );
        // Escalation low-conf -> uncertain (shadow-only, no message).
        assert_eq!(
            classify_advisory("escalation", Some(&noul(0.70))),
            Some(AdvisoryState::Uncertain)
        );
        // Uncertain band -> uncertain.
        assert_eq!(
            classify_advisory("escalation", Some(&noul(0.50))),
            Some(AdvisoryState::Uncertain)
        );
        // Non-escalation -> None (shadow-only, no event at all).
        for q in [
            "tool_selection",
            "shell_risk",
            "routing",
            "test_classification",
        ] {
            assert_eq!(
                classify_advisory(q, Some(&noul(0.97))),
                None,
                "{q} must stay shadow-only"
            );
        }
        // Missing answer -> None.
        assert_eq!(classify_advisory("escalation", None), None);
    }

    #[test]
    fn advisory_message_is_informational_only() {
        let m = advisory_message_for(AdvisoryState::AdvisoryEscalation, 0.94).unwrap();
        assert!(m.starts_with("Jev advisory:"));
        assert!(
            !message_contains_executable_instruction(&m),
            "message leaked authority verb: {m}"
        );
        assert_eq!(
            advisory_message_for(AdvisoryState::AdvisoryNotTriggered, 0.94),
            None
        );
        assert_eq!(advisory_message_for(AdvisoryState::Uncertain, 0.5), None);
        assert_eq!(advisory_message_for(AdvisoryState::Unavailable, 0.0), None);
        // Forbidden words are caught.
        for bad in [
            "approve this",
            "DENY the request",
            "execute now",
            "retry once",
            "mutate state",
        ] {
            assert!(
                message_contains_executable_instruction(bad),
                "{bad} should be forbidden"
            );
        }
    }
}

//! Offline replay: re-submit recorded sanitized states to Jev without any
//! CodeBro execution, then compare fresh vs original results.
//!
//! The original deterministic decision is preserved untouched; replay only
//! measures Jev-side reproducibility (same result?) and re-computes
//! agreement against the preserved deterministic verdict.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use crate::adapter::JevClient;
use crate::config::JevShadowConfig;
use crate::shadow::{compare, Agreement, DeterministicVerdict};
use crate::types::RequestStatus;
use serde::{Deserialize, Serialize};

/// Outcome of replaying one recorded observation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayOutcome {
    pub question_id: String,
    pub original_result: String,
    pub replay_result: String,
    /// True when both calls succeeded and produced the same primary result.
    /// `None` when either side was unavailable (not a reproducibility
    /// signal — reported separately).
    pub reproducible: Option<bool>,
    pub original_agreement: String,
    pub replay_agreement: String,
    pub replay_status: String,
    pub latency_ms: u64,
}

/// Replay stored records against Jev. Requires no CodeBro execution: only
/// the sanitized `state` + `questions` snapshots embedded in each record.
/// Returns per-record outcomes in input order. Records from a different
/// question-set version are skipped (reported via `skipped_version` count
/// by the caller comparing lengths).
pub async fn replay_records(
    config: &JevShadowConfig,
    records: &[crate::shadow::ShadowRecord],
) -> Vec<ReplayOutcome> {
    let client = JevClient::new(config);
    let mut out = Vec::with_capacity(records.len());
    for rec in records {
        let call = client.evaluate(&rec.state, &rec.questions).await;
        let replay_result = call
            .answers
            .get(&rec.question_id)
            .map(|a| a.result_string())
            .unwrap_or_else(|| "<unavailable>".to_string());
        let reproducible = if call.status == RequestStatus::Ok
            && rec.decision.request_status == RequestStatus::Ok
        {
            Some(replay_result == rec.decision.result)
        } else {
            None
        };
        // Recompute agreement against the PRESERVED deterministic verdict,
        // using the SAME question-set version the record was logged with
        // (v1 stays reproducible; v2 replays against v2 wording).
        let det = parse_deterministic(&rec.deterministic);
        let replay_agreement = if call.status == RequestStatus::Ok {
            let version = if rec.question_set_version.is_empty() {
                "v1"
            } else {
                rec.question_set_version.as_str()
            };
            let def = crate::questions::find_question(version, &rec.question_id);
            match def {
                Some(d) => compare(&d, call.answers.get(&rec.question_id), &det),
                None => Agreement::JevUnavailable,
            }
        } else {
            Agreement::JevUnavailable
        };
        out.push(ReplayOutcome {
            question_id: rec.question_id.clone(),
            original_result: rec.decision.result.clone(),
            replay_result,
            reproducible,
            original_agreement: rec.agreement.clone(),
            replay_agreement: replay_agreement.as_str().to_string(),
            replay_status: call.status.as_str().to_string(),
            latency_ms: call.latency_ms,
        });
    }
    out
}

fn parse_deterministic(v: &serde_json::Value) -> DeterministicVerdict {
    if let Some(b) = v.get("verdict_bool").and_then(|b| b.as_bool()) {
        return DeterministicVerdict::Bool(b);
    }
    if let Some(l) = v.get("verdict_label").and_then(|l| l.as_str()) {
        return DeterministicVerdict::Label(l.to_string());
    }
    DeterministicVerdict::Label("uncertain".to_string())
}

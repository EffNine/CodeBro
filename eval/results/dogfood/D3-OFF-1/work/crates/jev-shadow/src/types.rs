//! Typed shadow-decision abstraction over the actual Jev response schema.
//!
//! Shapes follow the live TypeSafe API (`POST /v1/systemone`, verified
//! against https://docs.typesafe.ai/api.md): the request carries
//! `{state, model, questions}`, answers come back keyed by question id as
//! `noul -> {type, noul}`, `choice -> {type, choice, probabilities,
//! confidence}`, `score -> {type, score, legend, probabilities, confidence}`,
//! plus top-level `{model, usage: {input_tokens, output_tokens}}`.
//! No fields are invented for the wire schema; [`ShadowDecision`] is the
//! internal normalized projection used for logging/analysis.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One Jev answer, mirroring the wire schema exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevAnswer {
    /// `"noul" | "choice" | "score"` (other values preserved, marked unknown).
    #[serde(rename = "type")]
    pub qtype: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noul: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<HashMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub legend: Option<HashMap<String, String>>,
}

impl JevAnswer {
    /// The primary scalar result for logging/comparison:
    /// noul probability, choice label, or score value.
    pub fn result_string(&self) -> String {
        match self.qtype.as_str() {
            "noul" => format!("{:.4}", self.noul.unwrap_or(f64::NAN)),
            "choice" => self.choice.clone().unwrap_or_default(),
            "score" => format!("{:.4}", self.score.unwrap_or(f64::NAN)),
            other => format!("unknown_type:{other}"),
        }
    }

    /// Structural validity per the documented schema.
    pub fn is_well_formed(&self) -> bool {
        match self.qtype.as_str() {
            "noul" => matches!(self.noul, Some(p) if (0.0..=1.0).contains(&p)),
            "choice" => {
                self.choice.is_some()
                    && matches!(&self.probabilities, Some(p) if !p.is_empty())
                    && matches!(self.confidence, Some(c) if (0.0..=1.0).contains(&c))
            }
            "score" => {
                self.score.is_some()
                    && matches!(&self.probabilities, Some(p) if !p.is_empty())
                    && self.legend.is_some()
                    && matches!(self.confidence, Some(c) if (0.0..=1.0).contains(&c))
            }
            _ => false,
        }
    }
}

/// Token usage as returned by the API.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JevUsage {
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
}

/// Successful Jev response envelope (wire schema).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevResponse {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub answers: HashMap<String, JevAnswer>,
    #[serde(default)]
    pub usage: JevUsage,
}

/// Shadow-side request outcome. Failures are values, never control flow:
//ifu the shadow layer is unavailable the deterministic path is unaffected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RequestStatus {
    Ok,
    Disabled,
    MissingKey,
    Http401,
    Http403,
    Http422,
    Http429,
    Http529,
    Http5xx,
    Timeout,
    Network,
    Malformed,
}

impl RequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RequestStatus::Ok => "ok",
            RequestStatus::Disabled => "disabled",
            RequestStatus::MissingKey => "missing_key",
            RequestStatus::Http401 => "http_401",
            RequestStatus::Http403 => "http_403",
            RequestStatus::Http422 => "http_422",
            RequestStatus::Http429 => "http_429",
            RequestStatus::Http529 => "http_529",
            RequestStatus::Http5xx => "http_5xx",
            RequestStatus::Timeout => "timeout",
            RequestStatus::Network => "network",
            RequestStatus::Malformed => "malformed",
        }
    }

    pub fn is_available(&self) -> bool {
        matches!(self, RequestStatus::Ok)
    }
}

/// Coarse error class for analysis (mirrors status, kept as a separate
/// human-facing label per the phase spec).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorClass {
    None,
    Disabled,
    MissingKey,
    Unauthorized,
    Forbidden,
    Validation,
    RateLimited,
    Overloaded,
    Server,
    Timeout,
    Network,
    Malformed,
}

impl ErrorClass {
    pub fn for_status(status: RequestStatus) -> Self {
        match status {
            RequestStatus::Ok => ErrorClass::None,
            RequestStatus::Disabled => ErrorClass::Disabled,
            RequestStatus::MissingKey => ErrorClass::MissingKey,
            RequestStatus::Http401 => ErrorClass::Unauthorized,
            RequestStatus::Http403 => ErrorClass::Forbidden,
            RequestStatus::Http422 => ErrorClass::Validation,
            RequestStatus::Http429 => ErrorClass::RateLimited,
            RequestStatus::Http529 => ErrorClass::Overloaded,
            RequestStatus::Http5xx => ErrorClass::Server,
            RequestStatus::Timeout => ErrorClass::Timeout,
            RequestStatus::Network => ErrorClass::Network,
            RequestStatus::Malformed => ErrorClass::Malformed,
        }
    }
}

/// Internal normalized shadow decision (phase-spec abstraction), projected
/// from one Jev answer plus transport metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowDecision {
    pub decision_type: String,
    pub question_id: String,
    pub state_hash: String,
    pub result: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probabilities: Option<HashMap<String, f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub latency_ms: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub model: String,
    pub request_status: RequestStatus,
    pub error_class: ErrorClass,
    pub timestamp: String,
}

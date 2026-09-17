//! Minimal isolated Jev HTTP adapter.
//!
//! Design rules (phase spec):
//! - API key from `TYPESAFE_API_KEY` env only, read per call, never stored,
//!   never logged, never serialized. The struct holds no secret (manual
//!   `Debug` impl proves it).
//! - Explicit bounded timeout; at most ONE retry and only for 429/529,
//!   honoring `Retry-After` capped at 2 s (else 500 ms backoff). Never
//!   retries indefinitely.
//! - Every failure mode maps to a [`RequestStatus`] value. The adapter
//!   never panics and never returns `Err` for transport conditions —
//!   adapter failure always resolves to "unavailable" so it cannot affect
//!   CodeBro execution.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use crate::config::{api_key_from_env, JevShadowConfig};
use crate::types::{ErrorClass, JevAnswer, JevResponse, JevUsage, RequestStatus, ShadowDecision};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Cap on honored `Retry-After` (seconds). Larger values are clamped, never
/// obeyed literally — the shadow path must stay bounded.
pub const MAX_RETRY_AFTER_SECS: u64 = 2;
/// Backoff when 429/529 carry no (parseable) `Retry-After`.
pub const DEFAULT_RETRY_BACKOFF: Duration = Duration::from_millis(500);

/// Outcome of one shadow evaluation: transport metadata plus either answers
/// or an unavailability status. Always a value — never an error that a
/// caller must handle on a hot path.
#[derive(Debug, Clone)]
pub struct JevCall {
    pub status: RequestStatus,
    pub http_status: Option<u16>,
    pub latency_ms: u64,
    pub model: String,
    pub answers: HashMap<String, JevAnswer>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl JevCall {
    pub fn unavailable(status: RequestStatus, http_status: Option<u16>) -> Self {
        JevCall {
            status,
            http_status,
            latency_ms: 0,
            model: String::new(),
            answers: HashMap::new(),
            input_tokens: 0,
            output_tokens: 0,
        }
    }

    pub fn error_class(&self) -> ErrorClass {
        ErrorClass::for_status(self.status)
    }
}

/// Minimal isolated Jev client. Holds endpoint/model/timeout only — no key.
pub struct JevClient {
    endpoint: String,
    model: String,
    timeout: Duration,
    max_retries: u8,
    http: reqwest::Client,
}

// Manual Debug: endpoint/model/timeout are safe; assert no secret exists.
impl std::fmt::Debug for JevClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JevClient")
            .field("endpoint", &self.endpoint)
            .field("model", &self.model)
            .field("timeout", &self.timeout)
            .field("max_retries", &self.max_retries)
            .finish()
    }
}

impl JevClient {
    pub fn new(config: &JevShadowConfig) -> Self {
        JevClient {
            endpoint: config.endpoint.clone(),
            model: config.model.clone(),
            timeout: config.timeout,
            max_retries: config.max_retries,
            http: reqwest::Client::builder()
                .timeout(config.timeout + Duration::from_millis(500))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new()),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Evaluate `state` against `questions` (a map of question-id to a
    /// `{type, instructions, criteria?}` object per the Jev schema).
    pub async fn evaluate(
        &self,
        state: &serde_json::Value,
        questions: &serde_json::Value,
    ) -> JevCall {
        let key = match api_key_from_env() {
            Some(k) => k,
            None => return JevCall::unavailable(RequestStatus::MissingKey, None),
        };
        let payload = serde_json::json!({
            "state": state,
            "model": self.model,
            "questions": questions,
        });

        let mut attempt: u8 = 0;
        loop {
            attempt += 1;
            let call = self.post_once(&payload, &key).await;
            let retryable = matches!(
                call.status,
                RequestStatus::Http429 | RequestStatus::Http529
            ) && attempt <= self.max_retries;
            if retryable {
                let wait = call
                    .retry_after_hint
                    .unwrap_or(DEFAULT_RETRY_BACKOFF)
                    .min(Duration::from_secs(MAX_RETRY_AFTER_SECS));
                tokio::time::sleep(wait).await;
                continue;
            }
            return call.into_call(&self.model);
        }
    }

    async fn post_once(
        &self,
        payload: &serde_json::Value,
        key: &str,
    ) -> SingleAttempt {
        let started = Instant::now();
        let send = tokio::time::timeout(self.timeout, self.http.post(&self.endpoint).bearer_auth(key).json(payload).send()).await;
        let elapsed_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        match send {
            Err(_) => SingleAttempt::timeout(elapsed_ms),
            Ok(Err(e)) => {
                if e.is_timeout() {
                    SingleAttempt::timeout(elapsed_ms)
                } else {
                    SingleAttempt::network(elapsed_ms)
                }
            }
            Ok(Ok(resp)) => {
                let http_status = resp.status().as_u16();
                let retry_after = parse_retry_after(resp.headers());
                let text = match tokio::time::timeout(
                    self.timeout,
                    resp.text(),
                )
                .await
                {
                    Err(_) => return SingleAttempt::timeout(elapsed_ms),
                    Ok(Err(_)) => return SingleAttempt::network(elapsed_ms),
                    Ok(Ok(t)) => t,
                };
                let latency_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
                match http_status {
                    200..=299 => match parse_success(&text) {
                        Some((answers, usage, model)) => SingleAttempt::ok(SuccessBody {
                            latency_ms,
                            http_status,
                            answers,
                            usage,
                            model,
                        }),
                        None => SingleAttempt::malformed(latency_ms, http_status),
                    },
                    401 => SingleAttempt::status(RequestStatus::Http401, latency_ms, http_status, retry_after),
                    403 => SingleAttempt::status(RequestStatus::Http403, latency_ms, http_status, retry_after),
                    422 => SingleAttempt::status(RequestStatus::Http422, latency_ms, http_status, retry_after),
                    429 => SingleAttempt::status(RequestStatus::Http429, latency_ms, http_status, retry_after),
                    529 => SingleAttempt::status(RequestStatus::Http529, latency_ms, http_status, retry_after),
                    500..=599 => SingleAttempt::status(RequestStatus::Http5xx, latency_ms, http_status, retry_after),
                    _ => SingleAttempt::status(RequestStatus::Http5xx, latency_ms, http_status, retry_after),
                }
            }
        }
    }
}

struct SuccessBody {
    latency_ms: u64,
    http_status: u16,
    answers: HashMap<String, JevAnswer>,
    usage: JevUsage,
    model: String,
}

struct SingleAttempt {
    status: RequestStatus,
    latency_ms: u64,
    http_status: Option<u16>,
    retry_after_hint: Option<Duration>,
    ok: Option<SuccessBody>,
}

impl SingleAttempt {
    fn ok(ok: SuccessBody) -> Self {
        SingleAttempt { status: RequestStatus::Ok, latency_ms: ok.latency_ms, http_status: Some(ok.http_status), retry_after_hint: None, ok: Some(ok) }
    }
    fn status(status: RequestStatus, latency_ms: u64, http_status: u16, retry_after_hint: Option<Duration>) -> Self {
        SingleAttempt { status, latency_ms, http_status: Some(http_status), retry_after_hint, ok: None }
    }
    fn timeout(latency_ms: u64) -> Self {
        SingleAttempt { status: RequestStatus::Timeout, latency_ms, http_status: None, retry_after_hint: None, ok: None }
    }
    fn network(latency_ms: u64) -> Self {
        SingleAttempt { status: RequestStatus::Network, latency_ms, http_status: None, retry_after_hint: None, ok: None }
    }
    fn malformed(latency_ms: u64, http_status: u16) -> Self {
        SingleAttempt { status: RequestStatus::Malformed, latency_ms, http_status: Some(http_status), retry_after_hint: None, ok: None }
    }

    fn into_call(self, model: &str) -> JevCall {
        match self.ok {
            Some(ok) => JevCall {
                status: RequestStatus::Ok,
                http_status: Some(ok.http_status),
                latency_ms: ok.latency_ms,
                model: if ok.model.is_empty() { model.to_string() } else { ok.model },
                answers: ok.answers,
                input_tokens: ok.usage.input_tokens,
                output_tokens: ok.usage.output_tokens,
            },
            None => {
                let mut c = JevCall::unavailable(self.status, self.http_status);
                c.latency_ms = self.latency_ms;
                c.model = model.to_string();
                c
            }
        }
    }
}

/// Parse a 2xx body into answers + usage. `None` = malformed (invalid JSON,
/// wrong shape, or structurally invalid answers).
fn parse_success(text: &str) -> Option<(HashMap<String, JevAnswer>, JevUsage, String)> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let obj = v.as_object()?;
    let model = obj.get("model").and_then(|m| m.as_str()).unwrap_or("").to_string();
    let answers_v = obj.get("answers")?;
    let answers_map = answers_v.as_object()?;
    let mut answers = HashMap::new();
    for (k, av) in answers_map {
        let ans: JevAnswer = serde_json::from_value(av.clone()).ok()?;
        if !ans.is_well_formed() {
            return None;
        }
        answers.insert(k.clone(), ans);
    }
    let usage: JevUsage = obj
        .get("usage")
        .cloned()
        .and_then(|u| serde_json::from_value(u).ok())
        .unwrap_or_default();
    Some((answers, usage, model))
}

/// Parse `Retry-After` (delta-seconds or HTTP date). Returns `None` when
/// absent/unparseable — the caller falls back to a fixed bounded backoff.
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let v = v.trim();
    if let Ok(secs) = v.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    // HTTP-date form: compute delta vs now, clamp at zero.
    if let Ok(date) = chrono::DateTime::parse_from_rfc2822(v) {
        let now = chrono::Utc::now();
        let delta = date.with_timezone(&chrono::Utc) - now;
        if delta.num_seconds() > 0 {
            return Some(Duration::from_secs(delta.num_seconds() as u64));
        }
    }
    None
}

/// Project one answer of a call into the normalized [`ShadowDecision`].
pub fn to_shadow_decision(
    question_id: &str,
    decision_type: &str,
    state_hash: &str,
    call: &JevCall,
) -> ShadowDecision {
    let (result, probabilities, confidence) = match call.answers.get(question_id) {
        Some(a) => (
            a.result_string(),
            a.probabilities.clone(),
            a.confidence.or_else(|| {
                // Noul carries no separate confidence: probability distance
                // from 0.5 is the uncertainty signal.
                a.noul.map(|p| (p - 0.5).abs() * 2.0)
            }),
        ),
        None => ("<unavailable>".to_string(), None, None),
    };
    ShadowDecision {
        decision_type: decision_type.to_string(),
        question_id: question_id.to_string(),
        state_hash: state_hash.to_string(),
        result,
        probabilities,
        confidence,
        latency_ms: call.latency_ms,
        input_tokens: call.input_tokens,
        output_tokens: call.output_tokens,
        model: call.model.clone(),
        request_status: call.status,
        error_class: call.error_class(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

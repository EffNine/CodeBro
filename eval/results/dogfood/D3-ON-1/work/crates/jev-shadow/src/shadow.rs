//! Post-decision shadow observer + agreement analysis.
//!
//! The observer is the ONLY integration seam. It receives already-computed
//! deterministic outcomes, optionally consults Jev on a detached-equivalent
//! async call, appends a log record, and returns the record. It returns
//! `None` (and performs zero network calls) when the flag is off or no key
//! is present. It never feeds anything back into an execution path: there
//! is no approve/deny/execute/retry surface here by design.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use crate::adapter::{to_shadow_decision, JevCall, JevClient};
use crate::config::JevShadowConfig;
use crate::logging::append_record;
use crate::questions::{state_hash, v1_questions, wire_questions_for, ShadowQuestionDef, QUESTION_SET_VERSION};
use crate::types::{JevAnswer, RequestStatus, ShadowDecision};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Agreement between Jev's shadow answer and the deterministic verdict.
/// Disagreement is preserved evidence, never auto-labelled a Jev failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Agreement {
    Agree,
    Disagree,
    JevUncertain,
    JevUnavailable,
}

impl Agreement {
    pub fn as_str(&self) -> &'static str {
        match self {
            Agreement::Agree => "AGREE",
            Agreement::Disagree => "DISAGREE",
            Agreement::JevUncertain => "JEV_UNCERTAIN",
            Agreement::JevUnavailable => "JEV_UNAVAILABLE",
        }
    }
}

/// Noul probability band treated as uncertainty (near 0.5 either way).
pub const NOUL_UNCERTAIN_LO: f64 = 0.4;
pub const NOUL_UNCERTAIN_HI: f64 = 0.6;
/// Choice/score confidence floor: below this Jev is "uncertain" rather than
/// agreeing or disagreeing.
pub const CONFIDENCE_FLOOR: f64 = 0.55;

/// The deterministic side of a comparison: a boolean verdict (for noul) or
/// a label (for choice).
#[derive(Debug, Clone)]
pub enum DeterministicVerdict {
    Bool(bool),
    Label(String),
}

/// Compare one Jev answer against the deterministic verdict.
pub fn compare(
    def: &ShadowQuestionDef,
    answer: Option<&JevAnswer>,
    deterministic: &DeterministicVerdict,
) -> Agreement {
    let Some(a) = answer else {
        return Agreement::JevUnavailable;
    };
    match def.kind {
        crate::questions::QuestionKind::Noul => {
            let Some(p) = a.noul else {
                return Agreement::JevUnavailable;
            };
            if !(0.0..=1.0).contains(&p) || (NOUL_UNCERTAIN_LO..=NOUL_UNCERTAIN_HI).contains(&p) {
                return Agreement::JevUncertain;
            }
            let jev_yes = p > 0.5;
            let det_yes = matches!(deterministic, DeterministicVerdict::Bool(true));
            if jev_yes == det_yes {
                Agreement::Agree
            } else {
                Agreement::Disagree
            }
        }
        crate::questions::QuestionKind::Choice => {
            let (Some(choice), Some(conf)) = (a.choice.as_deref(), a.confidence) else {
                return Agreement::JevUnavailable;
            };
            if !(0.0..=1.0).contains(&conf) || conf < CONFIDENCE_FLOOR {
                return Agreement::JevUncertain;
            }
            // A deterministic "uncertain" label means the pipeline makes no
            // claim: a confident Jev label is neither agreement nor a
            // meaningful disagreement.
            let det_label = match deterministic {
                DeterministicVerdict::Label(l) => l.as_str(),
                DeterministicVerdict::Bool(true) => "yes",
                DeterministicVerdict::Bool(false) => "no",
            };
            if det_label == "uncertain" {
                if choice == "uncertain" {
                    return Agreement::Agree;
                }
                return Agreement::JevUncertain;
            }
            if choice == det_label {
                Agreement::Agree
            } else {
                Agreement::Disagree
            }
        }
    }
}

/// One appended shadow-log record (JSONL). Contains the sanitized state and
/// questions snapshot so offline replay needs no CodeBro execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowRecord {
    pub timestamp: String,
    pub question_id: String,
    pub question_set_version: String,
    pub decision: ShadowDecision,
    pub state: serde_json::Value,
    pub questions: serde_json::Value,
    pub deterministic: serde_json::Value,
    pub agreement: String,
    pub http_status: Option<u16>,
    pub flag_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_task: Option<String>,
}

/// Post-decision observer. Construct per observation site from env; cheap.
pub struct ShadowObserver {
    config: JevShadowConfig,
    workspace_root: PathBuf,
}

impl ShadowObserver {
    pub fn new(config: JevShadowConfig, workspace_root: PathBuf) -> Self {
        ShadowObserver { config, workspace_root }
    }

    /// Standard construction at an observation site.
    pub fn from_env(workspace_root: &std::path::Path) -> Self {
        ShadowObserver {
            config: JevShadowConfig::from_env(),
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    pub fn config(&self) -> &JevShadowConfig {
        &self.config
    }

    /// Whether any network call may happen (flag on AND key present).
    pub fn is_live(&self) -> bool {
        self.config.is_live()
    }

    /// Whether advisory output may happen (BOTH flags on AND key present).
    /// Advisory ON without shadow is never live.
    pub fn is_advisory_live(&self) -> bool {
        self.config.is_advisory_live()
    }

    /// Observe one shadow decision. When not live: returns `None` and
    /// performs ZERO network calls. When live: always returns `Some`
    /// (ok or unavailable record) and appends it to the JSONL log.
    /// Never panics; never affects the caller.
    pub async fn observe(
        &self,
        question_id: &str,
        state: serde_json::Value,
        deterministic: DeterministicVerdict,
    ) -> Option<ShadowRecord> {
        self.observe_with_session(question_id, state, deterministic, None).await
    }

    pub async fn observe_with_session(
        &self,
        question_id: &str,
        state: serde_json::Value,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
    ) -> Option<ShadowRecord> {
        // Default runtime remains v1 unless the caller explicitly requests a
        // version via `observe_versioned`. No behavioral authority change.
        self.observe_versioned(QUESTION_SET_VERSION, question_id, state, deterministic, session_task)
            .await
    }

    /// Versioned observation: `question_set` is `v1`, `v2`, or `v3`
    /// (fail-closed on any other value: returns `None`, zero network calls).
    /// Logs carry the requested version so v1 stays reproducible and v2/v3
    /// are identifiable. Still post-decision/log-only: nothing flows back
    /// to any caller.
    pub async fn observe_versioned(
        &self,
        question_set: &str,
        question_id: &str,
        state: serde_json::Value,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
    ) -> Option<ShadowRecord> {
        if !self.config.enabled {
            return None;
        }
        let def = crate::questions::find_question(question_set, question_id)?;
        let questions = crate::questions::wire_questions_for_version(question_set, question_id)?;
        let hash = state_hash(&state);
        let client = JevClient::new(&self.config);
        let call: JevCall = client.evaluate(&state, &questions).await;
        Some(self.finish_versioned(
            question_set,
            &def,
            state,
            questions,
            hash,
            call,
            deterministic,
            session_task,
        ))
    }

    /// Escalation observation with limited advisory projection (Phase 8).
    ///
    /// Log-only, like [`Self::observe_versioned`], plus an optional
    /// [`crate::advisory::AdvisoryEvent`]:
    /// - always appends the shadow record (when live);
    /// - appends an advisory event ONLY when the advisory rule holds
    ///   (both flags live, escalation, successful call, confidence >= 0.80
    ///   envelope met for `ADVISORY_ESCALATION`/`ADVISORY_NOT_TRIGGERED`,
    ///   or `UNCERTAIN` for below-envelope escalation);
    /// - emits the human-visible advisory string via `tracing::info!` ONLY
    ///   for `ADVISORY_ESCALATION` (informational surface; never a response
    ///   mutation, never a tool call, never a state change).
    ///
    /// Returns `(shadow_record, advisory_event)`. The advisory element is
    /// `None` for shadow-only outcomes (non-escalation, below-envelope
    /// handling per [`crate::advisory::evaluate_advisory`], unavailable, or
    /// advisory not live). Nothing in the return value may be fed back into
    /// any execution path by the caller.
    pub async fn observe_escalation_with_advisory(
        &self,
        question_set: &str,
        state: serde_json::Value,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
        provenance: &str,
    ) -> Option<(ShadowRecord, Option<crate::advisory::AdvisoryEvent>)> {
        if !self.config.enabled {
            return None;
        }
        // Advisory path is escalation-only by construction.
        let def = crate::questions::find_question(question_set, "escalation")?;
        let questions =
            crate::questions::wire_questions_for_version(question_set, "escalation")?;
        let hash = state_hash(&state);
        let client = JevClient::new(&self.config);
        let call: JevCall = client.evaluate(&state, &questions).await;
        let det_json = match &deterministic {
            DeterministicVerdict::Bool(b) => serde_json::json!({"verdict_bool": b}),
            DeterministicVerdict::Label(l) => serde_json::json!({"verdict_label": l}),
        };
        // Phase-9 state-schema control (checked BEFORE the move into the
        // record): an incompatible state shape forces advisory OFF for this
        // observation (no event, no message) while the shadow record below
        // is still appended (change-matrix case C).
        let state_ok = crate::change_control::validate_state_shape("escalation", &state);
        let record = self.finish_versioned(
            question_set,
            &def,
            state,
            questions,
            hash,
            call.clone(),
            deterministic,
            session_task.clone(),
        );
        // Advisory projection: pure function of the already-obtained call.
        // Phase-9 state-schema control: an incompatible state shape forces
        // advisory OFF for this observation (no event, no message) while the
        // shadow record above is still appended (change-matrix case C).
        let answer = call.answers.get("escalation");
        let confidence = answer.and_then(|a| {
            a.confidence.or_else(|| a.noul.map(|p| (p - 0.5).abs() * 2.0))
        });
        let jev_result = answer
            .map(|a| a.result_string())
            .unwrap_or_else(|| "<unavailable>".to_string());
        // State-shape gate BEFORE advisory evaluation: incompatible shapes
        // (missing/extra keys, wrong types, over-length text) mean the
        // normalized input changed -> advisory OFF until revalidated.
        // (`state_ok` was computed before the move into the record above.)
        let event = if state_ok {
            crate::advisory::evaluate_advisory(
                &self.config,
                "escalation",
                question_set,
                answer,
                confidence,
                &jev_result,
                &det_json,
                provenance,
                session_task.as_deref(),
                call.latency_ms,
                call.input_tokens,
                call.output_tokens,
                call.status,
                call.http_status,
                &call.model,
            )
        } else {
            None
        };
        if let Some(ref ev) = event {
            let apath = self.config.advisory_log_path_for(&self.workspace_root);
            let _ = crate::advisory::append_advisory_event(&apath, ev);
            // Informational surface only: one tracing line for the escalation
            // signal. Never a response mutation, never a tool call, never a
            // state change. The "JEV ADVISORY" prefix plus "Informational
            // only." suffix are the CodeBro integration UX contract: the line
            // must never read as an authoritative CodeBro decision, and must
            // never carry an executable instruction (approve/deny/execute/
            // retry/delete/allow/block). The embedded `msg` itself stays the
            // frozen baseline shape ("Jev advisory: escalation signal
            // detected, confidence X."); this wrapper adds the operator
            // label without altering the locked message.
            if let Some(ref msg) = ev.advisory_message {
                tracing::info!(
                    "{}",
                    format_advisory_line(msg, &ev.advisory_state, ev.confidence)
                );
            }
        }
        Some((record, event))
    }

    /// Pure advisory projection from an already-obtained call (no I/O).
    /// Used by tests and offline collectors. Returns `None` for shadow-only
    /// outcomes per [`crate::advisory::evaluate_advisory`].
    pub fn project_advisory(
        &self,
        question_set: &str,
        question_id: &str,
        call: &JevCall,
        deterministic: &DeterministicVerdict,
        session_task: Option<String>,
        provenance: &str,
    ) -> Option<crate::advisory::AdvisoryEvent> {
        let det_json = match deterministic {
            DeterministicVerdict::Bool(b) => serde_json::json!({"verdict_bool": b}),
            DeterministicVerdict::Label(l) => serde_json::json!({"verdict_label": l}),
        };
        let answer = call.answers.get(question_id);
        let confidence = answer.and_then(|a| {
            a.confidence.or_else(|| a.noul.map(|p| (p - 0.5).abs() * 2.0))
        });
        let jev_result = answer
            .map(|a| a.result_string())
            .unwrap_or_else(|| "<unavailable>".to_string());
        crate::advisory::evaluate_advisory(
            &self.config,
            question_id,
            question_set,
            answer,
            confidence,
            &jev_result,
            &det_json,
            provenance,
            session_task.as_deref(),
            call.latency_ms,
            call.input_tokens,
            call.output_tokens,
            call.status,
            call.http_status,
            &call.model,
        )
    }

    /// Pure projection used by tests and the pilot: build the record from
    /// an already-obtained call without any I/O.
    pub fn project(
        &self,
        question_id: &str,
        state: serde_json::Value,
        questions: serde_json::Value,
        call: &JevCall,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
    ) -> Option<ShadowRecord> {
        let def = v1_questions().into_iter().find(|q| q.id == question_id)?;
        let hash = state_hash(&state);
        Some(self.finish_versioned(
            QUESTION_SET_VERSION,
            &def,
            state,
            questions,
            hash,
            call.clone(),
            deterministic,
            session_task,
        ))
    }

    /// Versioned pure projection (no I/O). Fail-closed on unknown version/id.
    pub fn project_versioned(
        &self,
        question_set: &str,
        question_id: &str,
        state: serde_json::Value,
        questions: serde_json::Value,
        call: &JevCall,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
    ) -> Option<ShadowRecord> {
        let def = crate::questions::find_question(question_set, question_id)?;
        let hash = state_hash(&state);
        Some(self.finish_versioned(
            question_set,
            &def,
            state,
            questions,
            hash,
            call.clone(),
            deterministic,
            session_task,
        ))
    }

    fn finish(
        &self,
        def: &ShadowQuestionDef,
        state: serde_json::Value,
        questions: serde_json::Value,
        hash: String,
        call: JevCall,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
    ) -> ShadowRecord {
        self.finish_versioned(
            QUESTION_SET_VERSION,
            def,
            state,
            questions,
            hash,
            call,
            deterministic,
            session_task,
        )
    }

    fn finish_versioned(
        &self,
        question_set: &str,
        def: &ShadowQuestionDef,
        state: serde_json::Value,
        questions: serde_json::Value,
        hash: String,
        call: JevCall,
        deterministic: DeterministicVerdict,
        session_task: Option<String>,
    ) -> ShadowRecord {
        let decision = to_shadow_decision(question_id_of(def), def.decision_type, &hash, &call);
        let agreement = if call.status == RequestStatus::Ok {
            compare(def, call.answers.get(def.id), &deterministic)
        } else {
            Agreement::JevUnavailable
        };
        let det_json = match &deterministic {
            DeterministicVerdict::Bool(b) => serde_json::json!({"verdict_bool": b}),
            DeterministicVerdict::Label(l) => serde_json::json!({"verdict_label": l}),
        };
        let record = ShadowRecord {
            timestamp: chrono::Utc::now().to_rfc3339(),
            question_id: def.id.to_string(),
            question_set_version: question_set.to_string(),
            decision,
            state,
            questions,
            deterministic: det_json,
            agreement: agreement.as_str().to_string(),
            http_status: call.http_status,
            flag_enabled: self.config.enabled,
            session_task: session_task.map(|s| crate::questions::hash_id(&s)),
        };
        // Logging is best-effort: a log failure must not propagate anywhere.
        let path = self.config.log_path_for(&self.workspace_root);
        let _ = append_record(&path, &record);
        record
    }
}

fn question_id_of(def: &ShadowQuestionDef) -> &str {
    def.id
}

/// Pure formatter for the single human-visible advisory tracing line.
///
/// CodeBro integration UX contract (pinned by tests):
/// - starts with `JEV ADVISORY: ` (unmistakably an observer note, never a
///   CodeBro decision),
/// - embeds the frozen baseline message (escalation signal + confidence),
/// - ends the sentence with `Informational only.` (no authority claim),
/// - appends `state=` / `confidence=` telemetry for operators,
/// - never contains an executable instruction per
///   [`crate::advisory::message_contains_executable_instruction`].
pub fn format_advisory_line(msg: &str, advisory_state: &str, confidence: Option<f64>) -> String {
    format!(
        "JEV ADVISORY: {msg} Informational only. state={advisory_state} confidence={confidence:?}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn noul_answer(p: f64) -> JevAnswer {
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

    fn choice_answer(choice: &str, conf: f64) -> JevAnswer {
        JevAnswer {
            qtype: "choice".to_string(),
            noul: None,
            choice: Some(choice.to_string()),
            score: None,
            probabilities: Some(HashMap::from([
                (choice.to_string(), conf),
                ("other".to_string(), 1.0 - conf),
            ])),
            confidence: Some(conf),
            legend: None,
        }
    }

    #[test]
    fn noul_agreement_thresholds() {
        let def = v1_questions().into_iter().find(|q| q.id == "shell_risk").unwrap();
        let t = DeterministicVerdict::Bool(true);
        assert_eq!(compare(&def, Some(&noul_answer(0.9)), &t), Agreement::Agree);
        assert_eq!(compare(&def, Some(&noul_answer(0.1)), &t), Agreement::Disagree);
        assert_eq!(compare(&def, Some(&noul_answer(0.5)), &t), Agreement::JevUncertain);
        assert_eq!(compare(&def, None, &t), Agreement::JevUnavailable);
    }

    #[test]
    fn choice_low_confidence_is_uncertain_not_disagreement() {
        let def = v1_questions().into_iter().find(|q| q.id == "routing").unwrap();
        let det = DeterministicVerdict::Label("debug".to_string());
        assert_eq!(
            compare(&def, Some(&choice_answer("implement", 0.4)), &det),
            Agreement::JevUncertain
        );
        assert_eq!(
            compare(&def, Some(&choice_answer("debug", 0.9)), &det),
            Agreement::Agree
        );
        assert_eq!(
            compare(&def, Some(&choice_answer("implement", 0.9)), &det),
            Agreement::Disagree
        );
    }

    #[test]
    fn deterministic_uncertain_never_counts_as_disagreement() {
        let def = v1_questions()
            .into_iter()
            .find(|q| q.id == "test_classification")
            .unwrap();
        let det = DeterministicVerdict::Label("uncertain".to_string());
        assert_eq!(
            compare(&def, Some(&choice_answer("product_failure", 0.95)), &det),
            Agreement::JevUncertain
        );
        assert_eq!(
            compare(&def, Some(&choice_answer("uncertain", 0.9)), &det),
            Agreement::Agree
        );
    }

    #[test]
    fn versioned_projection_logs_requested_version_and_defaults_to_v1() {
        use crate::adapter::JevCall;
        use crate::types::RequestStatus;
        let dir = tempfile::tempdir().unwrap();
        let cfg = crate::config::JevShadowConfig {
            enabled: true,
            advisory_enabled: false,
            endpoint: "http://127.0.0.1:9/unreachable".to_string(),
            model: crate::config::DEFAULT_MODEL.to_string(),
            timeout: std::time::Duration::from_millis(300),
            max_retries: 1,
            log_path_override: Some(dir.path().join("v.jsonl")),
            advisory_log_path_override: None,
        };
        let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
        let call = JevCall::unavailable(RequestStatus::MissingKey, None);
        let state = serde_json::json!({"probe": true});
        // Default path logs v1.
        let q1 = crate::questions::wire_questions_for(
            &crate::questions::find_question("v1", "routing").unwrap(),
        );
        let r1 = obs
            .project(
                "routing",
                state.clone(),
                q1,
                &call,
                DeterministicVerdict::Label("explore".to_string()),
                None,
            )
            .unwrap();
        assert_eq!(r1.question_set_version, "v1");
        // Explicit v2 logs v2 with identical comparison semantics.
        let q2 = crate::questions::wire_questions_for_version("v2", "routing").unwrap();
        let r2 = obs
            .project_versioned(
                "v2",
                "routing",
                state,
                q2,
                &call,
                DeterministicVerdict::Label("explore".to_string()),
                None,
            )
            .unwrap();
        assert_eq!(r2.question_set_version, "v2");
        // Unknown version fails closed.
        assert!(obs
            .project_versioned(
                "v9",
                "routing",
                serde_json::json!({}),
                serde_json::json!({}),
                &call,
                DeterministicVerdict::Label("explore".to_string()),
                None,
            )
            .is_none());
    }
}

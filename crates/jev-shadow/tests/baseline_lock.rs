//! Baseline-lock tests (Phase-10 audit task).
//!
//! Pins the validated baseline `jev-baseline-1` (evidence: Phases 3–10):
//! - record parses and matches every lock,
//! - EACH tripwire field independently forces mismatch (model, question
//!   version/hash, schema, threshold, scope id/version, authority, id),
//! - malformed records fail closed,
//! - the on-disk `baseline.json` is what the binary enforces,
//! - no moving alias (`jev-latest`) is the runtime baseline,
//! - validated operation still advises under the baseline gate.
//!
//! Env vars are process-global: key-needing tests serialize on `ENV_LOCK`.
//! No test prints `TYPESAFE_API_KEY`.

use codebro_jev_shadow::{
    advisory::ADVISORY_CONFIDENCE_THRESHOLD,
    change_control::{
        baseline_matches_runtime, baseline_record, current_v3_question_set_hash, verify_baseline,
        AUTHORITY_OWNER, BASELINE_ID, EXPECTED_V3_QUESTION_SET_HASH, LOCKED_MODEL,
        LOCKED_QUESTION_ID, LOCKED_QUESTION_SET_VERSION, STATE_SCHEMA_VERSION,
    },
    config::JevShadowConfig,
    shadow::{DeterministicVerdict, ShadowObserver},
    types::RequestStatus,
};
use std::sync::Mutex;
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
const FAKE_KEY: &str = "test-key-value-for-baseline-lock-only";

#[test]
fn baseline_record_parses_and_matches_locks() {
    let _guard = env_lock();
    let rec = baseline_record().expect("shipped baseline.json must parse");
    assert_eq!(rec.baseline_id, BASELINE_ID);
    assert_eq!(rec.baseline_id, "jev-baseline-1");
    assert_eq!(rec.model, LOCKED_MODEL);
    assert_eq!(rec.model, "jev-1.13.0");
    assert_eq!(rec.question_set_version, LOCKED_QUESTION_SET_VERSION);
    assert_eq!(rec.question_set_hash, EXPECTED_V3_QUESTION_SET_HASH);
    assert_eq!(rec.question_set_hash, current_v3_question_set_hash());
    assert_eq!(rec.state_schema_version, STATE_SCHEMA_VERSION);
    assert!((rec.confidence_threshold - 0.80).abs() < f64::EPSILON);
    assert!((rec.confidence_threshold - ADVISORY_CONFIDENCE_THRESHOLD).abs() < f64::EPSILON);
    assert_eq!(rec.advisory_scope.question_id, LOCKED_QUESTION_ID);
    assert_eq!(rec.advisory_scope.question_id, "escalation");
    assert_eq!(rec.advisory_scope.question_set_version, "v3");
    assert_eq!(rec.authority_owner, AUTHORITY_OWNER);
    assert_eq!(rec.authority_owner, "deterministic-codebro-policy");
    assert_eq!(rec.rollback_flag, "JEV_ADVISORY_ENABLED=false");
    assert!(
        verify_baseline(),
        "shipped baseline must verify against live runtime"
    );
}

#[test]
fn each_tripwire_field_independently_forces_mismatch() {
    let _guard = env_lock();
    let base = baseline_record().expect("baseline must parse");
    assert!(baseline_matches_runtime(&base));
    // Tamper exactly one field at a time: every single one must trip.
    type Tamper = Box<dyn Fn(&mut codebro_jev_shadow::change_control::BaselineRecord)>;
    let mut cases: Vec<(&str, Tamper)> = Vec::new();
    cases.push((
        "model",
        Box::new(|r| r.model = "jev-9.99.9-injected".to_string()),
    ));
    cases.push((
        "question_set_version",
        Box::new(|r| r.question_set_version = "v9".to_string()),
    ));
    cases.push((
        "question_set_hash",
        Box::new(|r| {
            r.question_set_hash = "0".repeat(64);
        }),
    ));
    cases.push((
        "state_schema_version",
        Box::new(|r| r.state_schema_version = "99".to_string()),
    ));
    cases.push((
        "confidence_threshold_up",
        Box::new(|r| r.confidence_threshold = 0.50),
    ));
    cases.push((
        "confidence_threshold_down",
        Box::new(|r| r.confidence_threshold = 0.95),
    ));
    cases.push((
        "scope_question_id",
        Box::new(|r| {
            r.advisory_scope.question_id = "routing".to_string();
        }),
    ));
    cases.push((
        "scope_question_version",
        Box::new(|r| {
            r.advisory_scope.question_set_version = "v2".to_string();
        }),
    ));
    cases.push((
        "authority_owner",
        Box::new(|r| {
            r.authority_owner = "jev".to_string();
        }),
    ));
    cases.push((
        "baseline_id",
        Box::new(|r| r.baseline_id = "jev-baseline-2-unvalidated".to_string()),
    ));
    assert!(cases.len() >= 10, "every lock field needs a tripwire case");
    for (name, tamper) in cases {
        let mut rec = base.clone();
        tamper(&mut rec);
        assert!(
            !baseline_matches_runtime(&rec),
            "tripwire {name} must force mismatch (advisory OFF)"
        );
    }
}

#[test]
fn malformed_baseline_records_fail_closed() {
    let _guard = env_lock();
    // Missing required fields -> parse fails (fail closed, never default-open).
    assert!(
        serde_json::from_str::<codebro_jev_shadow::change_control::BaselineRecord>("{}").is_err()
    );
    assert!(
        serde_json::from_str::<codebro_jev_shadow::change_control::BaselineRecord>("not json")
            .is_err()
    );
    // Wrong types -> parse fails.
    let bad_type = serde_json::json!({
        "baseline_id": "x",
        "model": "y",
        "question_set_version": "v3",
        "question_set_hash": "z",
        "state_schema_version": "1",
        "confidence_threshold": "high",
        "advisory_scope": {"question_id": "escalation", "question_set_version": "v3"},
        "authority_owner": "w"
    });
    assert!(
        serde_json::from_value::<codebro_jev_shadow::change_control::BaselineRecord>(bad_type)
            .is_err()
    );
}

#[test]
fn on_disk_baseline_json_is_what_runs() {
    let _guard = env_lock();
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("baseline.json");
    let text = std::fs::read_to_string(&path).expect("baseline.json must exist on disk");
    assert!(
        !text.contains("TYPESAFE"),
        "baseline must never carry secrets"
    );
    let disk: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(disk["baseline_id"], BASELINE_ID);
    assert_eq!(disk["model"], LOCKED_MODEL);
    assert_eq!(disk["question_set_hash"], current_v3_question_set_hash());
    assert_eq!(disk["confidence_threshold"], 0.80);
}

#[test]
fn no_moving_alias_is_the_runtime_baseline() {
    let _guard = env_lock();
    // jev-latest is upstream discovery only (Phase-3 eval scripts); the
    // locked runtime baseline MUST be the immutable validated version.
    let rec = baseline_record().expect("baseline must parse");
    assert!(
        !rec.model.contains("latest"),
        "moving alias must not be locked"
    );
    assert_eq!(JevShadowConfig::default().model, rec.model);
    // The alias may be DOCUMENTED as upstream-discovery-only (rule: never the
    // runtime baseline). If mentioned, the note must state non-runtime status.
    let text = include_str!("../baseline.json");
    if text.contains("jev-latest") {
        assert!(
            text.contains("MUST NOT be the runtime baseline")
                || text.contains("MUST NOT be locked"),
            "any alias mention must document non-runtime status"
        );
    }
}

#[test]
fn validated_operation_still_advises_under_baseline_gate() {
    let _guard = env_lock();
    // The new gate must not break validated operation: locked model + v3 +
    // high-confidence escalation still yields an event (verify_baseline true
    // inside evaluate_advisory).
    assert!(verify_baseline());
    let dir = tempfile::tempdir().unwrap();
    let cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: "http://127.0.0.1:9/unused".to_string(),
        model: LOCKED_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    };
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    let call = codebro_jev_shadow::adapter::JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 11,
        model: LOCKED_MODEL.to_string(),
        answers: [(
            "escalation".to_string(),
            codebro_jev_shadow::JevAnswer {
                qtype: "noul".to_string(),
                noul: Some(0.97),
                choice: None,
                score: None,
                probabilities: None,
                confidence: None,
                legend: None,
            },
        )]
        .into_iter()
        .collect(),
        input_tokens: 600,
        output_tokens: 22,
    };
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let ev = obs.project_advisory(
        "v3",
        "escalation",
        &call,
        &DeterministicVerdict::Bool(true),
        None,
        "baseline-lock:control",
    );
    std::env::remove_var("TYPESAFE_API_KEY");
    let ev = ev.expect("baseline-conformant call must still advise");
    assert_eq!(ev.advisory_state, "ADVISORY_ESCALATION");
}

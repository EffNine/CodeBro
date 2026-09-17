//! Phase-9 change-control + regression gate tests (offline, hermetic).
//!
//! Covers the validation-gate contract:
//! - model version pin (`jev-1.13.0`; requested AND resolved; any change OFF)
//! - question-set hash lock (v3 frozen; any content change OFF)
//! - state-schema version + shape validation (incompatible shape OFF)
//! - confidence threshold >= 0.80
//! - flags default OFF
//! - no authority surfaces (import + fn-name scan)
//! - regression fixtures (all 7 boundaries, oracle-pinned, never tuned)
//! - secret redaction of fixture/state builders
//! - failure isolation (unavailable -> no event)
//! - observability fields on every advisory event
//! - rollback returns to shadow-only
//!
//! Env vars are process-global: all tests serialize on `ENV_LOCK`.
//! No test prints `TYPESAFE_API_KEY`.

use codebro_jev_shadow::{
    adapter::JevCall,
    advisory::{classify_advisory, AdvisoryState},
    change_control::{
        current_v3_question_set_hash, model_matches_lock, question_set_matches_lock,
        reference_escalation_verdict, reference_routing_label, reference_shell_safe,
        validate_state_shape, EXPECTED_V3_QUESTION_SET_HASH, LOCKED_CONFIDENCE_THRESHOLD,
        LOCKED_MODEL, LOCKED_QUESTION_ID, LOCKED_QUESTION_SET_VERSION, STATE_SCHEMA_VERSION,
    },
    config::{JevShadowConfig, DEFAULT_MODEL},
    logging::scan_text_for_secret,
    questions::{self, state_escalation},
    shadow::{DeterministicVerdict, ShadowObserver},
    types::{JevAnswer, RequestStatus},
};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
const FAKE_KEY: &str = "test-key-value-for-change-control-tests-only";

// ── helpers ────────────────────────────────────────────────────────────

fn live_cfg() -> JevShadowConfig {
    JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: "http://127.0.0.1:9/unused".to_string(),
        model: LOCKED_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    }
}

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

fn ok_escalation_call(p: f64, model: &str) -> JevCall {
    JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 12,
        model: model.to_string(),
        answers: [("escalation".to_string(), noul_answer(p))]
            .into_iter()
            .collect(),
        input_tokens: 600,
        output_tokens: 22,
    }
}

fn with_live_env<T>(f: impl FnOnce() -> T) -> T {
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = f();
    std::env::remove_var("TYPESAFE_API_KEY");
    out
}

// ── 1. version control: model ──────────────────────────────────────────

#[test]
fn model_is_pinned_to_validated_version() {
    let _guard = env_lock();
    assert_eq!(LOCKED_MODEL, "jev-1.13.0");
    assert_eq!(DEFAULT_MODEL, "jev-1.13.0");
    assert_eq!(JevShadowConfig::default().model, "jev-1.13.0");
    // No silent jev-latest dependency anywhere in the lock surface.
    assert!(!LOCKED_MODEL.contains("latest"));
    assert!(!DEFAULT_MODEL.contains("latest"));
}

#[test]
fn model_gate_requires_requested_and_resolved_to_match_lock() {
    let _guard = env_lock();
    assert!(model_matches_lock("jev-1.13.0", "jev-1.13.0"));
    // Requested drift (e.g. operator set jev-latest) -> OFF.
    assert!(!model_matches_lock("jev-latest", "jev-1.13.0"));
    assert!(!model_matches_lock("jev-1.14.0", "jev-1.14.0"));
    // Provider-side substitution (resolved differs) -> OFF even when
    // requested is correct.
    assert!(!model_matches_lock("jev-1.13.0", "jev-1.14.0"));
    assert!(!model_matches_lock("jev-1.13.0", ""));
}

// ── 2. question version control ────────────────────────────────────────

#[test]
fn question_lock_hash_matches_expected() {
    let _guard = env_lock();
    assert_eq!(LOCKED_QUESTION_SET_VERSION, "v3");
    assert_eq!(LOCKED_QUESTION_ID, "escalation");
    assert_eq!(
        current_v3_question_set_hash(),
        EXPECTED_V3_QUESTION_SET_HASH
    );
    assert!(question_set_matches_lock());
    // The recorded hash is a full sha256 hex, not a placeholder.
    assert_eq!(EXPECTED_V3_QUESTION_SET_HASH.len(), 64);
    assert!(EXPECTED_V3_QUESTION_SET_HASH
        .chars()
        .all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn v3_questions_not_modified_during_phase9() {
    let _guard = env_lock();
    // Phase-9 rule: do not modify existing v3 questions. The canonical
    // escalation wording must still be the frozen Phase-7 text.
    assert!(questions::V3_ESCALATION_INSTRUCTIONS.contains("Skill-approve actions ALWAYS"));
    assert!(questions::V3_ESCALATION_TRUE.contains("any skill-approve action"));
    assert!(questions::V3_ESCALATION_FALSE.contains("ONLY after verified passing evidence"));
    for qid in [
        "tool_selection",
        "shell_risk",
        "routing",
        "test_classification",
    ] {
        let w2 = questions::wire_questions_for_version("v2", qid).unwrap();
        let w3 = questions::wire_questions_for_version("v3", qid).unwrap();
        assert_eq!(
            serde_json::to_string(&w2).unwrap(),
            serde_json::to_string(&w3).unwrap()
        );
    }
}

// ── 3. state schema control ────────────────────────────────────────────

#[test]
fn state_schema_version_is_recorded() {
    let _guard = env_lock();
    assert_eq!(STATE_SCHEMA_VERSION, "1");
}

#[test]
fn valid_builder_states_pass_shape_validation() {
    let _guard = env_lock();
    assert!(validate_state_shape(
        "escalation",
        &state_escalation("delete", "delete_memory key=x confirm=true")
    ));
    assert!(validate_state_shape(
        "tool_selection",
        &questions::state_tool_selection(&["shadow".to_string()], &["jev-shadow".to_string()])
    ));
    assert!(validate_state_shape(
        "shell_risk",
        &questions::state_shell_risk("git status --short", false, false, 30)
    ));
    assert!(validate_state_shape(
        "routing",
        &questions::state_routing("recall prior decision")
    ));
    assert!(validate_state_shape(
        "test_classification",
        &questions::state_test_classification("success", 0, Some("cargo test"), &[])
    ));
}

#[test]
fn incompatible_state_shapes_fail_closed() {
    let _guard = env_lock();
    // Missing key.
    assert!(!validate_state_shape(
        "escalation",
        &serde_json::json!({"action_kind": "task"})
    ));
    // Extra key (schema drift).
    assert!(!validate_state_shape(
        "escalation",
        &serde_json::json!({"action_kind": "task", "summary": "x", "policy_verdict": true})
    ));
    // Wrong types.
    assert!(!validate_state_shape(
        "escalation",
        &serde_json::json!({"action_kind": 7, "summary": "x"})
    ));
    // Non-object.
    assert!(!validate_state_shape(
        "escalation",
        &serde_json::json!("task complete")
    ));
    // Unknown question id.
    assert!(!validate_state_shape(
        "score_overall",
        &serde_json::json!({"score": 0.9})
    ));
    // shell_risk missing policy flags (the P5-12/P6-10 missing-context shape
    // must NOT validate as a complete state).
    assert!(!validate_state_shape(
        "shell_risk",
        &serde_json::json!({"command": "git status"})
    ));
}

// ── 4. advisory enforcement of all locks ───────────────────────────────

#[test]
fn advisory_enforces_model_question_locks() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let det = DeterministicVerdict::Bool(true);
    // Locked model + v3 + high-conf yes -> event present.
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let good = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, LOCKED_MODEL),
            &det,
            None,
            "test:locks-good",
        )
    });
    let ev = good.expect("locked config must produce advisory event");
    assert_eq!(ev.advisory_state, "ADVISORY_ESCALATION");
    assert!(ev.advisory_message.is_some());

    // Requested-model drift -> OFF.
    let mut drifted = live_cfg();
    drifted.model = "jev-latest".to_string();
    let obs_drift = ShadowObserver::new(drifted, dir.path().to_path_buf());
    let ev_drift = with_live_env(|| {
        obs_drift.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, "jev-latest"),
            &det,
            None,
            "test:locks-drifted",
        )
    });
    assert!(
        ev_drift.is_none(),
        "requested model drift must force advisory OFF"
    );

    // Provider-side resolved-model substitution -> OFF.
    let ev_sub = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, "jev-1.14.0"),
            &det,
            None,
            "test:locks-substituted",
        )
    });
    assert!(
        ev_sub.is_none(),
        "resolved model mismatch must force advisory OFF"
    );

    // Wrong question-set version -> OFF (advisory is v3-only).
    let v2_call = JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 5,
        model: LOCKED_MODEL.to_string(),
        answers: [("escalation".to_string(), noul_answer(0.97))]
            .into_iter()
            .collect(),
        input_tokens: 500,
        output_tokens: 20,
    };
    let ev_v2 = with_live_env(|| {
        obs.project_advisory("v2", "escalation", &v2_call, &det, None, "test:locks-v2")
    });
    assert!(
        ev_v2.is_none(),
        "non-v3 question set must force advisory OFF"
    );
}

#[test]
#[allow(clippy::assertions_on_constants)] // const-ness IS the gate: threshold must be a compile-time constant
fn confidence_threshold_is_at_least_0_80_and_enforced() {
    let _guard = env_lock();
    assert!(LOCKED_CONFIDENCE_THRESHOLD >= 0.80);
    assert_eq!(LOCKED_CONFIDENCE_THRESHOLD, 0.80);
    assert_eq!(
        codebro_jev_shadow::advisory::ADVISORY_CONFIDENCE_THRESHOLD,
        0.80
    );
    // Boundary: p=0.90 -> conf 0.80 exactly -> escalation; p=0.89 -> 0.78 -> uncertain.
    assert_eq!(
        classify_advisory("escalation", Some(&noul_answer(0.90))),
        Some(AdvisoryState::AdvisoryEscalation)
    );
    assert_eq!(
        classify_advisory("escalation", Some(&noul_answer(0.89))),
        Some(AdvisoryState::Uncertain)
    );
}

// ── 5. flags default OFF ───────────────────────────────────────────────

#[test]
fn flags_default_off_no_silent_enable() {
    let _guard = env_lock();
    let cfg = JevShadowConfig::default();
    assert!(!cfg.enabled);
    assert!(!cfg.advisory_enabled);
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    let env_cfg = JevShadowConfig::from_env();
    assert!(!env_cfg.enabled);
    assert!(!env_cfg.advisory_enabled);
    assert!(!env_cfg.is_live());
    assert!(!env_cfg.is_advisory_live());
}

// ── 6. no authority surfaces ───────────────────────────────────────────

#[test]
fn change_control_and_advisory_have_no_authority_surface() {
    let _guard = env_lock();
    for src in [
        include_str!("../src/change_control.rs"),
        include_str!("../src/advisory.rs"),
    ] {
        for banned in [
            "std::process",
            "tokio::process",
            "Command::new",
            "codebro_sandbox",
            "codebro_context",
            "codebro_change",
            "codebro_memory",
            "codebro_identity",
            "tool_router",
            "call_tool",
            "SandboxRuntime",
            "ChangeEngine",
        ] {
            assert!(!src.contains(banned), "authority surface leaked: {banned}");
        }
        for banned_fn in [
            "fn approve",
            "fn deny",
            "fn execute",
            "fn retry",
            "fn mutate",
        ] {
            assert!(
                !src.contains(banned_fn),
                "authority verb defined: {banned_fn}"
            );
        }
    }
    // The reference oracles are test-only mirrors: they must never be wired
    // into an execution path. Only tests + this module may name them.
    // (Enforced by review + this grep over the execution-path seam.)
    let hook_src = include_str!("../../mcp-server/src/jev_shadow_hook.rs");
    assert!(
        !hook_src.contains("reference_escalation_verdict"),
        "test oracle must not appear in execution paths"
    );
}

// ── 7. regression fixtures (all 7 boundaries) ──────────────────────────

#[derive(Debug, serde::Deserialize)]
struct Fixture {
    id: String,
    boundary: String,
    question_id: String,
    state: serde_json::Value,
    deterministic_bool: Option<bool>,
    deterministic_safe: Option<bool>,
    deterministic_label: Option<String>,
}

#[test]
fn regression_fixtures_match_frozen_oracles() {
    let _guard = env_lock();
    let raw = include_str!("fixtures/phase9_regression.json");
    let doc: serde_json::Value = serde_json::from_str(raw).expect("fixtures must parse");
    assert_eq!(doc["question_set_version"], "v3");
    assert_eq!(doc["model"], LOCKED_MODEL);
    assert_eq!(doc["state_schema_version"], STATE_SCHEMA_VERSION);
    let fixtures: Vec<Fixture> =
        serde_json::from_value(doc["fixtures"].clone()).expect("fixture shape must parse");
    assert!(!fixtures.is_empty(), "regression set must not be empty");

    // All 7 required boundaries present.
    let mut boundaries = std::collections::HashSet::new();
    for f in &fixtures {
        boundaries.insert(f.boundary.as_str());
    }
    for required in [
        "qualifier-sensitive completion",
        "skill-approve confirm=true",
        "propose+flag",
        "S16 boundary",
        "shell-risk boundary",
        "approval boundary",
        "routing boundary",
    ] {
        assert!(
            boundaries.contains(required),
            "missing boundary: {required}"
        );
    }

    // Every fixture: shape validates + frozen oracle reproduces the recorded
    // deterministic expectation (fixtures are never tuned to Jev output).
    let mut checked = 0usize;
    for f in &fixtures {
        assert!(
            validate_state_shape(&f.question_id, &f.state),
            "{}: state must validate under schema v{}",
            f.id,
            STATE_SCHEMA_VERSION
        );
        match f.question_id.as_str() {
            "escalation" => {
                let want = f
                    .deterministic_bool
                    .expect("escalation needs deterministic_bool");
                let got = reference_escalation_verdict(
                    f.state["action_kind"].as_str().unwrap_or(""),
                    f.state["summary"].as_str().unwrap_or(""),
                );
                assert_eq!(
                    got, want,
                    "{}: oracle mismatch (fixture frozen, oracle frozen)",
                    f.id
                );
            }
            "shell_risk" => {
                let want = f
                    .deterministic_safe
                    .expect("shell_risk needs deterministic_safe");
                let got = reference_shell_safe(f.state["command"].as_str().unwrap_or(""));
                assert_eq!(got, want, "{}: shell oracle mismatch", f.id);
            }
            "routing" => {
                let want = f
                    .deterministic_label
                    .as_deref()
                    .expect("routing needs label");
                let got = reference_routing_label(f.state["task_summary"].as_str().unwrap_or(""));
                assert_eq!(got, want, "{}: routing oracle mismatch", f.id);
            }
            other => panic!("unknown fixture question: {other}"),
        }
        checked += 1;
    }
    assert!(checked >= 40, "regression set too small: {checked}");
}

#[test]
fn non_escalation_fixtures_stay_shadow_only() {
    let _guard = env_lock();
    // Advisory is escalation-only: even a successful high-confidence
    // shell_risk/routing call must never produce an advisory event.
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let mut answers = HashMap::new();
    answers.insert(
        "routing".to_string(),
        JevAnswer {
            qtype: "choice".to_string(),
            noul: None,
            choice: Some("debug".to_string()),
            score: None,
            probabilities: Some([("debug".to_string(), 0.95)].into_iter().collect()),
            confidence: Some(0.95),
            legend: None,
        },
    );
    let call = JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 8,
        model: LOCKED_MODEL.to_string(),
        answers,
        input_tokens: 200,
        output_tokens: 10,
    };
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "routing",
            &call,
            &DeterministicVerdict::Label("debug".to_string()),
            None,
            "test:non-escalation-silence",
        )
    });
    assert!(ev.is_none(), "routing must stay shadow-only");
}

// ── 8. secret redaction + failure isolation ────────────────────────────

#[test]
fn fixture_states_and_builders_redact_secrets() {
    let _guard = env_lock();
    let secret = "sk-testsecret-ABCDEFGHIJKLMNOP-123456";
    let summary = format!("delete_memory key=x confirm=true Bearer {secret}");
    let state = state_escalation("delete", &summary);
    let serialized = serde_json::to_string(&state).unwrap();
    assert!(!scan_text_for_secret(&serialized, secret));
    assert!(serialized.contains("REDACTED"));
    // Frozen fixtures carry no secret-shaped bearer material.
    let raw = include_str!("fixtures/phase9_regression.json");
    assert!(!raw.contains("Bearer "));
    assert!(!raw.contains("sk-"));
}

#[test]
fn jev_failure_isolation_no_event_without_success() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    for status in [
        RequestStatus::Timeout,
        RequestStatus::Network,
        RequestStatus::Http429,
        RequestStatus::Http529,
        RequestStatus::Http5xx,
        RequestStatus::Malformed,
    ] {
        let call = JevCall::unavailable(status, None);
        let ev = with_live_env(|| {
            obs.project_advisory(
                "v3",
                "escalation",
                &call,
                &DeterministicVerdict::Bool(true),
                None,
                "test:failure-isolation",
            )
        });
        assert!(ev.is_none(), "{status:?} must not produce advisory output");
    }
}

// ── 9. observability fields ────────────────────────────────────────────

#[test]
fn advisory_events_carry_full_observability_envelope() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, LOCKED_MODEL),
            &DeterministicVerdict::Bool(true),
            Some("session-123".to_string()),
            "test:observability",
        )
    })
    .expect("locked high-conf escalation must produce event");
    assert_eq!(ev.model_requested, LOCKED_MODEL);
    assert_eq!(ev.model_resolved, LOCKED_MODEL);
    assert_eq!(ev.question_set_version, "v3");
    assert_eq!(ev.question_set_hash, EXPECTED_V3_QUESTION_SET_HASH);
    assert_eq!(ev.state_schema_version, STATE_SCHEMA_VERSION);
    assert!(ev.confidence.unwrap() >= 0.80);
    assert!(!ev.jev_result.is_empty());
    assert!(!ev.timestamp.is_empty());
    assert_eq!(ev.latency_ms, 12);
    assert_eq!(ev.input_tokens, 600);
    assert_eq!(ev.output_tokens, 22);
    assert_eq!(ev.provenance, "test:observability");
    // session_task is hashed, never raw.
    let hashed = ev.session_task_hash.clone().unwrap();
    assert!(!hashed.contains("session-123"));
    assert_eq!(hashed.len(), 16);
    // Serialized form carries every required observability key and no secret.
    let line = serde_json::to_string(&ev).unwrap();
    for key in [
        "model_requested",
        "model_resolved",
        "question_set_version",
        "question_set_hash",
        "state_schema_version",
        "confidence",
        "jev_result",
        "timestamp",
        "latency_ms",
        "input_tokens",
        "output_tokens",
        "provenance",
    ] {
        assert!(line.contains(key), "observability key missing: {key}");
    }
    assert!(!line.contains(FAKE_KEY), "event must never carry the key");
}

// ── 10. state-shape gate on the live observe path ──────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env vars (established suite pattern)
async fn incompatible_state_shape_disables_advisory_but_keeps_shadow_record() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    let _guard = env_lock();
    // Minimal mock: always 200 with high-conf escalation.
    let body = serde_json::json!({
        "model": LOCKED_MODEL,
        "answers": {"escalation": {"type": "noul", "noul": 0.97}},
        "usage": {"input_tokens": 600, "output_tokens": 22}
    })
    .to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let hits = Arc::clone(&hits_clone);
            let body = body.clone();
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 8192];
                let _ = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf)).await;
                hits.fetch_add(1, Ordering::SeqCst);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(body.as_bytes()).await;
            });
        }
    });
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = live_cfg();
    cfg.endpoint = format!("http://{addr}/v1/systemone");
    cfg.log_path_override = Some(dir.path().join("shadow.jsonl"));
    cfg.advisory_log_path_override = Some(dir.path().join("advisory.jsonl"));
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());

    // Invalid state shape (extra key = schema drift): shadow record still
    // appended, advisory event forced OFF.
    let bad_state = serde_json::json!({"action_kind": "task", "summary": "task complete", "ranking_score": 0.9});
    assert!(!validate_state_shape("escalation", &bad_state));
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = obs
        .observe_escalation_with_advisory(
            "v3",
            bad_state,
            DeterministicVerdict::Bool(false),
            None,
            "test:state-gate",
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    let (record, event) = out.expect("shadow record must still be appended");
    assert_eq!(record.question_id, "escalation");
    assert!(
        event.is_none(),
        "incompatible state must force advisory OFF"
    );
    assert!(
        !dir.path().join("advisory.jsonl").exists(),
        "no advisory file on state-gate trip"
    );

    // Valid state shape on the same path: advisory event present (control).
    let mut cfg2 = live_cfg();
    cfg2.endpoint = format!("http://{addr}/v1/systemone");
    cfg2.log_path_override = Some(dir.path().join("shadow.jsonl"));
    cfg2.advisory_log_path_override = Some(dir.path().join("advisory.jsonl"));
    let obs2 = ShadowObserver::new(cfg2, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out2 = obs2
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("delete", "delete_memory key=x confirm=true"),
            DeterministicVerdict::Bool(true),
            None,
            "test:state-gate-control",
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    let (_, event2) = out2.expect("control must log");
    assert!(event2.is_some(), "valid state must allow advisory");
}

// ── 11. rollback ───────────────────────────────────────────────────────

#[test]
fn rollback_to_shadow_only_removes_advisory_output() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    // Advisory ON: event present.
    let live = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let ev_live = with_live_env(|| {
        live.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, LOCKED_MODEL),
            &DeterministicVerdict::Bool(true),
            None,
            "test:rollback-live",
        )
    });
    assert!(ev_live.is_some(), "sanity: live advisory produces event");
    // Rollback: JEV_ADVISORY_ENABLED=false -> shadow-only, no event.
    let mut rolled = live_cfg();
    rolled.advisory_enabled = false;
    let obs_rolled = ShadowObserver::new(rolled, dir.path().to_path_buf());
    assert!(!obs_rolled.is_advisory_live());
    let ev_rolled = with_live_env(|| {
        obs_rolled.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, LOCKED_MODEL),
            &DeterministicVerdict::Bool(true),
            None,
            "test:rollback-off",
        )
    });
    assert!(ev_rolled.is_none(), "rollback must remove advisory output");
}

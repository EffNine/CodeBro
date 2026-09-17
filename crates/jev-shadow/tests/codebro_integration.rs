//! CodeBro integration tests for the locked `jev-baseline-1` advisory.
//!
//! Focused coverage for the live seam (`skill request_approval` →
//! `observe_escalation_advisory` → `observe_escalation_with_advisory` →
//! `evaluate_advisory`): flags OFF, locked-baseline validity, the >=0.80
//! gate, below-threshold and non-escalation silence, baseline mismatch,
//! Jev failure, secret redaction, authority isolation, and advisory wording.
//!
//! Does NOT run a benchmark, create a version, or modify the baseline.
//! Env vars are process-global: all tests serialize on `ENV_LOCK`.
//! No test prints `TYPESAFE_API_KEY`.

use codebro_jev_shadow::{
    advisory::{
        advisory_message_for, message_contains_executable_instruction, AdvisoryState,
        ADVISORY_CONFIDENCE_THRESHOLD,
    },
    change_control,
    config::{JevShadowConfig, DEFAULT_MODEL},
    questions::{self, state_escalation},
    shadow::{format_advisory_line, DeterministicVerdict, ShadowObserver},
    types::RequestStatus,
};
use std::sync::Mutex;
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
const FAKE_KEY: &str = "test-key-value-for-integration-tests-only";
const PROVENANCE: &str = "codebro:skill:request_approval";

fn live_cfg() -> JevShadowConfig {
    JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: "http://127.0.0.1:9/unused".to_string(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_millis(300),
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    }
}

fn with_live_env<T>(f: impl FnOnce() -> T) -> T {
    std::env::set_var("JEV_SHADOW_ENABLED", "true");
    std::env::set_var("JEV_ADVISORY_ENABLED", "true");
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = f();
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    std::env::remove_var("TYPESAFE_API_KEY");
    out
}

fn noul_answer(p: f64) -> codebro_jev_shadow::JevAnswer {
    codebro_jev_shadow::JevAnswer {
        qtype: "noul".to_string(),
        noul: Some(p),
        choice: None,
        score: None,
        probabilities: None,
        confidence: None,
        legend: None,
    }
}

fn ok_escalation_call(p: f64, model: &str) -> codebro_jev_shadow::adapter::JevCall {
    codebro_jev_shadow::adapter::JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 12,
        model: model.to_string(),
        answers: [("escalation".to_string(), noul_answer(p))].into_iter().collect(),
        input_tokens: 600,
        output_tokens: 22,
    }
}

// ── 1. flags OFF: zero advisory output ───────────────────────────────

#[test]
fn flags_off_by_default_and_advisory_not_live() {
    let _guard = env_lock();
    let cfg = JevShadowConfig::default();
    assert!(!cfg.enabled, "JEV_SHADOW_ENABLED MUST default false");
    assert!(!cfg.advisory_enabled, "JEV_ADVISORY_ENABLED MUST default false");
    assert!(!cfg.is_live());
    assert!(!cfg.is_advisory_live());
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    std::env::remove_var("TYPESAFE_API_KEY");
    let env_cfg = JevShadowConfig::from_env();
    assert!(!env_cfg.enabled);
    assert!(!env_cfg.advisory_enabled);
    assert!(!env_cfg.is_live());
    assert!(!env_cfg.is_advisory_live());
}

#[test]
fn advisory_off_returns_no_event_even_at_high_confidence() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    // Shadow ON + advisory OFF: pure projection stays shadow-only.
    let cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: false,
        endpoint: "http://127.0.0.1:9/unused".to_string(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_millis(300),
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    };
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let ev = obs.project_advisory(
        "v3",
        "escalation",
        &ok_escalation_call(0.97, DEFAULT_MODEL),
        &DeterministicVerdict::Bool(true),
        None,
        PROVENANCE,
    );
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev.is_none(), "advisory OFF must stay shadow-only");
}

// ── 2. valid locked baseline advises ─────────────────────────────────

#[test]
fn valid_locked_baseline_high_conf_escalation_advises_with_telemetry() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, DEFAULT_MODEL),
            &DeterministicVerdict::Bool(true),
            Some("session-xyz".to_string()),
            PROVENANCE,
        )
    })
    .expect("locked high-conf escalation must advise");
    assert_eq!(ev.advisory_state, "ADVISORY_ESCALATION");
    assert_eq!(ev.baseline_id, "jev-baseline-1");
    assert_eq!(ev.baseline_id, change_control::BASELINE_ID);
    assert_eq!(ev.model_requested, "jev-1.13.0");
    assert_eq!(ev.model_resolved, "jev-1.13.0");
    assert_eq!(ev.question_set_version, "v3");
    assert_eq!(ev.question_set_hash, change_control::EXPECTED_V3_QUESTION_SET_HASH);
    assert_eq!(ev.state_schema_version, "1");
    assert!(!ev.state_schema_hash.is_empty(), "state schema hash must be recorded");
    assert_eq!(ev.question_id, "escalation");
    assert!(ev.confidence.unwrap() >= ADVISORY_CONFIDENCE_THRESHOLD);
    assert_eq!(ev.provenance, PROVENANCE);
    assert!(!ev.timestamp.is_empty());
    assert_eq!(ev.latency_ms, 12);
    assert_eq!(ev.input_tokens, 600);
    assert_eq!(ev.output_tokens, 22);
    let msg = ev.advisory_message.clone().expect("escalation carries message");
    assert!(msg.starts_with("Jev advisory:"));
    assert!(!message_contains_executable_instruction(&msg));
    // Serialized telemetry carries every required key and no secret.
    let line = serde_json::to_string(&ev).unwrap();
    for key in [
        "baseline_id",
        "model_requested",
        "model_resolved",
        "question_set_version",
        "question_set_hash",
        "state_schema_version",
        "state_schema_hash",
        "question_id",
        "confidence",
        "jev_result",
        "advisory_state",
        "latency_ms",
        "input_tokens",
        "output_tokens",
        "provenance",
        "timestamp",
    ] {
        assert!(line.contains(key), "telemetry key missing: {key}");
    }
    assert!(!line.contains(FAKE_KEY));
}

// ── 3. threshold + silence rules ─────────────────────────────────────

#[test]
fn below_threshold_escalation_is_silent() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    // p=0.89 -> conf 0.78 < 0.80: UNCERTAIN, no message. Boundary pinned:
    // p=0.90 (conf 0.80) advises, p=0.89 does not.
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.89, DEFAULT_MODEL),
            &DeterministicVerdict::Bool(true),
            None,
            PROVENANCE,
        )
    });
    match ev {
        None => {}
        Some(ev) => {
            assert_ne!(ev.advisory_state, "ADVISORY_ESCALATION");
            assert!(ev.advisory_message.is_none());
        }
    }
    // Boundary: p=0.90 advises.
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.90, DEFAULT_MODEL),
            &DeterministicVerdict::Bool(true),
            None,
            PROVENANCE,
        )
    })
    .expect("p=0.90 (conf 0.80) must advise");
    assert_eq!(ev.advisory_state, "ADVISORY_ESCALATION");
    assert!(ev.advisory_message.is_some());
}

#[test]
fn non_escalation_question_stays_silent() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let call = codebro_jev_shadow::adapter::JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 10,
        model: DEFAULT_MODEL.to_string(),
        answers: [(
            "routing".to_string(),
            codebro_jev_shadow::JevAnswer {
                qtype: "choice".to_string(),
                noul: None,
                choice: Some("debug".to_string()),
                score: None,
                probabilities: Some([("debug".to_string(), 0.95)].into_iter().collect()),
                confidence: Some(0.95),
                legend: None,
            },
        )]
        .into_iter()
        .collect(),
        input_tokens: 100,
        output_tokens: 10,
    };
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "routing",
            &call,
            &DeterministicVerdict::Label("debug".to_string()),
            None,
            PROVENANCE,
        )
    });
    assert!(ev.is_none(), "non-escalation must stay shadow-only");
}

// ── 4. baseline mismatch suppresses ──────────────────────────────────

#[test]
fn baseline_mismatch_suppresses_advisory() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    // Model drift (requested or resolved) -> silent.
    for model in ["jev-latest", "jev-1.12.0", "jev-2.0.0"] {
        let ev = with_live_env(|| {
            obs.project_advisory(
                "v3",
                "escalation",
                &ok_escalation_call(0.97, model),
                &DeterministicVerdict::Bool(true),
                None,
                PROVENANCE,
            )
        });
        assert!(ev.is_none(), "model drift {model} must suppress advisory");
    }
    // Question-set version drift -> silent.
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v2",
            "escalation",
            &ok_escalation_call(0.97, DEFAULT_MODEL),
            &DeterministicVerdict::Bool(true),
            None,
            PROVENANCE,
        )
    });
    assert!(ev.is_none(), "question version drift must suppress advisory");
    // Baseline record itself verifies against the live runtime.
    assert!(change_control::verify_baseline(), "locked baseline must verify");
    assert_eq!(change_control::BASELINE_ID, "jev-baseline-1");
}

// ── 5. Jev failure suppresses ────────────────────────────────────────

#[test]
fn jev_failure_suppresses_advisory() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    for status in [
        RequestStatus::MissingKey,
        RequestStatus::Timeout,
        RequestStatus::Http401,
        RequestStatus::Http403,
        RequestStatus::Http422,
        RequestStatus::Http429,
        RequestStatus::Http529,
        RequestStatus::Http5xx,
        RequestStatus::Network,
        RequestStatus::Malformed,
    ] {
        let call = codebro_jev_shadow::adapter::JevCall {
            status,
            http_status: None,
            latency_ms: 5,
            model: DEFAULT_MODEL.to_string(),
            answers: Default::default(),
            input_tokens: 0,
            output_tokens: 0,
        };
        let ev = with_live_env(|| {
            obs.project_advisory(
                "v3",
                "escalation",
                &call,
                &DeterministicVerdict::Bool(true),
                None,
                PROVENANCE,
            )
        });
        assert!(ev.is_none(), "failure {status:?} must suppress advisory");
    }
}

// ── 6. secret redaction ──────────────────────────────────────────────

#[test]
fn integration_states_and_events_redact_secrets() {
    let _guard = env_lock();
    let secret = "sk-integration-SECRET-9876543210-abcdef";
    let summary = format!("skill approve candidate=x request=y Bearer {secret}");
    let state = state_escalation("skill", &summary);
    let serialized = serde_json::to_string(&state).unwrap();
    assert!(!serialized.contains(secret), "secret leaked into escalation state");
    assert!(serialized.contains("REDACTED"));
    // Advisory event built from a redacted state never carries the key.
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(live_cfg(), dir.path().to_path_buf());
    let ev = with_live_env(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &ok_escalation_call(0.97, DEFAULT_MODEL),
            &DeterministicVerdict::Bool(true),
            None,
            PROVENANCE,
        )
    })
    .unwrap();
    let line = serde_json::to_string(&ev).unwrap();
    assert!(!line.contains(secret));
    assert!(!line.contains(FAKE_KEY));
    // Client debug never carries the key.
    let client = codebro_jev_shadow::JevClient::new(&live_cfg());
    let dbg = format!("{client:?}");
    assert!(!dbg.contains(FAKE_KEY));
    assert!(!dbg.to_lowercase().contains("bearer"));
}

// ── 7. authority isolation ───────────────────────────────────────────

#[test]
fn integration_layers_have_no_authority_surface() {
    // Advisory + shadow sources must not grow an execution surface.
    let advisory = include_str!("../src/advisory.rs");
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
        assert!(!advisory.contains(banned), "advisory.rs must not contain {banned}");
    }
    for banned_fn in ["fn approve", "fn deny", "fn execute", "fn retry"] {
        assert!(!advisory.contains(banned_fn), "advisory.rs must not define {banned_fn}");
    }
    let shadow = include_str!("../src/shadow.rs");
    for banned in ["ChangeEngine", "SandboxRuntime", "call_tool", "std::process"] {
        assert!(!shadow.contains(banned), "shadow.rs must not contain {banned}");
    }
}

// ── 8. advisory wording ──────────────────────────────────────────────

#[test]
fn advisory_message_and_operator_line_are_informational_only() {
    let _guard = env_lock();
    // Frozen baseline message: escalation signal + confidence, no authority verb.
    let m = advisory_message_for(AdvisoryState::AdvisoryEscalation, 0.94).unwrap();
    assert_eq!(m, "Jev advisory: escalation signal detected, confidence 0.94.");
    assert!(!message_contains_executable_instruction(&m));
    assert_eq!(advisory_message_for(AdvisoryState::AdvisoryNotTriggered, 0.99), None);
    assert_eq!(advisory_message_for(AdvisoryState::Uncertain, 0.4), None);
    assert_eq!(advisory_message_for(AdvisoryState::Unavailable, 0.0), None);
    // Extended forbidden verbs (allow/block) are caught.
    for bad in [
        "approve this",
        "DENY the request",
        "execute now",
        "retry once",
        "mutate state",
        "delete the key",
        "allow this action",
        "ALLOWED",
        "block the request",
        "BLOCKED",
        "blocking this",
        "cancel everything",
    ] {
        assert!(message_contains_executable_instruction(bad), "{bad} should be forbidden");
    }
    // Operator tracing line: JEV ADVISORY label, escalation signal,
    // confidence, informational-only marker, no authority verb.
    let line = format_advisory_line(&m, "ADVISORY_ESCALATION", Some(0.94));
    assert!(line.starts_with("JEV ADVISORY: "), "line must carry JEV ADVISORY label: {line}");
    assert!(line.contains("escalation signal detected"), "line must carry escalation signal: {line}");
    assert!(line.contains("0.94"), "line must carry confidence: {line}");
    assert!(line.contains("Informational only."), "line must be marked informational-only: {line}");
    assert!(!message_contains_executable_instruction(&line), "operator line leaked authority verb: {line}");
    // The exact UX shape from the integration spec.
    assert_eq!(
        line,
        "JEV ADVISORY: Jev advisory: escalation signal detected, confidence 0.94. Informational only. state=ADVISORY_ESCALATION confidence=Some(0.94)"
    );
}

#[test]
fn v3_questions_and_baseline_pins_hold() {
    let _guard = env_lock();
    assert_eq!(questions::QUESTION_SET_VERSION_V3, "v3");
    assert_eq!(codebro_jev_shadow::advisory::ADVISORY_QUESTION_SET_VERSION, "v3");
    assert_eq!(codebro_jev_shadow::advisory::ADVISORY_QUESTION_ID, "escalation");
    assert!((ADVISORY_CONFIDENCE_THRESHOLD - 0.80).abs() < f64::EPSILON);
    assert!(questions::V3_ESCALATION_INSTRUCTIONS.contains("Skill-approve actions ALWAYS"));
    assert!(change_control::question_set_matches_lock());
    assert!(change_control::verify_baseline());
}

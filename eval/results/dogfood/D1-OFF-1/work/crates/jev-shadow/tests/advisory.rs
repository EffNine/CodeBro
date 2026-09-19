//! Phase-8 focused tests: limited advisory shadow rollout.
//!
//! Covers the required matrix:
//! - both flags OFF → zero calls
//! - shadow only → no advisory
//! - advisory ON without shadow → no advisory
//! - confidence <0.80 → no advisory message
//! - confidence >=0.80 escalation → advisory event only (informational)
//! - non-escalation question → no advisory
//! - Jev timeout → no authority impact
//! - Jev 401/403/422/429/529/5xx → no authority impact
//! - malformed response → no authority impact
//! - secret redaction
//! - advisory cannot invoke tools (message is informational-only)
//! - advisory cannot mutate policy (deterministic preserved)
//! - advisory cannot alter execution (state hash / inputs untouched)
//! - rollback flag disables advisory
//!
//! Env vars are process-global: all tests serialize on `ENV_LOCK`.
//! No test prints `TYPESAFE_API_KEY`.

use codebro_jev_shadow::{
    adapter::JevClient,
    advisory::{
        advisory_message_for, message_contains_executable_instruction, AdvisoryState,
        ADVISORY_CONFIDENCE_THRESHOLD,
    },
    config::{JevShadowConfig, DEFAULT_MODEL},
    logging::scan_text_for_secret,
    questions::{self, state_escalation},
    shadow::{DeterministicVerdict, ShadowObserver},
    types::RequestStatus,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());
/// Lock helper that tolerates poisoning (a panicking test must not cascade
/// into unrelated tests via a poisoned env lock).
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}
const FAKE_KEY: &str = "test-key-value-for-advisory-tests-only";

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
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let mut content_len = 0usize;
                let mut header_end = None;
                for _ in 0..50 {
                    match tokio::time::timeout(Duration::from_secs(2), sock.read(&mut tmp)).await {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                                header_end = Some(pos + 4);
                                let head = String::from_utf8_lossy(&buf[..pos + 4]).to_lowercase();
                                for line in head.lines() {
                                    if let Some(v) = line.strip_prefix("content-length:") {
                                        content_len = v.trim().parse().unwrap_or(0);
                                    }
                                }
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                if let Some(end) = header_end {
                    let mut have = buf.len().saturating_sub(end);
                    while have < content_len {
                        match tokio::time::timeout(Duration::from_secs(2), sock.read(&mut tmp)).await {
                            Ok(Ok(0)) => break,
                            Ok(Ok(n)) => have += n,
                            _ => break,
                        }
                    }
                }
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
                    529 => "Overloaded",
                    500 => "Internal Server Error",
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
    MockServer { url: format!("http://{addr}/v1/systemone"), hits }
}

fn ok_escalation_body(p: f64) -> String {
    serde_json::json!({
        "model": DEFAULT_MODEL,
        "answers": {"escalation": {"type": "noul", "noul": p}},
        "usage": {"input_tokens": 600, "output_tokens": 10}
    })
    .to_string()
}

fn cfg_for(url: &str) -> JevShadowConfig {
    JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: url.to_string(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    }
}

fn esc_state() -> serde_json::Value {
    state_escalation("delete", "delete_memory key=phase8:finding confirm=true")
}

fn esc_questions_v3() -> serde_json::Value {
    questions::wire_questions_for_version("v3", "escalation").unwrap()
}

fn with_key<T>(f: impl FnOnce() -> T) -> T {
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = f();
    std::env::remove_var("TYPESAFE_API_KEY");
    out
}

// ── flag matrix ──────────────────────────────────────────────────────

#[test]
fn both_flags_default_off() {
    let _guard = env_lock();
    let cfg = JevShadowConfig::default();
    assert!(!cfg.enabled, "JEV_SHADOW_ENABLED MUST default false");
    assert!(!cfg.advisory_enabled, "JEV_ADVISORY_ENABLED MUST default false");
    assert!(!cfg.is_live());
    assert!(!cfg.is_advisory_live());
    std::env::remove_var("JEV_SHADOW_ENABLED");
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    let env_cfg = JevShadowConfig::from_env();
    assert!(!env_cfg.enabled);
    assert!(!env_cfg.advisory_enabled);
    assert!(!env_cfg.is_live());
    assert!(!env_cfg.is_advisory_live());
}

#[test]
fn advisory_flag_parses_truthy_forms_but_never_without_shadow() {
    let _guard = env_lock();
    for v in ["1", "true", "TRUE", "yes", "on", " On "] {
        std::env::set_var("JEV_ADVISORY_ENABLED", v);
        std::env::remove_var("JEV_SHADOW_ENABLED");
        let cfg = JevShadowConfig::from_env();
        assert!(cfg.advisory_enabled, "{v} should enable advisory flag");
        // Advisory ON without shadow MUST NOT activate advisory behavior.
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        assert!(!cfg.is_advisory_live(), "advisory without shadow must not be live");
        std::env::remove_var("TYPESAFE_API_KEY");
    }
    for v in ["0", "false", "", "off", "nope"] {
        std::env::set_var("JEV_ADVISORY_ENABLED", v);
        assert!(!JevShadowConfig::from_env().advisory_enabled, "{v} must not enable");
    }
    std::env::remove_var("JEV_ADVISORY_ENABLED");
    std::env::remove_var("JEV_SHADOW_ENABLED");
}

#[tokio::test]
async fn both_flags_off_zero_calls() {
    let _guard = env_lock();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let cfg = JevShadowConfig {
        enabled: false,
        advisory_enabled: false,
        endpoint: server.url.clone(),
        ..JevShadowConfig::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    assert!(!obs.is_live());
    assert!(!obs.is_advisory_live());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let rec = obs
        .observe_versioned("v3", "escalation", esc_state(), DeterministicVerdict::Bool(true), None)
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(rec.is_none());
    assert_eq!(server.hits.load(Ordering::SeqCst), 0, "both OFF must make zero calls");
}

#[tokio::test]
async fn shadow_only_produces_no_advisory() {
    let _guard = env_lock();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.97),
        delay_ms: 0,
    }])
    .await;
    // SHADOW ON + ADVISORY OFF: shadow call happens, advisory stays None.
    // (Flag-shape assertion needs no key; liveness is checked with key set.)
    let dir = tempfile::tempdir().unwrap();
    let cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: false,
        endpoint: server.url.clone(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: Some(dir.path().join("shadow.jsonl")),
        advisory_log_path_override: Some(dir.path().join("advisory.jsonl")),
    };
    assert!(cfg.enabled);
    assert!(!cfg.advisory_enabled);
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    assert!(obs.is_live());
    assert!(!obs.is_advisory_live());
    let (record, event) = obs
        .observe_escalation_with_advisory(
            "v3",
            esc_state(),
            DeterministicVerdict::Bool(true),
            None,
            "test:shadow-only",
        )
        .await
        .expect("shadow live must log");
    std::env::remove_var("TYPESAFE_API_KEY");
    assert_eq!(record.question_id, "escalation");
    assert!(event.is_none(), "shadow-only must produce no advisory event");
    assert_eq!(server.hits.load(Ordering::SeqCst), 1, "shadow call itself happens");
    assert!(!dir.path().join("advisory.jsonl").exists(), "no advisory file when advisory off");
}

#[tokio::test]
async fn advisory_on_without_shadow_produces_no_advisory_and_zero_calls() {
    let _guard = env_lock();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = JevShadowConfig {
        enabled: false,
        advisory_enabled: true,
        endpoint: server.url.clone(),
        ..JevShadowConfig::default()
    };
    assert!(!cfg.is_live());
    assert!(!cfg.is_advisory_live(), "advisory without shadow must not be live");
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = obs
        .observe_escalation_with_advisory(
            "v3",
            esc_state(),
            DeterministicVerdict::Bool(true),
            None,
            "test:advisory-without-shadow",
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(out.is_none());
    assert_eq!(server.hits.load(Ordering::SeqCst), 0, "must not bypass shadow boundary");
}

// ── advisory rule ────────────────────────────────────────────────────

#[tokio::test]
async fn low_confidence_escalation_produces_no_advisory_message() {
    let _guard = env_lock();
    // p=0.70 -> conf 0.40 < 0.80: UNCERTAIN event, NO message.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.70),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = cfg_for(&server.url);
    cfg.log_path_override = Some(dir.path().join("shadow.jsonl"));
    cfg.advisory_log_path_override = Some(dir.path().join("advisory.jsonl"));
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let (_record, event) = obs
        .observe_escalation_with_advisory(
            "v3",
            esc_state(),
            DeterministicVerdict::Bool(true),
            None,
            "test:low-conf",
        )
        .await
        .unwrap();
    std::env::remove_var("TYPESAFE_API_KEY");
    match event {
        None => {} // shadow-only is also acceptable: key point is NO message
        Some(ev) => {
            assert_ne!(ev.advisory_state, "ADVISORY_ESCALATION");
            assert!(ev.advisory_message.is_none(), "low-conf must produce no message");
        }
    }
}

#[tokio::test]
async fn high_confidence_escalation_produces_advisory_event_only() {
    let _guard = env_lock();
    // p=0.97 -> conf 0.94 >= 0.80, Jev-yes: ADVISORY_ESCALATION + message.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = cfg_for(&server.url);
    cfg.log_path_override = Some(dir.path().join("shadow.jsonl"));
    cfg.advisory_log_path_override = Some(dir.path().join("advisory.jsonl"));
    let state = esc_state();
    let state_before = serde_json::to_string(&state).unwrap();
    let det = DeterministicVerdict::Bool(true);
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let (record, event) = obs
        .observe_escalation_with_advisory("v3", state.clone(), det, None, "test:high-conf")
        .await
        .unwrap();
    std::env::remove_var("TYPESAFE_API_KEY");
    // Advisory event present, informational only.
    let ev = event.expect("high-conf escalation must produce advisory event");
    assert_eq!(ev.advisory_state, "ADVISORY_ESCALATION");
    let msg = ev.advisory_message.clone().expect("escalation must carry message");
    assert!(msg.starts_with("Jev advisory:"));
    assert!(!message_contains_executable_instruction(&msg), "message must be informational-only: {msg}");
    assert!(ev.confidence.unwrap() >= ADVISORY_CONFIDENCE_THRESHOLD);
    assert_eq!(ev.question_id, "escalation");
    assert_eq!(ev.question_set_version, "v3");
    // Advisory cannot alter execution: deterministic + state preserved.
    assert_eq!(record.deterministic["verdict_bool"], true);
    assert_eq!(serde_json::to_string(&record.state).unwrap(), state_before);
    assert_eq!(record.question_set_version, "v3");
    // Advisory cannot mutate policy: deterministic verdict object in the
    // advisory event matches the input verdict.
    assert_eq!(ev.deterministic["verdict_bool"], true);
    // Structured fields present.
    assert!(!ev.timestamp.is_empty());
    assert!(!ev.model_resolved.is_empty());
    assert!(!ev.provenance.is_empty());
    assert_eq!(ev.provenance, "test:high-conf");
    // Logged to the advisory file (informational surface) and shadow file.
    let adv_text = std::fs::read_to_string(dir.path().join("advisory.jsonl")).unwrap();
    assert!(adv_text.contains("ADVISORY_ESCALATION"));
    let shadow_text = std::fs::read_to_string(dir.path().join("shadow.jsonl")).unwrap();
    assert!(shadow_text.contains("escalation"));
}

#[tokio::test]
async fn high_confidence_escalation_no_produces_not_triggered_without_message() {
    let _guard = env_lock();
    // p=0.03 -> conf 0.94, Jev-no: ADVISORY_NOT_TRIGGERED, NO message.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.03),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = cfg_for(&server.url);
    cfg.log_path_override = Some(dir.path().join("shadow.jsonl"));
    cfg.advisory_log_path_override = Some(dir.path().join("advisory.jsonl"));
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let (_record, event) = obs
        .observe_escalation_with_advisory(
            "v3",
            state_escalation("read", "engineering_facts query=ChangeEngine kind=symbol"),
            DeterministicVerdict::Bool(false),
            None,
            "test:not-triggered",
        )
        .await
        .unwrap();
    std::env::remove_var("TYPESAFE_API_KEY");
    let ev = event.expect("high-conf Jev-no must still log NOT_TRIGGERED for monitoring");
    assert_eq!(ev.advisory_state, "ADVISORY_NOT_TRIGGERED");
    assert!(ev.advisory_message.is_none(), "NOT_TRIGGERED must produce no human message");
}

#[tokio::test]
async fn non_escalation_question_produces_no_advisory() {
    let _guard = env_lock();
    // Direct pure projection: non-escalation calls never produce advisory,
    // even at high confidence.
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(cfg_for("http://127.0.0.1:9/unreachable"), dir.path().to_path_buf());
    let client_call = codebro_jev_shadow::adapter::JevCall {
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
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let ev = obs.project_advisory(
        "v3",
        "routing",
        &client_call,
        &DeterministicVerdict::Label("debug".to_string()),
        None,
        "test:non-escalation",
    );
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev.is_none(), "non-escalation must stay shadow-only");
}

// ── failure isolation: Jev failure cannot affect CodeBro execution ───

#[tokio::test]
async fn jev_timeout_has_no_authority_impact() {
    let _guard = env_lock();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.97),
        delay_ms: 3000,
    }])
    .await;
    let mut cfg = cfg_for(&server.url);
    cfg.timeout = Duration::from_millis(300);
    let client = JevClient::new(&cfg);
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let call = client.evaluate(&esc_state(), &esc_questions_v3()).await;
    std::env::remove_var("TYPESAFE_API_KEY");
    assert_eq!(call.status, RequestStatus::Timeout);
    // Pure advisory projection on a timed-out call: no event, no message.
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let ev = obs.project_advisory(
        "v3",
        "escalation",
        &call,
        &DeterministicVerdict::Bool(true),
        None,
        "test:timeout",
    );
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev.is_none(), "timeout must not produce advisory output");
}

#[tokio::test]
async fn jev_http_errors_have_no_authority_impact() {
    let _guard = env_lock();
    for (status, expect) in [
        (401u16, RequestStatus::Http401),
        (403, RequestStatus::Http403),
        (422, RequestStatus::Http422),
        (500, RequestStatus::Http5xx),
        (503, RequestStatus::Http5xx),
    ] {
        let server = start_mock(vec![MockResponse {
            status,
            body: r#"{"error":"x"}"#.to_string(),
            delay_ms: 0,
        }])
        .await;
        let client = JevClient::new(&cfg_for(&server.url));
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let call = client.evaluate(&esc_state(), &esc_questions_v3()).await;
        std::env::remove_var("TYPESAFE_API_KEY");
        assert_eq!(call.status, expect, "http {status}");
        let dir = tempfile::tempdir().unwrap();
        let obs = ShadowObserver::new(cfg_for("http://127.0.0.1:9/unused"), dir.path().to_path_buf());
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let ev = obs.project_advisory(
            "v3",
            "escalation",
            &call,
            &DeterministicVerdict::Bool(true),
            None,
            "test:http-error",
        );
        std::env::remove_var("TYPESAFE_API_KEY");
        assert!(ev.is_none(), "http {status} must not produce advisory output");
    }
}

#[tokio::test]
async fn jev_429_529_have_no_authority_impact_and_retry_bounded() {
    let _guard = env_lock();
    for status in [429u16, 529] {
        let server = start_mock(vec![MockResponse {
            status,
            body: r#"{"error":"busy"}"#.to_string(),
            delay_ms: 0,
        }])
        .await;
        let client = JevClient::new(&cfg_for(&server.url));
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let call = client.evaluate(&esc_state(), &esc_questions_v3()).await;
        std::env::remove_var("TYPESAFE_API_KEY");
        assert!(
            call.status == RequestStatus::Http429 || call.status == RequestStatus::Http529,
            "http {status}"
        );
        assert_eq!(server.hits.load(Ordering::SeqCst), 2, "at most one bounded retry");
    }
}

#[tokio::test]
async fn jev_malformed_has_no_authority_impact() {
    let _guard = env_lock();
    for body in [
        "not json".to_string(),
        r#"{"model":"x"}"#.to_string(),
        r#"{"model":"x","answers":{"escalation":{"type":"noul"}}}}"#.to_string(),
    ] {
        let server = start_mock(vec![MockResponse { status: 200, body, delay_ms: 0 }]).await;
        let client = JevClient::new(&cfg_for(&server.url));
        std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
        let call = client.evaluate(&esc_state(), &esc_questions_v3()).await;
        std::env::remove_var("TYPESAFE_API_KEY");
        assert_eq!(call.status, RequestStatus::Malformed);
    }
}

// ── secrecy / authority pins ─────────────────────────────────────────

#[test]
fn advisory_state_builders_and_logs_redact_secrets() {
    let _guard = env_lock();
    let secret = "sk-testsecret-ABCDEFGHIJKLMNOP-123456";
    let summary = format!("delete_memory key=x confirm=true Bearer {secret}");
    let state = state_escalation("delete", &summary);
    let serialized = serde_json::to_string(&state).unwrap();
    assert!(!scan_text_for_secret(&serialized, secret), "secret leaked into advisory state");
    assert!(serialized.contains("REDACTED"));
}

#[test]
fn advisory_message_never_carries_executable_instruction() {
    let _guard = env_lock();
    // The only message the module can ever surface.
    let m = advisory_message_for(AdvisoryState::AdvisoryEscalation, 0.94).unwrap();
    assert_eq!(m, "Jev advisory: escalation signal detected, confidence 0.94.");
    assert!(!message_contains_executable_instruction(&m));
    // No other state surfaces a message.
    assert_eq!(advisory_message_for(AdvisoryState::AdvisoryNotTriggered, 0.99), None);
    assert_eq!(advisory_message_for(AdvisoryState::Uncertain, 0.4), None);
    assert_eq!(advisory_message_for(AdvisoryState::Unavailable, 0.0), None);
}

#[test]
fn advisory_module_has_no_tool_or_execution_imports() {
    // Structural pin: the advisory source must not grow an execute surface.
    // Match import paths / API names (not prose words like "sandbox" that
    // appear in safety comments).
    let src = include_str!("../src/advisory.rs");
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
        assert!(!src.contains(banned), "advisory.rs must not contain {banned}");
    }
    // And the shadow observer's advisory path must stay detached/log-only:
    // no approval/deny/execute verbs as function names in advisory.rs.
    for banned_fn in ["fn approve", "fn deny", "fn execute", "fn retry"] {
        assert!(!src.contains(banned_fn), "advisory.rs must not define {banned_fn}");
    }
}

#[tokio::test]
async fn rollback_flag_disables_advisory() {
    let _guard = env_lock();
    // Rollback means JEV_ADVISORY_ENABLED=false → return to shadow-only.
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_escalation_body(0.97),
        delay_ms: 0,
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let live_cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: true,
        endpoint: server.url.clone(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: Some(dir.path().join("shadow.jsonl")),
        advisory_log_path_override: Some(dir.path().join("advisory.jsonl")),
    };
    let obs_live = ShadowObserver::new(live_cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let (_, ev_live) = obs_live
        .observe_escalation_with_advisory(
            "v3",
            esc_state(),
            DeterministicVerdict::Bool(true),
            None,
            "test:rollback-live",
        )
        .await
        .unwrap();
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev_live.is_some(), "sanity: live advisory produces event");

    // Rollback: same shadow config with advisory disabled.
    let rolled_cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: false,
        endpoint: server.url.clone(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_secs(2),
        max_retries: 1,
        log_path_override: Some(dir.path().join("shadow.jsonl")),
        advisory_log_path_override: Some(dir.path().join("advisory2.jsonl")),
    };
    let obs_rolled = ShadowObserver::new(rolled_cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let (_, ev_rolled) = obs_rolled
        .observe_escalation_with_advisory(
            "v3",
            esc_state(),
            DeterministicVerdict::Bool(true),
            None,
            "test:rollback-off",
        )
        .await
        .unwrap();
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(ev_rolled.is_none(), "rollback must return to shadow-only");
}

#[test]
fn model_remains_pinned_for_rollout() {
    let _guard = env_lock();
    assert_eq!(DEFAULT_MODEL, "jev-1.13.0", "rollout model must remain jev-1.13.0");
    assert_eq!(JevShadowConfig::default().model, "jev-1.13.0");
}

#[test]
fn v3_is_frozen_and_untuned() {
    let _guard = env_lock();
    assert_eq!(questions::QUESTION_SET_VERSION_V3, "v3");
    assert!(questions::V3_ESCALATION_INSTRUCTIONS.contains("Skill-approve actions ALWAYS"));
    assert!(questions::V3_ESCALATION_TRUE.contains("any skill-approve action"));
    assert!(questions::V3_ESCALATION_FALSE.contains("ONLY after verified passing evidence"));
    // Non-escalation v3 == v2 byte-identical (no tuning outside escalation).
    for qid in ["tool_selection", "shell_risk", "routing", "test_classification"] {
        let w2 = questions::wire_questions_for_version("v2", qid).unwrap();
        let w3 = questions::wire_questions_for_version("v3", qid).unwrap();
        assert_eq!(serde_json::to_string(&w2).unwrap(), serde_json::to_string(&w3).unwrap());
    }
}

#[test]
fn with_key_helper_keeps_hygiene() {
    let _guard = env_lock();
    with_key(|| {
        assert_eq!(std::env::var("TYPESAFE_API_KEY").unwrap(), FAKE_KEY);
    });
    assert!(std::env::var("TYPESAFE_API_KEY").is_err());
}

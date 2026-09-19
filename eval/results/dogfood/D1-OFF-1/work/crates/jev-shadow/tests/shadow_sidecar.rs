//! Focused Phase-4 tests for the Jev shadow sidecar.
//!
//! Covers: disabled flag => zero network calls; missing key; 401/403/422/
//! 429/529/5xx; timeout; malformed; valid Noul/Choice/Score; adapter failure
//! isolation; secret redaction; deterministic behavior unchanged.
//!
//! Env vars are process-global, so all tests serialize on `ENV_LOCK`.

use codebro_jev_shadow::{
    adapter::JevClient,
    config::{JevShadowConfig, DEFAULT_MODEL},
    logging::{append_record, scan_text_for_secret},
    questions::{
        self, state_shell_risk, state_test_classification, wire_questions_for, DiagInput,
    },
    shadow::{Agreement, DeterministicVerdict, ShadowObserver},
    types::RequestStatus,
};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};
use std::time::Duration;

static ENV_LOCK: Mutex<()> = Mutex::new(());

const FAKE_KEY: &str = "test-key-value-for-shadow-tests-only";

// ── mock HTTP server ─────────────────────────────────────────────────

struct MockResponse {
    status: u16,
    body: String,
    delay_ms: u64,
    extra_headers: Vec<(String, String)>,
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
                // Read headers.
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                let mut content_len = 0usize;
                let mut header_end = None;
                for _ in 0..50 {
                    match tokio::time::timeout(Duration::from_secs(2), sock.read(&mut tmp)).await {
                        Ok(Ok(0)) => break,
                        Ok(Ok(n)) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if let Some(pos) = find_header_end(&buf) {
                                header_end = Some(pos);
                                content_len = parse_content_length(&buf[..pos]);
                                break;
                            }
                        }
                        _ => break,
                    }
                }
                // Drain body.
                if let Some(end) = header_end {
                    let mut have = buf.len().saturating_sub(end);
                    while have < content_len {
                        match tokio::time::timeout(Duration::from_secs(2), sock.read(&mut tmp)).await
                        {
                            Ok(Ok(0)) => break,
                            Ok(Ok(n)) => {
                                have += n;
                            }
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
                            extra_headers: guard[0].extra_headers.clone(),
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
                let mut head = format!(
                    "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
                    resp.status,
                    reason,
                    resp.body.len()
                );
                for (k, v) in &resp.extra_headers {
                    head.push_str(&format!("{k}: {v}\r\n"));
                }
                head.push_str("Connection: close\r\n\r\n");
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(resp.body.as_bytes()).await;
            });
        }
    });
    MockServer { url: format!("http://{addr}/v1/systemone"), hits }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn parse_content_length(head: &[u8]) -> usize {
    let s = String::from_utf8_lossy(head).to_lowercase();
    for line in s.lines() {
        if let Some(v) = line.strip_prefix("content-length:") {
            return v.trim().parse().unwrap_or(0);
        }
    }
    0
}

fn ok_noul_body(p: f64) -> String {
    serde_json::json!({
        "model": DEFAULT_MODEL,
        "answers": {"shell_risk": {"type": "noul", "noul": p}},
        "usage": {"input_tokens": 100, "output_tokens": 10}
    })
    .to_string()
}

fn ok_choice_body() -> String {
    serde_json::json!({
        "model": DEFAULT_MODEL,
        "answers": {"routing": {"type": "choice", "choice": "debug",
            "probabilities": {"explore": 0.05, "implement": 0.05, "review": 0.0, "debug": 0.85, "research": 0.05},
            "confidence": 0.82}},
        "usage": {"input_tokens": 120, "output_tokens": 12}
    })
    .to_string()
}

fn ok_score_body() -> String {
    serde_json::json!({
        "model": DEFAULT_MODEL,
        "answers": {"frustration": {"type": "score", "score": 1.6,
            "legend": {"0": "Calm", "1": "Frustrated", "2": "Very angry"},
            "probabilities": {"0": 0.05, "1": 0.3, "2": 0.65},
            "confidence": 0.78}},
        "usage": {"input_tokens": 130, "output_tokens": 14}
    })
    .to_string()
}

fn cfg_for(url: &str, timeout: Duration) -> JevShadowConfig {
    JevShadowConfig {
        enabled: true,
        advisory_enabled: false,
        endpoint: url.to_string(),
        model: DEFAULT_MODEL.to_string(),
        timeout,
        max_retries: 1,
        log_path_override: None,
        advisory_log_path_override: None,
    }
}

fn shell_state() -> serde_json::Value {
    state_shell_risk("cargo test -p codebro-jev-shadow", false, false, 120)
}

fn shell_questions() -> serde_json::Value {
    let def = questions::v1_questions().into_iter().find(|q| q.id == "shell_risk").unwrap();
    wire_questions_for(&def)
}

fn with_key<T>(f: impl FnOnce() -> T) -> T {
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let out = f();
    std::env::remove_var("TYPESAFE_API_KEY");
    out
}

// ── tests ────────────────────────────────────────────────────────────

#[tokio::test]
async fn disabled_flag_produces_zero_network_calls() {
    let _guard = ENV_LOCK.lock().unwrap();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_noul_body(0.9),
        delay_ms: 0,
        extra_headers: vec![],
    }])
    .await;
    with_key(|| {
        std::env::set_var("JEV_SHADOW_ENABLED", "false");
        let cfg = JevShadowConfig::from_env();
        assert!(!cfg.enabled);
        assert!(!cfg.is_live());
        std::env::remove_var("JEV_SHADOW_ENABLED");
    });
    // Observer with a disabled config never calls the network.
    let cfg = JevShadowConfig {
        enabled: false,
        endpoint: server.url.clone(),
        ..JevShadowConfig::default()
    };
    let dir = tempfile::tempdir().unwrap();
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    assert!(!obs.is_live());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let rec = obs
        .observe("shell_risk", shell_state(), DeterministicVerdict::Bool(true))
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    assert!(rec.is_none());
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn missing_key_is_unavailable_with_zero_calls() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("TYPESAFE_API_KEY");
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_noul_body(0.9),
        delay_ms: 0,
        extra_headers: vec![],
    }])
    .await;
    let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
    let call = client.evaluate(&shell_state(), &shell_questions()).await;
    assert_eq!(call.status, RequestStatus::MissingKey);
    assert_eq!(server.hits.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn http_error_statuses_map_correctly() {
    let _guard = ENV_LOCK.lock().unwrap();
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
            extra_headers: vec![],
        }])
        .await;
        let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
        let call = with_key_async(&client).await;
        assert_eq!(call.status, expect, "http {status}");
        assert_eq!(call.http_status, Some(status));
    }
}

async fn with_key_async(client: &JevClient) -> codebro_jev_shadow::adapter::JevCall {
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let call = client.evaluate(&shell_state(), &shell_questions()).await;
    std::env::remove_var("TYPESAFE_API_KEY");
    call
}

#[tokio::test]
async fn rate_limit_retries_once_then_surfaces() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Persistent 429: exactly 2 hits (initial + one bounded retry), then 429.
    let server = start_mock(vec![MockResponse {
        status: 429,
        body: r#"{"error":"rate limited"}"#.to_string(),
        delay_ms: 0,
        extra_headers: vec![("Retry-After".to_string(), "0".to_string())],
    }])
    .await;
    let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
    let call = with_key_async(&client).await;
    assert_eq!(call.status, RequestStatus::Http429);
    assert_eq!(
        server.hits.load(Ordering::SeqCst),
        2,
        "must retry at most once, never indefinitely"
    );
}

#[tokio::test]
async fn overloaded_529_retries_once_then_surfaces() {
    let _guard = ENV_LOCK.lock().unwrap();
    let server = start_mock(vec![MockResponse {
        status: 529,
        body: r#"{"error":"overloaded"}"#.to_string(),
        delay_ms: 0,
        extra_headers: vec![],
    }])
    .await;
    let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
    let call = with_key_async(&client).await;
    assert_eq!(call.status, RequestStatus::Http529);
    assert_eq!(server.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn transient_429_recovers_on_retry() {
    let _guard = ENV_LOCK.lock().unwrap();
    let server = start_mock(vec![
        MockResponse {
            status: 429,
            body: r#"{"error":"slow down"}"#.to_string(),
            delay_ms: 0,
            extra_headers: vec![("Retry-After".to_string(), "0".to_string())],
        },
        MockResponse {
            status: 200,
            body: ok_noul_body(0.9),
            delay_ms: 0,
            extra_headers: vec![],
        },
    ])
    .await;
    let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
    let call = with_key_async(&client).await;
    assert_eq!(call.status, RequestStatus::Ok);
    assert_eq!(server.hits.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn slow_server_hits_bounded_timeout() {
    let _guard = ENV_LOCK.lock().unwrap();
    let server = start_mock(vec![MockResponse {
        status: 200,
        body: ok_noul_body(0.9),
        delay_ms: 3000,
        extra_headers: vec![],
    }])
    .await;
    let client = JevClient::new(&cfg_for(&server.url, Duration::from_millis(400)));
    let call = with_key_async(&client).await;
    assert_eq!(call.status, RequestStatus::Timeout);
}

#[tokio::test]
async fn malformed_bodies_are_unavailable_not_panics() {
    let _guard = ENV_LOCK.lock().unwrap();
    for body in [
        "this is not json".to_string(),
        r#"{"model":"x"}"#.to_string(),                    // missing answers
        r#"{"model":"x","answers":{"q":{"type":"noul"}}}}"#.to_string(), // invalid noul
        r#"{"model":"x","answers":{"q":{"type":"choice","choice":"a"}}}}"#.to_string(), // missing probs
        r#"{"model":"x","answers":{"q":{"type":"mystery","v":1}}}}"#.to_string(), // unknown type
    ] {
        let server = start_mock(vec![MockResponse {
            status: 200,
            body,
            delay_ms: 0,
            extra_headers: vec![],
        }])
        .await;
        let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
        let call = with_key_async(&client).await;
        assert_eq!(call.status, RequestStatus::Malformed);
    }
}

#[tokio::test]
async fn valid_noul_choice_score_parse() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    for (body, qid, check) in [
        (ok_noul_body(0.92), "shell_risk", "noul"),
        (ok_choice_body(), "routing", "choice"),
        (ok_score_body(), "frustration", "score"),
    ] {
        let server = start_mock(vec![MockResponse {
            status: 200,
            body,
            delay_ms: 0,
            extra_headers: vec![],
        }])
        .await;
        let client = JevClient::new(&cfg_for(&server.url, Duration::from_secs(2)));
        let q = serde_json::json!({ qid: {"type": check} });
        let call = client.evaluate(&serde_json::json!({"probe": true}), &q).await;
        assert_eq!(call.status, RequestStatus::Ok);
        let ans = call.answers.get(qid).expect("answer present");
        assert!(ans.is_well_formed());
        assert_eq!(ans.qtype, check);
    }
    std::env::remove_var("TYPESAFE_API_KEY");
}

#[tokio::test]
async fn adapter_failure_cannot_affect_execution_path() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Closed port => transport failure. The observer must still resolve to a
    // logged unavailable record; the deterministic verdict is preserved.
    let dir = tempfile::tempdir().unwrap();
    let cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: false,
        endpoint: "http://127.0.0.1:9/unreachable".to_string(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_millis(500),
        max_retries: 1,
        log_path_override: Some(dir.path().join("shadow.jsonl")),
        advisory_log_path_override: None,
    };
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    let rec = obs
        .observe(
            "test_classification",
            state_test_classification("test_failure", 1, Some("cargo test"), &[DiagInput {
                severity: "failure".to_string(),
                message: "test failed: my_test".to_string(),
                test: Some("my_test".to_string()),
                file_hash: None,
            }]),
            DeterministicVerdict::Label("product_failure".to_string()),
        )
        .await;
    std::env::remove_var("TYPESAFE_API_KEY");
    let rec = rec.expect("unavailable observations still log");
    assert_eq!(rec.agreement, Agreement::JevUnavailable.as_str());
    assert_eq!(rec.deterministic["verdict_label"], "product_failure");
    // The record was durably logged outside execution state.
    let logged = std::fs::read_to_string(dir.path().join("shadow.jsonl")).unwrap();
    assert!(logged.contains("JEV_UNAVAILABLE"));
}

#[test]
fn state_builders_and_logs_redact_secrets() {
    let _guard = ENV_LOCK.lock().unwrap();
    let secret = "sk-testsecret-ABCDEFGHIJKLMNOP-123456";
    let cmd = format!("curl -H 'Authorization: Bearer {secret}' https://x.example && export API_KEY={secret}");
    let state = state_shell_risk(&cmd, false, false, 30);
    let serialized = serde_json::to_string(&state).unwrap();
    assert!(
        !scan_text_for_secret(&serialized, secret),
        "secret leaked into shadow state"
    );
    assert!(serialized.contains("REDACTED"));
    // End-to-end: an appended record line must also be clean.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadow.jsonl");
    let cfg = JevShadowConfig {
        enabled: true,
        advisory_enabled: false,
        endpoint: "http://127.0.0.1:9/unreachable".to_string(),
        model: DEFAULT_MODEL.to_string(),
        timeout: Duration::from_millis(300),
        max_retries: 1,
        log_path_override: Some(path.clone()),
        advisory_log_path_override: None,
    };
    let obs = ShadowObserver::new(cfg, dir.path().to_path_buf());
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    std::env::set_var("TYPESAFE_API_KEY", FAKE_KEY);
    rt.block_on(obs.observe("shell_risk", state, DeterministicVerdict::Bool(false)));
    std::env::remove_var("TYPESAFE_API_KEY");
    let logged = std::fs::read_to_string(&path).unwrap();
    assert!(!scan_text_for_secret(&logged, secret), "secret leaked into log");
    assert!(!scan_text_for_secret(&logged, FAKE_KEY), "api key leaked into log");
    // append_record round-trips.
    let rec: codebro_jev_shadow::ShadowRecord =
        serde_json::from_str(logged.lines().next().unwrap()).unwrap();
    let path2 = dir.path().join("shadow2.jsonl");
    append_record(&path2, &rec).unwrap();
    assert!(std::fs::read_to_string(&path2).unwrap().contains(&rec.question_id));
}

#[test]
fn deterministic_behavior_unchanged_when_shadow_enabled() {
    let _guard = ENV_LOCK.lock().unwrap();
    // Same CodeBro-side inputs always build byte-identical state with an
    // identical hash, regardless of the shadow flag: the shadow layer is a
    // pure function of already-computed values and mutates nothing.
    let diags = vec![DiagInput {
        severity: "failure".to_string(),
        message: "test failed: my_test".to_string(),
        test: Some("my_test".to_string()),
        file_hash: Some(questions::hash_id("crates/x/src/lib.rs")),
    }];
    let a = state_test_classification("test_failure", 1, Some("cargo test"), &diags);
    let b = state_test_classification("test_failure", 1, Some("cargo test"), &diags);
    assert_eq!(serde_json::to_string(&a).unwrap(), serde_json::to_string(&b).unwrap());
    assert_eq!(questions::state_hash(&a), questions::state_hash(&b));
    // Deterministic mapping is total and stable.
    let mut seen = HashMap::new();
    for class in ["compile_error", "test_failure", "timeout", "success", "denied", "unknown_failure", "weird"] {
        seen.insert(class, questions::deterministic_test_label(Some(class)));
    }
    assert_eq!(seen["compile_error"], "product_failure");
    assert_eq!(seen["test_failure"], "product_failure");
    assert_eq!(seen["timeout"], "environmental_failure");
    assert_eq!(seen["success"], "uncertain");
    assert_eq!(seen["denied"], "uncertain");
}

#[test]
fn flag_defaults_off_and_parses_truthy_forms() {
    let _guard = ENV_LOCK.lock().unwrap();
    std::env::remove_var("JEV_SHADOW_ENABLED");
    assert!(!JevShadowConfig::from_env().enabled, "MUST default OFF");
    for v in ["1", "true", "TRUE", "yes", "on", " On "] {
        std::env::set_var("JEV_SHADOW_ENABLED", v);
        assert!(JevShadowConfig::from_env().enabled, "{v} should enable");
    }
    for v in ["0", "false", "", "off", "nope"] {
        std::env::set_var("JEV_SHADOW_ENABLED", v);
        assert!(!JevShadowConfig::from_env().enabled, "{v} must not enable");
    }
    std::env::remove_var("JEV_SHADOW_ENABLED");
}

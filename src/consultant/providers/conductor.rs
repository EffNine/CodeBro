//! Conductor HTTP consultant provider.
//!
//! Conductor is an OpenAI-compatible AI gateway. This provider sends the
//! consultation request over plain HTTP to Conductor's protected inference
//! endpoint (`POST /v1/chat/completions`) using `Authorization: Bearer
//! <CONDUCTOR_API_KEY>` and returns the normalized OpenAI-style completion
//! through the shared `ConsultantResponse` abstraction.
//!
//! Configuration follows CodeBro conventions:
//! - `CONDUCTOR_API_KEY` env var, falling back to the existing secure
//!   `CredentialStore` (`~/.codebro/credentials.json`, provider id `conductor`).
//! - `CONDUCTOR_BASE_URL` env var (default `http://127.0.0.1:8080` — the
//!   gateway's default listen address).
//! - `CONDUCTOR_MODEL` env var (default `auto` — Conductor runtime
//!   auto-selection when an auto selector is wired, or a route/alias/model id).
//!
//! The API key is never logged, echoed, or included in error messages.
//!
//! Mode mapping (CodeBro consultation mode → Conductor public mode):
//! - `architecture`  → `agentic`   (complex multi-step system design; routes
//!   to Conductor's elite agentic capability)
//! - `debugging`     → `coding`    (code generation/debugging/refactoring)
//! - `code_review`   → `coding`    (code inspection and review)
//! - `planning`      → `planning`  (planning profile)
//! - `research`      → `reasoning` (analysis, comparison, multi-step logic)
//! - `second_opinion`→ `reasoning` (analysis and evaluation)
//!
//! Only Conductor's supported public modes are emitted: `auto`, `coding`,
//! `reasoning`, `vision`, `fast`, `planning`, `agentic`, `long_horizon`.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;

use super::super::prompt::{build_prompt, truncate_answer};
use super::super::provider::{ConsultantError, ConsultantProvider};
use super::super::types::{AuthStatus, ConsultantMode, ConsultantRequest, ConsultantResponse};

/// Timeout for the entire consultation flow, matching the ChatGPT extension
/// provider's convention.
const CONSULTATION_TIMEOUT_SECS: u64 = 180;

/// Maximum length of the extracted answer (matches the ChatGPT providers).
const MAX_ANSWER_LENGTH: usize = 16_000;

/// Environment variables read by `ConductorProvider::new()`.
const ENV_API_KEY: &str = "CONDUCTOR_API_KEY";
const ENV_BASE_URL: &str = "CONDUCTOR_BASE_URL";
const ENV_MODEL: &str = "CONDUCTOR_MODEL";

/// Default base URL: Conductor's default listen address (host `0.0.0.0`,
/// port `8080`).
const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8080";

/// Default model id sent to Conductor. `auto` triggers the gateway's runtime
/// auto-selection when an auto selector is wired; a route name, alias, or
/// provider-prefixed model id also works.
const DEFAULT_MODEL: &str = "auto";

/// The credential-store provider id for the Conductor API key.
const CREDENTIAL_PROVIDER: &str = "conductor";

/// Map a CodeBro consultation mode to a Conductor public mode.
///
/// See the module docs for the full mapping rationale. Only Conductor's
/// supported public modes are returned.
pub fn conductor_mode(mode: &ConsultantMode) -> &'static str {
    match mode {
        ConsultantMode::Architecture => "agentic",
        ConsultantMode::Debugging => "coding",
        ConsultantMode::CodeReview => "coding",
        ConsultantMode::Planning => "planning",
        ConsultantMode::Research => "reasoning",
        ConsultantMode::SecondOpinion => "reasoning",
    }
}

/// Resolve the Conductor API key: `CONDUCTOR_API_KEY` env var first, then the
/// existing secure `CredentialStore` (provider id `conductor`).
fn resolve_api_key() -> Option<String> {
    if let Ok(key) = std::env::var(ENV_API_KEY) {
        if !key.trim().is_empty() {
            return Some(key);
        }
    }
    let mut store = crate::credentials::CredentialStore::new(crate::config::Config::config_dir());
    if store.load().is_ok() {
        if let Some(key) = store.get(CREDENTIAL_PROVIDER) {
            return Some(key.to_string());
        }
    }
    None
}

/// Resolve the Conductor base URL from `CONDUCTOR_BASE_URL` (defaults to the
/// gateway's default listen address).
fn resolve_base_url() -> String {
    std::env::var(ENV_BASE_URL)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_BASE_URL.to_string())
}

/// Resolve the model id sent to Conductor from `CONDUCTOR_MODEL`
/// (defaults to `auto`).
fn resolve_model() -> String {
    std::env::var(ENV_MODEL)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_MODEL.to_string())
}

/// HTTP consultant provider backed by the Conductor gateway.
pub struct ConductorProvider {
    base_url: String,
    model: String,
    api_key: Option<String>,
    client: reqwest::Client,
    timeout: Duration,
}

impl ConductorProvider {
    /// Construct from the environment and the existing credential store.
    pub fn new() -> Self {
        ConductorProvider {
            base_url: resolve_base_url(),
            model: resolve_model(),
            api_key: resolve_api_key(),
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(CONSULTATION_TIMEOUT_SECS),
        }
    }

    /// Construct with explicit configuration (for tests and embedding).
    pub fn with_config(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: Option<String>,
    ) -> Self {
        ConductorProvider {
            base_url: base_url.into(),
            model: model.into(),
            api_key,
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(CONSULTATION_TIMEOUT_SECS),
        }
    }

    /// Override the per-request timeout (for tests).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Whether an API key is configured.
    pub fn is_configured(&self) -> bool {
        self.api_key
            .as_deref()
            .map(|k| !k.trim().is_empty())
            .unwrap_or(false)
    }
}

impl Default for ConductorProvider {
    fn default() -> Self {
        Self::new()
    }
}

/// Debug output redacts the API key — the value must never leak through
/// diagnostics.
impl std::fmt::Debug for ConductorProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConductorProvider")
            .field("base_url", &self.base_url)
            .field("model", &self.model)
            .field("api_key", &self.api_key.as_deref().map(|_| "[REDACTED]"))
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// OpenAI-compatible chat completion response (the subset CodeBro needs).
#[derive(Debug, Deserialize)]
struct ChatCompletion {
    #[serde(default)]
    model: String,
    #[serde(default)]
    choices: Vec<ChatCompletionChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionChoice {
    #[serde(default)]
    message: Option<ChatCompletionMessage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionMessage {
    #[serde(default)]
    content: String,
}

/// Structured error envelope returned by Conductor on failures.
#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    #[serde(default)]
    error: Option<ErrorDetail>,
}

#[derive(Debug, Deserialize)]
struct ErrorDetail {
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    code: Option<String>,
    #[serde(rename = "type", default)]
    error_type: Option<String>,
    #[serde(default)]
    param: Option<String>,
}

impl ErrorEnvelope {
    /// Human-readable message from the error body, with the detail fields
    /// merged in. Never contains the API key (it is the upstream's body).
    fn message(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(detail) = &self.error {
            if let Some(msg) = &detail.message {
                parts.push(msg.clone());
            }
            if let Some(code) = &detail.code {
                parts.push(format!("code: {code}"));
            }
            if let Some(t) = &detail.error_type {
                parts.push(format!("type: {t}"));
            }
            if let Some(param) = &detail.param {
                parts.push(format!("param: {param}"));
            }
        }
        if parts.is_empty() {
            String::new()
        } else {
            parts.join("; ")
        }
    }
}

/// Build the chat completion request body sent to Conductor.
///
/// The `mode` field carries the mapped Conductor public mode; the user message
/// carries the question plus generated context (project context, git diff,
/// files) exactly once via the shared prompt builder.
fn build_body(provider: &ConductorProvider, request: &ConsultantRequest) -> serde_json::Value {
    let conductor_mode = conductor_mode(&request.mode);
    let user_prompt = build_prompt(request);
    let system_prompt = format!(
        "You are CodeBro's consultant operating through the Conductor gateway. \
         Mode: {conductor_mode}. Provide a clear, structured answer. \
         Do not modify the repository."
    );
    serde_json::json!({
        "model": provider.model,
        "mode": conductor_mode,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt },
        ],
        "stream": false,
    })
}

/// Map a reqwest transport failure to a useful consultant error without
/// leaking the API key or prompt contents.
fn map_transport_error(err: reqwest::Error) -> ConsultantError {
    if err.is_timeout() {
        return ConsultantError::Provider(
            "Conductor request timed out; the gateway did not respond in time.".to_string(),
        );
    }
    if err.is_connect() {
        return ConsultantError::Provider(format!("failed to connect to Conductor: {err}"));
    }
    ConsultantError::Provider(format!("Conductor request failed: {err}"))
}

/// Map a non-success HTTP status from Conductor to a consultant error.
fn map_http_error(status: reqwest::StatusCode, body: &str) -> ConsultantError {
    let envelope: ErrorEnvelope =
        serde_json::from_str(body).unwrap_or(ErrorEnvelope { error: None });
    let detail = envelope.message();
    let message = if detail.is_empty() {
        // Fall back to the raw body (truncated) when it is not structured.
        body.chars().take(512).collect::<String>()
    } else {
        detail
    };
    match status {
        reqwest::StatusCode::UNAUTHORIZED | reqwest::StatusCode::FORBIDDEN => {
            ConsultantError::AuthenticationRequired(format!(
                "Conductor rejected the API key (HTTP {status}). \
                 Verify CONDUCTOR_API_KEY and retry. Details: {message}"
            ))
        }
        reqwest::StatusCode::BAD_REQUEST => ConsultantError::Provider(format!(
            "Conductor rejected the request (HTTP 400): {message}"
        )),
        reqwest::StatusCode::NOT_FOUND => ConsultantError::Provider(format!(
            "Conductor could not route the model (HTTP 404): {message}"
        )),
        reqwest::StatusCode::TOO_MANY_REQUESTS => ConsultantError::Provider(format!(
            "Conductor rate limit exceeded (HTTP 429): {message}"
        )),
        s if s.is_server_error() => {
            ConsultantError::Provider(format!("Conductor server error (HTTP {s}): {message}"))
        }
        _ => ConsultantError::Provider(format!(
            "Conductor returned unexpected status HTTP {status}: {message}"
        )),
    }
}

#[async_trait]
impl ConsultantProvider for ConductorProvider {
    fn name(&self) -> &str {
        "conductor"
    }

    async fn consult(
        &self,
        request: &ConsultantRequest,
    ) -> Result<ConsultantResponse, ConsultantError> {
        let Some(api_key) = self
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        else {
            return Err(ConsultantError::AuthenticationRequired(
                self.unauthenticated_hint(),
            ));
        };

        let url = format!(
            "{}/v1/chat/completions",
            self.base_url.trim_end_matches('/')
        );
        let body = build_body(self, request);

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {api_key}"))
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .timeout(self.timeout)
            .send()
            .await
            .map_err(map_transport_error)?;

        let status = response.status();
        let text = response.text().await.map_err(|e| {
            ConsultantError::Provider(format!("failed to read Conductor response: {e}"))
        })?;

        if !status.is_success() {
            return Err(map_http_error(status, &text));
        }

        let completion: ChatCompletion = serde_json::from_str(&text).map_err(|e| {
            ConsultantError::Provider(format!(
                "Conductor returned a malformed response (HTTP 200, invalid JSON: {e})"
            ))
        })?;

        let choice = completion.choices.first().ok_or_else(|| {
            ConsultantError::Provider(
                "Conductor returned a malformed response: no choices in completion".to_string(),
            )
        })?;
        let raw_answer = choice
            .message
            .as_ref()
            .map(|m| m.content.trim().to_string())
            .unwrap_or_default();
        if raw_answer.is_empty() {
            return Err(ConsultantError::Provider(
                "Conductor returned an empty answer".to_string(),
            ));
        }

        // Respect the caller's max length; otherwise apply the provider cap.
        let max_len = if request.max_answer_length > 0 {
            request.max_answer_length.min(MAX_ANSWER_LENGTH)
        } else {
            MAX_ANSWER_LENGTH
        };
        let answer = truncate_answer(&raw_answer, max_len);

        let mut response = ConsultantResponse::simple("conductor", &answer);
        response.model = completion.model;
        let mut metadata = serde_json::Map::new();
        metadata.insert(
            "mode".to_string(),
            serde_json::Value::String(conductor_mode(&request.mode).to_string()),
        );
        response.metadata = metadata;
        Ok(response)
    }

    fn auth_status(&self) -> AuthStatus {
        // Cheap synchronous check, consistent with the existing providers:
        // configuration presence only. An invalid key or an unreachable
        // Conductor surfaces as a structured error during `consult`.
        if self.is_configured() {
            AuthStatus::Authenticated
        } else {
            AuthStatus::Unauthenticated
        }
    }

    fn login_url(&self) -> &str {
        DEFAULT_BASE_URL
    }

    fn unauthenticated_hint(&self) -> String {
        "Conductor is not configured. Set the CONDUCTOR_API_KEY environment variable \
         (or store a key for provider 'conductor' in ~/.codebro/credentials.json) and \
         ensure the Conductor gateway is reachable at CONDUCTOR_BASE_URL \
         (default http://127.0.0.1:8080)."
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::super::super::router::ConsultantRouter;
    use super::super::super::types::{ConsultantFileContext, ConsultantProvider as ProviderChoice};
    use super::*;

    /// A tiny one-shot HTTP mock server. Accepts a single request, calls
    /// `responder(headers, body)` and returns `(status, response_body)`.
    async fn start_mock(
        responder: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock listener");
        let addr = listener.local_addr().expect("mock addr");
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut sock, _) = listener.accept().await.expect("mock accept");
            let mut buf: Vec<u8> = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end;
            loop {
                let n = sock.read(&mut chunk).await.expect("mock read");
                if n == 0 {
                    return;
                }
                buf.extend_from_slice(&chunk[..n]);
                if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = pos + 4;
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&buf[..header_end]).into_owned();
            let content_length = headers
                .lines()
                .find_map(|l| {
                    let lower = l.to_ascii_lowercase();
                    lower
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            while buf.len() < header_end + content_length {
                let n = sock.read(&mut chunk).await.expect("mock body read");
                if n == 0 {
                    break;
                }
                buf.extend_from_slice(&chunk[..n]);
            }
            let body = String::from_utf8_lossy(
                &buf[header_end..(header_end + content_length).min(buf.len())],
            )
            .into_owned();

            let (status, resp_body) = responder(&headers, &body);
            let reason = match status {
                200 => "OK",
                400 => "Bad Request",
                401 => "Unauthorized",
                404 => "Not Found",
                429 => "Too Many Requests",
                500 => "Internal Server Error",
                502 => "Bad Gateway",
                503 => "Service Unavailable",
                _ => "Status",
            };
            let response = format!(
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\n\
                 Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp_body.len(),
                resp_body
            );
            let _ = sock.write_all(response.as_bytes()).await;
        });
        format!("http://{addr}")
    }

    fn openai_success(model: &str, answer: &str) -> String {
        format!(
            r#"{{
                "id": "chatcmpl-test",
                "object": "chat.completion",
                "model": "{model}",
                "choices": [
                    {{ "index": 0, "message": {{ "role": "assistant", "content": "{answer}" }}, "finish_reason": "stop" }}
                ],
                "usage": {{ "prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15 }}
            }}"#
        )
    }

    fn conductor_error(status: u16, message: &str, code: &str) -> String {
        format!(
            r#"{{ "error": {{ "message": "{message}", "type": "invalid_request_error", "code": "{code}" }} }}"#
        )
    }

    #[test]
    fn maps_consultant_modes_to_conductor_modes() {
        assert_eq!(conductor_mode(&ConsultantMode::Architecture), "agentic");
        assert_eq!(conductor_mode(&ConsultantMode::Debugging), "coding");
        assert_eq!(conductor_mode(&ConsultantMode::CodeReview), "coding");
        assert_eq!(conductor_mode(&ConsultantMode::Planning), "planning");
        assert_eq!(conductor_mode(&ConsultantMode::Research), "reasoning");
        assert_eq!(conductor_mode(&ConsultantMode::SecondOpinion), "reasoning");
    }

    #[test]
    fn mode_mapping_only_uses_supported_conductor_modes() {
        let supported = [
            "auto",
            "coding",
            "reasoning",
            "vision",
            "fast",
            "planning",
            "agentic",
            "long_horizon",
        ];
        for mode in [
            ConsultantMode::Architecture,
            ConsultantMode::Debugging,
            ConsultantMode::CodeReview,
            ConsultantMode::Planning,
            ConsultantMode::Research,
            ConsultantMode::SecondOpinion,
        ] {
            assert!(
                supported.contains(&conductor_mode(&mode)),
                "{} is not a supported Conductor mode",
                conductor_mode(&mode)
            );
        }
    }

    #[test]
    fn auth_status_reflects_configuration() {
        let configured =
            ConductorProvider::with_config("http://127.0.0.1:8080", "auto", Some("key".into()));
        assert!(matches!(
            configured.auth_status(),
            AuthStatus::Authenticated
        ));

        let missing = ConductorProvider::with_config("http://127.0.0.1:8080", "auto", None);
        assert!(matches!(missing.auth_status(), AuthStatus::Unauthenticated));

        let blank =
            ConductorProvider::with_config("http://127.0.0.1:8080", "auto", Some("  ".into()));
        assert!(matches!(blank.auth_status(), AuthStatus::Unauthenticated));
    }

    #[tokio::test]
    async fn missing_api_key_returns_auth_required() {
        let provider = ConductorProvider::with_config("http://127.0.0.1:9", "auto", None);
        let request = ConsultantRequest {
            question: "hello".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(
            matches!(err, ConsultantError::AuthenticationRequired(_)),
            "got: {err:?}"
        );
        assert!(err.to_string().contains("CONDUCTOR_API_KEY"));
    }

    #[tokio::test]
    async fn sends_correct_headers_and_endpoint() {
        let captured = Arc::new(std::sync::Mutex::new(None::<(String, String)>));
        let captured_clone = captured.clone();
        let base_url = start_mock(move |headers, body| {
            *captured_clone.lock().unwrap() = Some((headers.to_string(), body.to_string()));
            (200, openai_success("mock-1", "Hello from the mock."))
        })
        .await;

        let provider = ConductorProvider::with_config(
            base_url,
            "mock-1",
            Some("sk-conductor-secret-key-123456".into()),
        );
        let request = ConsultantRequest {
            question: "hi".to_string(),
            mode: ConsultantMode::Research,
            ..Default::default()
        };
        let response = provider.consult(&request).await.expect("consult succeeds");
        assert_eq!(response.provider, "conductor");
        assert_eq!(response.model, "mock-1");
        assert_eq!(response.answer, "Hello from the mock.");

        let (headers, _body) = captured.lock().unwrap().clone().expect("request captured");
        assert!(
            headers.starts_with("POST /v1/chat/completions HTTP/1.1"),
            "endpoint must be /v1/chat/completions, got: {headers}"
        );
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("authorization: bearer sk-conductor-secret-key-123456"),
            "Authorization header missing or wrong: {headers}"
        );
        assert!(
            headers
                .to_ascii_lowercase()
                .contains("content-type: application/json"),
            "Content-Type header missing: {headers}"
        );
    }

    #[tokio::test]
    async fn sends_correct_request_json_with_mode() {
        let captured = Arc::new(std::sync::Mutex::new(None::<String>));
        let captured_clone = captured.clone();
        let base_url = start_mock(move |_headers, body| {
            *captured_clone.lock().unwrap() = Some(body.to_string());
            (200, openai_success("mock-1", "ok"))
        })
        .await;

        let provider =
            ConductorProvider::with_config(base_url, "route-model", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "Should we refactor?".to_string(),
            mode: ConsultantMode::CodeReview,
            ..Default::default()
        };
        provider.consult(&request).await.expect("consult succeeds");

        let body = captured.lock().unwrap().clone().expect("body captured");
        let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
        assert_eq!(value["model"], "route-model");
        assert_eq!(value["mode"], "coding", "CodeReview must map to coding");
        assert_eq!(value["stream"], false);
        let messages = value["messages"].as_array().expect("messages array");
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "system");
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("Mode: coding"));
        assert_eq!(messages[1]["role"], "user");
        assert!(messages[1]["content"]
            .as_str()
            .unwrap()
            .contains("Should we refactor?"));
    }

    #[tokio::test]
    async fn propagates_question_and_context_exactly_once() {
        let captured = Arc::new(std::sync::Mutex::new(None::<String>));
        let captured_clone = captured.clone();
        let base_url = start_mock(move |_headers, body| {
            *captured_clone.lock().unwrap() = Some(body.to_string());
            (200, openai_success("mock-1", "ok"))
        })
        .await;

        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "Review the auth flow.".to_string(),
            context: Some("Project context block.".to_string()),
            files: vec![ConsultantFileContext {
                path: "src/auth.rs".to_string(),
                content: "pub fn auth() {}".to_string(),
            }],
            include_project_context: true,
            include_git_diff: true,
            ..Default::default()
        };
        provider.consult(&request).await.expect("consult succeeds");

        let body = captured.lock().unwrap().clone().expect("body captured");
        let value: serde_json::Value = serde_json::from_str(&body).expect("body is JSON");
        let user_content = value["messages"][1]["content"].as_str().unwrap();

        // Context appears exactly once (in the user message, via build_prompt).
        assert_eq!(user_content.matches("Project context block.").count(), 1);
        // Files appear once each.
        assert_eq!(user_content.matches("src/auth.rs").count(), 1);
        assert_eq!(user_content.matches("pub fn auth()").count(), 1);
        // Question preserved.
        assert!(user_content.contains("Review the auth flow."));
        // The user message carries the full prompt; the system message has no
        // context duplication.
        let system_content = value["messages"][0]["content"].as_str().unwrap();
        assert!(!system_content.contains("Project context block."));
    }

    #[tokio::test]
    async fn parses_openai_response() {
        let base_url = start_mock(|_headers, _body| {
            (
                200,
                openai_success("nvidia_nim/meta/llama-3.1-8b", "The answer is 42."),
            )
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "What is 6x7?".to_string(),
            mode: ConsultantMode::Research,
            ..Default::default()
        };
        let response = provider.consult(&request).await.expect("consult succeeds");
        assert_eq!(response.provider, "conductor");
        assert_eq!(response.model, "nvidia_nim/meta/llama-3.1-8b");
        assert_eq!(response.answer, "The answer is 42.");
        assert_eq!(response.summary, "The answer is 42.");
        assert_eq!(response.metadata["mode"], "reasoning");
    }

    #[tokio::test]
    async fn handles_401_as_auth_error() {
        let base_url = start_mock(|_headers, _body| {
            (
                401,
                conductor_error(401, "invalid api key", "invalid_api_key"),
            )
        })
        .await;
        let provider =
            ConductorProvider::with_config(base_url, "auto", Some("wrong-key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(
            matches!(err, ConsultantError::AuthenticationRequired(_)),
            "got: {err:?}"
        );
        assert!(err.to_string().contains("401"));
    }

    #[tokio::test]
    async fn handles_400_invalid_request_or_mode() {
        let base_url = start_mock(|_headers, _body| {
            (
                400,
                conductor_error(400, "invalid mode \"bogus\"", "invalid_request"),
            )
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("400"));
        assert!(err.to_string().contains("invalid mode"));
    }

    #[tokio::test]
    async fn handles_404_model_not_found() {
        let base_url = start_mock(|_headers, _body| {
            (
                404,
                conductor_error(404, "Model 'bogus' not found", "model_not_found"),
            )
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "bogus", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("404"));
    }

    #[tokio::test]
    async fn handles_429_rate_limit() {
        let base_url = start_mock(|_headers, _body| {
            (
                429,
                conductor_error(429, "rate limit exceeded", "rate_limit_exceeded"),
            )
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("429"));
        assert!(err.to_string().contains("rate limit"));
    }

    #[tokio::test]
    async fn handles_5xx_server_error() {
        for status in [500u16, 502, 503] {
            let base_url = start_mock(move |_headers, _body| {
                (
                    status,
                    conductor_error(status, "upstream exploded", "upstream_error"),
                )
            })
            .await;
            let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
            let request = ConsultantRequest {
                question: "hi".to_string(),
                ..Default::default()
            };
            let err = provider.consult(&request).await.unwrap_err();
            assert!(
                matches!(err, ConsultantError::Provider(_)),
                "status {status}: got {err:?}"
            );
            assert!(err.to_string().contains(&status.to_string()));
        }
    }

    #[tokio::test]
    async fn handles_connection_refused() {
        // Port 1 on loopback: nothing is listening.
        let provider =
            ConductorProvider::with_config("http://127.0.0.1:1", "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("failed to connect"), "got: {err}");
    }

    #[tokio::test]
    async fn handles_timeout() {
        let base_url = start_mock(|_headers, _body| {
            std::thread::sleep(std::time::Duration::from_secs(3));
            (200, openai_success("mock-1", "too late"))
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()))
            .with_timeout(std::time::Duration::from_millis(150));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("timed out"), "got: {err}");
    }

    #[tokio::test]
    async fn handles_malformed_response() {
        let base_url = start_mock(|_headers, _body| (200, "not json at all".to_string())).await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("malformed"), "got: {err}");
    }

    #[tokio::test]
    async fn handles_empty_choices() {
        let base_url = start_mock(|_headers, _body| {
            (
                200,
                r#"{"id":"x","object":"chat.completion","model":"m","choices":[]}"#.to_string(),
            )
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        assert!(matches!(err, ConsultantError::Provider(_)), "got: {err:?}");
        assert!(err.to_string().contains("no choices"), "got: {err}");
    }

    #[tokio::test]
    async fn respects_max_answer_length() {
        let full = "First sentence. Second sentence. Third.";
        let base_url =
            start_mock(move |_headers, _body| (200, openai_success("mock-1", full))).await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some("key-123".into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            max_answer_length: 15,
            ..Default::default()
        };
        let response = provider.consult(&request).await.expect("consult succeeds");
        // Truncated at the first sentence boundary with the truncation marker
        // (the shared truncate_answer convention used by the ChatGPT providers).
        assert!(response.answer.starts_with("First sentence."));
        assert!(response.answer.ends_with("[truncated]"));
        assert!(
            response.answer.len() < full.len(),
            "got: {}",
            response.answer
        );
    }

    #[tokio::test]
    async fn api_key_never_appears_in_errors_or_logs() {
        let secret = "sk-super-secret-conductor-key-99887766554433221100";

        // 401 error path
        let base_url = start_mock(|_headers, _body| {
            (
                401,
                conductor_error(401, "invalid api key", "invalid_api_key"),
            )
        })
        .await;
        let provider = ConductorProvider::with_config(base_url, "auto", Some(secret.into()));
        let request = ConsultantRequest {
            question: "hi".to_string(),
            ..Default::default()
        };
        let err = provider.consult(&request).await.unwrap_err();
        let display = err.to_string();
        assert!(
            !display.contains(secret),
            "error leaked the API key: {display}"
        );

        // Transport error path (connection refused).
        let provider =
            ConductorProvider::with_config("http://127.0.0.1:1", "auto", Some(secret.into()));
        let err = provider.consult(&request).await.unwrap_err();
        let display = err.to_string();
        assert!(
            !display.contains(secret),
            "transport error leaked the API key: {display}"
        );

        // Debug formatting of the provider must not leak the key either.
        let debug = format!("{provider:?}");
        assert!(
            !debug.contains(secret),
            "Debug output leaked the API key: {debug}"
        );
    }

    #[tokio::test]
    async fn provider_is_registered_in_the_router() {
        let mut router = ConsultantRouter::new();
        for p in super::super::default_providers() {
            router.register(p);
        }
        let names = router.registered_providers();
        assert!(names.contains(&"conductor".to_string()), "got: {names:?}");

        let provider = router
            .resolve(&ProviderChoice::Conductor)
            .expect("conductor resolves");
        assert_eq!(provider.name(), "conductor");
    }
}

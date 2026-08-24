//! Model discovery against OpenAI-compatible providers.
//!
//! Discovery always attempts the provider's `GET {base_url}/models`
//! endpoint first. When the endpoint is unavailable or returns nothing
//! usable, providers with a deterministic official catalog fall back to
//! that catalog — clearly labelled `ProviderDefault` so fallback models are
//! never presented as if they were discovered.
//!
//! # Error honesty
//!
//! Errors are classified (auth, balance, not-found, rate limit, provider
//! down) and surfaced with actionable guidance. Raw HTTP bodies are never
//! dumped into the UI; secrets never appear in error text.

use crate::providers::catalog::{
    fallback_catalog, provider_display_name, CatalogModel, ModelMetadata, ModelSource,
};
use anyhow::{Context, Result};
use std::time::Duration;

/// A model known to CodeBro, with its provenance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredModel {
    pub id: String,
    /// How this model became known (discovered / provider default / user).
    pub source: ModelSource,
    /// Metadata actually known about the model (unknowns stay `None`).
    pub metadata: ModelMetadata,
}

/// The outcome of a discovery attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelDiscovery {
    pub models: Vec<DiscoveredModel>,
    /// True when the provider-known fallback catalog was used because
    /// `/models` was unavailable or incomplete.
    pub used_fallback: bool,
    /// The classified error from the `/models` attempt, if any. Kept so
    /// callers can show why the fallback was used.
    pub error: Option<DiscoveryError>,
}

/// A classified model-discovery failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The endpoint could not be reached (DNS, connect, TLS, timeout).
    Network(String),
    /// The endpoint answered with an HTTP status.
    Http(u16),
    /// The endpoint answered but the body was not a usable model list.
    InvalidResponse(String),
    /// The endpoint answered with an empty model list.
    Empty,
}

impl DiscoveryError {
    /// A short, sanitized one-liner. Never contains secrets or raw bodies.
    pub fn human(&self) -> String {
        match self {
            DiscoveryError::Network(msg) => {
                let sanitized = msg.chars().take(120).collect::<String>();
                if sanitized.is_empty() {
                    "network error".to_string()
                } else {
                    sanitized
                }
            }
            DiscoveryError::Http(401) => "HTTP 401 — authentication failed".to_string(),
            DiscoveryError::Http(402) => "HTTP 402 — insufficient provider balance".to_string(),
            DiscoveryError::Http(404) => "HTTP 404 — endpoint/model path not found".to_string(),
            DiscoveryError::Http(429) => "HTTP 429 — rate limited".to_string(),
            DiscoveryError::Http(code) if *code >= 500 && *code < 600 => {
                format!("HTTP {} — provider unavailable", code)
            }
            DiscoveryError::Http(code) => format!("HTTP {}", code),
            DiscoveryError::InvalidResponse(msg) => {
                format!(
                    "invalid response: {}",
                    msg.chars().take(120).collect::<String>()
                )
            }
            DiscoveryError::Empty => "provider returned no models".to_string(),
        }
    }
}

/// Whether a failure is auth-related (an invalid key, not a transport issue).
pub fn is_auth_failure(err: &DiscoveryError) -> bool {
    matches!(err, DiscoveryError::Http(401) | DiscoveryError::Http(403))
}

/// Build an actionable, multi-line error message for a discovery failure.
pub fn actionable_model_error(provider: &str, err: &DiscoveryError) -> String {
    let name = provider_display_name(provider);
    match err {
        DiscoveryError::Http(401) | DiscoveryError::Http(403) => {
            format!(
                "{} model discovery failed\nHTTP {} — authentication failed\nCheck //apikey {}",
                name,
                http_code(err),
                provider
            )
        }
        DiscoveryError::Http(402) => {
            format!(
                "{} model discovery failed\nHTTP 402 — insufficient provider balance",
                name
            )
        }
        DiscoveryError::Http(404) => {
            format!(
                "{} model discovery failed\nHTTP 404 — endpoint/model path not found",
                name
            )
        }
        DiscoveryError::Http(429) => {
            format!("{} model discovery failed\nHTTP 429 — rate limited", name)
        }
        DiscoveryError::Http(code) if *code >= 500 && *code < 600 => {
            format!(
                "{} model discovery failed\nHTTP {} — provider unavailable",
                name, code
            )
        }
        _ => {
            format!(
                "{} model discovery failed\nModel endpoint unavailable",
                name
            )
        }
    }
}

fn http_code(err: &DiscoveryError) -> u16 {
    match err {
        DiscoveryError::Http(c) => *c,
        DiscoveryError::Network(_) => 0,
        DiscoveryError::InvalidResponse(_) => 0,
        DiscoveryError::Empty => 0,
    }
}

/// Fetches the model ids advertised by an OpenAI-compatible provider via
/// `GET {base_url}/models`. Classifies HTTP failures instead of returning
/// opaque anyhow errors.
pub async fn fetch_models_raw(
    base_url: &str,
    api_key: Option<&str>,
) -> Result<Vec<String>, DiscoveryError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| DiscoveryError::Network(e.to_string()))?;

    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let mut req = client.get(&url);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }

    let res = req
        .send()
        .await
        .map_err(|e| DiscoveryError::Network(e.to_string()))?;

    let status = res.status();
    if !status.is_success() {
        return Err(DiscoveryError::Http(status.as_u16()));
    }

    let json: serde_json::Value = res
        .json()
        .await
        .map_err(|e| DiscoveryError::InvalidResponse(e.to_string()))?;

    let mut models = Vec::new();
    if let Some(data) = json["data"].as_array() {
        for entry in data {
            if let Some(id) = entry["id"].as_str() {
                if is_chat_model(id) {
                    models.push(id.to_string());
                }
            }
        }
    }

    if models.is_empty() {
        return Err(DiscoveryError::Empty);
    }
    Ok(models)
}

/// Fetch the list of model ids advertised by an OpenAI-compatible provider.
/// Legacy convenience wrapper; prefer [`fetch_models_raw`] when callers need
/// failure classification.
pub async fn fetch_models(base_url: &str, api_key: Option<&str>) -> Result<Vec<String>> {
    let url = format!("{}/models", base_url.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(8))
        .build()?;

    let mut req = client.get(&url);
    if let Some(key) = api_key.filter(|k| !k.is_empty()) {
        req = req.bearer_auth(key);
    }

    let res = req
        .send()
        .await
        .with_context(|| format!("Failed to fetch models from {}", url))?;

    let json: serde_json::Value = res.json().await?;
    let mut models = Vec::new();
    if let Some(data) = json["data"].as_array() {
        for entry in data {
            if let Some(id) = entry["id"].as_str() {
                models.push(id.to_string());
            }
        }
    }

    Ok(models)
}

/// Whether a model id is a chat/completions model rather than an embeddings,
/// audio, image, or utility model.
fn is_chat_model(id: &str) -> bool {
    let l = id.to_lowercase();
    ![
        "embedding",
        "whisper",
        "tts",
        "dall-e",
        "moderation",
        "audio",
        "realtime",
        "rerank",
        "-search-",
    ]
    .iter()
    .any(|pat| l.contains(pat))
}

/// Picks the best default chat model from a list, preferring widely used
/// general-purpose models.
///
/// Legacy DeepSeek names (`deepseek-chat`, `deepseek-reasoner`) are retained
/// ONLY as compatibility aliases for existing installs. They are deprecated
/// by DeepSeek; current installs should use `deepseek-v4-flash` /
/// `deepseek-v4-pro` from the official catalog.
pub fn pick_default(models: &[String]) -> Option<String> {
    let priority: &[(&str, u32)] = &[
        ("gpt-4o-mini", 60),
        ("gpt-4o", 55),
        ("deepseek-v4-pro", 72),
        ("deepseek-v4-flash", 70),
        ("deepseek/deepseek-v3", 66),
        // Deprecated compatibility aliases — kept for existing configs only.
        ("deepseek-reasoner", 65),
        ("deepseek-chat", 64),
        ("qwen3-coder-plus", 66),
        ("qwen3-coder", 64),
        ("qwen3.5-plus", 62),
        ("qwen3.7-plus", 62),
        ("qwen3-max", 62),
        ("qwen-plus", 55),
        ("claude-3.5-sonnet", 53),
        ("claude-sonnet", 52),
        ("gpt-oss", 58),
        ("gemini-3", 54),
        ("gemini-1.5", 42),
        ("mistral-large", 52),
        ("llama-3.3", 48),
        ("llama-3.1", 46),
        ("llama-3", 45),
        ("qwen2.5", 44),
        ("gpt-4", 38),
        ("gpt-3.5", 30),
    ];

    let mut best: Option<(String, u32, usize)> = None;
    for (idx, id) in models.iter().enumerate() {
        if !is_chat_model(id) {
            continue;
        }
        let lower = id.to_lowercase();
        let score = priority
            .iter()
            .find(|(pat, _)| lower.contains(pat))
            .map(|(_, s)| *s)
            .unwrap_or(10);
        match &best {
            Some((_, s, _)) if *s >= score => {}
            _ => best = Some((id.clone(), score, idx)),
        }
    }

    best.map(|(id, _, _)| id)
}

/// Convert a shipped catalog entry into discovery output.
fn catalog_models(catalog: &[CatalogModel], source: ModelSource) -> Vec<DiscoveredModel> {
    catalog
        .iter()
        .map(|m| DiscoveredModel {
            id: m.id.to_string(),
            source,
            metadata: ModelMetadata {
                display_name: Some(m.display_name.to_string()),
                tool_calling: m.tool_calling,
                context_tokens: m.context_tokens,
            },
        })
        .collect()
}

/// Discover models for a provider: try `GET /models`, then fall back to the
/// provider-known catalog when the endpoint is unavailable or incomplete.
///
/// The fallback is only used for providers with a deterministic official
/// catalog (e.g. DeepSeek). Models are never fabricated for unknown
/// providers.
pub async fn discover_models(
    base_url: &str,
    api_key: Option<&str>,
    provider: &str,
) -> ModelDiscovery {
    match fetch_models_raw(base_url, api_key).await {
        Ok(ids) => {
            let models = ids
                .into_iter()
                .map(|id| DiscoveredModel {
                    source: ModelSource::Discovered,
                    metadata: ModelMetadata::unknown(),
                    id,
                })
                .collect();
            ModelDiscovery {
                models,
                used_fallback: false,
                error: None,
            }
        }
        Err(err) => {
            let fallback = fallback_catalog(provider);
            if let Some(catalog) = fallback {
                ModelDiscovery {
                    models: catalog_models(catalog, ModelSource::ProviderDefault),
                    used_fallback: true,
                    error: Some(err),
                }
            } else {
                ModelDiscovery {
                    models: Vec::new(),
                    used_fallback: false,
                    error: Some(err),
                }
            }
        }
    }
}

/// Attempts to auto-discover a sensible default model from the provider,
/// falling back to the provider-known catalog when the endpoint is
/// unavailable. Returns `None` only when nothing usable is known.
pub async fn discover_model(
    base_url: &str,
    api_key: Option<&str>,
    provider: &str,
) -> Option<String> {
    let discovery = discover_models(base_url, api_key, provider).await;
    pick_default_from_discovery(&discovery)
}

/// Pick the best default from a discovery result (fallback-aware).
pub fn pick_default_from_discovery(discovery: &ModelDiscovery) -> Option<String> {
    let ids: Vec<String> = discovery.models.iter().map(|m| m.id.clone()).collect();
    if let Some(best) = pick_default(&ids) {
        return Some(best);
    }
    discovery.models.first().map(|m| m.id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ─── Error classification ───────────────────────────────────────────

    #[test]
    fn test_auth_error_human() {
        assert_eq!(
            DiscoveryError::Http(401).human(),
            "HTTP 401 — authentication failed"
        );
        assert!(is_auth_failure(&DiscoveryError::Http(401)));
        assert!(is_auth_failure(&DiscoveryError::Http(403)));
        assert!(!is_auth_failure(&DiscoveryError::Http(500)));
        assert!(!is_auth_failure(&DiscoveryError::Network("x".into())));
    }

    #[test]
    fn test_actionable_error_401_suggests_apikey() {
        let msg = actionable_model_error("deepseek", &DiscoveryError::Http(401));
        assert!(msg.contains("DeepSeek model discovery failed"));
        assert!(msg.contains("HTTP 401 — authentication failed"));
        assert!(msg.contains("Check //apikey deepseek"));
    }

    #[test]
    fn test_actionable_error_classification() {
        assert!(actionable_model_error("deepseek", &DiscoveryError::Http(402)).contains("balance"));
        assert!(
            actionable_model_error("deepseek", &DiscoveryError::Http(404)).contains("not found")
        );
        assert!(
            actionable_model_error("deepseek", &DiscoveryError::Http(429)).contains("rate limited")
        );
        assert!(
            actionable_model_error("deepseek", &DiscoveryError::Http(503)).contains("unavailable")
        );
        assert!(
            actionable_model_error("deepseek", &DiscoveryError::Network("boom".into()))
                .contains("endpoint unavailable")
        );
        assert!(
            !actionable_model_error("deepseek", &DiscoveryError::Http(401)).contains("boom"),
            "raw error text must not leak into auth guidance"
        );
    }

    #[test]
    fn test_fallback_marked_not_discovered() {
        let discovery = ModelDiscovery {
            models: catalog_models(
                crate::providers::catalog::DEEPSEEK_CATALOG,
                ModelSource::ProviderDefault,
            ),
            used_fallback: true,
            error: Some(DiscoveryError::Http(503)),
        };
        assert!(discovery.used_fallback);
        assert_eq!(discovery.models.len(), 2);
        assert_eq!(discovery.models[0].id, "deepseek-v4-flash");
        assert_eq!(discovery.models[0].source, ModelSource::ProviderDefault);
        assert_eq!(discovery.models[0].metadata.tool_calling, Some(true));
        assert_eq!(discovery.models[0].metadata.context_tokens, Some(1_000_000));
    }

    #[test]
    fn test_pick_default_prefers_deepseek_v4_over_legacy() {
        let models = vec![
            "deepseek-chat".to_string(),
            "deepseek-v4-flash".to_string(),
            "deepseek-reasoner".to_string(),
        ];
        assert_eq!(pick_default(&models).as_deref(), Some("deepseek-v4-flash"));
    }

    #[test]
    fn test_legacy_deepseek_aliases_remain_usable() {
        // Compatibility aliases must keep working for existing configs.
        let models = vec!["deepseek-chat".to_string()];
        assert_eq!(pick_default(&models).as_deref(), Some("deepseek-chat"));
        let models = vec!["deepseek-reasoner".to_string()];
        assert_eq!(pick_default(&models).as_deref(), Some("deepseek-reasoner"));
    }

    #[test]
    fn test_pick_default_prefers_gpt4o() {
        let models = vec![
            "text-embedding-3-small".to_string(),
            "gpt-4o".to_string(),
            "gpt-4o-mini".to_string(),
        ];
        assert_eq!(pick_default(&models).as_deref(), Some("gpt-4o-mini"));
    }

    #[test]
    fn test_pick_default_filters_non_chat() {
        let models = vec![
            "whisper-1".to_string(),
            "dall-e-3".to_string(),
            "deepseek-chat".to_string(),
        ];
        assert_eq!(pick_default(&models).as_deref(), Some("deepseek-chat"));
    }

    #[test]
    fn test_pick_default_empty() {
        assert_eq!(pick_default(&[]), None);
    }

    #[test]
    fn test_pick_default_unknown_uses_first() {
        let models = vec!["custom-model-7b".to_string(), "another".to_string()];
        assert_eq!(pick_default(&models).as_deref(), Some("custom-model-7b"));
    }

    #[test]
    fn test_pick_default_from_discovery() {
        let discovery = ModelDiscovery {
            models: catalog_models(
                crate::providers::catalog::DEEPSEEK_CATALOG,
                ModelSource::ProviderDefault,
            ),
            used_fallback: true,
            error: Some(DiscoveryError::Http(503)),
        };
        assert_eq!(
            pick_default_from_discovery(&discovery).as_deref(),
            Some("deepseek-v4-pro")
        );
    }

    // ─── Deterministic HTTP discovery tests (local mock server) ─────────

    /// A tiny one-shot HTTP server that answers with a canned response.
    async fn mock_server(status: u16, body: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{}", addr);
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut buf = [0u8; 4096];
            let _ = socket.read(&mut buf).await;
            let content_length = body.len();
            let response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status, content_length, body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
        url
    }

    #[tokio::test]
    async fn test_discovery_success_from_models_endpoint() {
        let url = mock_server(
            200,
            r#"{"data":[{"id":"deepseek-v4-flash"},{"id":"deepseek-v4-pro"},{"id":"embedding-x"}]}"#,
        )
        .await;
        let discovery = discover_models(&url, Some("sk-test"), "deepseek").await;
        assert!(!discovery.used_fallback);
        assert!(discovery.error.is_none());
        let ids: Vec<&str> = discovery.models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, vec!["deepseek-v4-flash", "deepseek-v4-pro"]);
        assert!(
            discovery
                .models
                .iter()
                .all(|m| m.source == ModelSource::Discovered),
            "advertised models must be marked discovered"
        );
    }

    #[tokio::test]
    async fn test_discovery_auth_failure_classified_401() {
        let url = mock_server(401, r#"{"error":"invalid key"}"#).await;
        let discovery = discover_models(&url, Some("sk-wrong"), "deepseek").await;
        assert!(discovery.used_fallback);
        let err = discovery.error.expect("classified error");
        assert_eq!(err, DiscoveryError::Http(401));
        assert!(is_auth_failure(&err));
        // Fallback still offers the official catalog, honestly labelled.
        assert_eq!(discovery.models[0].source, ModelSource::ProviderDefault);
    }

    #[tokio::test]
    async fn test_discovery_insufficient_balance_402() {
        let url = mock_server(402, r#"{"error":"insufficient_quota"}"#).await;
        let discovery = discover_models(&url, None, "deepseek").await;
        assert_eq!(discovery.error, Some(DiscoveryError::Http(402)));
        assert_eq!(
            discovery.error.as_ref().unwrap().human(),
            "HTTP 402 — insufficient provider balance"
        );
    }

    #[tokio::test]
    async fn test_discovery_not_found_404() {
        let url = mock_server(404, r#"{"error":"not_found"}"#).await;
        let discovery = discover_models(&url, None, "deepseek").await;
        assert_eq!(discovery.error, Some(DiscoveryError::Http(404)));
    }

    #[tokio::test]
    async fn test_discovery_rate_limited_429() {
        let url = mock_server(429, r#"{"error":"rate limited"}"#).await;
        let discovery = discover_models(&url, None, "deepseek").await;
        assert_eq!(discovery.error, Some(DiscoveryError::Http(429)));
    }

    #[tokio::test]
    async fn test_discovery_provider_unavailable_5xx() {
        let url = mock_server(503, r#"{"error":"busy"}"#).await;
        let discovery = discover_models(&url, None, "deepseek").await;
        assert_eq!(discovery.error, Some(DiscoveryError::Http(503)));
        assert!(
            discovery
                .error
                .as_ref()
                .unwrap()
                .human()
                .contains("unavailable"),
            "5xx must be humanized as provider unavailable"
        );
    }

    #[tokio::test]
    async fn test_discovery_unreachable_endpoint_falls_back() {
        // No server: connection refused.
        let discovery = discover_models("http://127.0.0.1:1", Some("sk-x"), "deepseek").await;
        assert!(discovery.used_fallback);
        assert_eq!(discovery.models.len(), 2);
        assert_eq!(discovery.models[0].source, ModelSource::ProviderDefault);
        assert!(
            matches!(discovery.error, Some(DiscoveryError::Network(_))),
            "unreachable endpoint must be a network error, not fabricated success"
        );
    }

    #[tokio::test]
    async fn test_discovery_empty_models_is_fallback_for_deepseek() {
        let url = mock_server(200, r#"{"data":[]}"#).await;
        let discovery = discover_models(&url, Some("sk-x"), "deepseek").await;
        assert!(discovery.used_fallback);
        assert_eq!(discovery.error, Some(DiscoveryError::Empty));
    }

    #[tokio::test]
    async fn test_unknown_provider_gets_no_fabricated_models() {
        let discovery = discover_models("http://127.0.0.1:1", Some("sk-x"), "mystery-vendor").await;
        assert!(!discovery.used_fallback);
        assert!(discovery.models.is_empty(), "never fabricate models");
        assert!(
            matches!(discovery.error, Some(DiscoveryError::Network(_))),
            "unknown provider failures stay honest"
        );
    }
}

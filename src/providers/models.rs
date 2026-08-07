#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::{Context, Result};
use std::time::Duration;

/// Fetches the list of model ids advertised by an OpenAI-compatible provider
/// via `GET {base_url}/models`.
pub async fn fetch_models(base_url: &str, api_key: Option<&str>) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .connect_timeout(Duration::from_secs(8))
        .build()?;

    let url = format!("{}/models", base_url.trim_end_matches('/'));
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
        "dall-e",
        "moderation",
        "audio",
        "realtime",
        "rerank",
        "-search-",
        "whisper-1",
    ]
    .iter()
    .any(|pat| l.contains(pat))
}

/// Picks the best default chat model from a list, preferring widely used
/// general-purpose models.
pub fn pick_default(models: &[String]) -> Option<String> {
    let priority: &[(&str, u32)] = &[
        ("gpt-4o-mini", 60),
        ("gpt-4o", 55),
        ("deepseek-v4-pro", 72),
        ("deepseek-v4-flash", 70),
        ("deepseek/deepseek-v3", 66),
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

/// Attempts to auto-discover a sensible default model from the provider.
/// Returns `None` if the endpoint is unreachable or returns nothing usable.
pub async fn discover_model(base_url: &str, api_key: Option<&str>) -> Option<String> {
    match fetch_models(base_url, api_key).await {
        Ok(models) => pick_default(&models),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}

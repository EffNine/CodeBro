#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::config::Config;
use crate::providers::Provider;
use anyhow::{Context, Result};

pub struct OpenAiProvider {
    config: Config,
}

impl OpenAiProvider {
    pub fn new(config: Config) -> Self {
        OpenAiProvider { config }
    }
}

impl Provider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
    }

    fn base_url(&self) -> &str {
        &self.config.base_url
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn api_key(&self) -> Option<&str> {
        self.config.api_key.as_deref()
    }

    fn send_message(
        &self,
        message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        let config = self.config.clone();
        let message = message.to_string();
        Box::pin(async move {
            let client = reqwest::Client::new();
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("CODEBRO_API_KEY").ok())
                .unwrap_or_default();

            let body = serde_json::json!({
                "model": config.model,
                "messages": [
                    {"role": "user", "content": message}
                ],
                "stream": false,
                "max_tokens": 4096
            });

            let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
            let res = client
                .post(&url)
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
                .with_context(|| "Failed to send request to provider")?;

            let json: serde_json::Value = res.json().await?;
            let content = json["choices"][0]["message"]["content"]
                .as_str()
                .unwrap_or("No response")
                .to_string();

            Ok(content)
        })
    }

    fn stream_response(
        &self,
        message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<tokio::sync::mpsc::UnboundedReceiver<String>>>
                + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let message = message.to_string();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let client = reqwest::Client::new();
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("CODEBRO_API_KEY").ok())
                .unwrap_or_default();

            let body = serde_json::json!({
                "model": config.model,
                "messages": [
                    {"role": "user", "content": message}
                ],
                "stream": true,
                "max_tokens": 4096
            });

            let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
            let res = match client
                .post(&url)
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(format!("Error: {}", e));
                    return Ok(rx);
                }
            };

            let mut stream = res.bytes_stream();
            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let text = String::from_utf8_lossy(&bytes);
                        for line in text.lines() {
                            if line.starts_with("data: ") {
                                let data = line.strip_prefix("data: ").unwrap_or(line);
                                if data == "[DONE]" {
                                    continue;
                                }
                                if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                    if let Some(content) =
                                        json["choices"][0]["delta"]["content"].as_str()
                                    {
                                        let _ = tx.send(content.to_string());
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => break,
                }
            }

            Ok(rx)
        })
    }
}

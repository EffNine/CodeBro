#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use crate::config::Config;
use crate::providers::{Provider, StructuredToolCall, ToolDefinition};
use anyhow::{Context, Result};
use std::collections::HashMap;

pub struct OpenAiProvider {
    config: Config,
}

impl OpenAiProvider {
    pub fn new(config: Config) -> Self {
        OpenAiProvider { config }
    }

    /// Build the request body, optionally including structured tool
    /// definitions (OpenAI function-calling format).
    fn build_body(
        config: &Config,
        message: &str,
        tools: &[ToolDefinition],
        stream: bool,
    ) -> serde_json::Value {
        let mut body = serde_json::json!({
            "model": config.model,
            "messages": [
                {"role": "user", "content": message}
            ],
            "stream": stream,
            "max_tokens": 4096
        });

        if !tools.is_empty() {
            let tool_objs: Vec<serde_json::Value> = tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "type": "function",
                        "function": {
                            "name": t.name,
                            "description": t.description,
                            "parameters": t.parameters,
                        }
                    })
                })
                .collect();
            body["tools"] = serde_json::Value::Array(tool_objs);
        }

        body
    }

    /// Accumulate tool-call fragments from a streaming SSE response.
    ///
    /// OpenAI streams tool calls as incremental `delta.tool_calls` arrays
    /// where each element carries an `index`. Arguments arrive across many
    /// chunks and must be concatenated per-index before the call is complete.
    fn accumulate_tool_calls(
        acc: &mut HashMap<usize, (String, String, String)>,
        delta_tool_calls: &serde_json::Value,
    ) {
        let Some(arr) = delta_tool_calls.as_array() else {
            return;
        };
        for item in arr {
            let Some(index) = item.get("index").and_then(|v| v.as_u64()) else {
                continue;
            };
            let idx = index as usize;
            let entry = acc
                .entry(idx)
                .or_insert_with(|| (String::new(), String::new(), String::new()));
            if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                entry.0 = id.to_string();
            }
            if let Some(fname) = item
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
            {
                entry.1 = fname.to_string();
            }
            if let Some(fargs) = item
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
            {
                entry.2.push_str(fargs);
            }
        }
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

    fn supports_function_calling(&self) -> bool {
        true
    }

    fn capabilities(&self) -> Vec<crate::provider_runtime::Capability> {
        vec![
            crate::provider_runtime::Capability::Streaming,
            crate::provider_runtime::Capability::ToolCalling,
            crate::provider_runtime::Capability::FunctionCalling,
        ]
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

            let body = Self::build_body(&config, &message, &[], false);

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

            let body = Self::build_body(&config, &message, &[], true);

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

    /// Stream a response with optional structured tool definitions.
    ///
    /// When `tools` is non-empty, the provider is asked to use native
    /// function calling. The returned text is the assistant's prose content
    /// and the returned structured calls are the native `tool_calls`.
    ///
    /// Streaming tool calls are accumulated by index across chunks so a call
    /// is never executed before its arguments are complete.
    fn stream_response_with_tools(
        &self,
        message: &str,
        tools: &[ToolDefinition],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<(String, Vec<StructuredToolCall>), anyhow::Error>,
                > + Send
                + '_,
        >,
    > {
        let config = self.config.clone();
        let message = message.to_string();
        let tools = tools.to_vec();
        Box::pin(async move {
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            let client = reqwest::Client::new();
            let api_key = config
                .api_key
                .clone()
                .or_else(|| std::env::var("CODEBRO_API_KEY").ok())
                .unwrap_or_default();

            let body = Self::build_body(&config, &message, &tools, true);

            let url = format!("{}/chat/completions", config.base_url.trim_end_matches('/'));
            let res = client
                .post(&url)
                .bearer_auth(api_key)
                .json(&body)
                .send()
                .await
                .with_context(|| "Failed to send request to provider")?;

            // Accumulators for streaming tool calls: index -> (id, name, args_fragments).
            let mut text = String::new();
            let mut tool_acc: HashMap<usize, (String, String, String)> = HashMap::new();

            let mut stream = res.bytes_stream();
            use futures::StreamExt;
            while let Some(chunk) = stream.next().await {
                match chunk {
                    Ok(bytes) => {
                        let chunk_text = String::from_utf8_lossy(&bytes);
                        for line in chunk_text.lines() {
                            if !line.starts_with("data: ") {
                                continue;
                            }
                            let data = line.strip_prefix("data: ").unwrap_or(line);
                            if data == "[DONE]" {
                                continue;
                            }
                            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data) {
                                let choice = &json["choices"][0];
                                if let Some(content) = choice["delta"]["content"].as_str() {
                                    text.push_str(content);
                                    let _ = tx.send(content.to_string());
                                }
                                let tool_calls_val = &choice["delta"]["tool_calls"];
                                if !tool_calls_val.is_null() {
                                    Self::accumulate_tool_calls(&mut tool_acc, tool_calls_val);
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(format!("Error: {}", e));
                        break;
                    }
                }
            }

            let structured: Vec<StructuredToolCall> = tool_acc
                .into_iter()
                .map(|(_, (id, name, args))| StructuredToolCall {
                    id,
                    name,
                    arguments: args,
                })
                .collect();

            Ok((text, structured))
        })
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;
use serde::{Deserialize, Serialize};

/// A tool definition suitable for OpenAI-compatible function calling.
/// This is the single authoritative representation sent to providers that
/// support native structured tool calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// The name of the tool / function.
    pub name: String,
    /// Human-readable description of what the tool does.
    pub description: String,
    /// JSON Schema describing the tool's input parameters.
    pub parameters: serde_json::Value,
}

/// A structured tool call extracted from a provider's native function-calling
/// response (e.g. OpenAI `tool_calls` array).
#[derive(Debug, Clone)]
pub struct StructuredToolCall {
    /// Provider-assigned call id (e.g. "call_abc123").
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Raw JSON-stringified arguments as returned by the provider.
    pub arguments: String,
}

pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn base_url(&self) -> &str;
    fn model(&self) -> &str;
    fn api_key(&self) -> Option<&str>;
    fn send_message(
        &self,
        message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>>;
    fn stream_response(
        &self,
        message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<tokio::sync::mpsc::UnboundedReceiver<String>>>
                + Send
                + '_,
        >,
    >;

    /// Whether this provider supports native structured function calling
    /// (OpenAI-compatible `tools` parameter with `tool_calls` responses).
    ///
    /// When `true`, the canonical runtime will send tool definitions and
    /// expect structured `tool_calls` in the response. When `false` (default),
    /// the runtime falls back to text-based parsing.
    fn supports_function_calling(&self) -> bool {
        false
    }

    /// Stream a response that may include structured tool calls.
    ///
    /// Returns `(text_content, structured_tool_calls)`. The text content is
    /// the assistant's prose; structured_tool_calls are native function-calling
    /// objects from the provider. Callers should prefer structured calls when
    /// available and fall back to text parsing otherwise.
    ///
    /// The default implementation delegates to [`Provider::stream_response`]
    /// and returns an empty tool-calls list, preserving backward compatibility
    /// for providers that do not override this method.
    fn stream_response_with_tools(
        &self,
        message: &str,
        _tools: &[ToolDefinition],
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<(String, Vec<StructuredToolCall>), anyhow::Error>,
                > + Send
                + '_,
        >,
    > {
        let fut = self.stream_response(message);
        Box::pin(async move {
            let mut rx = fut.await?;
            let mut text = String::new();
            while let Some(chunk) = rx.recv().await {
                text.push_str(&chunk);
            }
            Ok((text, Vec::new()))
        })
    }

    /// Optional provider-declared capabilities.
    ///
    /// When non-empty, the adapter uses these as the registered metadata
    /// instead of the hard-coded fallback. Providers that self-describe must
    /// include every capability they actually support (e.g. `Streaming`,
    /// `ToolCalling`).
    fn capabilities(&self) -> Vec<crate::provider_runtime::Capability> {
        Vec::new()
    }

    /// Optional provider-declared pricing model.
    ///
    /// When `Some`, the adapter uses it as the registered cost metadata
    /// instead of the default.
    fn cost(&self) -> Option<crate::provider_runtime::ProviderCost> {
        None
    }
}

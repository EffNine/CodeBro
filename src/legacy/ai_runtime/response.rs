use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt;

use super::types::{AIRRuntimeError, AIRRuntimeResult, ModelId};

/// Token usage statistics for a response.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResponseUsage {
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub total_tokens: usize,
    pub cache_read_tokens: Option<usize>,
    pub cache_creation_tokens: Option<usize>,
}

impl ResponseUsage {
    pub fn new(prompt_tokens: usize, completion_tokens: usize, total_tokens: usize) -> Self {
        ResponseUsage {
            prompt_tokens,
            completion_tokens,
            total_tokens,
            cache_read_tokens: None,
            cache_creation_tokens: None,
        }
    }

    pub fn estimated_cost(&self, cost_per_million: f64) -> f64 {
        let total = self.total_tokens as f64;
        (total / 1_000_000.0) * cost_per_million
    }
}

/// A choice returned by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Choice {
    pub index: usize,
    pub message: ResponseMessage,
    pub finish_reason: Option<String>,
}

impl Choice {
    pub fn new(index: usize, message: ResponseMessage, finish_reason: Option<String>) -> Self {
        Choice {
            index,
            message,
            finish_reason,
        }
    }
}

/// The response message from the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: Vec<super::request::ToolCall>,
}

impl ResponseMessage {
    pub fn new(role: impl Into<String>, content: Option<String>) -> Self {
        ResponseMessage {
            role: role.into(),
            content,
            tool_calls: Vec::new(),
        }
    }

    pub fn with_tool_calls(mut self, tool_calls: Vec<super::request::ToolCall>) -> Self {
        self.tool_calls = tool_calls;
        self
    }

    pub fn is_tool_call(&self) -> bool {
        !self.tool_calls.is_empty()
    }
}

/// A response delta for streaming.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ResponseDelta {
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<Vec<ToolCallDelta>>,
}

/// A tool call delta for streaming.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ToolCallDelta {
    pub index: usize,
    pub id: Option<String>,
    pub r#type: Option<String>,
    pub function: Option<FunctionCallDelta>,
}

/// Function call delta for streaming.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FunctionCallDelta {
    pub name: Option<String>,
    pub arguments: Option<String>,
}

/// A complete model response — provider-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelResponse {
    pub id: String,
    pub model_id: ModelId,
    pub choices: Vec<Choice>,
    pub usage: ResponseUsage,
    pub created_at: u64,
    pub provider_type: String,
    pub raw_response: Option<serde_json::Value>,
}

impl ModelResponse {
    pub fn new(
        id: impl Into<String>,
        model_id: impl Into<ModelId>,
        choices: Vec<Choice>,
        usage: ResponseUsage,
        created_at: u64,
        provider_type: impl Into<String>,
    ) -> Self {
        ModelResponse {
            id: id.into(),
            model_id: model_id.into(),
            choices,
            usage,
            created_at,
            provider_type: provider_type.into(),
            raw_response: None,
        }
    }

    pub fn with_raw_response(mut self, raw: serde_json::Value) -> Self {
        self.raw_response = Some(raw);
        self
    }

    /// Get the primary content from the first choice.
    pub fn content(&self) -> Option<&str> {
        self.choices
            .first()
            .and_then(|c| c.message.content.as_deref())
    }

    /// Get all tool calls from the response.
    pub fn tool_calls(&self) -> &[super::request::ToolCall] {
        self.choices
            .first()
            .map(|c| c.message.tool_calls.as_slice())
            .unwrap_or(&[])
    }

    /// Check if this response contains any tool calls.
    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls().is_empty()
    }

    /// Serialize this response to JSON.
    pub fn to_json(&self) -> AIRRuntimeResult<serde_json::Value> {
        serde_json::to_value(self).map_err(|e| AIRRuntimeError::SerializationError(e.to_string()))
    }

    /// Deserialize a response from JSON.
    pub fn from_json(json: &str) -> AIRRuntimeResult<Self> {
        serde_json::from_str(json).map_err(|e| AIRRuntimeError::JsonParseError(e.to_string()))
    }
}

impl fmt::Display for ModelResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let content = self.content().unwrap_or("");
        write!(
            f,
            "ModelResponse(id={}, model={}, tokens={}, content_len={})",
            self.id,
            self.model_id,
            self.usage.total_tokens,
            content.len()
        )
    }
}

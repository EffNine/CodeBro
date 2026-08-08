use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use super::types::{AIRRuntimeError, AIRRuntimeResult, ModelId};

/// The role of a message in a conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MessageRole {
    System,
    User,
    Assistant,
    Tool,
}

impl fmt::Display for MessageRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MessageRole::System => write!(f, "system"),
            MessageRole::User => write!(f, "user"),
            MessageRole::Assistant => write!(f, "assistant"),
            MessageRole::Tool => write!(f, "tool"),
        }
    }
}

impl FromStr for MessageRole {
    type Err = AIRRuntimeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "system" => Ok(MessageRole::System),
            "user" => Ok(MessageRole::User),
            "assistant" => Ok(MessageRole::Assistant),
            "tool" => Ok(MessageRole::Tool),
            _ => Err(AIRRuntimeError::InvalidMessageRole(s.to_string())),
        }
    }
}

/// A single message in a model request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub tool_call_id: Option<String>,
}

impl Message {
    pub fn new(role: MessageRole, content: impl Into<String>) -> Self {
        Message {
            role,
            content: content.into(),
            tool_call_id: None,
        }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Message::new(MessageRole::System, content)
    }

    pub fn user(content: impl Into<String>) -> Self {
        Message::new(MessageRole::User, content)
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Message::new(MessageRole::Assistant, content)
    }

    pub fn tool(content: impl Into<String>, tool_call_id: impl Into<String>) -> Self {
        let mut msg = Message::new(MessageRole::Tool, content);
        msg.tool_call_id = Some(tool_call_id.into());
        msg
    }
}

/// Tool call requested by the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, function: FunctionCall) -> Self {
        ToolCall {
            id: id.into(),
            r#type: "function".to_string(),
            function,
        }
    }
}

/// The function to call as specified by a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

impl FunctionCall {
    pub fn new(name: impl Into<String>, arguments: serde_json::Value) -> Self {
        FunctionCall {
            name: name.into(),
            arguments,
        }
    }

    pub fn new_with_args(name: impl Into<String>, args: &str) -> AIRRuntimeResult<Self> {
        let arguments = serde_json::from_str(args)
            .map_err(|e| AIRRuntimeError::JsonParseError(e.to_string()))?;
        Ok(FunctionCall::new(name, arguments))
    }
}

/// Tool result message sent back to the model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResultMessage {
    pub tool_call_id: String,
    pub content: String,
}

impl ToolResultMessage {
    pub fn new(tool_call_id: impl Into<String>, content: impl Into<String>) -> Self {
        ToolResultMessage {
            tool_call_id: tool_call_id.into(),
            content: content.into(),
        }
    }
}

/// A request to the AI model — provider-agnostic.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelRequest {
    pub model_id: ModelId,
    pub messages: Vec<Message>,
    pub tools: Vec<super::tool_contract::ToolDefinition>,
    pub stream: bool,
    pub max_tokens: Option<usize>,
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub stop_sequences: Vec<String>,
    pub reasoning_effort: Option<String>,
    pub structured_output: Option<super::structured_output::StructuredOutputSchema>,
    pub presence_penalty: Option<f64>,
    pub frequency_penalty: Option<f64>,
}

impl ModelRequest {
    pub fn new(model_id: impl Into<ModelId>, messages: Vec<Message>) -> Self {
        ModelRequest {
            model_id: model_id.into(),
            messages,
            tools: Vec::new(),
            stream: false,
            max_tokens: None,
            temperature: None,
            top_p: None,
            stop_sequences: Vec::new(),
            reasoning_effort: None,
            structured_output: None,
            presence_penalty: None,
            frequency_penalty: None,
        }
    }

    pub fn with_tools(mut self, tools: Vec<super::tool_contract::ToolDefinition>) -> Self {
        self.tools = tools;
        self
    }

    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_top_p(mut self, top_p: f64) -> Self {
        self.top_p = Some(top_p);
        self
    }

    pub fn with_stop_sequences(mut self, stops: Vec<String>) -> Self {
        self.stop_sequences = stops;
        self
    }

    pub fn with_reasoning_effort(mut self, effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(effort.into());
        self
    }

    pub fn with_structured_output(
        mut self,
        schema: super::structured_output::StructuredOutputSchema,
    ) -> Self {
        self.structured_output = Some(schema);
        self
    }

    pub fn with_penalty(mut self, presence: f64, frequency: f64) -> Self {
        self.presence_penalty = Some(presence);
        self.frequency_penalty = Some(frequency);
        self
    }

    /// Check if this request requires any special capabilities.
    pub fn required_capabilities(&self) -> Vec<super::capabilities::Capability> {
        let mut caps = Vec::new();
        if self.stream {
            caps.push(super::capabilities::Capability::Streaming);
        }
        if self.structured_output.is_some() {
            caps.push(super::capabilities::Capability::StructuredOutput);
        }
        if !self.tools.is_empty() {
            caps.push(super::capabilities::Capability::ToolCalling);
        }
        if self.reasoning_effort.is_some() {
            caps.push(super::capabilities::Capability::Reasoning);
        }
        caps
    }

    /// Serialize this request to JSON (provider-agnostic format).
    pub fn to_json(&self) -> AIRRuntimeResult<serde_json::Value> {
        serde_json::to_value(self).map_err(|e| AIRRuntimeError::SerializationError(e.to_string()))
    }

    /// Deserialize a request from JSON.
    pub fn from_json(json: &str) -> AIRRuntimeResult<Self> {
        serde_json::from_str(json).map_err(|e| AIRRuntimeError::JsonParseError(e.to_string()))
    }
}

impl fmt::Display for ModelRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "ModelRequest(model={}, messages={}, tools={}, stream={})",
            self.model_id,
            self.messages.len(),
            self.tools.len(),
            self.stream
        )
    }
}

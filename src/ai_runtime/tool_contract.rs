use serde::{Deserialize, Serialize};
use std::fmt;

/// A tool definition for model tool calling.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub r#type: String,
    pub function: FunctionDefinition,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: serde_json::Value) -> Self {
        ToolDefinition {
            r#type: "function".to_string(),
            function: FunctionDefinition {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }

    pub fn with_strict(mut self, strict: bool) -> Self {
        if strict {
            self.function.parameters
                .as_object_mut()
                .unwrap()
                .insert("strict".to_string(), serde_json::Value::Bool(true));
        }
        self
    }
}

/// Definition of a function within a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

impl FunctionDefinition {
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        parameters: serde_json::Value,
    ) -> Self {
        FunctionDefinition {
            name: name.into(),
            description: description.into(),
            parameters,
        }
    }
}

/// Schema definition for tool arguments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolSchema {
    pub r#type: String,
    pub properties: serde_json::Map<String, serde_json::Value>,
    pub required: Vec<String>,
}

impl ToolSchema {
    pub fn new() -> Self {
        ToolSchema {
            r#type: "object".to_string(),
            properties: serde_json::Map::new(),
            required: Vec::new(),
        }
    }

    pub fn add_property(mut self, name: &str, schema: &serde_json::Value) -> Self {
        self.properties.insert(name.to_string(), schema.clone());
        self
    }

    pub fn add_required(mut self, name: &str) -> Self {
        if !self.required.contains(&name.to_string()) {
            self.required.push(name.to_string());
        }
        self
    }
}

impl Default for ToolSchema {
    fn default() -> Self {
        Self::new()
    }
}

/// A single argument for a tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolArgument {
    pub name: String,
    pub value: serde_json::Value,
}

impl ToolArgument {
    pub fn new(name: impl Into<String>, value: serde_json::Value) -> Self {
        ToolArgument {
            name: name.into(),
            value,
        }
    }
}

/// The result of executing a tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolResult {
    Success {
        content: String,
    },
    Error {
        error: String,
    },
}

impl fmt::Display for ToolResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ToolResult::Success { content } => write!(f, "Success({})", content),
            ToolResult::Error { error } => write!(f, "Error({})", error),
        }
    }
}

impl ToolResult {
    pub fn success(content: impl Into<String>) -> Self {
        ToolResult::Success {
            content: content.into(),
        }
    }

    pub fn error(error: impl Into<String>) -> Self {
        ToolResult::Error {
            error: error.into(),
        }
    }

    pub fn is_success(&self) -> bool {
        matches!(self, ToolResult::Success { .. })
    }

    pub fn content(&self) -> Option<&str> {
        match self {
            ToolResult::Success { content } => Some(content),
            ToolResult::Error { .. } => None,
        }
    }

    pub fn error_msg(&self) -> Option<&str> {
        match self {
            ToolResult::Success { .. } => None,
            ToolResult::Error { error } => Some(error),
        }
    }
}

use serde::{Deserialize, Serialize};
use std::fmt;

use super::capabilities::CapabilitySet;

/// Type of provider (used for routing metadata only, never hardcoded).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    Ollama,
    OpenRouter,
    DeepSeek,
    LMStudio,
    Azure,
    Custom(String),
}

impl fmt::Display for ProviderType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProviderType::OpenAI => write!(f, "openai"),
            ProviderType::Anthropic => write!(f, "anthropic"),
            ProviderType::Ollama => write!(f, "ollama"),
            ProviderType::OpenRouter => write!(f, "openrouter"),
            ProviderType::DeepSeek => write!(f, "deepseek"),
            ProviderType::LMStudio => write!(f, "lmstudio"),
            ProviderType::Azure => write!(f, "azure"),
            ProviderType::Custom(name) => write!(f, "{}", name),
        }
    }
}

impl ProviderType {
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "openai" => ProviderType::OpenAI,
            "anthropic" => ProviderType::Anthropic,
            "ollama" => ProviderType::Ollama,
            "openrouter" => ProviderType::OpenRouter,
            "deepseek" => ProviderType::DeepSeek,
            "lmstudio" => ProviderType::LMStudio,
            "azure" => ProviderType::Azure,
            _ => ProviderType::Custom(s.to_string()),
        }
    }
}

/// A model identifier — provider-agnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelId {
    pub id: String,
    pub provider: ProviderType,
}

impl ModelId {
    pub fn new(id: impl Into<String>, provider: ProviderType) -> Self {
        ModelId {
            id: id.into(),
            provider,
        }
    }

    pub fn openai(id: impl Into<String>) -> Self {
        ModelId::new(id, ProviderType::OpenAI)
    }

    pub fn anthropic(id: impl Into<String>) -> Self {
        ModelId::new(id, ProviderType::Anthropic)
    }

    pub fn ollama(id: impl Into<String>) -> Self {
        ModelId::new(id, ProviderType::Ollama)
    }
}

impl fmt::Display for ModelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.id)
    }
}

impl From<String> for ModelId {
    fn from(id: String) -> Self {
        ModelId {
            id,
            provider: ProviderType::Custom("unknown".to_string()),
        }
    }
}

impl From<&str> for ModelId {
    fn from(id: &str) -> Self {
        ModelId {
            id: id.to_string(),
            provider: ProviderType::Custom("unknown".to_string()),
        }
    }
}

impl AsRef<str> for ModelId {
    fn as_ref(&self) -> &str {
        &self.id
    }
}

/// Priority level for requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Priority {
    Low,
    Normal,
    High,
    Critical,
}

impl Priority {
    pub fn score(&self) -> u8 {
        match self {
            Priority::Low => 0,
            Priority::Normal => 1,
            Priority::High => 2,
            Priority::Critical => 3,
        }
    }
}

impl Default for Priority {
    fn default() -> Self {
        Priority::Normal
    }
}

/// Cost estimate for a model invocation.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct CostEstimate {
    pub input_cost_per_million: f64,
    pub output_cost_per_million: f64,
    pub cache_read_cost_per_million: Option<f64>,
    pub cache_creation_cost_per_million: Option<f64>,
}

impl CostEstimate {
    pub fn estimate(
        &self,
        input_tokens: usize,
        output_tokens: usize,
        cache_read_tokens: Option<usize>,
    ) -> f64 {
        let cache_tokens = cache_read_tokens.unwrap_or(0) as f64;
        let input_cost = (input_tokens as f64 / 1_000_000.0) * self.input_cost_per_million;
        let output_cost = (output_tokens as f64 / 1_000_000.0) * self.output_cost_per_million;
        let cache_cost = if cache_tokens > 0.0 {
            if let Some(cache_rate) = self.cache_read_cost_per_million {
                (cache_tokens / 1_000_000.0) * cache_rate
            } else {
                0.0
            }
        } else {
            0.0
        };
        input_cost + output_cost - cache_cost
    }
}

/// Health status of a model provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl HealthStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(self, HealthStatus::Healthy | HealthStatus::Degraded)
    }
}

impl Default for HealthStatus {
    fn default() -> Self {
        HealthStatus::Unknown
    }
}

/// Errors specific to the AI Runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AIRRuntimeError {
    /// No suitable provider found for the request
    NoSuitableProvider(String),
    /// Capability mismatch between request and provider
    CapabilityMismatch {
        requested: Vec<String>,
        available: Vec<String>,
    },
    /// Request validation failed
    InvalidRequest(String),
    /// Response parsing failed
    ResponseParseError(String),
    /// Structured output validation failed
    StructuredOutputValidation(String),
    /// Tool contract violation
    ToolContractViolation(String),
    /// Routing decision failed
    RoutingError(String),
    /// Diagnostic error
    DiagnosticError(String),
    /// Stream pipeline error
    StreamingError(String),
    /// Serialization error
    SerializationError(String),
    /// JSON parse error
    JsonParseError(String),
    /// Unknown capability
    CapabilityUnknown(String),
    /// Invalid message role
    InvalidMessageRole(String),
    /// Generic error
    Generic(String),
}

impl fmt::Display for AIRRuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AIRRuntimeError::NoSuitableProvider(msg) => write!(f, "No suitable provider: {}", msg),
            AIRRuntimeError::CapabilityMismatch {
                requested,
                available,
            } => {
                write!(
                    f,
                    "Capability mismatch: requested {:?}, available {:?}",
                    requested, available
                )
            }
            AIRRuntimeError::InvalidRequest(msg) => write!(f, "Invalid request: {}", msg),
            AIRRuntimeError::ResponseParseError(msg) => write!(f, "Response parse error: {}", msg),
            AIRRuntimeError::StructuredOutputValidation(msg) => {
                write!(f, "Structured output validation failed: {}", msg)
            }
            AIRRuntimeError::ToolContractViolation(msg) => {
                write!(f, "Tool contract violation: {}", msg)
            }
            AIRRuntimeError::RoutingError(msg) => write!(f, "Routing error: {}", msg),
            AIRRuntimeError::DiagnosticError(msg) => write!(f, "Diagnostic error: {}", msg),
            AIRRuntimeError::StreamingError(msg) => write!(f, "Streaming error: {}", msg),
            AIRRuntimeError::SerializationError(msg) => write!(f, "Serialization error: {}", msg),
            AIRRuntimeError::JsonParseError(msg) => write!(f, "JSON parse error: {}", msg),
            AIRRuntimeError::CapabilityUnknown(cap) => write!(f, "Unknown capability: {}", cap),
            AIRRuntimeError::InvalidMessageRole(role) => {
                write!(f, "Invalid message role: {}", role)
            }
            AIRRuntimeError::Generic(msg) => write!(f, "Runtime error: {}", msg),
        }
    }
}

impl std::error::Error for AIRRuntimeError {}

pub type AIRRuntimeResult<T> = Result<T, AIRRuntimeError>;

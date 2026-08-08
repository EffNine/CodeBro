use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use super::types::{AIRRuntimeError, AIRRuntimeResult};

/// Core AI capabilities that can be negotiated between runtime and providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Model supports streaming responses (SSE)
    Streaming,
    /// Model supports structured JSON output
    StructuredOutput,
    /// Model supports tool/function calling
    ToolCalling,
    /// Model supports vision (image input)
    Vision,
    /// Model supports chain-of-thought reasoning
    Reasoning,
    /// Model supports text embeddings
    Embeddings,
    /// Model supports audio input/output
    Audio,
    /// Model supports image generation
    ImageGeneration,
}

impl Capability {
    pub fn description(&self) -> &str {
        match self {
            Capability::Streaming => "Streaming responses via Server-Sent Events",
            Capability::StructuredOutput => "Structured JSON output with schema validation",
            Capability::ToolCalling => "Tool/function calling capabilities",
            Capability::Vision => "Vision input — ability to process images",
            Capability::Reasoning => "Chain-of-thought reasoning support",
            Capability::Embeddings => "Text embedding generation",
            Capability::Audio => "Audio input/output support",
            Capability::ImageGeneration => "Image generation capability",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::Streaming => write!(f, "streaming"),
            Capability::StructuredOutput => write!(f, "structured_output"),
            Capability::ToolCalling => write!(f, "tool_calling"),
            Capability::Vision => write!(f, "vision"),
            Capability::Reasoning => write!(f, "reasoning"),
            Capability::Embeddings => write!(f, "embeddings"),
            Capability::Audio => write!(f, "audio"),
            Capability::ImageGeneration => write!(f, "image_generation"),
        }
    }
}

impl FromStr for Capability {
    type Err = AIRRuntimeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "streaming" => Ok(Capability::Streaming),
            "structured_output" | "structured-output" | "json_output" => {
                Ok(Capability::StructuredOutput)
            }
            "tool_calling" | "tool-calling" | "function_calling" => Ok(Capability::ToolCalling),
            "vision" | "image_input" => Ok(Capability::Vision),
            "reasoning" | "chain_of_thought" => Ok(Capability::Reasoning),
            "embeddings" => Ok(Capability::Embeddings),
            "audio" => Ok(Capability::Audio),
            "image_generation" | "image-generation" => Ok(Capability::ImageGeneration),
            _ => Err(AIRRuntimeError::CapabilityUnknown(s.to_string())),
        }
    }
}

/// Set of capabilities supported by a model or provider.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    pub capabilities: HashSet<Capability>,
}

impl CapabilitySet {
    pub fn new(capabilities: impl IntoIterator<Item = Capability>) -> Self {
        CapabilitySet {
            capabilities: capabilities.into_iter().collect(),
        }
    }

    pub fn empty() -> Self {
        CapabilitySet {
            capabilities: HashSet::new(),
        }
    }

    pub fn has(&self, capability: &Capability) -> bool {
        self.capabilities.contains(capability)
    }

    pub fn has_all(&self, required: &[Capability]) -> bool {
        required.iter().all(|c| self.has(c))
    }

    pub fn has_any(&self, required: &[Capability]) -> bool {
        required.iter().any(|c| self.has(c))
    }

    pub fn merge(&mut self, other: &CapabilitySet) {
        for cap in &other.capabilities {
            self.capabilities.insert(*cap);
        }
    }

    pub fn intersection(&self, other: &CapabilitySet) -> CapabilitySet {
        CapabilitySet {
            capabilities: self
                .capabilities
                .intersection(&other.capabilities)
                .copied()
                .collect(),
        }
    }

    pub fn required_for_request(
        request: &crate::ai_runtime::request::ModelRequest,
    ) -> Vec<Capability> {
        let mut caps = Vec::new();
        if request.stream {
            caps.push(Capability::Streaming);
        }
        if request.structured_output.is_some() {
            caps.push(Capability::StructuredOutput);
        }
        if !request.tools.is_empty() {
            caps.push(Capability::ToolCalling);
        }
        if request.reasoning_effort.is_some() {
            caps.push(Capability::Reasoning);
        }
        caps
    }
}

/// Result of capability negotiation between request and provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityNegotiation {
    pub request_capabilities: Vec<Capability>,
    pub provider_capabilities: Vec<Capability>,
    pub negotiated: Vec<Capability>,
    pub missing: Vec<Capability>,
    pub compatible: bool,
}

impl CapabilityNegotiation {
    pub fn new(
        request: &crate::ai_runtime::request::ModelRequest,
        provider_caps: &CapabilitySet,
    ) -> Self {
        let required = CapabilitySet::required_for_request(request);
        let negotiated: Vec<Capability> = required
            .iter()
            .filter(|c| provider_caps.has(c))
            .copied()
            .collect();
        let missing: Vec<Capability> = required
            .iter()
            .filter(|c| !provider_caps.has(c))
            .copied()
            .collect();
        let compatible = missing.is_empty();
        CapabilityNegotiation {
            request_capabilities: required,
            provider_capabilities: provider_caps.capabilities.iter().copied().collect(),
            negotiated,
            missing,
            compatible,
        }
    }
}

/// Descriptor of all capabilities a model might support.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SupportedCapabilities {
    pub model_id: String,
    pub provider_type: String,
    pub capabilities: CapabilitySet,
    pub confidence: f64,
}

impl SupportedCapabilities {
    pub fn new(
        model_id: impl Into<String>,
        provider_type: impl Into<String>,
        caps: CapabilitySet,
    ) -> Self {
        SupportedCapabilities {
            model_id: model_id.into(),
            provider_type: provider_type.into(),
            capabilities: caps,
            confidence: 1.0,
        }
    }

    pub fn with_confidence(mut self, confidence: f64) -> Self {
        self.confidence = confidence;
        self
    }
}

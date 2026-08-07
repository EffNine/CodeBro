#![allow(dead_code, clippy::all)]
//! Capability descriptors used for provider negotiation.
//!
//! Capability matching is INDEPENDENT of provider identity. A request
//! names the capabilities it needs; a provider either supports them or
//! not. The future MUST be additive — new capabilities are added as new
//! enum variants without disturbing existing matching.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use super::types::{ProviderRuntimeError, ProviderRuntimeResult};

/// A capability descriptor a provider may support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Streaming responses.
    Streaming,
    /// Structured/schema-validated output.
    StructuredOutput,
    /// Tool / function calling.
    ToolCalling,
    /// Vision (image input).
    Vision,
    /// Embeddings.
    Embeddings,
    /// Reasoning / chain-of-thought.
    Reasoning,
    /// Audio input / output.
    Audio,
    /// Image generation.
    ImageGeneration,
    /// Large context windows.
    LongContext,
    /// JSON-only mode.
    JsonMode,
}

impl Capability {
    /// All currently known capabilities, in a stable declaration order.
    pub fn all() -> &'static [Capability] {
        &[
            Capability::Streaming,
            Capability::StructuredOutput,
            Capability::ToolCalling,
            Capability::Vision,
            Capability::Embeddings,
            Capability::Reasoning,
            Capability::Audio,
            Capability::ImageGeneration,
            Capability::LongContext,
            Capability::JsonMode,
        ]
    }

    pub fn description(&self) -> &'static str {
        match self {
            Capability::Streaming => "Streaming responses",
            Capability::StructuredOutput => "Structured JSON output",
            Capability::ToolCalling => "Tool/function calling",
            Capability::Vision => "Vision — image input",
            Capability::Embeddings => "Text embeddings",
            Capability::Reasoning => "Chain-of-thought reasoning",
            Capability::Audio => "Audio input/output",
            Capability::ImageGeneration => "Image generation",
            Capability::LongContext => "Large context window",
            Capability::JsonMode => "JSON mode",
        }
    }

    /// Token name for diagnostics.
    pub fn code(&self) -> &'static str {
        match self {
            Capability::Streaming => "streaming",
            Capability::StructuredOutput => "structured_output",
            Capability::ToolCalling => "tool_calling",
            Capability::Vision => "vision",
            Capability::Embeddings => "embeddings",
            Capability::Reasoning => "reasoning",
            Capability::Audio => "audio",
            Capability::ImageGeneration => "image_generation",
            Capability::LongContext => "long_context",
            Capability::JsonMode => "json_mode",
        }
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.code())
    }
}

impl FromStr for Capability {
    type Err = ProviderRuntimeError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "streaming" => Ok(Capability::Streaming),
            "structured_output" | "structured-output" | "structured" => {
                Ok(Capability::StructuredOutput)
            }
            "tool_calling" | "tool-calling" | "function_calling" => Ok(Capability::ToolCalling),
            "vision" | "image_input" => Ok(Capability::Vision),
            "embeddings" => Ok(Capability::Embeddings),
            "reasoning" => Ok(Capability::Reasoning),
            "audio" => Ok(Capability::Audio),
            "image_generation" | "image-generation" | "imagegen" => Ok(Capability::ImageGeneration),
            "long_context" | "long-context" | "large_context" => Ok(Capability::LongContext),
            "json_mode" | "json-mode" | "json" => Ok(Capability::JsonMode),
            other => Err(ProviderRuntimeError::UnknownCapability(other.to_string())),
        }
    }
}

/// An ordered, hash-backed set of capabilities.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilitySet {
    capabilities: HashSet<Capability>,
}

impl CapabilitySet {
    pub fn new(iter: impl IntoIterator<Item = Capability>) -> Self {
        let capabilities: HashSet<Capability> = iter.into_iter().collect();
        CapabilitySet { capabilities }
    }

    pub fn empty() -> Self {
        CapabilitySet {
            capabilities: HashSet::new(),
        }
    }

    pub fn has(&self, cap: &Capability) -> bool {
        self.capabilities.contains(cap)
    }

    pub fn has_all(&self, required: &[Capability]) -> bool {
        required.iter().all(|c| self.capabilities.contains(c))
    }

    pub fn has_any(&self, candidates: &[Capability]) -> bool {
        candidates.iter().any(|c| self.capabilities.contains(c))
    }

    pub fn insert(&mut self, cap: Capability) {
        self.capabilities.insert(cap);
    }

    pub fn extend(&mut self, iter: impl IntoIterator<Item = Capability>) {
        self.capabilities.extend(iter);
    }

    pub fn remove(&mut self, cap: &Capability) {
        self.capabilities.remove(cap);
    }

    /// Iterate over capabilities in declaration order (stable, additive).
    pub fn iter(&self) -> impl Iterator<Item = Capability> + '_ {
        Capability::all()
            .iter()
            .copied()
            .filter(|c| self.capabilities.contains(c))
    }

    pub fn len(&self) -> usize {
        self.capabilities.len()
    }

    pub fn is_empty(&self) -> bool {
        self.capabilities.is_empty()
    }

    pub fn intersection(&self, other: &CapabilitySet) -> CapabilitySet {
        CapabilitySet {
            capabilities: self.capabilities.intersection(&other.capabilities).copied().collect(),
        }
    }

    pub fn union(&self, other: &CapabilitySet) -> CapabilitySet {
        let mut out = self.clone();
        out.extend(other.iter());
        out
    }
}

/// Result of matching a required set against a provider's set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityMatch {
    pub required: Vec<Capability>,
    pub provider: Vec<Capability>,
    pub satisfied: Vec<Capability>,
    pub missing: Vec<Capability>,
    pub compatible: bool,
}

impl CapabilityMatch {
    /// Match `required` against `provider`. Independent of identity.
    pub fn new(required: &[Capability], provider: &CapabilitySet) -> Self {
        let satisfied = required
            .iter()
            .copied()
            .filter(|c| provider.has(c))
            .collect::<Vec<_>>();
        let missing = required
            .iter()
            .copied()
            .filter(|c| !provider.has(c))
            .collect::<Vec<_>>();
        let compatible = missing.is_empty();
        CapabilityMatch {
            required: required.to_vec(),
            provider: provider.iter().collect(),
            satisfied,
            missing,
            compatible,
        }
    }
}

/// Parse a pre-comma-separated capability list into a set.
/// Unknown tokens are rejected (strict, deterministic).
pub fn parse_capabilities(input: &[&str]) -> ProviderRuntimeResult<CapabilitySet> {
    let mut set = CapabilitySet::empty();
    for token in input {
        let cap = Capability::from_str(token)?;
        set.insert(cap);
    }
    Ok(set)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[Capability]) -> CapabilitySet {
        CapabilitySet::new(xs.iter().copied())
    }

    #[test]
    fn test_capability_counts_are_additive() {
        assert_eq!(Capability::all().len(), 10);
    }

    #[test]
    fn test_capability_has_and_has_all() {
        let s = set(&[Capability::Streaming, Capability::ToolCalling]);
        assert!(s.has(&Capability::Streaming));
        assert!(!s.has(&Capability::Vision));
        assert!(s.has_all(&[Capability::Streaming, Capability::ToolCalling]));
        assert!(!s.has_all(&[Capability::Streaming, Capability::Vision]));
    }

    #[test]
    fn test_capability_empty() {
        let s = CapabilitySet::empty();
        assert!(s.is_empty());
        assert_eq!(s.len(), 0);
        assert!(!s.has(&Capability::Audio));
    }

    #[test]
    fn test_capability_insert_extend_remove() {
        let mut s = CapabilitySet::empty();
        s.insert(Capability::Vision);
        assert!(s.has(&Capability::Vision));
        s.extend(vec![Capability::Audio, Capability::JsonMode]);
        assert_eq!(s.len(), 3);
        s.remove(&Capability::Vision);
        assert!(!s.has(&Capability::Vision));
    }

    #[test]
    fn test_intersection_and_union() {
        let a = set(&[Capability::Streaming, Capability::Audio]);
        let b = set(&[Capability::Audio, Capability::Vision]);
        let inter = a.intersection(&b);
        assert!(inter.has(&Capability::Audio));
        assert!(!inter.has(&Capability::Streaming));
        let uni = a.union(&b);
        assert_eq!(uni.len(), 3);
    }

    #[test]
    fn test_capability_match_compatible() {
        let req = vec![Capability::Streaming, Capability::ToolCalling];
        let m = CapabilityMatch::new(&req, &set(&[
            Capability::Streaming,
            Capability::ToolCalling,
            Capability::Vision,
        ]));
        assert!(m.compatible);
        assert!(m.missing.is_empty());
        assert_eq!(m.satisfied.len(), 2);
    }

    #[test]
    fn test_capability_match_incompatible() {
        let req = vec![Capability::Streaming, Capability::ToolCalling];
        let m = CapabilityMatch::new(&req, &set(&[Capability::Streaming]));
        assert!(!m.compatible);
        assert_eq!(m.missing, vec![Capability::ToolCalling]);
    }

    #[test]
    fn test_capability_match_no_requirements() {
        let m = CapabilityMatch::new(&[], &CapabilitySet::empty());
        assert!(m.compatible);
        assert!(m.missing.is_empty());
    }

    #[test]
    fn test_capability_serialization_roundtrip() {
        let caps = set(&[Capability::Streaming, Capability::LongContext]);
        let json = serde_json::to_string(&caps).unwrap();
        let back: CapabilitySet = serde_json::from_str(&json).unwrap();
        assert_eq!(caps, back);
    }

    #[test]
    fn test_parse_capabilities_ok() {
        let set = parse_capabilities(&["streaming", "tool-calling", "long_context"]).unwrap();
        assert!(set.has(&Capability::Streaming));
        assert!(set.has(&Capability::ToolCalling));
        assert!(set.has(&Capability::LongContext));
    }

    #[test]
    fn test_parse_capabilities_unknown_rejected() {
        assert!(parse_capabilities(&["streaming", "no_such_cap"]).is_err());
    }

    #[test]
    fn test_capability_from_str_case_insensitive() {
        assert_eq!("STREAMING".parse::<Capability>().unwrap(), Capability::Streaming);
        assert_eq!("Tool-Calling".parse::<Capability>().unwrap(), Capability::ToolCalling);
    }

    #[test]
    fn test_capability_descriptions() {
        assert!(Capability::LongContext.description().contains("context"));
        assert!(Capability::JsonMode.description().contains("JSON"));
    }

    #[test]
    fn test_iter_is_declaration_ordered() {
        let s = set(&[Capability::Audio, Capability::Streaming, Capability::Vision]);
        let order: Vec<Capability> = s.iter().collect();
        assert_eq!(order[0], Capability::Streaming);
        assert_eq!(order[1], Capability::Vision);
        assert_eq!(order[2], Capability::Audio);
    }
}
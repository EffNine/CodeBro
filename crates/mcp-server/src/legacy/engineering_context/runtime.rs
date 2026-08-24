//! Runtime context — provider, budget, and execution metadata.
//!
//! Also defines the `EngineeringContextProvider` trait for future
//! subsystem integration.

use super::context::EngineeringContext;
use serde::{Deserialize, Serialize};

/// Provider identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderInfo {
    pub name: String,
    pub model: String,
}

impl ProviderInfo {
    pub fn new(name: impl Into<String>, model: impl Into<String>) -> Self {
        ProviderInfo {
            name: name.into(),
            model: model.into(),
        }
    }
}

/// Immutable runtime execution metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RuntimeContext {
    pub provider: Option<ProviderInfo>,
    pub budget_tokens: usize,
    pub max_tokens: usize,
    pub temperature: f64,
    pub seed: Option<u64>,
    pub stream: bool,
}

impl RuntimeContext {
    pub fn new() -> Self {
        RuntimeContext {
            provider: None,
            budget_tokens: 4096,
            max_tokens: 8192,
            temperature: 0.0,
            seed: Some(42),
            stream: false,
        }
    }

    pub fn with_provider(mut self, name: impl Into<String>, model: impl Into<String>) -> Self {
        self.provider = Some(ProviderInfo::new(name, model));
        self
    }

    pub fn with_budget(mut self, tokens: usize) -> Self {
        self.budget_tokens = tokens;
        self
    }

    pub fn with_max_tokens(mut self, tokens: usize) -> Self {
        self.max_tokens = tokens;
        self
    }

    pub fn with_temperature(mut self, temp: f64) -> Self {
        self.temperature = temp;
        self
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    pub fn with_stream(mut self, stream: bool) -> Self {
        self.stream = stream;
        self
    }

    pub fn provider_name(&self) -> Option<&str> {
        self.provider.as_ref().map(|p| p.name.as_str())
    }

    pub fn provider_model(&self) -> Option<&str> {
        self.provider.as_ref().map(|p| p.model.as_str())
    }
}

impl Default for RuntimeContext {
    fn default() -> Self {
        RuntimeContext::new()
    }
}

/// Trait for subsystems that can produce or consume `EngineeringContext`.
///
/// Future modules (Project Identity, Engineering Memory, Task Graph,
/// Reflection, Learning Engine, Planning Engine) may implement this
/// trait to integrate with the runtime contract.
pub trait EngineeringContextProvider {
    /// Returns the subsystem's name for diagnostics.
    fn provider_name(&self) -> &str;

    /// Read the current context without mutating it.
    fn read_context(&self, ctx: &EngineeringContext) -> Result<(), String>;
}

impl std::fmt::Debug for dyn EngineeringContextProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineeringContextProvider")
            .field("name", &self.provider_name())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_runtime() {
        let rt = RuntimeContext::default();
        assert_eq!(rt.budget_tokens, 4096);
        assert_eq!(rt.max_tokens, 8192);
        assert_eq!(rt.temperature, 0.0);
        assert_eq!(rt.seed, Some(42));
        assert!(!rt.stream);
        assert!(rt.provider.is_none());
    }

    #[test]
    fn test_runtime_builder() {
        let rt = RuntimeContext::new()
            .with_provider("openai", "gpt-4")
            .with_budget(2000)
            .with_max_tokens(4000)
            .with_temperature(0.7)
            .with_seed(123)
            .with_stream(true);

        assert_eq!(rt.provider_name(), Some("openai"));
        assert_eq!(rt.provider_model(), Some("gpt-4"));
        assert_eq!(rt.budget_tokens, 2000);
        assert_eq!(rt.temperature, 0.7);
        assert_eq!(rt.seed, Some(123));
        assert!(rt.stream);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let rt = RuntimeContext::new()
            .with_provider("anthropic", "claude-3")
            .with_budget(1000);
        let json = serde_json::to_string(&rt).expect("serialize");
        let decoded: RuntimeContext = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(rt, decoded);
    }

    #[test]
    fn test_provider_trait() {
        #[derive(Debug)]
        struct TestProvider {
            name: String,
        }

        impl EngineeringContextProvider for TestProvider {
            fn provider_name(&self) -> &str {
                &self.name
            }

            fn read_context(&self, _ctx: &EngineeringContext) -> Result<(), String> {
                Ok(())
            }
        }

        let provider = TestProvider {
            name: "test-provider".to_string(),
        };
        assert_eq!(provider.provider_name(), "test-provider");
    }
}

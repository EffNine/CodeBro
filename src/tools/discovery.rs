//! Tool Discovery
//!
//! Discovers available tools from registered providers and builds tool lists.

use super::metadata::ToolDefinition;
use super::provider::ToolProvider;
use std::sync::Arc;

/// A discovered tool with its provider info.
#[derive(Debug, Clone)]
pub struct DiscoveredTool {
    pub definition: ToolDefinition,
    pub provider_name: String,
    pub provider_available: bool,
}

/// Result of a tool discovery operation.
#[derive(Debug, Clone)]
pub struct DiscoveryResult {
    pub tools: Vec<DiscoveredTool>,
    pub providers_discovered: Vec<String>,
    pub providers_unavailable: Vec<String>,
}

impl DiscoveryResult {
    pub fn empty() -> Self {
        DiscoveryResult {
            tools: Vec::new(),
            providers_discovered: Vec::new(),
            providers_unavailable: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools
            .iter()
            .map(|t| t.definition.metadata.name.clone())
            .collect()
    }

    pub fn tools_by_provider(&self, provider: &str) -> Vec<&DiscoveredTool> {
        self.tools
            .iter()
            .filter(|t| t.provider_name == provider)
            .collect()
    }
}

/// Discovers tools from all registered providers.
#[derive(Default)]
pub struct ToolDiscovery {
    providers: Vec<Arc<dyn ToolProvider>>,
}

impl std::fmt::Debug for ToolDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolDiscovery")
            .field("providers", &self.providers.len())
            .finish()
    }
}

impl ToolDiscovery {
    pub fn new() -> Self {
        ToolDiscovery {
            providers: Vec::new(),
        }
    }

    /// Add a provider for discovery.
    pub fn add_provider(&mut self, provider: Arc<dyn ToolProvider>) {
        self.providers.push(provider);
    }

    /// Discover all available tools from all providers.
    pub fn discover(&self) -> DiscoveryResult {
        let mut result = DiscoveryResult::empty();

        for provider in &self.providers {
            let provider_name = provider.provider_name().to_string();
            let available = provider.is_available();

            if available {
                result.providers_discovered.push(provider_name.clone());
                let definitions = provider.discover_tools();
                for def in definitions {
                    result.tools.push(DiscoveredTool {
                        definition: def,
                        provider_name: provider_name.clone(),
                        provider_available: true,
                    });
                }
            } else {
                result.providers_unavailable.push(provider_name.clone());
            }
        }

        result
    }

    /// Get tool names from all providers.
    pub fn tool_names(&self) -> Vec<String> {
        self.discover().tool_names()
    }

    /// Check if a specific tool is available.
    pub fn has_tool(&self, name: &str) -> bool {
        self.discover()
            .tools
            .iter()
            .any(|t| t.definition.metadata.name == name)
    }

    /// Get tools from a specific provider.
    pub fn tools_from_provider(&self, provider: &str) -> Vec<DiscoveredTool> {
        self.discover()
            .tools_by_provider(provider)
            .into_iter()
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::capabilities::ToolCapabilities;

    struct MockProvider {
        name: String,
        available: bool,
        tools: Vec<ToolDefinition>,
    }

    impl MockProvider {
        fn new(name: &str, available: bool) -> Self {
            MockProvider {
                name: name.to_string(),
                available,
                tools: Vec::new(),
            }
        }
    }

    impl ToolProvider for MockProvider {
        fn provider_name(&self) -> &str {
            &self.name
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn discover_tools(&self) -> Vec<ToolDefinition> {
            self.tools.clone()
        }

        fn register_tools(
            &self,
            _registry: &mut crate::dispatcher::ToolRegistry,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        fn health_check(&self) -> crate::tools::ToolHealth {
            crate::tools::ToolHealth::Healthy
        }
    }

    #[test]
    fn test_discovery_empty() {
        let discovery = ToolDiscovery::new();
        let result = discovery.discover();
        assert!(result.is_empty());
        assert!(result.providers_discovered.is_empty());
    }

    #[test]
    fn test_discovery_with_provider() {
        let mut discovery = ToolDiscovery::new();
        let provider = MockProvider::new("test_provider", true);
        discovery.add_provider(Arc::new(provider));

        let result = discovery.discover();
        assert_eq!(result.providers_discovered.len(), 1);
        assert_eq!(result.providers_discovered[0], "test_provider");
    }

    #[test]
    fn test_discovery_unavailable_provider() {
        let mut discovery = ToolDiscovery::new();
        let provider = MockProvider::new("down_provider", false);
        discovery.add_provider(Arc::new(provider));

        let result = discovery.discover();
        assert_eq!(result.providers_unavailable.len(), 1);
        assert!(result.tools.is_empty());
    }

    #[test]
    fn test_has_tool() {
        let mut discovery = ToolDiscovery::new();
        let provider = MockProvider::new("prov", true);
        discovery.add_provider(Arc::new(provider));

        assert!(!discovery.has_tool("nonexistent"));
    }
}

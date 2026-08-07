//! Tool Provider Abstraction
//!
//! Defines the `ToolProvider` trait that all tool sources (built-in, MCP, plugin)
//! must implement. This abstraction enables future expansion without modifying
//! the registry or dispatcher.

use anyhow::Result;
use std::sync::Arc;

use super::diagnostics::ToolHealth;
use super::metadata::ToolDefinition;
use crate::dispatcher::ToolRegistry;

/// A provider of tools. Each provider is responsible for:
/// - Discovering what tools it offers
/// - Registering those tools into a registry
/// - Reporting its own health status
///
/// Implementations include:
/// - `BuiltInProvider`: Registers all tools in `src/tools/`
/// - `McpProvider` (future): Connects to MCP servers
/// - `PluginProvider` (future): Loads tools from plugin files
/// - `ExternalProvider` (future): Wraps external tool executables
pub trait ToolProvider: Send + Sync {
    /// Unique name for this provider (e.g., "builtin", "mcp:github").
    fn provider_name(&self) -> &str;

    /// Semantic version of this provider.
    fn provider_version(&self) -> &str {
        "1.0.0"
    }

    /// Whether this provider is currently available.
    /// Returns false if dependencies are missing, network is unavailable, etc.
    fn is_available(&self) -> bool;

    /// Discover tool definitions offered by this provider.
    /// Called during discovery phase, before registration.
    fn discover_tools(&self) -> Vec<ToolDefinition>;

    /// Register tools from this provider into the given registry.
    /// Called during initialization.
    fn register_tools(&self, registry: &mut ToolRegistry) -> Result<()>;

    /// Health check for this provider.
    fn health_check(&self) -> ToolHealth;

    /// Get a human-readable description of this provider.
    fn description(&self) -> &str {
        "Unknown provider"
    }
}

/// Built-in provider that registers all tools defined in `src/tools/`.
///
/// This is the default provider and is always available.
#[derive(Debug, Clone, Default)]
pub struct BuiltInProvider;

impl ToolProvider for BuiltInProvider {
    fn provider_name(&self) -> &str {
        "builtin"
    }

    fn description(&self) -> &str {
        "Built-in tools (filesystem, shell, git, patch)"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn discover_tools(&self) -> Vec<ToolDefinition> {
        // This is populated at runtime by the registry initialization.
        // Discovery returns what providers know about; registration populates the registry.
        Vec::new()
    }

    fn register_tools(&self, _registry: &mut ToolRegistry) -> Result<()> {
        // Built-in registration happens through the registry's built-in registration methods.
        // This provider acts as a marker/placeholder.
        Ok(())
    }

    fn health_check(&self) -> ToolHealth {
        ToolHealth::Healthy
    }
}

/// Registry of all tool providers.
#[derive(Default)]
pub struct ProviderRegistry {
    providers: Vec<Arc<dyn ToolProvider>>,
}

impl std::fmt::Debug for ProviderRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderRegistry")
            .field("providers", &self.providers.len())
            .finish()
    }
}

impl ProviderRegistry {
    pub fn new() -> Self {
        ProviderRegistry {
            providers: Vec::new(),
        }
    }

    /// Add a provider to the registry.
    pub fn add_provider(&mut self, provider: Arc<dyn ToolProvider>) {
        self.providers.push(provider);
    }

    /// Get all providers.
    pub fn providers(&self) -> &[Arc<dyn ToolProvider>] {
        &self.providers
    }

    /// Get a provider by name.
    pub fn get_provider(&self, name: &str) -> Option<&Arc<dyn ToolProvider>> {
        self.providers.iter().find(|p| p.provider_name() == name)
    }

    /// Register all available providers into a tool registry.
    pub fn register_all(&self, tool_registry: &mut ToolRegistry) -> Result<()> {
        for provider in &self.providers {
            if provider.is_available() {
                tracing::info!(
                    "Registering tools from provider: {}",
                    provider.provider_name()
                );
                provider.register_tools(tool_registry)?;
            } else {
                tracing::warn!(
                    "Provider {} is not available, skipping registration",
                    provider.provider_name()
                );
            }
        }
        Ok(())
    }

    /// Check health of all providers.
    pub fn health_status(&self) -> Vec<(&str, ToolHealth)> {
        self.providers
            .iter()
            .map(|p| (p.provider_name(), p.health_check()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestProvider {
        name: String,
        available: bool,
    }

    impl TestProvider {
        fn new(name: &str, available: bool) -> Self {
            TestProvider {
                name: name.to_string(),
                available,
            }
        }
    }

    impl ToolProvider for TestProvider {
        fn provider_name(&self) -> &str {
            &self.name
        }

        fn is_available(&self) -> bool {
            self.available
        }

        fn discover_tools(&self) -> Vec<ToolDefinition> {
            Vec::new()
        }

        fn register_tools(&self, _registry: &mut ToolRegistry) -> Result<()> {
            Ok(())
        }

        fn health_check(&self) -> ToolHealth {
            ToolHealth::Healthy
        }
    }

    #[test]
    fn test_built_in_provider() {
        let provider = BuiltInProvider::default();
        assert_eq!(provider.provider_name(), "builtin");
        assert!(provider.is_available());
        assert_eq!(provider.health_check(), ToolHealth::Healthy);
    }

    #[test]
    fn test_provider_registry() {
        let mut registry = ProviderRegistry::new();
        registry.add_provider(Arc::new(TestProvider::new("test1", true)));
        registry.add_provider(Arc::new(TestProvider::new("test2", false)));

        assert_eq!(registry.providers().len(), 2);
        assert!(registry.get_provider("test1").is_some());
        assert!(registry.get_provider("nonexistent").is_none());

        let health = registry.health_status();
        assert_eq!(health.len(), 2);
    }

    #[test]
    fn test_provider_registry_register_all() {
        let mut tool_reg = ToolRegistry::new();
        let provider_reg = ProviderRegistry::new();
        // Empty registry should succeed
        let result = provider_reg.register_all(&mut tool_reg);
        assert!(result.is_ok());
    }
}

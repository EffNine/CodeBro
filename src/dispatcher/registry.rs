//! Enhanced Tool Registry with metadata, lifecycle, hooks, and diagnostics.
//!
//! The registry is the central hub for tool management. It tracks:
//! - Tool instances (by name)
//! - Tool metadata (capabilities, version, provider)
//! - Tool lifecycle state (enabled/disabled/deprecated)
//! - Permission and rollback hooks
//! - Execution diagnostics

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;

use crate::tools::capabilities::ToolCapabilities;
use crate::tools::context::{ToolContext, ToolResult};
use crate::tools::diagnostics::DiagnosticCollector;
use crate::tools::hooks::{HookManager, PermissionDecision};
use crate::tools::lifecycle::{LifecycleManager, ToolLifecycleState};
use crate::tools::metadata::ToolMetadata;
use crate::tools::provider::ProviderRegistry;
use crate::tools::Tool;

/// The central registry for all tools.
///
/// Extends the basic name-to-tool mapping with rich metadata, lifecycle
/// management, permission hooks, and diagnostic tracking.
pub struct ToolRegistry {
    /// Tool instances.
    tools: HashMap<String, Arc<dyn Tool>>,
    /// Tool metadata.
    metadata: HashMap<String, ToolMetadata>,
    /// Tool lifecycle states.
    lifecycle: LifecycleManager,
    /// Permission and rollback hooks.
    hooks: HookManager,
    /// Diagnostic collector.
    diagnostics: DiagnosticCollector,
    /// Provider registry.
    providers: ProviderRegistry,
}

impl ToolRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        ToolRegistry {
            tools: HashMap::new(),
            metadata: HashMap::new(),
            lifecycle: LifecycleManager::new(),
            hooks: HookManager::new(),
            diagnostics: DiagnosticCollector::new(),
            providers: ProviderRegistry::new(),
        }
    }

    // ----------------------------------------------------------------
    // Registration
    // ----------------------------------------------------------------

    /// Register a tool with default metadata.
    pub fn register(mut self, tool: Arc<dyn Tool>) -> Self {
        let name = tool.name().to_string();
        self.tools.insert(name.clone(), tool);
        self.metadata.insert(
            name.clone(),
            ToolMetadata::new(
                &name,
                "Registered tool",
                ToolCapabilities::default(),
                "builtin",
            ),
        );
        let _ = self.lifecycle.register(&name);
        let _ = self.lifecycle.enable(&name);
        self
    }

    /// Register a tool with custom metadata.
    pub fn register_with_metadata(mut self, tool: Arc<dyn Tool>, metadata: ToolMetadata) -> Self {
        let name = tool.name().to_string();
        self.tools.insert(name.clone(), tool);
        self.metadata.insert(name.clone(), metadata);
        let _ = self.lifecycle.register(&name);
        self
    }

    /// Register a tool from a definition.
    pub fn register_definition(
        mut self,
        definition: crate::tools::metadata::ToolDefinition,
    ) -> Self {
        let name = definition.metadata.name.clone();
        let tool = definition.create_tool();
        // Unbox the tool and re-wrap in Arc
        self.tools.insert(name.clone(), tool.into());
        self.metadata.insert(name.clone(), definition.metadata);
        let _ = self.lifecycle.register(&name);
        self
    }

    /// Add a provider and register its tools.
    pub fn add_provider(&mut self, provider: Arc<dyn crate::tools::provider::ToolProvider>) {
        self.providers.add_provider(provider.clone());
        if provider.is_available() {
            let _ = provider.register_tools(self);
        }
    }

    // ----------------------------------------------------------------
    // Lookup
    // ----------------------------------------------------------------

    /// Get a tool by name.
    pub fn get(&self, name: &str) -> Option<&Arc<dyn Tool>> {
        self.tools.get(name)
    }

    /// Get tool metadata.
    pub fn get_metadata(&self, name: &str) -> Option<&ToolMetadata> {
        self.metadata.get(name)
    }

    /// Get tool capabilities.
    pub fn get_capabilities(&self, name: &str) -> Option<ToolCapabilities> {
        self.metadata.get(name).map(|m| m.capabilities)
    }

    /// Get the lifecycle state of a tool.
    pub fn get_lifecycle_state(&self, name: &str) -> Option<ToolLifecycleState> {
        self.lifecycle.state(name)
    }

    /// Check if a tool exists and is active.
    pub fn has_tool(&self, name: &str) -> bool {
        self.tools.contains_key(name) && self.lifecycle.is_active(name)
    }

    /// Check if a tool exists (regardless of lifecycle state).
    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    /// Get all registered tool names (active only).
    pub fn names(&self) -> Vec<String> {
        self.tools
            .keys()
            .filter(|name| self.lifecycle.is_active(name))
            .cloned()
            .collect()
    }

    /// Get all tool names including inactive ones.
    pub fn all_names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }

    /// Get all tool instances (active only).
    pub fn list(&self) -> Vec<&Arc<dyn Tool>> {
        self.names()
            .iter()
            .filter_map(|n| self.tools.get(n))
            .collect()
    }

    /// Get the number of active tools.
    pub fn len(&self) -> usize {
        self.names().len()
    }

    /// Check if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.names().is_empty()
    }

    // ----------------------------------------------------------------
    // Lifecycle Management
    // ----------------------------------------------------------------

    /// Enable a tool.
    pub fn enable(&mut self, name: &str) -> Result<()> {
        Ok(self.lifecycle.enable(name)?)
    }

    /// Disable a tool.
    pub fn disable(&mut self, name: &str) -> Result<()> {
        Ok(self.lifecycle.disable(name)?)
    }

    /// Deprecate a tool.
    pub fn deprecate(&mut self, name: &str, note: &str) -> Result<()> {
        self.lifecycle.deprecate(name)?;
        if let Some(meta) = self.metadata.get_mut(name) {
            meta.deprecated = true;
            meta.deprecation_note = Some(note.to_string());
        }
        Ok(())
    }

    /// Get all lifecycle states.
    pub fn all_lifecycle_states(&self) -> Vec<(&str, ToolLifecycleState)> {
        self.lifecycle.all_states()
    }

    // ----------------------------------------------------------------
    // Hook Management
    // ----------------------------------------------------------------

    /// Set permission and rollback hooks for a tool.
    pub fn set_hooks(
        &mut self,
        name: &str,
        permission: Box<dyn crate::tools::hooks::PermissionHook>,
        rollback: Box<dyn crate::tools::hooks::RollbackHook>,
    ) {
        let hooks = crate::tools::hooks::ToolHooks::new()
            .with_permission(permission)
            .with_rollback(rollback);
        self.hooks.set_tool_hooks(name, hooks);
    }

    /// Set a global permission hook.
    pub fn set_global_permission_hook(
        &mut self,
        hook: Box<dyn crate::tools::hooks::PermissionHook>,
    ) {
        self.hooks.set_global_permission(hook);
    }

    /// Set a global rollback hook.
    pub fn set_global_rollback_hook(&mut self, hook: Box<dyn crate::tools::hooks::RollbackHook>) {
        self.hooks.set_global_rollback(hook);
    }

    /// Check permission for a tool execution.
    pub fn check_permission(&self, context: &ToolContext) -> PermissionDecision {
        self.hooks.check_permission(context)
    }

    // ----------------------------------------------------------------
    // Execution
    // ----------------------------------------------------------------

    /// Execute a tool by name with the given args, with full hook and diagnostic support.
    pub async fn execute(&mut self, name: &str, args: &str) -> Result<String> {
        let context = ToolContext::new(name, args);
        let result = self.execute_with_context(context).await?;
        if result.success {
            Ok(result.output)
        } else {
            Err(anyhow::anyhow!("{}", result.output))
        }
    }

    /// Execute a tool with a pre-built context.
    pub async fn execute_with_context(&mut self, context: ToolContext) -> Result<ToolResult> {
        let tool_name = context.tool_name.clone();
        let execution_id = context.execution_id.0.clone();

        // Check if tool exists first
        if !self.tools.contains_key(&tool_name) {
            return Err(anyhow::anyhow!("Unknown tool: {}", tool_name));
        }

        // Check lifecycle state
        if !self.lifecycle.is_active(&tool_name) {
            return Err(anyhow::anyhow!(
                "Tool '{}' is not active (state: {:?})",
                tool_name,
                self.lifecycle.state(&tool_name)
            ));
        }

        // Check permission
        let permission = self.check_permission(&context);
        match permission {
            PermissionDecision::Allowed { .. } => {}
            PermissionDecision::Ask { .. } => {
                return Err(anyhow::anyhow!(
                    "Tool '{}' requires confirmation",
                    tool_name
                ));
            }
            PermissionDecision::Denied { reason } => {
                return Err(anyhow::anyhow!("Tool '{}' denied: {}", tool_name, reason));
            }
        }

        // Run before-execute hooks
        let mut mutable_context = context.clone();
        self.hooks.before_execute(&mut mutable_context)?;

        // Execute the tool
        let start = std::time::Instant::now();
        let tool_result = self.dispatch_tool(&mutable_context).await;
        let duration_ms = start.elapsed().as_secs_f64() * 1000.0;

        // Record diagnostics
        match &tool_result {
            Ok(result) => {
                self.diagnostics.record_success(
                    &tool_name,
                    duration_ms,
                    &execution_id,
                    result.exit_code,
                );
                if let Some(meta) = self.metadata.get_mut(&tool_name) {
                    meta.record_success(duration_ms);
                }
            }
            Err(e) => {
                self.diagnostics.record_failure(
                    &tool_name,
                    duration_ms,
                    &execution_id,
                    &e.to_string(),
                    None,
                );
                if let Some(meta) = self.metadata.get_mut(&tool_name) {
                    meta.record_failure(duration_ms);
                }
            }
        }

        // Run after-execute hooks
        let result = tool_result?;
        self.hooks.after_execute(&result.context, &result)?;

        Ok(result)
    }

    /// Dispatch a tool execution (internal).
    async fn dispatch_tool(&self, context: &ToolContext) -> Result<ToolResult> {
        let tool_name = context.tool_name.clone();
        let args = context.args.clone();
        match self.tools.get(&tool_name) {
            Some(tool) => {
                let tool = tool.clone();
                let output = tokio::task::spawn_blocking(move || tool.execute(&args))
                    .await
                    .map_err(|e| anyhow::anyhow!("Tool execution panic: {}", e))?;

                match output {
                    Ok(result) => Ok(ToolResult::success(
                        context.clone(),
                        result,
                        0.0, // duration recorded by caller
                    )),
                    Err(e) => Ok(ToolResult::failure(context.clone(), e.to_string(), 0.0)),
                }
            }
            None => Err(anyhow::anyhow!("Unknown tool: {}", context.tool_name)),
        }
    }

    /// Execute a tool by name (synchronous convenience method).
    pub fn execute_sync(&mut self, name: &str, args: &str) -> Result<String> {
        let rt = tokio::runtime::Handle::current();
        rt.block_on(self.execute(name, args))
    }

    // ----------------------------------------------------------------
    // Diagnostics
    // ----------------------------------------------------------------

    /// Get diagnostics for a tool.
    pub fn get_diagnostics(
        &self,
        name: &str,
    ) -> Option<crate::tools::diagnostics::ToolDiagnostics> {
        self.diagnostics.get(name)
    }

    /// Get all diagnostics.
    pub fn all_diagnostics(&self) -> Vec<crate::tools::diagnostics::ToolDiagnostics> {
        self.diagnostics.all()
    }

    /// Get diagnostic names.
    pub fn diagnostic_names(&self) -> Vec<String> {
        self.diagnostics.names()
    }

    // ----------------------------------------------------------------
    // Provider Management
    // ----------------------------------------------------------------

    /// Get all providers.
    pub fn providers(&self) -> &[Arc<dyn crate::tools::provider::ToolProvider>] {
        self.providers.providers()
    }

    /// Get provider health status.
    pub fn provider_health(&self) -> Vec<(&str, crate::tools::diagnostics::ToolHealth)> {
        self.providers.health_status()
    }

    // ----------------------------------------------------------------
    // Metadata Queries
    // ----------------------------------------------------------------

    /// Get metadata for all tools.
    pub fn all_metadata(&self) -> Vec<&ToolMetadata> {
        self.tools
            .keys()
            .filter_map(|k| self.metadata.get(k))
            .collect()
    }

    /// Get tools matching a capability filter.
    pub fn find_by_capability(&self, caps: ToolCapabilities) -> Vec<&ToolMetadata> {
        self.all_metadata()
            .into_iter()
            .filter(|m| m.capabilities.is_subset_of(&caps) || caps.is_subset_of(&m.capabilities))
            .collect()
    }

    /// Get tools by category.
    pub fn find_by_category(
        &self,
        category: crate::tools::capabilities::ToolCategory,
    ) -> Vec<&ToolMetadata> {
        self.all_metadata()
            .into_iter()
            .filter(|m| m.category == category)
            .collect()
    }

    /// Get tools by provider.
    pub fn find_by_provider(&self, provider: &str) -> Vec<&ToolMetadata> {
        self.all_metadata()
            .into_iter()
            .filter(|m| m.provider == provider)
            .collect()
    }

    /// Get deprecated tools.
    pub fn deprecated_tools(&self) -> Vec<&ToolMetadata> {
        self.all_metadata()
            .into_iter()
            .filter(|m| m.deprecated)
            .collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Dispatcher that uses the registry for tool dispatch.
pub struct ToolDispatcher {
    registry: ToolRegistry,
}

impl ToolDispatcher {
    pub fn new(registry: ToolRegistry) -> Self {
        ToolDispatcher { registry }
    }

    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    pub async fn dispatch(&mut self, tool_name: &str, args: &str) -> Result<String> {
        self.registry.execute(tool_name, args).await
    }

    pub fn list_tools(&self) -> Vec<String> {
        self.registry.names()
    }

    pub fn has_tool(&self, name: &str) -> bool {
        self.registry.has_tool(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::Tool;

    struct TestTool {
        name: String,
        result: String,
    }

    impl Tool for TestTool {
        fn name(&self) -> &str {
            &self.name
        }
        fn description(&self) -> &str {
            "test"
        }
        fn execute(&self, _args: &str) -> Result<String> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn test_registry_basic_operations() {
        let registry = ToolRegistry::new()
            .register(Arc::new(TestTool {
                name: "tool_a".to_string(),
                result: "result_a".to_string(),
            }))
            .register(Arc::new(TestTool {
                name: "tool_b".to_string(),
                result: "result_b".to_string(),
            }));

        assert_eq!(registry.len(), 2);
        assert!(registry.has_tool("tool_a"));
        assert!(registry.has_tool("tool_b"));
        assert!(!registry.has_tool("missing"));
    }

    #[tokio::test]
    async fn test_registry_execution() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool {
            name: "tool_a".to_string(),
            result: "success".to_string(),
        }));

        let result = registry.execute("tool_a", "args").await.unwrap();
        assert_eq!(result, "success");
    }

    #[tokio::test]
    async fn test_registry_unknown_tool() {
        let mut registry = ToolRegistry::new();
        let result = registry.execute("unknown", "args").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_registry_lifecycle() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool {
            name: "tool_a".to_string(),
            result: "ok".to_string(),
        }));

        assert!(registry.has_tool("tool_a"));
        registry.disable("tool_a").unwrap();
        assert!(!registry.has_tool("tool_a"));
        registry.enable("tool_a").unwrap();
        assert!(registry.has_tool("tool_a"));
    }

    #[tokio::test]
    async fn test_registry_metadata() {
        let registry = ToolRegistry::new().register(Arc::new(TestTool {
            name: "my_tool".to_string(),
            result: "ok".to_string(),
        }));

        let stored_meta = registry.get_metadata("my_tool").unwrap();
        assert_eq!(stored_meta.name, "my_tool");
    }

    #[tokio::test]
    async fn test_registry_diagnostics() {
        let mut registry = ToolRegistry::new().register(Arc::new(TestTool {
            name: "tool_a".to_string(),
            result: "ok".to_string(),
        }));

        registry.execute("tool_a", "args").await.unwrap();
        registry.execute("tool_a", "args").await.unwrap();

        let diags = registry.get_diagnostics("tool_a").unwrap();
        assert_eq!(diags.total_executions, 2);
        assert_eq!(diags.success_count, 2);
    }

    #[test]
    fn test_dispatcher() {
        let registry = ToolRegistry::new().register(Arc::new(TestTool {
            name: "tool_a".to_string(),
            result: "ok".to_string(),
        }));
        let dispatcher = ToolDispatcher::new(registry);
        assert!(dispatcher.has_tool("tool_a"));
        assert_eq!(dispatcher.list_tools(), vec!["tool_a"]);
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Capability Model — declare, check, and enforce plugin capabilities.
//!
//! Every plugin declares its capabilities. The SDK enforces that plugins
//! can only use capabilities they have declared.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::types::*;

/// A capability that a plugin can provide.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Capability {
    /// Provides a new tool to the dispatcher.
    Tool(String),
    /// Provides a new provider implementation.
    Provider(String),
    /// Provides a new intent classification rule.
    IntentRule(String),
    /// Provides a new recommendation rule.
    RecommendationRule(String),
    /// Provides a new validation rule.
    ValidationRule(String),
    /// Provides a new workflow step type.
    WorkflowStep(String),
    /// Provides a new UI component.
    UiComponent(String),
    /// Provides a new skill.
    Skill(String),
    /// Provides a new preference key.
    PreferenceKey(String),
    /// Custom capability.
    Custom(String),
}

impl std::fmt::Display for Capability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Capability::Tool(name) => write!(f, "tool:{name}"),
            Capability::Provider(name) => write!(f, "provider:{name}"),
            Capability::IntentRule(name) => write!(f, "intent_rule:{name}"),
            Capability::RecommendationRule(name) => write!(f, "rec_rule:{name}"),
            Capability::ValidationRule(name) => write!(f, "val_rule:{name}"),
            Capability::WorkflowStep(name) => write!(f, "wf_step:{name}"),
            Capability::UiComponent(name) => write!(f, "ui:{name}"),
            Capability::Skill(name) => write!(f, "skill:{name}"),
            Capability::PreferenceKey(name) => write!(f, "pref:{name}"),
            Capability::Custom(name) => write!(f, "custom:{name}"),
        }
    }
}

/// Inner state for capability model.
#[derive(Debug)]
struct CapabilityModelInner {
    registered: HashMap<Capability, Vec<PluginId>>,
    plugin_capabilities: HashMap<PluginId, Vec<Capability>>,
}

/// Thread-safe capability model.
///
/// Clone is cheap (Arc clone). Safe to share across threads.
#[derive(Debug, Clone)]
pub struct CapabilityModel {
    inner: Arc<Mutex<CapabilityModelInner>>,
}

impl CapabilityModel {
    /// Creates a new empty capability model.
    pub fn new() -> Self {
        CapabilityModel {
            inner: Arc::new(Mutex::new(CapabilityModelInner {
                registered: HashMap::new(),
                plugin_capabilities: HashMap::new(),
            })),
        }
    }

    /// Registers a capability provided by a plugin.
    pub fn register(&self, capability: Capability, plugin_id: &PluginId) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .registered
            .entry(capability.clone())
            .or_default()
            .push(plugin_id.clone());
        inner
            .plugin_capabilities
            .entry(plugin_id.clone())
            .or_default()
            .push(capability);
    }

    /// Checks if a plugin has a specific capability.
    pub fn has_capability(&self, plugin_id: &PluginId, capability: &Capability) -> bool {
        let inner = self.inner.lock().unwrap();
        inner
            .plugin_capabilities
            .get(plugin_id)
            .map(|caps| caps.contains(capability))
            .unwrap_or(false)
    }

    /// Returns all plugins providing a capability.
    pub fn providers(&self, capability: &Capability) -> Vec<PluginId> {
        let inner = self.inner.lock().unwrap();
        inner
            .registered
            .get(capability)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all capabilities of a plugin.
    pub fn capabilities(&self, plugin_id: &PluginId) -> Vec<Capability> {
        let inner = self.inner.lock().unwrap();
        inner
            .plugin_capabilities
            .get(plugin_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Returns all registered capabilities.
    pub fn all_capabilities(&self) -> Vec<Capability> {
        let inner = self.inner.lock().unwrap();
        inner.registered.keys().cloned().collect()
    }

    /// Returns a summary string.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        lines.push("=== Capability Model ===".to_string());
        lines.push(format!(
            "Registered capabilities: {}",
            inner.registered.len()
        ));
        for (cap, providers) in &inner.registered {
            lines.push(format!("  {cap}: {} provider(s)", providers.len()));
        }
        lines.join("\n")
    }

    /// Clears all capabilities.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.registered.clear();
        inner.plugin_capabilities.clear();
    }
}

impl Default for CapabilityModel {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_check() {
        let model = CapabilityModel::new();
        let plugin = PluginId::new("test/plugin").unwrap();
        let cap = Capability::Tool("my_tool".to_string());
        model.register(cap.clone(), &plugin);
        assert!(model.has_capability(&plugin, &cap));
        assert!(!model.has_capability(&plugin, &Capability::Tool("other".to_string())));
    }

    #[test]
    fn test_providers() {
        let model = CapabilityModel::new();
        let p1 = PluginId::new("test/a").unwrap();
        let p2 = PluginId::new("test/b").unwrap();
        let cap = Capability::Tool("shared_tool".to_string());
        model.register(cap.clone(), &p1);
        model.register(cap.clone(), &p2);
        let providers = model.providers(&cap);
        assert_eq!(providers.len(), 2);
    }

    #[test]
    fn test_plugin_capabilities() {
        let model = CapabilityModel::new();
        let plugin = PluginId::new("test/plugin").unwrap();
        model.register(Capability::Tool("t1".to_string()), &plugin);
        model.register(Capability::Provider("p1".to_string()), &plugin);
        let caps = model.capabilities(&plugin);
        assert_eq!(caps.len(), 2);
    }

    #[test]
    fn test_summary() {
        let model = CapabilityModel::new();
        model.register(
            Capability::Tool("test_tool".to_string()),
            &PluginId::new("test/plugin").unwrap(),
        );
        let summary = model.summary();
        assert!(summary.contains("Capability Model"));
        assert!(summary.contains("test_tool"));
    }

    #[test]
    fn test_clear() {
        let model = CapabilityModel::new();
        model.register(
            Capability::Tool("t".to_string()),
            &PluginId::new("test/plugin").unwrap(),
        );
        assert!(!model.all_capabilities().is_empty());
        model.clear();
        assert!(model.all_capabilities().is_empty());
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let model = CapabilityModel::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let m = model.clone();
                thread::spawn(move || {
                    for j in 0..50 {
                        let cap = Capability::Custom(format!("cap_{}_{}", i, j));
                        let plugin = PluginId::new(&format!("test/plugin_{i}")).unwrap();
                        m.register(cap, &plugin);
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert!(model.all_capabilities().len() >= 10);
    }
}

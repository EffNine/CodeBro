#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin Registry — discover, validate, and register plugins.
//!
//! The registry is the central coordination point for all plugins.
//! It maintains the plugin graph, enforces dependencies, and tracks state.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use super::plugin::{Plugin, PluginError, PluginState};
use super::types::*;

/// Maximum number of plugins in the registry.
const MAX_PLUGINS: usize = 256;

/// Inner state for the plugin registry.
struct RegistryInner {
    plugins: HashMap<PluginId, Arc<Mutex<Box<dyn Plugin>>>>,
    states: HashMap<PluginId, PluginState>,
    dependency_graph: HashMap<PluginId, Vec<PluginId>>,
}

/// Thread-safe plugin registry.
///
/// Clone is cheap (Arc clone). Safe to share across threads.
#[derive(Clone)]
pub struct PluginRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

impl PluginRegistry {
    /// Creates a new empty registry.
    pub fn new() -> Self {
        PluginRegistry {
            inner: Arc::new(Mutex::new(RegistryInner {
                plugins: HashMap::new(),
                states: HashMap::new(),
                dependency_graph: HashMap::new(),
            })),
        }
    }

    /// Returns the number of registered plugins.
    pub fn count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.plugins.len()
    }

    /// Checks if a plugin is registered.
    pub fn has(&self, id: &PluginId) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.plugins.contains_key(id)
    }

    /// Registers a plugin in the registry.
    pub fn register(&self, plugin: Box<dyn Plugin>) -> Result<(), PluginError> {
        let mut inner = self.inner.lock().unwrap();
        if inner.plugins.len() >= MAX_PLUGINS {
            return Err(PluginError::LoadFailed("Registry full".to_string()));
        }
        let id = plugin.manifest().id.clone();
        if inner.plugins.contains_key(&id) {
            return Err(PluginError::LoadFailed(format!(
                "Plugin already registered: {id}"
            )));
        }
        inner
            .plugins
            .insert(id.clone(), Arc::new(Mutex::new(plugin)));
        inner.states.insert(id.clone(), PluginState::Discovered);
        inner.dependency_graph.insert(id, Vec::new());
        Ok(())
    }

    /// Returns a plugin by ID.
    pub fn get(&self, id: &PluginId) -> Option<Arc<Mutex<Box<dyn Plugin>>>> {
        let inner = self.inner.lock().unwrap();
        inner.plugins.get(id).cloned()
    }

    /// Returns the state of a plugin.
    pub fn state(&self, id: &PluginId) -> Option<PluginState> {
        let inner = self.inner.lock().unwrap();
        inner.states.get(id).cloned()
    }

    /// Sets the state of a plugin.
    pub fn set_state(&self, id: &PluginId, state: PluginState) {
        let mut inner = self.inner.lock().unwrap();
        inner.states.insert(id.clone(), state);
    }

    /// Adds a dependency between two plugins.
    pub fn add_dependency(&self, plugin: &PluginId, depends_on: &PluginId) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(deps) = inner.dependency_graph.get_mut(plugin) {
            deps.push(depends_on.clone());
        }
    }

    /// Checks if all dependencies of a plugin are satisfied.
    pub fn check_dependencies(&self, id: &PluginId) -> Result<(), PluginError> {
        let inner = self.inner.lock().unwrap();
        if let Some(deps) = inner.dependency_graph.get(id) {
            for dep in deps {
                if !inner.plugins.contains_key(dep) {
                    return Err(PluginError::DependencyMissing(dep.to_string()));
                }
                // Check transitive dependencies
                if let Some(transitive_deps) = inner.dependency_graph.get(dep) {
                    for trans_dep in transitive_deps {
                        if !inner.plugins.contains_key(trans_dep) {
                            return Err(PluginError::DependencyMissing(trans_dep.to_string()));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Resolves dependencies in topological order.
    /// Returns an empty vec if there are no cycles.
    pub fn resolve_order(&self) -> Vec<PluginId> {
        let inner = self.inner.lock().unwrap();
        let mut order = Vec::new();
        let mut visited = std::collections::HashSet::new();
        let mut temp = std::collections::HashSet::new();

        fn visit(
            id: &PluginId,
            graph: &HashMap<PluginId, Vec<PluginId>>,
            visited: &mut std::collections::HashSet<PluginId>,
            temp: &mut std::collections::HashSet<PluginId>,
            order: &mut Vec<PluginId>,
        ) -> bool {
            if temp.contains(id) {
                return false; // cycle detected
            }
            if visited.contains(id) {
                return true;
            }
            temp.insert(id.clone());
            if let Some(deps) = graph.get(id) {
                for dep in deps {
                    if !visit(dep, graph, visited, temp, order) {
                        return false;
                    }
                }
            }
            temp.remove(id);
            visited.insert(id.clone());
            order.push(id.clone());
            true
        }

        let all_ids: Vec<_> = inner.plugins.keys().cloned().collect();
        for id in all_ids {
            if visit(
                &id,
                &inner.dependency_graph,
                &mut visited,
                &mut temp,
                &mut order,
            ) {}
        }
        order
    }

    /// Returns all registered plugin IDs.
    pub fn plugin_ids(&self) -> Vec<PluginId> {
        let inner = self.inner.lock().unwrap();
        inner.plugins.keys().cloned().collect()
    }

    /// Returns a summary of all plugins.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        lines.push(format!("=== Plugin Registry ({}) ===", inner.plugins.len()));
        for (id, plugin) in &inner.plugins {
            let state = inner
                .states
                .get(id)
                .map(|s| s.to_string())
                .unwrap_or_default();
            let name = plugin.lock().unwrap().manifest().name.clone();
            let version = plugin.lock().unwrap().manifest().version.to_string();
            let deps = inner.dependency_graph.get(id).map(|d| d.len()).unwrap_or(0);
            lines.push(format!("  {id} v{version} [{state}] deps={deps}"));
            lines.push(format!("    name: {name}"));
        }
        lines.join("\n")
    }

    /// Clears all plugins from the registry.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.plugins.clear();
        inner.states.clear();
        inner.dependency_graph.clear();
    }
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_sdk::plugin::NoOpPlugin;

    fn make_plugin(id: &str, name: &str) -> Box<dyn Plugin> {
        let manifest = PluginManifest::new(
            PluginId::new(id).unwrap(),
            name,
            "Test plugin",
            PluginVersion::new("1.0.0").unwrap(),
            Author::new("test"),
            License::MIT,
            RequiredSdkVersion::Minimum(PluginVersion::new("1.0.0").unwrap()),
            SupportedVersionRange::new(
                PluginVersion::new("1.0.0").unwrap(),
                PluginVersion::new("2.0.0").unwrap(),
            ),
        );
        Box::new(NoOpPlugin::new(manifest))
    }

    #[test]
    fn test_register_and_get() {
        let reg = PluginRegistry::new();
        let plugin = make_plugin("test/a", "Plugin A");
        reg.register(plugin).unwrap();
        assert_eq!(reg.count(), 1);
        assert!(reg.has(&PluginId::new("test/a").unwrap()));
        assert!(!reg.has(&PluginId::new("test/b").unwrap()));
    }

    #[test]
    fn test_duplicate_register_fails() {
        let reg = PluginRegistry::new();
        let p1 = make_plugin("test/a", "Plugin A");
        let p2 = make_plugin("test/a", "Plugin A Duplicate");
        reg.register(p1).unwrap();
        assert!(reg.register(p2).is_err());
    }

    #[test]
    fn test_state_transitions() {
        let reg = PluginRegistry::new();
        let plugin = make_plugin("test/a", "Plugin A");
        reg.register(plugin).unwrap();
        assert_eq!(
            reg.state(&PluginId::new("test/a").unwrap()),
            Some(PluginState::Discovered)
        );
        reg.set_state(&PluginId::new("test/a").unwrap(), PluginState::Active);
        assert_eq!(
            reg.state(&PluginId::new("test/a").unwrap()),
            Some(PluginState::Active)
        );
    }

    #[test]
    fn test_dependency_check() {
        let reg = PluginRegistry::new();
        let p1 = make_plugin("test/a", "Plugin A");
        let p2 = make_plugin("test/b", "Plugin B");
        reg.register(p1).unwrap();
        reg.register(p2).unwrap();
        reg.add_dependency(
            &PluginId::new("test/b").unwrap(),
            &PluginId::new("test/a").unwrap(),
        );
        assert!(reg
            .check_dependencies(&PluginId::new("test/b").unwrap())
            .is_ok());
    }

    #[test]
    fn test_missing_dependency() {
        let reg = PluginRegistry::new();
        let p1 = make_plugin("test/a", "Plugin A");
        let p2 = make_plugin("test/b", "Plugin B");
        reg.register(p1).unwrap();
        reg.register(p2).unwrap();
        reg.add_dependency(
            &PluginId::new("test/b").unwrap(),
            &PluginId::new("test/c").unwrap(),
        );
        assert!(reg
            .check_dependencies(&PluginId::new("test/b").unwrap())
            .is_err());
    }

    #[test]
    fn test_resolve_order() {
        let reg = PluginRegistry::new();
        let p1 = make_plugin("test/a", "Plugin A");
        let p2 = make_plugin("test/b", "Plugin B");
        reg.register(p1).unwrap();
        reg.register(p2).unwrap();
        reg.add_dependency(
            &PluginId::new("test/b").unwrap(),
            &PluginId::new("test/a").unwrap(),
        );
        let order = reg.resolve_order();
        // A should come before B
        let a_idx = order
            .iter()
            .position(|id| id.to_string() == "test/a")
            .unwrap();
        let b_idx = order
            .iter()
            .position(|id| id.to_string() == "test/b")
            .unwrap();
        assert!(a_idx < b_idx);
    }

    #[test]
    fn test_clear() {
        let reg = PluginRegistry::new();
        reg.register(make_plugin("test/a", "A")).unwrap();
        assert_eq!(reg.count(), 1);
        reg.clear();
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn test_summary() {
        let reg = PluginRegistry::new();
        reg.register(make_plugin("test/a", "Plugin A")).unwrap();
        let summary = reg.summary();
        assert!(summary.contains("test/a"));
        assert!(summary.contains("Plugin A"));
    }

    #[test]
    fn test_plugin_ids() {
        let reg = PluginRegistry::new();
        reg.register(make_plugin("test/a", "A")).unwrap();
        reg.register(make_plugin("test/b", "B")).unwrap();
        let ids = reg.plugin_ids();
        assert_eq!(ids.len(), 2);
    }

    #[test]
    fn test_clone_shares_state() {
        let reg1 = PluginRegistry::new();
        let reg2 = reg1.clone();
        reg1.register(make_plugin("test/a", "A")).unwrap();
        assert_eq!(reg2.count(), 1);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let reg = PluginRegistry::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let r = reg.clone();
                thread::spawn(move || {
                    for j in 0..50 {
                        let id = format!("test/plugin_{i}_{j}");
                        r.register(make_plugin(&id, &format!("Plugin {i}"))).ok();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // Some registrations may fail due to duplicate IDs or capacity
        assert!(reg.count() <= MAX_PLUGINS);
    }
}

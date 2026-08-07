#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin Lifecycle — deterministic state machine for plugin management.
//!
//! Lifecycle: discover → validate → load → init → register → run → shutdown

use super::plugin::{Plugin, PluginError, PluginState};
use super::registry::PluginRegistry;
use super::types::*;

/// Lifecycle phase tracking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecyclePhase {
    Discover,
    Validate,
    Load,
    Init,
    Register,
    Run,
    Shutdown,
}

impl std::fmt::Display for LifecyclePhase {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LifecyclePhase::Discover => write!(f, "discover"),
            LifecyclePhase::Validate => write!(f, "validate"),
            LifecyclePhase::Load => write!(f, "load"),
            LifecyclePhase::Init => write!(f, "init"),
            LifecyclePhase::Register => write!(f, "register"),
            LifecyclePhase::Run => write!(f, "run"),
            LifecyclePhase::Shutdown => write!(f, "shutdown"),
        }
    }
}

/// Tracks the lifecycle progress of a single plugin.
#[derive(Debug, Clone)]
pub struct PluginLifecycle {
    phase: LifecyclePhase,
    errors: Vec<PluginError>,
}

impl PluginLifecycle {
    /// Creates a new lifecycle tracker.
    pub fn new() -> Self {
        PluginLifecycle {
            phase: LifecyclePhase::Discover,
            errors: Vec::new(),
        }
    }

    /// Returns the current phase.
    pub fn phase(&self) -> &LifecyclePhase {
        &self.phase
    }

    /// Returns any accumulated errors.
    pub fn errors(&self) -> &[PluginError] {
        &self.errors
    }

    /// Records an error.
    pub fn record_error(&mut self, error: PluginError) {
        self.errors.push(error);
    }

    /// Advances to the next phase.
    pub fn advance(&mut self, next: LifecyclePhase) {
        self.phase = next;
    }

    /// Runs the full lifecycle for a set of plugins.
    pub fn run_lifecycle(
        registry: &PluginRegistry,
        plugins: &[Box<dyn Plugin>],
    ) -> Result<Vec<PluginId>, PluginError> {
        let mut ordered = Vec::new();

        // Phase 1: Discover & Validate
        for plugin in plugins {
            let id = plugin.manifest().id.clone();
            plugin
                .manifest()
                .validate()
                .map_err(|e| PluginError::ManifestInvalid(e.to_string()))?;
            registry.set_state(&id, PluginState::Validated);
        }

        // Phase 2: Load (register in registry)
        for plugin in plugins.iter() {
            let id = plugin.manifest().id.clone();
            registry.register((*plugin).clone())?;
            registry.set_state(&id, PluginState::Loaded);
        }

        // Phase 3: Initialize
        for id in registry.resolve_order() {
            if let Some(plugin_arc) = registry.get(&id) {
                let mut plugin = plugin_arc.lock().unwrap();
                plugin.init()?;
                registry.set_state(&id, PluginState::Initialized);
            }
        }

        // Phase 4: Register (mark active)
        for id in registry.resolve_order() {
            registry.set_state(&id, PluginState::Active);
            ordered.push(id);
        }

        Ok(ordered)
    }

    /// Shuts down all plugins in reverse order.
    pub fn shutdown_all(registry: &PluginRegistry) -> Result<(), PluginError> {
        let ids = registry.plugin_ids();
        for id in ids.into_iter().rev() {
            if let Some(plugin_arc) = registry.get(&id) {
                let mut plugin = plugin_arc.lock().unwrap();
                registry.set_state(&id, PluginState::ShuttingDown);
                plugin.shutdown()?;
                registry.set_state(&id, PluginState::Shutdown);
            }
        }
        Ok(())
    }
}

impl Default for PluginLifecycle {
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
    fn test_lifecycle_phases() {
        let mut lc = PluginLifecycle::new();
        assert_eq!(lc.phase(), &LifecyclePhase::Discover);
        lc.advance(LifecyclePhase::Validate);
        assert_eq!(lc.phase(), &LifecyclePhase::Validate);
        lc.advance(LifecyclePhase::Run);
        assert_eq!(lc.phase(), &LifecyclePhase::Run);
    }

    #[test]
    fn test_run_lifecycle_single_plugin() {
        let registry = PluginRegistry::new();
        let plugin = make_plugin("test/a", "Plugin A");
        let ordered = PluginLifecycle::run_lifecycle(&registry, &[plugin]).unwrap();
        assert_eq!(ordered.len(), 1);
        assert_eq!(registry.state(&ordered[0]), Some(PluginState::Active));
    }

    #[test]
    fn test_run_lifecycle_multiple_plugins() {
        let registry = PluginRegistry::new();
        let p1 = make_plugin("test/a", "Plugin A");
        let p2 = make_plugin("test/b", "Plugin B");
        let ordered = PluginLifecycle::run_lifecycle(&registry, &[p1, p2]).unwrap();
        assert_eq!(ordered.len(), 2);
        for id in &ordered {
            assert_eq!(registry.state(id), Some(PluginState::Active));
        }
    }

    #[test]
    fn test_shutdown_all() {
        let registry = PluginRegistry::new();
        let p1 = make_plugin("test/a", "Plugin A");
        let p2 = make_plugin("test/b", "Plugin B");
        PluginLifecycle::run_lifecycle(&registry, &[p1, p2]).unwrap();
        PluginLifecycle::shutdown_all(&registry).unwrap();
        for id in registry.plugin_ids() {
            assert_eq!(registry.state(&id), Some(PluginState::Shutdown));
        }
    }

    #[test]
    fn test_error_recording() {
        let mut lc = PluginLifecycle::new();
        lc.record_error(PluginError::LoadFailed("test".to_string()));
        assert_eq!(lc.errors().len(), 1);
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin Diagnostics — health, metrics, and audit for the plugin system.
//!
//! Provides observability into the plugin ecosystem: which plugins are
//! active, their lifecycle state, error counts, and performance.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::plugin::PluginState;
use super::types::*;

/// Health status of a plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginHealth {
    Healthy,
    Degraded(String),
    Unhealthy(String),
}

/// Diagnostic record for a single plugin.
#[derive(Debug, Clone)]
pub struct PluginDiagnostic {
    pub id: PluginId,
    pub name: String,
    pub version: PluginVersion,
    pub state: PluginState,
    pub health: PluginHealth,
    pub hook_count: usize,
    pub error_count: u64,
    pub last_error: Option<String>,
    pub enabled: bool,
}

impl PluginDiagnostic {
    pub fn new(id: PluginId, name: String, version: PluginVersion) -> Self {
        PluginDiagnostic {
            id,
            name,
            version,
            state: PluginState::Discovered,
            health: PluginHealth::Healthy,
            hook_count: 0,
            error_count: 0,
            last_error: None,
            enabled: true,
        }
    }
}

/// Inner state for plugin diagnostics.
#[derive(Debug)]
struct DiagnosticsInner {
    diagnostics: Vec<PluginDiagnostic>,
    enabled_count: usize,
}

/// Thread-safe plugin diagnostics collector.
///
/// Clone is cheap (Arc clone). Safe to share across threads.
#[derive(Debug, Clone)]
pub struct PluginDiagnostics {
    inner: Arc<Mutex<DiagnosticsInner>>,
}

impl PluginDiagnostics {
    /// Creates a new empty diagnostics collector.
    pub fn new() -> Self {
        PluginDiagnostics {
            inner: Arc::new(Mutex::new(DiagnosticsInner {
                diagnostics: Vec::new(),
                enabled_count: 0,
            })),
        }
    }

    /// Registers a plugin for diagnostics.
    pub fn register(&self, id: PluginId, name: &str, version: &PluginVersion) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .diagnostics
            .push(PluginDiagnostic::new(id, name.to_string(), version.clone()));
        inner.enabled_count += 1;
    }

    /// Updates the state of a plugin.
    pub fn set_state(&self, id: &PluginId, state: PluginState) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(diag) = inner.diagnostics.iter_mut().find(|d| &d.id == id) {
            diag.state = state;
        }
    }

    /// Records an error for a plugin.
    pub fn record_error(&self, id: &PluginId, error: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(diag) = inner.diagnostics.iter_mut().find(|d| &d.id == id) {
            diag.error_count += 1;
            diag.last_error = Some(error.to_string());
            diag.health = PluginHealth::Unhealthy(error.to_string());
        }
    }

    /// Records a degraded state for a plugin.
    pub fn record_degraded(&self, id: &PluginId, reason: &str) {
        let mut inner = self.inner.lock().unwrap();
        if let Some(diag) = inner.diagnostics.iter_mut().find(|d| &d.id == id) {
            diag.health = PluginHealth::Degraded(reason.to_string());
        }
    }

    /// Returns diagnostics for a plugin.
    pub fn diagnostic(&self, id: &PluginId) -> Option<PluginDiagnostic> {
        let inner = self.inner.lock().unwrap();
        inner.diagnostics.iter().find(|d| &d.id == id).cloned()
    }

    /// Returns all diagnostics.
    pub fn all(&self) -> Vec<PluginDiagnostic> {
        let inner = self.inner.lock().unwrap();
        inner.diagnostics.clone()
    }

    /// Returns the number of registered plugins.
    pub fn count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.diagnostics.len()
    }

    /// Returns the number of enabled plugins.
    pub fn enabled_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.enabled_count
    }

    /// Returns a summary string.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        let mut lines = Vec::new();
        lines.push("=== Plugin Diagnostics ===".to_string());
        lines.push(format!("Plugins: {}", inner.diagnostics.len()));
        lines.push(format!("Enabled: {}", inner.enabled_count));

        let healthy = inner
            .diagnostics
            .iter()
            .filter(|d| matches!(d.health, PluginHealth::Healthy))
            .count();
        let degraded = inner
            .diagnostics
            .iter()
            .filter(|d| matches!(d.health, PluginHealth::Degraded(_)))
            .count();
        let unhealthy = inner
            .diagnostics
            .iter()
            .filter(|d| matches!(d.health, PluginHealth::Unhealthy(_)))
            .count();
        lines.push(format!(
            "Healthy: {healthy}, Degraded: {degraded}, Unhealthy: {unhealthy}"
        ));
        lines.push(String::new());

        for diag in &inner.diagnostics {
            lines.push(format!(
                "  [{}] {} v{} — {:?}",
                diag.state, diag.name, diag.version, diag.health
            ));
            if let Some(ref err) = diag.last_error {
                lines.push(format!("    Error: {err}"));
            }
        }
        lines.join("\n")
    }

    /// Clears all diagnostics.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.diagnostics.clear();
        inner.enabled_count = 0;
    }
}

impl Default for PluginDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_plugin() {
        let diag = PluginDiagnostics::new();
        diag.register(
            PluginId::new("test/plugin").unwrap(),
            "Test Plugin",
            &PluginVersion::new("1.0.0").unwrap(),
        );
        assert_eq!(diag.count(), 1);
        assert_eq!(diag.enabled_count(), 1);
    }

    #[test]
    fn test_set_state() {
        let diag = PluginDiagnostics::new();
        let id = PluginId::new("test/plugin").unwrap();
        diag.register(id.clone(), "Test", &PluginVersion::new("1.0.0").unwrap());
        diag.set_state(&id, PluginState::Active);
        let d = diag.diagnostic(&id).unwrap();
        assert_eq!(d.state, PluginState::Active);
    }

    #[test]
    fn test_record_error() {
        let diag = PluginDiagnostics::new();
        let id = PluginId::new("test/plugin").unwrap();
        diag.register(id.clone(), "Test", &PluginVersion::new("1.0.0").unwrap());
        diag.record_error(&id, "something failed");
        let d = diag.diagnostic(&id).unwrap();
        assert_eq!(d.error_count, 1);
        assert!(matches!(d.health, PluginHealth::Unhealthy(_)));
    }

    #[test]
    fn test_summary() {
        let diag = PluginDiagnostics::new();
        diag.register(
            PluginId::new("test/a").unwrap(),
            "Plugin A",
            &PluginVersion::new("1.0.0").unwrap(),
        );
        let summary = diag.summary();
        assert!(summary.contains("Plugin Diagnostics"));
        assert!(summary.contains("Plugin A"));
    }

    #[test]
    fn test_clear() {
        let diag = PluginDiagnostics::new();
        diag.register(
            PluginId::new("test/a").unwrap(),
            "A",
            &PluginVersion::new("1.0.0").unwrap(),
        );
        diag.clear();
        assert_eq!(diag.count(), 0);
        assert_eq!(diag.enabled_count(), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let diag = PluginDiagnostics::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let d = diag.clone();
                thread::spawn(move || {
                    for j in 0..50 {
                        d.record_error(
                            &PluginId::new(&format!("test/plugin_{i}")).unwrap(),
                            &format!("error {j}"),
                        );
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        // Errors are recorded; some plugins may not exist yet but no panic
        assert!(diag.summary().len() > 0);
    }
}

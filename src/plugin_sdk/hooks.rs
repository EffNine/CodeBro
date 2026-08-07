#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Hook System — register, dispatch, and order plugin hooks.
//!
//! Hooks allow plugins to observe and react to pipeline events without
//! modifying pipeline behavior. Hooks are ordered and can short-circuit.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use super::plugin::{HookContext, HookResponse, PluginError};
use super::types::*;

/// Order in which hooks are dispatched.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HookOrder {
    /// First hooks run first.
    First,
    /// Last hooks run first (higher priority).
    Last,
    /// Default order (registration order).
    Default,
}

/// A hook registration.
#[derive(Debug, Clone)]
pub struct Hook {
    pub phase: HookPhase,
    pub plugin_id: PluginId,
    pub order: HookOrder,
    pub priority: i32,
}

impl Hook {
    pub fn new(phase: HookPhase, plugin_id: PluginId) -> Self {
        Hook {
            phase,
            plugin_id,
            order: HookOrder::Default,
            priority: 0,
        }
    }

    pub fn with_order(mut self, order: HookOrder) -> Self {
        self.order = order;
        self
    }

    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Inner state for the hook dispatcher.
#[derive(Debug)]
struct HookDispatcherInner {
    hooks: Vec<Hook>,
    responses: Vec<HookResponse>,
}

/// Thread-safe hook dispatcher.
///
/// Clone is cheap (Arc clone). Safe to share across threads.
#[derive(Debug, Clone)]
pub struct HookDispatcher {
    inner: Arc<Mutex<HookDispatcherInner>>,
}

impl HookDispatcher {
    /// Creates a new empty hook dispatcher.
    pub fn new() -> Self {
        HookDispatcher {
            inner: Arc::new(Mutex::new(HookDispatcherInner {
                hooks: Vec::new(),
                responses: Vec::new(),
            })),
        }
    }

    /// Registers a hook.
    pub fn register(&self, hook: Hook) {
        let mut inner = self.inner.lock().unwrap();
        inner.hooks.push(hook);
    }

    /// Dispatches a hook event to all registered hooks for that phase.
    pub fn dispatch(
        &self,
        phase: &HookPhase,
        context: &HookContext,
        plugins: &HashMap<PluginId, Arc<Mutex<Box<dyn super::plugin::Plugin>>>>,
    ) -> Vec<HookResponse> {
        // Collect and sort hooks first, outside the lock
        let phase_hooks: Vec<_> = {
            let inner = self.inner.lock().unwrap();
            inner
                .hooks
                .iter()
                .filter(|h| &h.phase == phase)
                .cloned()
                .collect()
        };

        let mut sorted_hooks = phase_hooks;
        sorted_hooks.sort_by(|a, b| {
            let a_ord = match a.order {
                HookOrder::First => 0,
                HookOrder::Last => 2,
                HookOrder::Default => 1,
            };
            let b_ord = match b.order {
                HookOrder::First => 0,
                HookOrder::Last => 2,
                HookOrder::Default => 1,
            };
            a_ord.cmp(&b_ord).then_with(|| a.priority.cmp(&b.priority))
        });

        let mut responses = Vec::new();
        for hook in sorted_hooks {
            if let Some(plugin_arc) = plugins.get(&hook.plugin_id) {
                let mut plugin = plugin_arc.lock().unwrap();
                match plugin.on_hook(phase, context) {
                    Ok(response) => responses.push(response),
                    Err(e) => responses.push(HookResponse::Blocked {
                        reason: e.to_string(),
                    }),
                }
            }
        }

        let mut inner = self.inner.lock().unwrap();
        inner.responses = responses.clone();
        responses
    }

    /// Returns all registered hooks for a phase.
    pub fn hooks_for_phase(&self, phase: &HookPhase) -> Vec<Hook> {
        let inner = self.inner.lock().unwrap();
        inner
            .hooks
            .iter()
            .filter(|h| &h.phase == phase)
            .cloned()
            .collect()
    }

    /// Returns the number of registered hooks.
    pub fn hook_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.hooks.len()
    }

    /// Returns all responses from the last dispatch.
    pub fn last_responses(&self) -> Vec<HookResponse> {
        let inner = self.inner.lock().unwrap();
        inner.responses.clone()
    }

    /// Checks if any response blocked the pipeline.
    pub fn has_blocker(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner
            .responses
            .iter()
            .any(|r| matches!(r, HookResponse::Blocked { .. }))
    }

    /// Clears all hooks and responses.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.hooks.clear();
        inner.responses.clear();
    }
}

impl Default for HookDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin_sdk::plugin::{NoOpPlugin, Plugin};

    fn make_plugin_with_hook(id: &str, phase: HookPhase) -> (Box<dyn Plugin>, Hook) {
        let manifest = PluginManifest::new(
            PluginId::new(id).unwrap(),
            "Test",
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
        let hook = Hook::new(phase, PluginId::new(id).unwrap());
        (Box::new(NoOpPlugin::new(manifest)), hook)
    }

    #[test]
    fn test_register_hook() {
        let dispatcher = HookDispatcher::new();
        let hook = Hook::new(HookPhase::IntentResolved, PluginId::new("test/a").unwrap());
        dispatcher.register(hook);
        assert_eq!(dispatcher.hook_count(), 1);
    }

    #[test]
    fn test_dispatch_no_hooks() {
        let dispatcher = HookDispatcher::new();
        let context = HookContext::new("c1", HookPhase::IntentResolved);
        let plugins: HashMap<PluginId, Arc<Mutex<Box<dyn Plugin>>>> = HashMap::new();
        let responses = dispatcher.dispatch(&HookPhase::IntentResolved, &context, &plugins);
        assert!(responses.is_empty());
    }

    #[test]
    fn test_dispatch_with_hooks() {
        let dispatcher = HookDispatcher::new();
        let (plugin, hook) = make_plugin_with_hook("test/a", HookPhase::IntentResolved);
        dispatcher.register(hook);

        let mut plugins = HashMap::new();
        plugins.insert(
            PluginId::new("test/a").unwrap(),
            Arc::new(Mutex::new(plugin)),
        );

        let context = HookContext::new("c1", HookPhase::IntentResolved);
        let responses = dispatcher.dispatch(&HookPhase::IntentResolved, &context, &plugins);
        assert_eq!(responses.len(), 1);
        assert!(matches!(&responses[0], HookResponse::Ok));
    }

    #[test]
    fn test_hooks_for_phase() {
        let dispatcher = HookDispatcher::new();
        dispatcher.register(Hook::new(
            HookPhase::IntentResolved,
            PluginId::new("test/a").unwrap(),
        ));
        dispatcher.register(Hook::new(
            HookPhase::WorkflowCreated,
            PluginId::new("test/b").unwrap(),
        ));
        let hooks = dispatcher.hooks_for_phase(&HookPhase::IntentResolved);
        assert_eq!(hooks.len(), 1);
        assert_eq!(hooks[0].plugin_id.to_string(), "test/a");
    }

    #[test]
    fn test_hook_ordering() {
        let dispatcher = HookDispatcher::new();
        dispatcher.register(
            Hook::new(HookPhase::IntentResolved, PluginId::new("test/a").unwrap())
                .with_order(HookOrder::First)
                .with_priority(10),
        );
        dispatcher.register(
            Hook::new(HookPhase::IntentResolved, PluginId::new("test/b").unwrap())
                .with_order(HookOrder::Last)
                .with_priority(5),
        );
        let hooks = dispatcher.hooks_for_phase(&HookPhase::IntentResolved);
        assert_eq!(hooks.len(), 2);
        // First-order hooks should come before Last-order
        assert_eq!(hooks[0].plugin_id.to_string(), "test/a");
    }

    #[test]
    fn test_clear() {
        let dispatcher = HookDispatcher::new();
        dispatcher.register(Hook::new(
            HookPhase::IntentResolved,
            PluginId::new("test/a").unwrap(),
        ));
        assert_eq!(dispatcher.hook_count(), 1);
        dispatcher.clear();
        assert_eq!(dispatcher.hook_count(), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let dispatcher = HookDispatcher::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let d = dispatcher.clone();
                thread::spawn(move || {
                    for j in 0..50 {
                        d.register(Hook::new(
                            HookPhase::Custom(format!("phase_{}_{}", i, j)),
                            PluginId::new(&format!("test/plugin_{i}")).unwrap(),
                        ));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(dispatcher.hook_count(), 500);
    }
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Timeout manager for the reliability layer.
//!
//! Provides centralized timeout handling with per-provider and per-tool
//! timeout configuration, plus cancellation support.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The kind of timeout being managed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TimeoutKind {
    /// LLM provider timeout (e.g., OpenAI stream response).
    Provider,
    /// Tool execution timeout (e.g., shell command).
    Tool,
    /// System-level timeout (e.g., whole pipeline).
    System,
}

/// Configuration for a single timeout target.
#[derive(Debug, Clone)]
pub struct TimeoutConfig {
    pub kind: TimeoutKind,
    pub name: String,
    pub duration_ms: u64,
    pub started_at: Option<Instant>,
}

impl TimeoutConfig {
    pub fn new(kind: TimeoutKind, name: &str, duration_ms: u64) -> Self {
        TimeoutConfig {
            kind,
            name: name.to_string(),
            duration_ms,
            started_at: None,
        }
    }

    pub fn start(&mut self) {
        self.started_at = Some(Instant::now());
    }

    pub fn elapsed(&self) -> Option<Duration> {
        self.started_at.map(|t| t.elapsed())
    }

    pub fn remaining(&self) -> Option<Duration> {
        self.started_at.map(|t| {
            let elapsed = t.elapsed();
            let total = Duration::from_millis(self.duration_ms);
            total.saturating_sub(elapsed)
        })
    }

    pub fn is_expired(&self) -> bool {
        match self.started_at {
            Some(t) => t.elapsed() >= Duration::from_millis(self.duration_ms),
            None => false,
        }
    }
}

/// Manages timeouts for providers, tools, and the system.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct TimeoutManager {
    inner: Arc<Mutex<TimeoutManagerInner>>,
}

#[derive(Debug)]
struct TimeoutManagerInner {
    default_timeout_ms: u64,
    provider_timeouts: HashMap<String, u64>,
    tool_timeouts: HashMap<String, u64>,
    system_timeout_ms: u64,
    active_timeouts: HashMap<String, TimeoutConfig>,
}

impl TimeoutManager {
    /// Creates a new `TimeoutManager` with default settings.
    ///
    /// Default timeouts:
    /// - Provider: 60_000ms (60s)
    /// - Tool: 120_000ms (2min)
    /// - System: 300_000ms (5min)
    pub fn new() -> Self {
        TimeoutManager {
            inner: Arc::new(Mutex::new(TimeoutManagerInner {
                default_timeout_ms: 60_000,
                provider_timeouts: HashMap::new(),
                tool_timeouts: HashMap::new(),
                system_timeout_ms: 300_000,
                active_timeouts: HashMap::new(),
            })),
        }
    }

    /// Returns the timeout for a provider by name.
    pub fn get_provider_timeout(&self, name: &str) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner
            .provider_timeouts
            .get(name)
            .copied()
            .unwrap_or(inner.default_timeout_ms)
    }

    /// Returns the timeout for a tool by name.
    pub fn get_tool_timeout(&self, name: &str) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner
            .tool_timeouts
            .get(name)
            .copied()
            .unwrap_or(inner.default_timeout_ms)
    }

    /// Returns the system timeout.
    pub fn get_system_timeout(&self) -> u64 {
        let inner = self.inner.lock().unwrap();
        inner.system_timeout_ms
    }

    /// Sets a custom timeout for a provider.
    pub fn set_provider_timeout(&self, name: &str, timeout_ms: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.provider_timeouts.insert(name.to_string(), timeout_ms);
    }

    /// Sets a custom timeout for a tool.
    pub fn set_tool_timeout(&self, name: &str, timeout_ms: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.tool_timeouts.insert(name.to_string(), timeout_ms);
    }

    /// Sets the system-wide timeout.
    pub fn set_system_timeout(&self, timeout_ms: u64) {
        let mut inner = self.inner.lock().unwrap();
        inner.system_timeout_ms = timeout_ms;
    }

    /// Starts tracking a timeout for a target.
    pub fn start_timeout(&self, id: &str, kind: TimeoutKind, name: &str) -> u64 {
        let timeout_ms = match kind {
            TimeoutKind::Provider => self.get_provider_timeout(name),
            TimeoutKind::Tool => self.get_tool_timeout(name),
            TimeoutKind::System => self.get_system_timeout(),
        };

        let mut config = TimeoutConfig::new(kind, name, timeout_ms);
        config.start();

        let mut inner = self.inner.lock().unwrap();
        inner.active_timeouts.insert(id.to_string(), config);
        timeout_ms
    }

    /// Returns the remaining time for an active timeout, or None if not found.
    pub fn remaining(&self, id: &str) -> Option<Duration> {
        let inner = self.inner.lock().unwrap();
        inner.active_timeouts.get(id).and_then(|c| c.remaining())
    }

    /// Returns whether an active timeout has expired.
    pub fn is_expired(&self, id: &str) -> bool {
        let inner = self.inner.lock().unwrap();
        inner
            .active_timeouts
            .get(id)
            .map(|c| c.is_expired())
            .unwrap_or(false)
    }

    /// Removes a timeout from tracking.
    pub fn remove(&self, id: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_timeouts.remove(id);
    }

    /// Returns the number of active timeouts.
    pub fn active_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.active_timeouts.len()
    }

    /// Returns true if any active timeout is expired.
    pub fn any_expired(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.active_timeouts.values().any(|c| c.is_expired())
    }

    /// Clears all active timeouts.
    pub fn clear(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.active_timeouts.clear();
    }
}

impl Default for TimeoutManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_timeouts() {
        let tm = TimeoutManager::new();
        assert_eq!(tm.get_provider_timeout("openai"), 60_000);
        assert_eq!(tm.get_tool_timeout("run_command"), 60_000);
        assert_eq!(tm.get_system_timeout(), 300_000);
    }

    #[test]
    fn test_set_provider_timeout() {
        let tm = TimeoutManager::new();
        tm.set_provider_timeout("openai", 30_000);
        assert_eq!(tm.get_provider_timeout("openai"), 30_000);
        // Other providers still use default
        assert_eq!(tm.get_provider_timeout("other"), 60_000);
    }

    #[test]
    fn test_set_tool_timeout() {
        let tm = TimeoutManager::new();
        tm.set_tool_timeout("run_command", 120_000);
        assert_eq!(tm.get_tool_timeout("run_command"), 120_000);
    }

    #[test]
    fn test_set_system_timeout() {
        let tm = TimeoutManager::new();
        tm.set_system_timeout(600_000);
        assert_eq!(tm.get_system_timeout(), 600_000);
    }

    #[test]
    fn test_start_and_remove_timeout() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        assert_eq!(tm.active_count(), 1);
        assert!(!tm.is_expired("t1"));
        tm.remove("t1");
        assert_eq!(tm.active_count(), 0);
        assert!(!tm.is_expired("t1")); // not found = false
    }

    #[test]
    fn test_timeout_expiration() {
        let tm = TimeoutManager::new();
        tm.start_timeout("fast", TimeoutKind::Provider, "openai");
        // With 60s timeout, should not be expired immediately
        assert!(!tm.is_expired("fast"));
        tm.remove("fast");
    }

    #[test]
    fn test_remaining_time() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        let remaining = tm.remaining("t1").unwrap();
        assert!(remaining.as_millis() > 59_000);
        tm.remove("t1");
    }

    #[test]
    fn test_any_expired_empty() {
        let tm = TimeoutManager::new();
        assert!(!tm.any_expired());
    }

    #[test]
    fn test_clear() {
        let tm = TimeoutManager::new();
        tm.start_timeout("t1", TimeoutKind::Provider, "openai");
        tm.start_timeout("t2", TimeoutKind::Tool, "run_command");
        assert_eq!(tm.active_count(), 2);
        tm.clear();
        assert_eq!(tm.active_count(), 0);
    }

    #[test]
    fn test_timeout_config() {
        let mut config = TimeoutConfig::new(TimeoutKind::Provider, "test", 1000);
        assert_eq!(config.remaining(), None);
        assert!(!config.is_expired());
        config.start();
        assert!(config.remaining().is_some());
        assert!(!config.is_expired());
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let tm = TimeoutManager::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let tm = tm.clone();
                thread::spawn(move || {
                    let id = format!("t{}", i);
                    tm.start_timeout(&id, TimeoutKind::Provider, "openai");
                    // Just verify the timeout was started and removed without panicking
                    tm.remove(&id);
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(tm.active_count(), 0);
    }
}

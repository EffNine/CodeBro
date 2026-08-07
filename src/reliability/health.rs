#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Health monitoring for the reliability layer.
//!
//// Tracks the health status of providers, tools, the runtime, and resources.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The health status of a monitored component.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Component is operating normally.
    Healthy,
    /// Component is degraded (intermittent failures).
    Degraded,
    /// Component is unhealthy (consistent failures).
    Unhealthy,
    /// Component has no health data yet.
    Unknown,
}

/// The type of component being monitored.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HealthTarget {
    /// An LLM provider (e.g., "openai").
    Provider(String),
    /// A tool (e.g., "run_command").
    Tool(String),
    /// The runtime system itself.
    Runtime,
    /// System resources (memory, CPU).
    Resources,
}

/// Health data for a single component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthEntry {
    pub status: HealthStatus,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub total_failures: u32,
    pub total_successes: u32,
    pub last_failure_time: Option<String>,
    pub last_success_time: Option<String>,
}

impl HealthEntry {
    pub fn new() -> Self {
        HealthEntry {
            status: HealthStatus::Unknown,
            consecutive_failures: 0,
            consecutive_successes: 0,
            total_failures: 0,
            total_successes: 0,
            last_failure_time: None,
            last_success_time: None,
        }
    }

    pub fn record_success(&mut self) {
        self.consecutive_successes += 1;
        self.consecutive_failures = 0;
        self.total_successes += 1;
        self.last_success_time = Some(chrono::Local::now().to_rfc3339());
        self.update_status();
    }

    pub fn record_failure(&mut self) {
        self.consecutive_failures += 1;
        self.consecutive_successes = 0;
        self.total_failures += 1;
        self.last_failure_time = Some(chrono::Local::now().to_rfc3339());
        self.update_status();
    }

    fn update_status(&mut self) {
        if self.consecutive_failures >= 5 {
            self.status = HealthStatus::Unhealthy;
        } else if self.consecutive_failures >= 2 {
            self.status = HealthStatus::Degraded;
        } else if self.total_successes >= 3 && self.consecutive_successes >= 3 {
            self.status = HealthStatus::Healthy;
        } else if self.status == HealthStatus::Unknown && self.total_failures == 0 {
            self.status = HealthStatus::Healthy;
        }
    }
}

impl Default for HealthEntry {
    fn default() -> Self {
        Self::new()
    }
}

/// Monitors the health of providers, tools, runtime, and resources.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct HealthMonitor {
    inner: Arc<Mutex<HealthMonitorInner>>,
}

#[derive(Debug)]
struct HealthMonitorInner {
    providers: HashMap<String, HealthEntry>,
    tools: HashMap<String, HealthEntry>,
    runtime: HealthEntry,
    resources: HealthEntry,
    degradation_threshold: u32,
    recovery_threshold: u32,
}

impl HealthMonitor {
    /// Creates a new `HealthMonitor` with default settings.
    pub fn new() -> Self {
        HealthMonitor {
            inner: Arc::new(Mutex::new(HealthMonitorInner {
                providers: HashMap::new(),
                tools: HashMap::new(),
                runtime: HealthEntry::new(),
                resources: HealthEntry::new(),
                degradation_threshold: 2,
                recovery_threshold: 3,
            })),
        }
    }

    /// Returns the health status of a provider.
    pub fn check_provider(&self, name: &str) -> HealthStatus {
        let inner = self.inner.lock().unwrap();
        inner
            .providers
            .get(name)
            .map(|e| e.status.clone())
            .unwrap_or(HealthStatus::Unknown)
    }

    /// Returns the health status of a tool.
    pub fn check_tool(&self, name: &str) -> HealthStatus {
        let inner = self.inner.lock().unwrap();
        inner
            .tools
            .get(name)
            .map(|e| e.status.clone())
            .unwrap_or(HealthStatus::Unknown)
    }

    /// Returns the runtime health status.
    pub fn check_runtime(&self) -> HealthStatus {
        let inner = self.inner.lock().unwrap();
        inner.runtime.status.clone()
    }

    /// Returns the resource health status.
    pub fn check_resources(&self) -> HealthStatus {
        let inner = self.inner.lock().unwrap();
        inner.resources.status.clone()
    }

    /// Records a success for a provider.
    pub fn record_provider_success(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .providers
            .entry(name.to_string())
            .or_insert_with(HealthEntry::new)
            .record_success();
    }

    /// Records a failure for a provider.
    pub fn record_provider_failure(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .providers
            .entry(name.to_string())
            .or_insert_with(HealthEntry::new)
            .record_failure();
    }

    /// Records a success for a tool.
    pub fn record_tool_success(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .tools
            .entry(name.to_string())
            .or_insert_with(HealthEntry::new)
            .record_success();
    }

    /// Records a failure for a tool.
    pub fn record_tool_failure(&self, name: &str) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .tools
            .entry(name.to_string())
            .or_insert_with(HealthEntry::new)
            .record_failure();
    }

    /// Records a success for the runtime.
    pub fn record_runtime_success(&self) {
        self.inner.lock().unwrap().runtime.record_success();
    }

    /// Records a failure for the runtime.
    pub fn record_runtime_failure(&self) {
        self.inner.lock().unwrap().runtime.record_failure();
    }

    /// Records a success for resources.
    pub fn record_resources_success(&self) {
        self.inner.lock().unwrap().resources.record_success();
    }

    /// Records a failure for resources.
    pub fn record_resources_failure(&self) {
        self.inner.lock().unwrap().resources.record_failure();
    }

    /// Returns the health entry for a provider (for detailed inspection).
    pub fn get_provider_entry(&self, name: &str) -> Option<HealthEntry> {
        let inner = self.inner.lock().unwrap();
        inner.providers.get(name).cloned()
    }

    /// Returns the health entry for a tool (for detailed inspection).
    pub fn get_tool_entry(&self, name: &str) -> Option<HealthEntry> {
        let inner = self.inner.lock().unwrap();
        inner.tools.get(name).cloned()
    }

    /// Returns the count of known providers.
    pub fn provider_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.providers.len()
    }

    /// Returns the count of known tools.
    pub fn tool_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.tools.len()
    }

    /// Returns true if any monitored component is unhealthy.
    pub fn is_system_healthy(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.runtime.status != HealthStatus::Unhealthy
            && inner.resources.status != HealthStatus::Unhealthy
            && inner
                .providers
                .values()
                .all(|e| e.status != HealthStatus::Unhealthy)
            && inner
                .tools
                .values()
                .all(|e| e.status != HealthStatus::Unhealthy)
    }

    /// Returns the number of degraded or unhealthy components.
    pub fn unhealthy_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        let runtime_unhealthy = matches!(
            inner.runtime.status,
            HealthStatus::Degraded | HealthStatus::Unhealthy
        );
        let resources_unhealthy = matches!(
            inner.resources.status,
            HealthStatus::Degraded | HealthStatus::Unhealthy
        );
        let provider_unhealthy = inner
            .providers
            .values()
            .filter(|e| matches!(e.status, HealthStatus::Degraded | HealthStatus::Unhealthy))
            .count();
        let tool_unhealthy = inner
            .tools
            .values()
            .filter(|e| matches!(e.status, HealthStatus::Degraded | HealthStatus::Unhealthy))
            .count();
        runtime_unhealthy as usize
            + resources_unhealthy as usize
            + provider_unhealthy
            + tool_unhealthy
    }
}

impl Default for HealthMonitor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_unknown_status_initially() {
        let hm = HealthMonitor::new();
        assert_eq!(hm.check_provider("openai"), HealthStatus::Unknown);
        assert_eq!(hm.check_tool("run_command"), HealthStatus::Unknown);
        assert_eq!(hm.check_runtime(), HealthStatus::Unknown);
        assert_eq!(hm.check_resources(), HealthStatus::Unknown);
    }

    #[test]
    fn test_provider_becomes_healthy_after_successes() {
        let hm = HealthMonitor::new();
        hm.record_provider_success("openai");
        hm.record_provider_success("openai");
        hm.record_provider_success("openai");
        assert_eq!(hm.check_provider("openai"), HealthStatus::Healthy);
    }

    #[test]
    fn test_provider_becomes_degraded_after_failures() {
        let hm = HealthMonitor::new();
        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        assert_eq!(hm.check_provider("openai"), HealthStatus::Degraded);
    }

    #[test]
    fn test_provider_becomes_unhealthy_after_many_failures() {
        let hm = HealthMonitor::new();
        for _ in 0..5 {
            hm.record_provider_failure("openai");
        }
        assert_eq!(hm.check_provider("openai"), HealthStatus::Unhealthy);
    }

    #[test]
    fn test_tool_health_tracking() {
        let hm = HealthMonitor::new();
        hm.record_tool_success("run_command");
        hm.record_tool_success("run_command");
        hm.record_tool_success("run_command");
        assert_eq!(hm.check_tool("run_command"), HealthStatus::Healthy);

        hm.record_tool_failure("run_command");
        hm.record_tool_failure("run_command");
        assert_eq!(hm.check_tool("run_command"), HealthStatus::Degraded);
    }

    #[test]
    fn test_runtime_health() {
        let hm = HealthMonitor::new();
        hm.record_runtime_success();
        hm.record_runtime_success();
        hm.record_runtime_success();
        assert_eq!(hm.check_runtime(), HealthStatus::Healthy);
    }

    #[test]
    fn test_resources_health() {
        let hm = HealthMonitor::new();
        hm.record_resources_success();
        hm.record_resources_success();
        hm.record_resources_success();
        assert_eq!(hm.check_resources(), HealthStatus::Healthy);
    }

    #[test]
    fn test_success_resets_failure_streak() {
        let hm = HealthMonitor::new();
        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        assert_eq!(hm.check_provider("openai"), HealthStatus::Degraded);

        // A success resets the streak
        hm.record_provider_success("openai");
        hm.record_provider_success("openai");
        hm.record_provider_success("openai");
        assert_eq!(hm.check_provider("openai"), HealthStatus::Healthy);
    }

    #[test]
    fn test_is_system_healthy() {
        let hm = HealthMonitor::new();
        assert!(hm.is_system_healthy());

        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        hm.record_provider_failure("openai");
        assert!(!hm.is_system_healthy());
    }

    #[test]
    fn test_unhealthy_count() {
        let hm = HealthMonitor::new();
        assert_eq!(hm.unhealthy_count(), 0);

        hm.record_provider_failure("p1");
        hm.record_provider_failure("p1");
        hm.record_provider_failure("p2");
        hm.record_provider_failure("p2");
        hm.record_provider_failure("p2");

        assert_eq!(hm.unhealthy_count(), 2); // p1 degraded, p2 unhealthy
    }

    #[test]
    fn test_get_provider_entry() {
        let hm = HealthMonitor::new();
        hm.record_provider_failure("openai");
        let entry = hm.get_provider_entry("openai").unwrap();
        assert_eq!(entry.consecutive_failures, 1);
        assert_eq!(entry.total_failures, 1);
        assert!(entry.last_failure_time.is_some());
    }

    #[test]
    fn test_get_tool_entry() {
        let hm = HealthMonitor::new();
        hm.record_tool_success("read_file");
        let entry = hm.get_tool_entry("read_file").unwrap();
        assert_eq!(entry.consecutive_successes, 1);
        assert_eq!(entry.total_successes, 1);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let hm = HealthMonitor::new();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let hm = hm.clone();
                thread::spawn(move || {
                    for _ in 0..10 {
                        hm.record_provider_success(&format!("provider_{}", i));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(hm.provider_count(), 10);
        for i in 0..10 {
            assert_eq!(
                hm.check_provider(&format!("provider_{}", i)),
                HealthStatus::Healthy
            );
        }
    }
}

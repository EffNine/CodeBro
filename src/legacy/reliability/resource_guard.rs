#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Resource guard for the reliability layer.
//!
/// Provides memory limits, operation limits, and safe shutdown support.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Status of the resource guard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ResourceStatus {
    /// All resources are within limits.
    OK,
    /// Memory usage is high (warning threshold).
    MemoryWarning,
    /// Memory usage has exceeded the limit.
    MemoryLimitExceeded,
    /// Operation count has exceeded the limit.
    OperationLimitExceeded,
    /// Shutdown has been requested.
    ShutdownRequested,
}

/// Configuration for the resource guard.
#[derive(Debug, Clone)]
pub struct ResourceGuardConfig {
    pub memory_limit_mb: usize,
    pub operation_limit: usize,
    pub memory_warning_threshold: f32,
}

impl Default for ResourceGuardConfig {
    fn default() -> Self {
        ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        }
    }
}

/// Guards system resources and enforces limits.
///
/// Thread-safe: can be shared across tasks via `Arc`.
#[derive(Debug, Clone)]
pub struct ResourceGuard {
    inner: Arc<Mutex<ResourceGuardInner>>,
}

#[derive(Debug)]
struct ResourceGuardInner {
    config: ResourceGuardConfig,
    current_memory_mb: usize,
    operations_count: usize,
    shutdown_requested: bool,
    shutdown_requested_at: Option<Instant>,
}

impl ResourceGuard {
    /// Creates a new `ResourceGuard` with default configuration.
    pub fn new() -> Self {
        ResourceGuard {
            inner: Arc::new(Mutex::new(ResourceGuardInner {
                config: ResourceGuardConfig::default(),
                current_memory_mb: 0,
                operations_count: 0,
                shutdown_requested: false,
                shutdown_requested_at: None,
            })),
        }
    }

    /// Creates a new `ResourceGuard` with custom configuration.
    pub fn with_config(config: ResourceGuardConfig) -> Self {
        ResourceGuard {
            inner: Arc::new(Mutex::new(ResourceGuardInner {
                config,
                current_memory_mb: 0,
                operations_count: 0,
                shutdown_requested: false,
                shutdown_requested_at: None,
            })),
        }
    }

    /// Updates the current memory usage and returns the status.
    pub fn update_memory(&self, memory_mb: usize) -> ResourceStatus {
        let mut inner = self.inner.lock().unwrap();
        inner.current_memory_mb = memory_mb;
        self.compute_status(&inner)
    }

    /// Increments the operation count and returns the status.
    pub fn record_operation(&self) -> ResourceStatus {
        let mut inner = self.inner.lock().unwrap();
        inner.operations_count += 1;
        self.compute_status(&inner)
    }

    /// Returns the current memory usage in MB.
    pub fn current_memory_mb(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.current_memory_mb
    }

    /// Returns the current operation count.
    pub fn operations_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.operations_count
    }

    /// Returns the configured memory limit.
    pub fn memory_limit_mb(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.config.memory_limit_mb
    }

    /// Returns the configured operation limit.
    pub fn operation_limit(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.config.operation_limit
    }

    /// Requests a graceful shutdown.
    pub fn request_shutdown(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.shutdown_requested = true;
        inner.shutdown_requested_at = Some(Instant::now());
    }

    /// Returns whether shutdown has been requested.
    pub fn should_shutdown(&self) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.shutdown_requested
    }

    /// Returns how long shutdown has been pending.
    pub fn shutdown_pending_duration(&self) -> Option<Duration> {
        let inner = self.inner.lock().unwrap();
        inner.shutdown_requested_at.map(|t| t.elapsed())
    }

    /// Returns the current resource status.
    pub fn status(&self) -> ResourceStatus {
        let inner = self.inner.lock().unwrap();
        self.compute_status(&inner)
    }

    /// Resets the resource guard to initial state.
    pub fn reset(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.current_memory_mb = 0;
        inner.operations_count = 0;
        inner.shutdown_requested = false;
        inner.shutdown_requested_at = None;
    }

    fn compute_status(&self, inner: &ResourceGuardInner) -> ResourceStatus {
        if inner.shutdown_requested {
            return ResourceStatus::ShutdownRequested;
        }
        if inner.operations_count >= inner.config.operation_limit {
            return ResourceStatus::OperationLimitExceeded;
        }
        let memory_ratio = inner.current_memory_mb as f32 / inner.config.memory_limit_mb as f32;
        if memory_ratio >= 1.0 {
            return ResourceStatus::MemoryLimitExceeded;
        }
        if memory_ratio >= inner.config.memory_warning_threshold {
            return ResourceStatus::MemoryWarning;
        }
        ResourceStatus::OK
    }
}

impl Default for ResourceGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_status_ok() {
        let guard = ResourceGuard::new();
        assert_eq!(guard.status(), ResourceStatus::OK);
        assert_eq!(guard.current_memory_mb(), 0);
        assert_eq!(guard.operations_count(), 0);
        assert!(!guard.should_shutdown());
    }

    #[test]
    fn test_memory_warning() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        });
        assert_eq!(guard.update_memory(450), ResourceStatus::MemoryWarning);
        assert_eq!(guard.update_memory(300), ResourceStatus::OK);
    }

    #[test]
    fn test_memory_limit_exceeded() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 10000,
            memory_warning_threshold: 0.8,
        });
        assert_eq!(
            guard.update_memory(512),
            ResourceStatus::MemoryLimitExceeded
        );
        assert_eq!(
            guard.update_memory(600),
            ResourceStatus::MemoryLimitExceeded
        );
    }

    #[test]
    fn test_operation_limit() {
        let guard = ResourceGuard::with_config(ResourceGuardConfig {
            memory_limit_mb: 512,
            operation_limit: 5,
            memory_warning_threshold: 0.8,
        });
        for _ in 0..4 {
            assert_eq!(guard.record_operation(), ResourceStatus::OK);
        }
        assert_eq!(
            guard.record_operation(),
            ResourceStatus::OperationLimitExceeded
        );
    }

    #[test]
    fn test_shutdown_request() {
        let guard = ResourceGuard::new();
        assert!(!guard.should_shutdown());
        guard.request_shutdown();
        assert!(guard.should_shutdown());
        assert_eq!(guard.status(), ResourceStatus::ShutdownRequested);
        assert!(guard.shutdown_pending_duration().is_some());
    }

    #[test]
    fn test_reset() {
        let guard = ResourceGuard::new();
        guard.update_memory(500);
        guard.request_shutdown();
        guard.reset();
        assert_eq!(guard.status(), ResourceStatus::OK);
        assert!(!guard.should_shutdown());
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let guard = ResourceGuard::new();
        let handles: Vec<_> = (0..10)
            .map(|_| {
                let guard = guard.clone();
                thread::spawn(move || {
                    for _ in 0..100 {
                        guard.record_operation();
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(guard.operations_count(), 1000);
    }
}

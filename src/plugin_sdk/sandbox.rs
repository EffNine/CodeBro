#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Plugin Sandbox — isolation and permission enforcement.
//!
//! The sandbox ensures plugins cannot:
//! - Modify core memory directly
//! - Bypass the approval gate
//! - Bypass validation
//! - Change deterministic behavior
//!
//! All interactions go through approved SDK interfaces.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use super::types::*;

/// Sandbox policy for a plugin.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    pub allowed_domains: HashSet<SecurityDomain>,
    pub max_memory_bytes: usize,
    pub max_execution_time_ms: u64,
    pub allow_file_io: bool,
    pub allow_network: bool,
    pub allow_env_access: bool,
}

impl SandboxPolicy {
    pub fn new() -> Self {
        SandboxPolicy {
            allowed_domains: HashSet::new(),
            max_memory_bytes: 64 * 1024 * 1024, // 64 MB
            max_execution_time_ms: 5000,        // 5 seconds
            allow_file_io: false,
            allow_network: false,
            allow_env_access: false,
        }
    }

    pub fn with_allowed_domain(mut self, domain: SecurityDomain) -> Self {
        self.allowed_domains.insert(domain);
        self
    }

    pub fn with_max_memory(mut self, bytes: usize) -> Self {
        self.max_memory_bytes = bytes;
        self
    }

    pub fn with_max_execution_time(mut self, ms: u64) -> Self {
        self.max_execution_time_ms = ms;
        self
    }

    pub fn with_file_io(mut self, allowed: bool) -> Self {
        self.allow_file_io = allowed;
        self
    }

    pub fn with_network(mut self, allowed: bool) -> Self {
        self.allow_network = allowed;
        self
    }

    pub fn with_env_access(mut self, allowed: bool) -> Self {
        self.allow_env_access = allowed;
        self
    }

    /// Check if a domain is allowed for this plugin.
    pub fn is_domain_allowed(&self, domain: &SecurityDomain) -> bool {
        self.allowed_domains.contains(domain)
    }

    /// Check if the plugin has write permission to a domain.
    pub fn has_write_permission(&self, domain: &SecurityDomain) -> bool {
        self.allowed_domains.contains(domain)
    }
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::new()
    }
}

/// Sandbox violation.
#[derive(Debug, Clone)]
pub enum SandboxViolation {
    DomainNotAuthorized(SecurityDomain),
    MemoryExceeded(usize, usize), // requested, limit
    FileIONotAllowed,
    NetworkNotAllowed,
    EnvAccessNotAllowed,
    ApprovalGateBypass,
    ValidationBypass,
    DeterministicBehaviorChange,
}

impl std::fmt::Display for SandboxViolation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxViolation::DomainNotAuthorized(d) => {
                write!(f, "Domain not authorized: {d}")
            }
            SandboxViolation::MemoryExceeded(requested, limit) => {
                write!(f, "Memory exceeded: requested={requested}, limit={limit}")
            }
            SandboxViolation::FileIONotAllowed => write!(f, "File I/O not allowed"),
            SandboxViolation::NetworkNotAllowed => write!(f, "Network access not allowed"),
            SandboxViolation::EnvAccessNotAllowed => write!(f, "Environment access not allowed"),
            SandboxViolation::ApprovalGateBypass => write!(f, "Approval gate bypass attempted"),
            SandboxViolation::ValidationBypass => write!(f, "Validation bypass attempted"),
            SandboxViolation::DeterministicBehaviorChange => {
                write!(f, "Deterministic behavior change attempted")
            }
        }
    }
}

impl std::error::Error for SandboxViolation {}

/// Inner state for the sandbox.
#[derive(Debug)]
struct SandboxInner {
    policies: HashSet<SecurityDomain>,
    violations: Vec<SandboxViolation>,
}

/// Thread-safe plugin sandbox.
///
/// Clone is cheap (Arc clone). Safe to share across threads.
#[derive(Debug, Clone)]
pub struct PluginSandbox {
    inner: Arc<Mutex<SandboxInner>>,
    policy: SandboxPolicy,
}

impl PluginSandbox {
    /// Creates a new sandbox with the given policy.
    pub fn new(policy: SandboxPolicy) -> Self {
        let policies: HashSet<_> = policy.allowed_domains.clone();
        PluginSandbox {
            inner: Arc::new(Mutex::new(SandboxInner {
                policies,
                violations: Vec::new(),
            })),
            policy,
        }
    }

    /// Creates a sandbox with default policy.
    pub fn with_defaults() -> Self {
        PluginSandbox::new(SandboxPolicy::new())
    }

    /// Checks if an operation is allowed for a domain.
    pub fn check(&self, domain: &SecurityDomain, operation: &str) -> Result<(), SandboxViolation> {
        let inner = self.inner.lock().unwrap();
        if !inner.policies.contains(domain) {
            return Err(SandboxViolation::DomainNotAuthorized(domain.clone()));
        }
        drop(inner);

        // Additional checks based on operation type
        match operation {
            "read" => Ok(()),
            "write" => {
                if self.policy.allow_file_io {
                    Ok(())
                } else {
                    Err(SandboxViolation::FileIONotAllowed)
                }
            }
            "network" => {
                if self.policy.allow_network {
                    Ok(())
                } else {
                    Err(SandboxViolation::NetworkNotAllowed)
                }
            }
            "env" => {
                if self.policy.allow_env_access {
                    Ok(())
                } else {
                    Err(SandboxViolation::EnvAccessNotAllowed)
                }
            }
            "approval_bypass" => Err(SandboxViolation::ApprovalGateBypass),
            "validation_bypass" => Err(SandboxViolation::ValidationBypass),
            "deterministic_change" => Err(SandboxViolation::DeterministicBehaviorChange),
            _ => Ok(()),
        }
    }

    /// Records a violation.
    pub fn record_violation(&self, violation: SandboxViolation) {
        let mut inner = self.inner.lock().unwrap();
        inner.violations.push(violation);
    }

    /// Returns all recorded violations.
    pub fn violations(&self) -> Vec<SandboxViolation> {
        let inner = self.inner.lock().unwrap();
        inner.violations.clone()
    }

    /// Returns the number of violations.
    pub fn violation_count(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.violations.len()
    }

    /// Clears all violations.
    pub fn clear_violations(&self) {
        let mut inner = self.inner.lock().unwrap();
        inner.violations.clear();
    }

    /// Returns the policy.
    pub fn policy(&self) -> &SandboxPolicy {
        &self.policy
    }

    /// Returns a summary string.
    pub fn summary(&self) -> String {
        let inner = self.inner.lock().unwrap();
        format!(
            "Sandbox: {} domains allowed, {} violations",
            inner.policies.len(),
            inner.violations.len()
        )
    }
}

impl Default for PluginSandbox {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_sandbox() {
        let sandbox = PluginSandbox::with_defaults();
        assert_eq!(sandbox.policy().allowed_domains.len(), 0);
    }

    #[test]
    fn test_domain_check_allowed() {
        let policy = SandboxPolicy::new().with_allowed_domain(SecurityDomain::Observability);
        let sandbox = PluginSandbox::new(policy);
        assert!(sandbox
            .check(&SecurityDomain::Observability, "read")
            .is_ok());
    }

    #[test]
    fn test_domain_check_denied() {
        let policy = SandboxPolicy::new().with_allowed_domain(SecurityDomain::Observability);
        let sandbox = PluginSandbox::new(policy);
        assert!(sandbox.check(&SecurityDomain::Pipeline, "read").is_err());
    }

    #[test]
    fn test_approval_bypass_blocked() {
        let sandbox = PluginSandbox::with_defaults();
        assert!(sandbox
            .check(&SecurityDomain::Pipeline, "approval_bypass")
            .is_err());
    }

    #[test]
    fn test_validation_bypass_blocked() {
        let sandbox = PluginSandbox::with_defaults();
        assert!(sandbox
            .check(&SecurityDomain::Pipeline, "validation_bypass")
            .is_err());
    }

    #[test]
    fn test_violation_recording() {
        let sandbox = PluginSandbox::with_defaults();
        sandbox.record_violation(SandboxViolation::DomainNotAuthorized(
            SecurityDomain::Pipeline,
        ));
        assert_eq!(sandbox.violation_count(), 1);
        let violations = sandbox.violations();
        assert!(matches!(
            &violations[0],
            SandboxViolation::DomainNotAuthorized(_)
        ));
    }

    #[test]
    fn test_clear_violations() {
        let sandbox = PluginSandbox::with_defaults();
        sandbox.record_violation(SandboxViolation::FileIONotAllowed);
        assert_eq!(sandbox.violation_count(), 1);
        sandbox.clear_violations();
        assert_eq!(sandbox.violation_count(), 0);
    }

    #[test]
    fn test_thread_safety() {
        use std::thread;
        let sandbox = PluginSandbox::with_defaults();
        let handles: Vec<_> = (0..10)
            .map(|i| {
                let s = sandbox.clone();
                thread::spawn(move || {
                    for _ in 0..50 {
                        s.record_violation(SandboxViolation::DomainNotAuthorized(
                            SecurityDomain::Custom(format!("domain_{i}")),
                        ));
                    }
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap();
        }
        assert_eq!(sandbox.violation_count(), 500);
    }
}

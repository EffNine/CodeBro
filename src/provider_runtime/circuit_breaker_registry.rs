#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Circuit Breaker Registry for the Provider Runtime (P17.0).
//!
//! Manages one independent `CircuitBreaker` per registered provider.
//! Provider A failing never affects provider B.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use super::circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};
use super::types::ProviderId;

/// Registry of per-provider circuit breakers.
///
/// Thread-safe: `Clone` shares the same underlying registry.
#[derive(Debug, Clone, Default)]
pub struct CircuitBreakerRegistry {
    inner: Arc<Mutex<RegistryInner>>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    breakers: HashMap<ProviderId, CircuitBreaker>,
    default_config: CircuitBreakerConfig,
}

impl CircuitBreakerRegistry {
    /// Creates an empty registry with default configuration.
    pub fn new() -> Self {
        CircuitBreakerRegistry {
            inner: Arc::new(Mutex::new(RegistryInner {
                breakers: HashMap::new(),
                default_config: CircuitBreakerConfig::default(),
            })),
        }
    }

    /// Creates a registry with a custom default configuration.
    pub fn with_config(config: CircuitBreakerConfig) -> Self {
        CircuitBreakerRegistry {
            inner: Arc::new(Mutex::new(RegistryInner {
                breakers: HashMap::new(),
                default_config: config,
            })),
        }
    }

    /// Returns the breaker for a provider, creating one with the
    /// default config if none exists.
    pub fn get_or_create(&self, provider: &ProviderId) -> CircuitBreaker {
        let config = {
            let inner = self.inner.lock().unwrap();
            inner.default_config.clone()
        };
        let mut inner = self.inner.lock().unwrap();
        if !inner.breakers.contains_key(provider) {
            inner
                .breakers
                .insert(provider.clone(), CircuitBreaker::with_config(config));
        }
        inner.breakers.get(provider).unwrap().clone()
    }

    /// Returns the breaker for a provider, or `None` if not registered.
    pub fn get(&self, provider: &ProviderId) -> Option<CircuitBreaker> {
        let inner = self.inner.lock().unwrap();
        inner.breakers.get(provider).cloned()
    }

    /// Register a provider with a custom config.
    pub fn register(&self, provider: &ProviderId, config: CircuitBreakerConfig) {
        let mut inner = self.inner.lock().unwrap();
        inner
            .breakers
            .insert(provider.clone(), CircuitBreaker::with_config(config));
    }

    /// Remove a provider's breaker.
    pub fn unregister(&self, provider: &ProviderId) {
        let mut inner = self.inner.lock().unwrap();
        inner.breakers.remove(provider);
    }

    /// Returns whether a provider has a breaker in the registry.
    pub fn contains(&self, provider: &ProviderId) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.breakers.contains_key(provider)
    }

    /// Returns the number of breakers in the registry.
    pub fn len(&self) -> usize {
        let inner = self.inner.lock().unwrap();
        inner.breakers.len()
    }

    /// Returns whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns all provider ids that have breakers.
    pub fn providers(&self) -> Vec<ProviderId> {
        let inner = self.inner.lock().unwrap();
        inner.breakers.keys().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::circuit_breaker::CircuitBreakerState;
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_registry_empty() {
        let reg = CircuitBreakerRegistry::new();
        assert!(reg.is_empty());
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_get_or_create() {
        let reg = CircuitBreakerRegistry::new();
        let id = ProviderId::new("openai");
        let cb = reg.get_or_create(&id);
        assert_eq!(cb.state(), CircuitBreakerState::Closed);
        assert_eq!(reg.len(), 1);
        assert!(reg.contains(&id));
    }

    #[test]
    fn test_independent_breakers() {
        let reg = CircuitBreakerRegistry::new();
        let a = ProviderId::new("a");
        let b = ProviderId::new("b");

        reg.get_or_create(&a);
        reg.get_or_create(&b);

        // Fail provider a until its breaker opens.
        let cb_a = reg.get(&a).unwrap();
        for _ in 0..5 {
            cb_a.record_failure();
        }
        assert_eq!(cb_a.state(), CircuitBreakerState::Open);

        // Provider b should still be closed.
        let cb_b = reg.get(&b).unwrap();
        assert_eq!(cb_b.state(), CircuitBreakerState::Closed);
    }

    #[test]
    fn test_custom_config() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 1,
            cooldown_duration: Duration::from_millis(100),
            ..Default::default()
        };
        let reg = CircuitBreakerRegistry::with_config(config.clone());
        let id = ProviderId::new("p");
        let cb = reg.get_or_create(&id);
        assert_eq!(cb.config().failure_threshold, 2);

        // Also test explicit register.
        let custom = CircuitBreakerConfig {
            failure_threshold: 10,
            ..Default::default()
        };
        reg.register(&id, custom);
        let cb2 = reg.get(&id).unwrap();
        assert_eq!(cb2.config().failure_threshold, 10);
    }

    #[test]
    fn test_unregister() {
        let reg = CircuitBreakerRegistry::new();
        let id = ProviderId::new("x");
        reg.get_or_create(&id);
        assert!(reg.contains(&id));
        reg.unregister(&id);
        assert!(!reg.contains(&id));
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn test_clone_shares_registry() {
        let reg = CircuitBreakerRegistry::new();
        let id = ProviderId::new("p");
        reg.get_or_create(&id);
        let reg2 = reg.clone();
        assert_eq!(reg2.len(), 1);
        assert!(reg2.contains(&id));
    }
}

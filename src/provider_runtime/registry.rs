#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider Registry — the authoritative set of registered providers.
//!
//! Responsibilities:
//!   - register / unregister / replace
//!   - deterministic iteration in registration order
//!   - lookup by id
//!
//! The registry owns no I/O. It is thread-safe and observable.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use super::provider::{Provider, RegisteredProvider};
use super::types::{ProviderId, ProviderRuntimeError, ProviderRuntimeResult};

/// Registration order iterator is stable because the backing list is
/// appended-only for inserts.
#[derive(Debug, Default)]
pub struct ProviderRegistry {
    inner: Arc<RwLock<RegistryInner>>,
    seq: Arc<RwLock<u64>>,
}

#[derive(Debug, Default)]
struct RegistryInner {
    /// Providers in registration order (stable, deterministic).
    ordered: Vec<RegisteredProvider>,
    /// Index by id.
    by_id: HashMap<ProviderId, usize>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        ProviderRegistry {
            inner: Arc::new(RwLock::new(RegistryInner::default())),
            seq: Arc::new(RwLock::new(0)),
        }
    }

    /// Register a provider plugin. Duplicate ids are rejected.
    pub fn register(&self, provider: &dyn Provider) -> ProviderRuntimeResult<()> {
        self.register_seq_provider(provider, &mut self.seq.write().unwrap())
    }

    /// Register a value-based descriptor (used by tests / adapters).
    pub fn register_value(&self, provider: RegisteredProvider) -> ProviderRuntimeResult<()> {
        self.register_value_seq(provider, &mut self.seq.write().unwrap())
    }

    fn register_seq_provider(
        &self,
        provider: &dyn Provider,
        pool: &mut u64,
    ) -> ProviderRuntimeResult<()> {
        let rec = RegisteredProvider::from_provider(provider, pool);
        self.register_value_seq(rec, pool)
    }

    fn register_value_seq(
        &self,
        mut provider: RegisteredProvider,
        pool: &mut u64,
    ) -> ProviderRuntimeResult<()> {
        let mut inner = self.inner.write().unwrap();
        if inner.by_id.contains_key(&provider.id) {
            return Err(ProviderRuntimeError::Duplicate(provider.id.clone()));
        }
        provider.registration_seq = *pool;
        *pool += 1;
        let idx = inner.ordered.len();
        inner.ordered.push(provider.clone());
        inner.by_id.insert(provider.id.clone(), idx);
        Ok(())
    }

    /// Unregister a provider by id.
    pub fn unregister(&self, id: &ProviderId) -> ProviderRuntimeResult<()> {
        let mut inner = self.inner.write().unwrap();
        let idx = inner
            .by_id
            .remove(id)
            .ok_or_else(|| ProviderRuntimeError::NotFound(id.clone()))?;
        inner.ordered.swap_remove(idx);
        // Rebuild index (id list is small; correctness over cleverness).
        inner.reindex();
        Ok(())
    }

    /// Get a provider by id.
    pub fn get(&self, id: &ProviderId) -> Option<RegisteredProvider> {
        let inner = self.inner.read().unwrap();
        inner
            .by_id
            .get(id)
            .and_then(|i| inner.ordered.get(*i))
            .cloned()
    }

    /// True if a provider with the id is registered.
    pub fn contains(&self, id: &ProviderId) -> bool {
        let inner = self.inner.read().unwrap();
        inner.by_id.contains_key(id)
    }

    /// Registered providers in registration order.
    pub fn all(&self) -> Vec<RegisteredProvider> {
        let inner = self.inner.read().unwrap();
        inner.ordered.clone()
    }

    pub fn list_ids(&self) -> Vec<ProviderId> {
        let inner = self.inner.read().unwrap();
        inner.by_id.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        let inner = self.inner.read().unwrap();
        inner.ordered.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Pool of the next registration sequence (useful for the router to
    /// snapshot deterministic order).
    pub fn next_seq(&self) -> u64 {
        *self.seq.read().unwrap()
    }
}

impl RegistryInner {
    fn reindex(&mut self) {
        self.by_id.clear();
        for (i, p) in self.ordered.iter().enumerate() {
            self.by_id.insert(p.id.clone(), i);
        }
    }
}

impl Clone for ProviderRegistry {
    fn clone(&self) -> Self {
        ProviderRegistry {
            inner: Arc::clone(&self.inner),
            seq: Arc::clone(&self.seq),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_runtime::{
        capabilities::Capability,
        types::{Priority, ProviderCost},
    };

    fn caps(xs: &[Capability]) -> crate::provider_runtime::CapabilitySet {
        crate::provider_runtime::CapabilitySet::new(xs.iter().copied())
    }

    fn rec(id: &str, c: &[Capability], cost: f64) -> RegisteredProvider {
        RegisteredProvider::new(
            id,
            caps(c),
            ProviderCost {
                input_per_million: cost,
                output_per_million: cost,
                cache_read_per_million: None,
            },
            Priority::Normal,
        )
    }

    #[test]
    fn test_registry_empty() {
        let r = ProviderRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.len(), 0);
        assert!(r.all().is_empty());
    }

    #[test]
    fn test_registry_register_and_get() {
        let r = ProviderRegistry::new();
        r.register_value(rec("alpha", &[Capability::Streaming], 1.0))
            .unwrap();
        let p = r.get(&ProviderId::new("alpha")).unwrap();
        assert_eq!(p.id.as_str(), "alpha");
        assert!(p.supports_all(&[Capability::Streaming]));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn test_registry_rejects_duplicate() {
        let r = ProviderRegistry::new();
        r.register_value(rec("dup", &[], 1.0)).unwrap();
        let err = r.register_value(rec("dup", &[], 2.0)).unwrap_err();
        assert!(matches!(err, ProviderRuntimeError::Duplicate(_)));
        assert_eq!(r.len(), 1);
    }

    #[test]
    fn test_registry_unregister() {
        let r = ProviderRegistry::new();
        r.register_value(rec("a", &[], 1.0)).unwrap();
        r.register_value(rec("b", &[], 1.0)).unwrap();
        r.unregister(&ProviderId::new("a")).unwrap();
        assert!(!r.contains(&ProviderId::new("a")));
        assert_eq!(r.len(), 1);
        assert!(r.get(&ProviderId::new("b")).is_some());
    }

    #[test]
    fn test_registry_unregister_missing() {
        let r = ProviderRegistry::new();
        let err = r.unregister(&ProviderId::new("nope")).unwrap_err();
        assert!(matches!(err, ProviderRuntimeError::NotFound(_)));
    }

    #[test]
    fn test_registry_get_missing() {
        let r = ProviderRegistry::new();
        assert!(r.get(&ProviderId::new("zzz")).is_none());
    }

    #[test]
    fn test_registry_deterministic_order() {
        let r = ProviderRegistry::new();
        for id in ["first", "second", "third"] {
            r.register_value(rec(id, &[], 1.0)).unwrap();
        }
        let ids: Vec<String> = r.all().iter().map(|p| p.id.to_string()).collect();
        assert_eq!(ids, vec!["first", "second", "third"]);
        assert_eq!(r.list_ids().len(), 3);
    }

    #[test]
    fn test_registration_seq_assigned() {
        let r = ProviderRegistry::new();
        r.register_value(rec("a", &[], 1.0)).unwrap();
        r.register_value(rec("b", &[], 1.0)).unwrap();
        let a = r.get(&ProviderId::new("a")).unwrap();
        let b = r.get(&ProviderId::new("b")).unwrap();
        assert_eq!(a.registration_seq, 0);
        assert_eq!(b.registration_seq, 1);
    }

    #[test]
    fn test_register_trait_object() {
        struct P {
            id: ProviderId,
            caps: crate::provider_runtime::CapabilitySet,
        }
        impl Provider for P {
            fn id(&self) -> &ProviderId {
                &self.id
            }
            fn capabilities(&self) -> &crate::provider_runtime::CapabilitySet {
                &self.caps
            }
            fn cost(&self) -> &ProviderCost {
                static DEFAULT_COST: ProviderCost = ProviderCost {
                    input_per_million: 0.0,
                    output_per_million: 0.0,
                    cache_read_per_million: None,
                };
                &DEFAULT_COST
            }
            fn priority(&self) -> Priority {
                Priority::Normal
            }
        }
        let p = P {
            id: ProviderId::new("traitobj"),
            caps: crate::provider_runtime::CapabilitySet::empty(),
        };
        let r = ProviderRegistry::new();
        r.register(&p).unwrap();
        assert!(r.contains(&ProviderId::new("traitobj")));
    }

    #[test]
    fn test_registry_clone_shares_state() {
        let r = ProviderRegistry::new();
        r.register_value(rec("x", &[], 1.0)).unwrap();
        let r2 = r.clone();
        assert_eq!(r2.len(), 1);
        r2.register_value(rec("y", &[], 1.0)).unwrap();
        assert_eq!(r.len(), 2);
    }

    #[test]
    fn test_registry_serializable_value() {
        let r = ProviderRegistry::new();
        r.register_value(rec("s", &[Capability::JsonMode], 3.0).with_seq(9))
            .unwrap();
        // RegisteredProvider must be serializable for persistence.
        let rec = r.get(&ProviderId::new("s")).unwrap();
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("JsonMode"));
    }
}

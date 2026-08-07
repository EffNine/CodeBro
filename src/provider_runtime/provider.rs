#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Provider plugin contract and registered-provider record.
//!
//! # Separation
//!
//! - **Provider** is the trait a provider PLUGIN implements. It answers
//!   only descriptive questions (id, capabilities, cost, priority,
//!   metadata). It performs no network I/O.
//! - **RegisteredProvider** is a snapshot the runtime keeps about a
//!   registered provider (descriptor + stable registration sequence).
//!
//! A provider plugin can be implemented WITHOUT modifying Provider
//! Runtime: implement the `Provider` trait and register it.

use serde::{Deserialize, Serialize};

use super::capabilities::CapabilitySet;
use super::types::{Priority, ProviderCost, ProviderId};

/// The contract every provider plugin implements.
///
/// This trait is intentionally small and purely descriptive. All I/O
/// (HTTP, WebSocket, vendor SDKs, auth) lives in the plugin behind this
/// contract, never in the runtime.
pub trait Provider: Send + Sync {
    /// Unique id.
    fn id(&self) -> &ProviderId;

    /// Capabilities this provider supports.
    fn capabilities(&self) -> &CapabilitySet;

    /// Descriptive pricing model.
    fn cost(&self) -> &ProviderCost;

    /// Provider priority used as a late tie-break in routing.
    fn priority(&self) -> Priority;

    /// Human-readable name for display only — never used for routing.
    fn display_name(&self) -> &str {
        self.id().as_str()
    }

    /// Optional additional metadata.
    fn metadata(&self) -> &[(&str, String)] {
        &[]
    }
}

/// A registered provider: the plugin descriptor plus runtime bookkeeping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RegisteredProvider {
    /// Opaque identifier.
    pub id: ProviderId,
    /// Capability descriptor.
    pub capabilities: CapabilitySet,
    /// Descriptive cost model.
    pub cost: ProviderCost,
    /// Priority for tie-breaking.
    pub priority: Priority,
    /// Display name (never used for routing).
    pub display_name: String,
    /// Stable, monotonically increasing registration sequence used as the
    /// final deterministic tie-breaker. Lower is earlier.
    pub registration_seq: u64,
}

impl RegisteredProvider {
    /// Build a registered record from a provider plugin and a pool.
    pub fn from_provider(p: &dyn Provider, pool: &mut u64) -> Self {
        let seq = *pool;
        *pool += 1;
        RegisteredProvider {
            id: p.id().clone(),
            capabilities: p.capabilities().clone(),
            cost: p.cost().clone(),
            priority: p.priority(),
            display_name: p.display_name().to_string(),
            registration_seq: seq,
        }
    }

    /// Build a registered record directly (used by tests and plugins that
    /// prefer value registration).
    pub fn new(
        id: impl Into<ProviderId>,
        capabilities: CapabilitySet,
        cost: ProviderCost,
        priority: Priority,
    ) -> Self {
        let id = id.into();
        let display_name = id.to_string();
        RegisteredProvider {
            id,
            capabilities,
            cost,
            priority,
            display_name,
            registration_seq: 0,
        }
    }

    pub fn with_display_name(mut self, name: impl Into<String>) -> Self {
        self.display_name = name.into();
        self
    }

    pub fn with_seq(mut self, seq: u64) -> Self {
        self.registration_seq = seq;
        self
    }

    pub fn with_priority(mut self, priority: Priority) -> Self {
        self.priority = priority;
        self
    }

    pub fn with_cost(mut self, cost: ProviderCost) -> Self {
        self.cost = cost;
        self
    }

    pub fn supports_all(&self, required: &[super::capabilities::Capability]) -> bool {
        self.capabilities.has_all(required)
    }

    /// Convenience accessor for the capability set.
    pub fn caps(&self) -> &CapabilitySet {
        &self.capabilities
    }
}

/// Convenience helpers for building descriptor-only providers in tests.
impl RegisteredProvider {
    pub fn minimal(id: &str) -> Self {
        RegisteredProvider::new(id, CapabilitySet::empty(), ProviderCost::default(), Priority::Normal)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::capabilities::Capability;

    #[test]
    fn test_registered_provider_fields() {
        let mut caps = CapabilitySet::empty();
        caps.insert(Capability::Streaming);
        let p = RegisteredProvider::new("alpha", caps.clone(), ProviderCost::default(), Priority::High);
        assert_eq!(p.id.as_str(), "alpha");
        assert_eq!(p.priority, Priority::High);
        assert!(p.supports_all(&[Capability::Streaming]));
        assert!(!p.supports_all(&[Capability::Vision]));
    }

    #[test]
    fn test_registered_provider_builder() {
        let p = RegisteredProvider::minimal("beta")
            .with_display_name("Beta Vendor")
            .with_seq(7)
            .with_priority(Priority::Low);
        assert_eq!(p.registration_seq, 7);
        assert_eq!(p.display_name, "Beta Vendor");
        assert_eq!(p.priority, Priority::Low);
        assert!(!p.capabilities.has(&Capability::ToolCalling));
    }

    #[test]
    fn test_from_provider_monotonic_seq() {
        struct Dummy {
            id: ProviderId,
            caps: CapabilitySet,
            cost: ProviderCost,
            prio: Priority,
        }
        impl Provider for Dummy {
            fn id(&self) -> &ProviderId {
                &self.id
            }
fn capabilities(&self) -> &CapabilitySet {
                &self.caps
            }
            fn cost(&self) -> &ProviderCost {
                &self.cost
            }
            fn priority(&self) -> Priority {
                self.prio
            }
        }
        let a = Dummy {
            id: ProviderId::new("a"),
            caps: CapabilitySet::empty(),
            cost: ProviderCost::default(),
            prio: Priority::Normal,
        };
        let b = Dummy {
            id: ProviderId::new("b"),
            caps: CapabilitySet::empty(),
            cost: ProviderCost::default(),
            prio: Priority::Normal,
        };
        let mut pool = 0u64;
        let ra = RegisteredProvider::from_provider(&a, &mut pool);
        let rb = RegisteredProvider::from_provider(&b, &mut pool);
        assert_eq!(ra.registration_seq, 0);
        assert_eq!(rb.registration_seq, 1);
        assert!(ra.registration_seq < rb.registration_seq);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let p = RegisteredProvider::minimal("gamma").with_display_name("Gamma");
        let json = serde_json::to_string(&p).unwrap();
        let back: RegisteredProvider = serde_json::from_str(&json).unwrap();
        assert_eq!(p, back);
    }
}
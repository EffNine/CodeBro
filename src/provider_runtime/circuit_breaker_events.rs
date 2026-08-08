#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Circuit Breaker Events for the Provider Runtime (P17.0).
//!
//! Extends the existing `ProviderEvent` enum with circuit-breaker-specific
//! events. This reuses the existing event infrastructure rather than
//! inventing a new system.

use serde::{Deserialize, Serialize};

use super::circuit_breaker::CircuitBreakerState;
use super::types::ProviderId;

/// Additional diagnostic events emitted by the circuit breaker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CircuitBreakerEvent {
    /// The circuit breaker for a provider opened.
    BreakerOpened {
        provider: ProviderId,
        failure_count: u32,
        correlation_id: String,
    },
    /// The circuit breaker for a provider closed (recovered).
    BreakerClosed {
        provider: ProviderId,
        correlation_id: String,
    },
    /// The circuit breaker transitioned to half-open.
    BreakerHalfOpened {
        provider: ProviderId,
        correlation_id: String,
    },
    /// A request was rejected because the breaker was open.
    RequestRejected {
        provider: ProviderId,
        correlation_id: String,
    },
    /// A recovery probe succeeded in half-open state.
    RecoverySucceeded {
        provider: ProviderId,
        correlation_id: String,
    },
    /// A recovery probe failed in half-open state.
    RecoveryFailed {
        provider: ProviderId,
        correlation_id: String,
    },
}

impl CircuitBreakerEvent {
    /// Short summary for observers.
    pub fn summary(&self) -> String {
        match self {
            CircuitBreakerEvent::BreakerOpened {
                provider,
                failure_count,
                ..
            } => {
                format!("breaker opened for {provider} (failures={failure_count})")
            }
            CircuitBreakerEvent::BreakerClosed { provider, .. } => {
                format!("breaker closed for {provider}")
            }
            CircuitBreakerEvent::BreakerHalfOpened { provider, .. } => {
                format!("breaker half-open for {provider}")
            }
            CircuitBreakerEvent::RequestRejected { provider, .. } => {
                format!("request rejected for {provider}")
            }
            CircuitBreakerEvent::RecoverySucceeded { provider, .. } => {
                format!("recovery succeeded for {provider}")
            }
            CircuitBreakerEvent::RecoveryFailed { provider, .. } => {
                format!("recovery failed for {provider}")
            }
        }
    }
}

/// Combines provider events and circuit breaker events into a unified
/// event stream for diagnostics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProviderRuntimeEvent {
    Provider(super::diagnostics::ProviderEvent),
    CircuitBreaker(CircuitBreakerEvent),
}

impl ProviderRuntimeEvent {
    pub fn summary(&self) -> String {
        match self {
            ProviderRuntimeEvent::Provider(e) => e.summary(),
            ProviderRuntimeEvent::CircuitBreaker(e) => e.summary(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_breaker_opened_summary() {
        let e = CircuitBreakerEvent::BreakerOpened {
            provider: ProviderId::new("openai"),
            failure_count: 5,
            correlation_id: "c1".to_string(),
        };
        assert!(e.summary().contains("opened"));
        assert!(e.summary().contains("openai"));
    }

    #[test]
    fn test_breaker_closed_summary() {
        let e = CircuitBreakerEvent::BreakerClosed {
            provider: ProviderId::new("openai"),
            correlation_id: "c1".to_string(),
        };
        assert!(e.summary().contains("closed"));
    }

    #[test]
    fn test_request_rejected_summary() {
        let e = CircuitBreakerEvent::RequestRejected {
            provider: ProviderId::new("deepseek"),
            correlation_id: "c2".to_string(),
        };
        assert!(e.summary().contains("rejected"));
    }

    #[test]
    fn test_unified_event_summary() {
        let inner = super::super::diagnostics::ProviderEvent::ProviderSelected {
            provider: ProviderId::new("p"),
            reason: "test".to_string(),
            correlation_id: "c".to_string(),
        };
        let e = ProviderRuntimeEvent::Provider(inner);
        assert!(e.summary().contains("selected"));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let e = CircuitBreakerEvent::BreakerOpened {
            provider: ProviderId::new("test"),
            failure_count: 3,
            correlation_id: "corr".to_string(),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: CircuitBreakerEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(e, back);
    }
}

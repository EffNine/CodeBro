//! Adapter bridging the I/O provider trait to the descriptive runtime trait.
//!
//! CodeBro has two provider contracts:
//!
//! - `providers::Provider` — the I/O trait. Implementations actually talk to
//!   a vendor (`send_message`, `stream_response`).
//! - `provider_runtime::Provider` — the descriptive trait. Implementations
//!   only answer questions (id, capabilities, cost, priority) and perform no
//!   network I/O. This is the contract the Provider Runtime routes against.
//!
//! `ProviderAdapter` implements the descriptive trait over an I/O provider so
//! existing provider plugins can be registered with the canonical
//! `ProviderRuntime` / `IntelligentProviderRouter` and still be executed by
//! the runtime through their I/O handler.

use std::sync::Arc;

use crate::provider_runtime::capabilities::CapabilitySet;
use crate::provider_runtime::types::{Priority, ProviderCost, ProviderId};
use crate::provider_runtime::Provider as RuntimeProvider;

/// A descriptive provider view over an I/O provider plugin.
pub struct ProviderAdapter {
    provider: Arc<dyn crate::providers::Provider>,
    id: ProviderId,
    capabilities: CapabilitySet,
    cost: ProviderCost,
    priority: Priority,
}

impl ProviderAdapter {
    /// Wrap an I/O provider into a descriptive provider.
    ///
    /// The provider id is derived from the I/O provider's name. Registered
    /// metadata prefers provider-declared values (`Provider::capabilities` /
    /// `Provider::cost`) when present, falling back to the production
    /// defaults (`Streaming` + `ToolCalling`; default cost) for providers
    /// that do not self-describe.
    pub fn new(provider: Arc<dyn crate::providers::Provider>) -> Self {
        let id = ProviderId::new(provider.name());

        let declared_caps = provider.capabilities();
        let capabilities = if declared_caps.is_empty() {
            CapabilitySet::new([
                crate::provider_runtime::Capability::Streaming,
                crate::provider_runtime::Capability::ToolCalling,
            ])
        } else {
            CapabilitySet::new(declared_caps)
        };

        let cost = provider.cost().unwrap_or_default();

        ProviderAdapter {
            provider,
            id,
            capabilities,
            cost,
            priority: Priority::Normal,
        }
    }

    /// The underlying I/O provider id.
    pub fn provider_id(&self) -> &ProviderId {
        &self.id
    }

    /// Access to the underlying I/O handler (used for execution).
    pub fn io(&self) -> &Arc<dyn crate::providers::Provider> {
        &self.provider
    }
}

impl std::fmt::Debug for ProviderAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderAdapter")
            .field("id", &self.id)
            .field("model", &self.provider.model())
            .finish()
    }
}

impl RuntimeProvider for ProviderAdapter {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn capabilities(&self) -> &CapabilitySet {
        &self.capabilities
    }

    fn cost(&self) -> &ProviderCost {
        &self.cost
    }

    fn priority(&self) -> Priority {
        self.priority
    }

    fn display_name(&self) -> &str {
        self.provider.model()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_runtime::Capability;

    struct DeclarativeProvider {
        caps: Vec<Capability>,
        cost: Option<ProviderCost>,
    }

    impl crate::providers::Provider for DeclarativeProvider {
        fn name(&self) -> &str {
            "declarative"
        }
        fn base_url(&self) -> &str {
            "mock://localhost"
        }
        fn model(&self) -> &str {
            "declarative-model"
        }
        fn api_key(&self) -> Option<&str> {
            None
        }
        fn send_message(
            &self,
            _m: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
        {
            Box::pin(async move { Ok(String::new()) })
        }
        fn stream_response(
            &self,
            _m: &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                    > + Send
                    + '_,
            >,
        > {
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
            Box::pin(async move { Ok(rx) })
        }
        fn capabilities(&self) -> Vec<Capability> {
            self.caps.clone()
        }
        fn cost(&self) -> Option<ProviderCost> {
            self.cost.clone()
        }
    }

    struct PlainProvider;

    impl crate::providers::Provider for PlainProvider {
        fn name(&self) -> &str {
            "plain"
        }
        fn base_url(&self) -> &str {
            "mock://localhost"
        }
        fn model(&self) -> &str {
            "plain-model"
        }
        fn api_key(&self) -> Option<&str> {
            None
        }
        fn send_message(
            &self,
            _m: &str,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>>
        {
            Box::pin(async move { Ok(String::new()) })
        }
        fn stream_response(
            &self,
            _m: &str,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = anyhow::Result<tokio::sync::mpsc::UnboundedReceiver<String>>,
                    > + Send
                    + '_,
            >,
        > {
            let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
            Box::pin(async move { Ok(rx) })
        }
    }

    #[test]
    fn test_adapter_prefers_provider_declared_metadata() {
        let provider = DeclarativeProvider {
            caps: vec![Capability::Vision, Capability::Streaming],
            cost: Some(ProviderCost {
                input_per_million: 1.5,
                output_per_million: 6.0,
                cache_read_per_million: None,
            }),
        };
        let adapter = ProviderAdapter::new(Arc::new(provider));

        assert!(adapter.capabilities().has(&Capability::Vision));
        assert!(adapter.capabilities().has(&Capability::Streaming));
        assert_eq!(adapter.cost().input_per_million, 1.5);
        assert_eq!(adapter.cost().output_per_million, 6.0);
        assert_eq!(adapter.id.as_str(), "declarative");
    }

    #[test]
    fn test_adapter_falls_back_to_production_defaults() {
        let adapter = ProviderAdapter::new(Arc::new(PlainProvider));

        // Legacy providers that do not self-describe keep the production
        // defaults (Streaming + ToolCalling; default cost).
        assert!(adapter.capabilities().has(&Capability::Streaming));
        assert!(adapter.capabilities().has(&Capability::ToolCalling));
        assert_eq!(adapter.id.as_str(), "plain");
    }
}

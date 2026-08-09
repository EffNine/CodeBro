#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
use anyhow::Result;

pub trait Provider: Send + Sync {
    fn name(&self) -> &str;
    fn base_url(&self) -> &str;
    fn model(&self) -> &str;
    fn api_key(&self) -> Option<&str>;
    fn send_message(
        &self,
        message: &str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>>;
    fn stream_response(
        &self,
        message: &str,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<tokio::sync::mpsc::UnboundedReceiver<String>>>
                + Send
                + '_,
        >,
    >;

    /// Optional provider-declared capabilities.
    ///
    /// When non-empty, the adapter uses these as the registered metadata
    /// instead of the hard-coded fallback. Providers that self-describe must
    /// include every capability they actually support (e.g. `Streaming`,
    /// `ToolCalling`).
    fn capabilities(&self) -> Vec<crate::provider_runtime::Capability> {
        Vec::new()
    }

    /// Optional provider-declared pricing model.
    ///
    /// When `Some`, the adapter uses it as the registered cost metadata
    /// instead of the default.
    fn cost(&self) -> Option<crate::provider_runtime::ProviderCost> {
        None
    }
}

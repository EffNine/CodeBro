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
}

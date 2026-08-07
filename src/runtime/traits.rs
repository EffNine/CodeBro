#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime traits for CodeBro.
//!
//! Defines the trait abstractions that the runtime pipeline uses to
//! interact with providers, tools, and the event system in a
//! decoupled, testable way.
//!
//! These traits are intentionally minimal — they expose only what the
//! pipeline needs and nothing more.

use std::future::Future;
use std::pin::Pin;
use std::sync::mpsc::Sender;

use anyhow::Result;
use futures::Stream;

use super::context::RuntimeContext;
use super::events::RuntimeEvent;

/// A provider that the runtime can use for LLM communication.
///
/// This is a thin wrapper around the `Provider` trait that adds
/// correlation-ID support for observability.
pub trait RuntimeProvider: Send + Sync {
    /// Returns the provider name.
    fn name(&self) -> &str;

    /// Sends a single message and returns the full response.
    fn send_message(
        &self,
        message: &str,
        correlation_id: &str,
    ) -> Pin<Box<dyn Future<Output = Result<String>> + Send + '_>>;

    /// Streams a response, yielding chunks over time.
    fn stream_response(
        &self,
        message: &str,
        correlation_id: &str,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Pin<Box<dyn Stream<Item = Result<String>> + Send>>,
                    >,
                > + Send
                + '_,
        >,
    >;
}

/// A tool registry that the runtime can use for tool dispatch.
pub trait RuntimeToolRegistry: Send + Sync {
    /// Returns whether a tool with the given name is available.
    fn has_tool(&self, name: &str) -> bool;

    /// Lists all available tool names.
    fn tool_names(&self) -> Vec<String>;

    /// Executes a tool by name with the given arguments.
    fn execute_tool(
        &mut self,
        name: &str,
        args: &str,
        correlation_id: &str,
    ) -> Result<String>;

    /// Returns a description of the tool (for LLM context).
    fn tool_description(&self, name: &str) -> Option<String>;
}

/// An event emitter that the runtime uses to notify the TUI.
pub trait RuntimeEventEmitter: Send + Sync {
    /// Emits a runtime event to the TUI.
    fn emit(&self, event: RuntimeEvent) -> bool;

    /// Emits a typed event directly.
    fn emit_raw(&self, event: RuntimeEvent) -> bool;
}

/// A context factory that creates `RuntimeContext` instances.
pub trait RuntimeContextFactory: Send + Sync {
    /// Creates a new runtime context for the given request.
    fn create_context(&self, request: &str) -> RuntimeContext;
}

/// A simple factory that uses the default context constructor.
#[derive(Debug, Clone, Default)]
pub struct DefaultContextFactory;

impl RuntimeContextFactory for DefaultContextFactory {
    fn create_context(&self, request: &str) -> RuntimeContext {
        RuntimeContext::new(request)
    }
}

/// Trait for components that can report their health to the runtime.
pub trait RuntimeHealthReportable: Send + Sync {
    /// Returns the health status of this component.
    fn health_status(&self) -> super::traits::HealthStatus;

    /// Returns a human-readable health summary.
    fn health_summary(&self) -> String;
}

/// Health status used by runtime health reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unhealthy,
    Unknown,
}

impl HealthStatus {
    pub fn label(&self) -> &'static str {
        match self {
            HealthStatus::Healthy => "healthy",
            HealthStatus::Degraded => "degraded",
            HealthStatus::Unhealthy => "unhealthy",
            HealthStatus::Unknown => "unknown",
        }
    }
}

/// Trait for things that can be observed (produce an observation).
pub trait RuntimeObservable: Send + Sync {
    /// Produces the current observation as a string.
    fn observe(&self) -> String;
}

/// A mock provider for testing.
#[derive(Debug, Clone, Default)]
pub struct MockRuntimeProvider {
    pub name: String,
    pub response: String,
    pub chunks: Vec<String>,
    pub fail_next: bool,
}

impl MockRuntimeProvider {
    pub fn new(name: &str, response: &str) -> Self {
        MockRuntimeProvider {
            name: name.to_string(),
            response: response.to_string(),
            chunks: vec![response.to_string()],
            fail_next: false,
        }
    }

    pub fn with_chunks(name: &str, chunks: Vec<&str>) -> Self {
        MockRuntimeProvider {
            name: name.to_string(),
            response: chunks.join(""),
            chunks: chunks.into_iter().map(|s| s.to_string()).collect(),
            fail_next: false,
        }
    }

    pub fn fail(&mut self) {
        self.fail_next = true;
    }
}

impl RuntimeProvider for MockRuntimeProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn send_message(
        &self,
        _message: &str,
        _correlation_id: &str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<String>> + Send + '_>> {
        let response = if self.fail_next {
            Err(anyhow::anyhow!("mock provider error"))
        } else {
            Ok(self.response.clone())
        };
        Box::pin(async move { response })
    }

    fn stream_response(
        &self,
        _message: &str,
        _correlation_id: &str,
    ) -> Pin<
        Box<
            dyn Future<
                    Output = Result<
                        Pin<Box<dyn Stream<Item = Result<String>> + Send>>,
                    >,
                > + Send
                + '_,
        >,
    > {
        let chunks = self.chunks.clone();
        let fail = self.fail_next;
        Box::pin(async move {
            if fail {
                return Err(anyhow::anyhow!("mock provider error"));
            }
            let (tx, rx): (tokio::sync::mpsc::UnboundedSender<Result<String>>, tokio::sync::mpsc::UnboundedReceiver<Result<String>>) = tokio::sync::mpsc::unbounded_channel();
            for chunk in &chunks {
                let _ = tx.send(Ok(chunk.clone()));
            }
            let stream = futures::stream::iter(
                chunks
                    .into_iter()
                    .map(|c| Ok(c) as Result<String>),
            );
            Ok(Box::pin(stream) as Pin<Box<dyn Stream<Item = Result<String>> + Send>>)
        })
    }
}

/// A mock tool registry for testing.
#[derive(Debug, Default)]
pub struct MockRuntimeToolRegistry {
    tools: Vec<(String, String, String)>, // (name, description, result)
    fail_next: bool,
}

impl MockRuntimeToolRegistry {
    pub fn new() -> Self {
        MockRuntimeToolRegistry {
            tools: Vec::new(),
            fail_next: false,
        }
    }

    pub fn with_tool(mut self, name: &str, description: &str, result: &str) -> Self {
        self.tools.push((
            name.to_string(),
            description.to_string(),
            result.to_string(),
        ));
        self
    }

    pub fn fail(&mut self) {
        self.fail_next = true;
    }
}

impl RuntimeToolRegistry for MockRuntimeToolRegistry {
    fn has_tool(&self, name: &str) -> bool {
        self.tools.iter().any(|(n, _, _)| n == name)
    }

    fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|(n, _, _)| n.clone()).collect()
    }

    fn execute_tool(
        &mut self,
        name: &str,
        _args: &str,
        _correlation_id: &str,
    ) -> Result<String> {
        if self.fail_next {
            return Err(anyhow::anyhow!("mock tool error"));
        }
        self.tools
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, _, r)| r.clone())
            .ok_or_else(|| anyhow::anyhow!("unknown tool: {}", name))
    }

    fn tool_description(&self, name: &str) -> Option<String> {
        self.tools
            .iter()
            .find(|(n, _, _)| n == name)
            .map(|(_, d, _)| d.clone())
    }
}

/// A mock event emitter for testing.
#[derive(Debug, Default)]
pub struct MockRuntimeEventEmitter {
    pub events: std::sync::Mutex<Vec<RuntimeEvent>>,
}

impl MockRuntimeEventEmitter {
    pub fn new() -> Self {
        MockRuntimeEventEmitter {
            events: std::sync::Mutex::new(Vec::new()),
        }
    }

    pub fn events(&self) -> Vec<RuntimeEvent> {
        self.events.lock().unwrap().clone()
    }
}

impl RuntimeEventEmitter for MockRuntimeEventEmitter {
    fn emit(&self, event: RuntimeEvent) -> bool {
        self.events.lock().unwrap().push(event);
        true
    }

    fn emit_raw(&self, event: RuntimeEvent) -> bool {
        self.events.lock().unwrap().push(event);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mock_provider_send() {
        let provider = MockRuntimeProvider::new("test", "hello world");
        assert_eq!(provider.name(), "test");
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.send_message("hi", "corr"));
        assert_eq!(result.unwrap(), "hello world");
    }

    #[test]
    fn test_mock_provider_fail() {
        let mut provider = MockRuntimeProvider::new("test", "hello");
        provider.fail();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(provider.send_message("hi", "corr"));
        assert!(result.is_err());
    }

    #[test]
    fn test_mock_registry() {
        let registry = MockRuntimeToolRegistry::new()
            .with_tool("read_file", "Read a file", "file contents");

        assert!(registry.has_tool("read_file"));
        assert!(!registry.has_tool("write_file"));
        assert_eq!(registry.tool_names(), vec!["read_file"]);
        assert_eq!(
            registry.tool_description("read_file").unwrap(),
            "Read a file"
        );
    }

    #[test]
    fn test_mock_registry_execute() {
        let mut registry = MockRuntimeToolRegistry::new()
            .with_tool("read_file", "Read a file", "file contents");
        let result = registry
            .execute_tool("read_file", "path", "corr")
            .unwrap();
        assert_eq!(result, "file contents");
    }

    #[test]
    fn test_mock_emitter() {
        let emitter = MockRuntimeEventEmitter::new();
        assert!(emitter.emit(RuntimeEvent::StateChange {
            from: super::super::state::RuntimeState::Idle,
            to: super::super::state::RuntimeState::Observing,
        }));
        assert_eq!(emitter.events().len(), 1);
    }

    #[test]
    fn test_health_status_label() {
        assert_eq!(HealthStatus::Healthy.label(), "healthy");
        assert_eq!(HealthStatus::Degraded.label(), "degraded");
        assert_eq!(HealthStatus::Unhealthy.label(), "unhealthy");
        assert_eq!(HealthStatus::Unknown.label(), "unknown");
    }
}

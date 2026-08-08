pub mod capabilities;
pub mod diagnostics;
pub mod request;
pub mod response;
pub mod router;
pub mod stream;
pub mod structured_output;
pub mod tool_contract;
pub mod types;

#[cfg(test)]
mod tests;

pub use capabilities::{Capability, CapabilityNegotiation, CapabilitySet, SupportedCapabilities};
pub use diagnostics::{DiagnosticEvent, DiagnosticLevel, RuntimeDiagnostics};
pub use request::{MessageRole, ModelRequest};
pub use response::{Choice, ModelResponse, ResponseUsage};
pub use router::{ModelCandidate, RoutingConfig, RoutingDecision, RuntimeRouter};
pub use stream::{StreamEvent, StreamPipeline, StreamSegment, StreamingOutput};
pub use structured_output::{JsonSchema, StructuredOutputSchema, StructuredOutputValidator};
pub use tool_contract::{ToolArgument, ToolDefinition, ToolResult as ToolResultData, ToolSchema};
pub use types::{
    AIRRuntimeError, AIRRuntimeResult, CostEstimate, HealthStatus, ModelId, Priority, ProviderType,
};

/// High-level AI Runtime that wraps the router and provides a unified interface.
pub struct AIRRuntime {
    router: RuntimeRouter,
}

impl AIRRuntime {
    pub fn new(config: RoutingConfig) -> Self {
        AIRRuntime {
            router: RuntimeRouter::new(config),
        }
    }

    pub fn router(&self) -> &RuntimeRouter {
        &self.router
    }

    pub fn router_mut(&mut self) -> &mut RuntimeRouter {
        &mut self.router
    }
}

impl std::fmt::Debug for AIRRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AIRRuntime")
            .field("router", &self.router)
            .finish()
    }
}

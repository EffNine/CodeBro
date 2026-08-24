//! LEGACY provider implementations (pre-MCP TUI era).
//!
//! The OpenAI-compatible chat provider and its capability/cost trait lived
//! here before the MCP pivot; the live runtime talks to providers only
//! through the consultant layer (`crate::consultant`) and model discovery
//! (`crate::providers::models`). Compiled only for the legacy regression
//! suite — see `crate::legacy`.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

pub mod openai;
pub mod provider;

#[allow(unused_imports)]
pub use openai::OpenAiProvider;
#[allow(unused_imports)]
pub use provider::{Provider, StructuredToolCall, ToolDefinition};
/// Compatibility alias: legacy code imports the provider-era ToolDefinition
/// through this module.
pub use provider::ToolDefinition as ToolDefinitionReexport;

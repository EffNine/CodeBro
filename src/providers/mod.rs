#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
mod models;
mod openai;
mod provider;

pub use models::{discover_model, fetch_models, pick_default};
pub use openai::OpenAiProvider;
pub use provider::{Provider, StructuredToolCall, ToolDefinition};

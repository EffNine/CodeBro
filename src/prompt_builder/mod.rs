//! Prompt Builder v2 — Engineering Intelligence Compiler.
//!
//! The Prompt Builder compiles an engineering context package into an
//! optimal prompt based on project state, task intent, and runtime knowledge.
//! It is NOT a string concatenation utility.
//!
//! Pipeline:
//!
//! ```text
//! User Request
//!   ↓
//! Intent Classification (IntentPlan)
//!   ↓
//! Context Assembly (Context)
//!   ↓
//! Engineering Memory (MemoryResolution)
//!   ↓
//! Project Identity (ProjectInfo)
//!   ↓
//! Prompt Builder v2
//!   ↓
//! Compiled Prompt
//!   ↓
//! Provider Runtime
//! ```
//!
//! Determinism: The same inputs always generate the same prompt.
//! No random ordering. No HashMap iteration ordering.

pub mod builder;
pub mod compiler;
pub mod diagnostics;
pub mod ordering;
pub mod sections;
pub mod statistics;
pub mod template;

pub use builder::PromptBuilder;
pub use compiler::CompiledPrompt;
pub use diagnostics::PromptDiagnostics;
pub use ordering::PromptOrdering;
pub use statistics::PromptStatistics;
pub use template::{PromptSection, PromptTemplate, TemplateSelection};

//! Engineering Context Runtime — the canonical runtime contract for CodeBro.
//!
//! `EngineeringContext` is the single source of truth describing everything
//! required to execute one engineering task. It is consumed by every
//! subsystem (Prompt Builder, Project Identity, Engineering Memory,
//! Context Assembly, Task Graph, Provider Runtime, Reflection, and
//! future intelligence systems).
//!
//! ## Architecture
//!
//! ```text
//! Context Assembly
//!     ↓
//! EngineeringContext (immutable runtime state)
//!     ↓
//! Prompt Builder → compile(context) → CompiledPrompt
//! ```
//!
//! ## Ownership Rules
//!
//! - `EngineeringContext` owns immutable runtime state.
//! - Subsystems may read but never mutate.
//! - Mutations happen through `EngineeringContextBuilder` or future runtime services.
//!
//! ## Determinism
//!
//! `EngineeringContext` is fully deterministic. No `HashMap` ordering,
//! no runtime randomness, stable serialization.
//!
//! ## Serialization
//!
//! All types implement `serde::Serialize` and `serde::Deserialize`.

pub mod builder;
pub mod constraints;
pub mod context;
pub mod diagnostics;
pub mod identity;
pub mod memory;
pub mod runtime;
pub mod statistics;
pub mod workspace;

pub use builder::EngineeringContextBuilder;
pub use context::{ContextFragment, ConversationMessage, EngineeringContext, IntentPlan};
pub use diagnostics::EngineeringContextDiagnostics;
pub use identity::ProjectIdentity;
pub use memory::EngineeringMemoryContext;
pub use runtime::{EngineeringContextProvider, RuntimeContext};
pub use statistics::EngineeringContextStatistics;
pub use workspace::WorkspaceContext;

pub use constraints::{ConstraintContext, EngineeringConstraint};

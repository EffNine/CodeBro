//! Engineering Objective — project goal awareness.
//!
//! Sprint 27 makes CodeBro reason about *why* it is doing a task, *where*
//! the task fits in the project, and *what the smallest correct change is*.
//!
//! This module is intentionally small. It is NOT another memory system,
//! context system, task system, or planning engine. It extends the existing
//! canonical architecture:
//!
//! ```text
//! END GOAL
//!     ↓
//! PROJECT VISION
//!     ↓
//! CURRENT OBJECTIVE
//!     ↓
//! CURRENT MILESTONE / SPRINT
//!     ↓
//! CURRENT TASK
//!     ↓
//! CURRENT ACTION
//! ```
//!
//! ## Components
//!
//! | File | Purpose |
//! |------|---------|
//! | `objective.rs` | Compact `EngineeringObjective` model + goal alignment |
//! | `storage.rs` | `.codebro/engineering_objective.json` persistence |
//! | `provider.rs` | `EngineeringObjectiveRuntime` + provider trait |
//! | `alignment.rs` | Deterministic goal alignment + lazy-execution policy |
//! | `diagnostics.rs` | Objective runtime diagnostics |
//!
//! ## Authority
//!
//! Values come from the repository's project documentation. The documented
//! precedence is:
//!
//! ```text
//! Product Vision > Architecture / ADR > Current Objective > Sprint / Milestone > Task > Temporary Memory
//! ```
//!
//! The model only ever sees the compact block rendered by
//! [`EngineeringObjective::render_compact`]. The project can know
//! everything; the model sees only what it needs.

pub mod alignment;
pub mod diagnostics;
pub mod objective;
pub mod provider;
pub mod storage;

pub use alignment::{classify_change_scope, ChangeScope, LazyExecutionPolicy};
pub use diagnostics::{ObjectiveDiagnostics, ObjectiveSource};
pub use objective::{EngineeringObjective, GoalAlignment, CURRENT_SCHEMA_VERSION};
pub use provider::{
    EngineeringObjectiveProvider, EngineeringObjectiveRuntime, ObjectiveRuntimeError,
};
pub use storage::{ObjectiveFile, ObjectiveStorage, ObjectiveStorageError};

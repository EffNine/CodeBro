//! Project Identity Runtime — persistent engineering knowledge for CodeBro.
//!
//! `ProjectIdentityRuntime` is the runtime responsible for maintaining
//! persistent project identity across sessions. It loads, validates,
//! updates, and persists the engineering memory of a repository.
//!
//! ## Architecture
//!
//! ```text
//! Repository
//!     ↓
//! ProjectIdentityRuntime (load, validate, update, persist)
//!     ↓
//! EngineeringContextBuilder (snapshot → consume)
//!     ↓
//! EngineeringContext
//!     ↓
//! PromptBuilder
//!     ↓
//! Provider Runtime
//! ```
//!
//! ## Key Types
//!
//! | Type | Responsibility |
//! |------|---------------|
//! | `ProjectIdentity` | The engineering memory snapshot |
//! | `ProjectIdentityRuntime` | Runtime: load, create, update, persist |
//! | `ProjectIdentityBuilder` | Fluent construction with validation |
//! | `ProjectIdentityLoader` | Loading from `.codebro/` files |
//! | `ProjectIdentityStorage` | File-based persistence |
//! | `ProjectIdentityUpdater` | Applying engineering state changes |
//! | `ProjectIdentityStatistics` | Aggregate metrics |
//! | `ProjectIdentityDiagnostics` | Runtime diagnostics |
//! | `ProjectIdentityProvider` | Trait for future runtimes |
//!
//! ## What Is NOT Stored
//!
//! - Conversation history
//! - LLM responses
//! - Prompt text
//! - Provider state
//! - Temporary diagnostics
//! - Runtime caches
//! - Anything session-specific
//!
//! ## Persistence Layout
//!
//! All files live under `<workspace_root>/.codebro/`:
//! - `project_identity.json` — full identity snapshot
//! - `workspace.json` — workspace metadata
//! - `architecture.json` — architecture summary and patterns
//! - `engineering_decisions.json` — recorded decisions
//! - `constraints.json` — known constraints
//! - `roadmap.json` — roadmap items
//! - `current_sprint.json` — active sprint
//! - `metadata.json` — schema version and timestamps

pub mod builder;
pub mod diagnostics;
pub mod identity;
pub mod loader;
pub mod migration;
pub mod statistics;
pub mod storage;
pub mod updater;
pub mod validation;
pub mod runtime;

pub use builder::{IdentityBuildError, ProjectIdentityBuilder};
pub use diagnostics::{IdentitySource, ProjectIdentityDiagnostics};
pub use identity::{
    CURRENT_SCHEMA_VERSION, DecisionStatus, EngineeringDecision, ProjectIdentity,
    RoadmapItem, RoadmapStatus,
};
pub use loader::{LoadError, LoadResult, ProjectIdentityLoader};
pub use runtime::{ProjectIdentityProvider, ProjectIdentityRuntime, RuntimeError};
pub use statistics::ProjectIdentityStatistics;
pub use storage::{ProjectIdentityStorage, StorageError};
pub use updater::{IdentityChanges, ProjectIdentityUpdater, UpdateResult};
pub use validation::{ValidationIssue, ValidationReport, validate_identity};

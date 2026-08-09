//! Engineering Memory Runtime — persistent, project-tier engineering knowledge.
//!
//! Sprint 24 adds project-tier memory to CodeBro. This module extends the
//! existing `memory_runtime` through a thin integration layer that:
//!
//! - Persists only project-tier entries in `.codebro/engineering_memory.json`.
//! - Stays isolated from `project_identity.json`: identity is stable repository
//!   knowledge; memory is curated, task-relevant engineering knowledge.
//! - Depends on `ProjectIdentityProvider` to verify project scope and expose
//!   project-aware diagnostics without mutating identity.
//! - Provides explicit operations only: load, record, update, delete, snapshot,
//!   and resolve for a task. No automatic learning, reflection, or LLM-driven
//!   memory writes.
//!
//! ## Resolution Pipeline
//!
//! When resolving memory for an `EngineeringContext`:
//! 1. Read project-tier entries from persistent storage.
//! 2. Filter by task keywords (key or value contains the keyword).
//! 3. Filter by active-file tags (entry must carry at least one matching tag).
//! 4. Filter by minimum confidence.
//! 5. Rank by importance descending, confidence descending, key ascending, id ascending.
//! 6. Enforce a fixed entry budget and token budget.
//! 7. Map selected entries into the immutable `EngineeringMemoryContext`.
//!
//! ## Architecture
//!
//! ```text
//! .codebro/engineering_memory.json
//!     ↓
//! EngineeringMemoryStore (load / persist)
//!     ↓
//! EngineeringMemoryRuntime (record / update / delete / snapshot)
//!     ↓
//! EngineeringMemoryProvider (trait — consumed by Context Assembly)
//!     ↓
//! EngineeringContextBuilder.memory(...)
//!     ↓
//! EngineeringMemoryContext → Prompt Builder (unchanged)
//! ```

pub mod provider;
pub mod resolver;
pub mod runtime;
pub mod store;
pub mod types;

pub use provider::EngineeringMemoryProvider;
pub use resolver::EngineeringMemoryResolver;
pub use runtime::EngineeringMemoryRuntime;
pub use store::EngineeringMemoryStore;
pub use types::{
    EngineeringMemoryEntry, EngineeringMemoryFile, EngineeringMemoryMetadata,
    EngineeringMemoryResolveError, EngineeringMemoryResolveResult,
};

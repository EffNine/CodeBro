#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Runtime Foundation for CodeBro (P10.0).
//!
//! This module provides the shared runtime infrastructure that all
//! runtime variants (AI Runtime, Provider Runtime, Memory Runtime, etc.)
//! build upon. It contains five components:
//!
//! - **RuntimeContext** (`context`): Shared, immutable snapshot of data
//!   passed through every pipeline phase.
//! - **RuntimeLifecycle** (`lifecycle`): Host-level lifecycle management
//!   (Created → Running → Paused → Stopping → Stopped).
//! - **RuntimeTraits** (`traits`): Trait abstractions for providers,
//!   tool registries, event emitters, and context factories used by the
//!   pipeline.
//! - **RuntimeEvents** (`events`): Events emitted by the runtime pipeline
//!   to notify the TUI and observers of pipeline progress.
//! - **RuntimeDiagnostics** (`diagnostics`): Phase-aware diagnostics
//!   collecting per-phase timing, state transition counts, and error
//!   traces.
//!
//! # Architecture
//!
//! The runtime foundation is intentionally additive. It does not modify
//! existing traits (`Provider`, `Tool`), the `AgentEvent` enum, or the
//! `RuntimeState` machine (in `state.rs`). Instead, it wraps and
//! observes existing operations through the traits defined here.
//!
//! # Thread Safety
//!
//! All components are `Clone` (via `Arc<Mutex<>>` where needed) and safe
//! to share across tasks. No component requires `UnsafeCell` or raw
//! pointers.

pub mod context;
pub mod diagnostics;
pub mod events;
pub mod lifecycle;
pub mod state;
pub mod traits;

pub use context::RuntimeContext;
pub use diagnostics::{PipelineDiagnostics, RuntimeDiagnostics};
pub use events::RuntimeEvent;
pub use lifecycle::{RuntimeLifecycle, RuntimeLifecycleState};
pub use state::RuntimeState;
pub use traits::{
    HealthStatus, MockRuntimeEventEmitter, MockRuntimeProvider,
    MockRuntimeToolRegistry, RuntimeContextFactory, RuntimeEventEmitter,
    RuntimeObservable, RuntimeProvider, RuntimeToolRegistry,
};

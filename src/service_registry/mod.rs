#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Service Registry — the official communication layer between plugins.
//!
//! Plugins MUST NOT:
//! - Keep direct references to each other
//! - Call each other directly
//! - Share mutable state
//!
//! Everything goes through the Service Registry.
//!
//! # Architecture
//!
//! ```text
//! Service Registry
//!   ├─ types        — ServiceId, ServiceName, ServiceVersion, Capability, etc.
//!   ├─ service      — Service definition and builder
//!   ├─ registry     — Core: register, unregister, activate, deactivate, enumerate
//!   ├─ resolver     — Deterministic lookup, version negotiation, capability matching
//!   ├─ discovery    — Metadata queries, filtering, search
//!   ├─ permissions  — Ownership, visibility, access validation
//!   ├─ lifecycle    — State machine: Registered ↔ Activated ↔ Deactivated ↔ Error
//!   └─ diagnostics  — Statistics, failed lookups, violations, events
//! ```
//!
//! # Design Rules
//!
//! - **No direct plugin communication**: All inter-plugin calls go through the registry.
//! - **Deterministic resolution**: Priority → Version → Registration order. Never random.
//! - **Thread-safe**: All public types are `Send + Sync + Clone`.
//! - **Observable**: Emits events via the observability platform (P9.2).
//! - **Permission enforced**: Access checks on every resolution.
//! - **Future compatible**: Supports AI Runtime, LLM Providers, Enterprise, Marketplace,
//!   Cloud Services, Remote Services without redesign.

pub mod diagnostics;
pub mod discovery;
pub mod lifecycle;
pub mod permissions;
pub mod registry;
pub mod resolver;
pub mod service;
pub mod types;

pub use diagnostics::{DiagnosticSnapshot, ServiceDiagnostics};
pub use discovery::{DiscoveryResult, ServiceDiscovery};
pub use lifecycle::{LifecycleState, LifecycleTransition, ServiceLifecycle};
pub use permissions::{AccessResult, ServicePermissions};
pub use registry::{RegistryError, ServiceRegistry};
pub use resolver::{DependencyCheckError, ServiceResolver};
pub use service::{Service, ServiceBuildError, ServicePermission};
pub use types::*;

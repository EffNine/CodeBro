#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Consultant provider registry.

use std::sync::Arc;

pub mod conductor;
pub mod mock;

pub use conductor::ConductorProvider;
pub use mock::MockProvider;

/// Build the default provider set registered with the consultant system.
///
/// Conductor is the API-backed provider: it is registered by default and
/// selected explicitly via `provider: "conductor"` (or by `auto`).
pub fn default_providers() -> Vec<Arc<dyn super::provider::ConsultantProvider>> {
    vec![Arc::new(ConductorProvider::new())]
}

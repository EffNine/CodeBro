//! Preference Engine — P6.1 Foundation
//!
//! The single source of truth for all developer preferences.
//!
//! - Deterministic: no LLM calls, no automatic inference
//! - Thread-safe: all public methods are safe across threads
//! - Platform independent: no Runtime/Tool/Intelligence coupling
//! - Fully testable: every operation is explicit and observable
//!
//! Architecture:
//!
//! ```text
//! Preference API (public)
//!   ├── PreferenceSchema (typed model)
//!   ├── PreferenceStore (persistent storage abstraction)
//!   ├── PreferenceValidator (schema/values/compatibility/version/migration)
//!   ├── PreferencePersistence (atomic writes, backup, rollback, corruption detection)
//!   ├── PreferenceEvent (observers for preference changes)
//!   └── PreferenceDiagnostics (failure tracking)
//! ```

pub mod diagnostics;
pub mod events;
pub mod persistence;
pub mod schema;
pub mod store;
pub mod validation;

pub use diagnostics::*;
pub use events::*;
pub use persistence::*;
pub use schema::*;
pub use store::*;
pub use validation::*;

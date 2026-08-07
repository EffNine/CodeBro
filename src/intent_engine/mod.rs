//! Intent Engine — P6.2 Foundation
//!
//! Translates natural language into deterministic Intent Plans and Commands.
//!
//! Pipeline:
//!
//! ```text
//! User Input
//!   ↓
//! Intent Classifier (deterministic rules/patterns)
//!   ↓
//! Intent Plan (structured, explainable, serializable)
//!   ↓
//! Intent Resolver (plan → immutable commands)
//!   ↓
//! Preference Commands (immutable, auditable)
//!   ↓
//! Approval Preview (read-only, no mutations)
//!   ↓
//! Approval Gate
//!   ↓
//! Preference Engine
//! ```
//!
//! Design rules:
//! - Never modifies preferences directly
//! - Never owns state
//! - Never bypasses Approval Gate
//! - Deterministic first, AI fallback only as architecture
//! - Fully testable, platform independent

pub mod ambiguity;
pub mod classifier;
pub mod confidence;
pub mod diagnostics;
pub mod preview;
pub mod resolver;
pub mod types;

pub use ambiguity::*;
pub use classifier::*;
pub use confidence::*;
pub use diagnostics::*;
pub use preview::*;
pub use resolver::*;
pub use types::*;

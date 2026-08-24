//! Adaptive Validation — P6.5 Foundation
//!
//! Quality guardian of the complete decision pipeline.
//! Validates every plan before Preview and Approval.
//!
//! - Never changes intent
//! - Never changes recommendations
//! - Never changes workflows
//! - Never executes commands
//! - Read-only evaluation
//!
//! Pipeline:
//!
//! ```text
//! User Input
//!   ↓
//! Intent Engine
//!   ↓
//! Recommendation Engine
//!   ↓
//! Workflow Engine
//!   ↓
//! Adaptive Validation (read-only evaluator)
//!   ↓
//! Preview
//!   ↓
//! Approval Gate
//!   ↓
//! Preference Engine
//! ```
//!
//! Design rules:
//! - Stateless observer
//! - Deterministic evaluation
//! - Policy-driven validation
//! - Immutable outputs
//! - Thread-safe

pub mod confidence;
pub mod diagnostics;
pub mod engine;
pub mod policy;
pub mod risk;
pub mod rules;
pub mod types;
pub mod validator;

pub use confidence::*;
pub use diagnostics::*;
pub use engine::*;
pub use policy::*;
pub use risk::*;
pub use rules::*;
pub use types::*;
pub use validator::*;

//! Recommendation Engine — P6.3 Foundation
//!
//! Observer module that consumes Intent Plans and produces optional recommendations.
//!
//! - Never owns state
//! - Never mutates preferences
//! - Never executes commands
//! - Deterministic and rule-based
//! - Fully explainable and auditable
//!
//! Pipeline:
//!
//! ```text
//! Intent Plan
//!   ↓
//! RecommendationEngine (observer)
//!   ↓
//! Vec<Recommendation> (optional, read-only)
//!   ↓
//! Preview (merged with Intent Engine preview)
//!   ↓
//! Approval Gate
//! ```
//!
//! Design rules:
//! - Observes only, never modifies
//! - Rule-based deterministic matching
//! - Every recommendation includes source rule and evidence
//! - Thread-safe, platform independent
//! - No LLM, no AI, no adaptive learning

pub mod diagnostics;
pub mod engine;
pub mod filter;
pub mod ranking;
pub mod rules;
pub mod types;

pub use diagnostics::*;
pub use engine::*;
pub use filter::*;
pub use ranking::*;
pub use rules::*;
pub use types::*;

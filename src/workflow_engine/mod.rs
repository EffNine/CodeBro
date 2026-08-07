//! Workflow Engine — P6.4 Foundation
//!
//! Composes approved commands into deterministic workflow plans.
//!
//! - Never owns state
//! - Never mutates preferences
//! - Never executes commands
//! - Never bypasses Approval Gate
//!
//! Pipeline:
//!
//! ```text
//! Intent Plan + RecommendationSet
//!   ↓
//! WorkflowEngine (planner)
//!   ↓
//! WorkflowPlan (immutable, deterministic)
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
//! - Deterministic: same inputs → same output
//! - Immutable outputs
//! - Dependency-aware planning
//! - Explainable via structured issues

pub mod dependency;
pub mod diagnostics;
pub mod ordering;
pub mod planner;
pub mod preview;
pub mod types;
pub mod validator;

pub use dependency::*;
pub use diagnostics::*;
pub use ordering::*;
pub use planner::*;
pub use preview::*;
pub use types::*;
pub use validator::*;

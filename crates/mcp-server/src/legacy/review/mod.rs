//! Sprint 30G — Autonomous Review Subagent.
//!
//! A read-only reviewer that inspects the repository state, compares intended
//! changes against actual changes, evaluates verification evidence, detects
//! plan deviations and unverified changes, and surfaces concrete,
//! evidence-backed findings for the main LLM to consume.
//!
//! The reviewer never mutates the repository. It is failure-isolated from the
//! parent task.

pub mod contract;
pub mod limits;
pub mod permissions;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use contract::{
    ReviewFinding, ReviewRequest, ReviewResult, ReviewSeverity, ReviewTermination, ReviewVerdict,
};
pub use limits::ReviewLimits;
pub use permissions::{build_review_tool_registry, ReviewPermissionHook, ReviewTooling};
pub use runtime::ReviewSubagent;

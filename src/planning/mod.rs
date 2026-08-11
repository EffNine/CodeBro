//! Sprint 30E — Autonomous Planning Subagent.
//!
//! The third genuinely autonomous specialist: it turns
//!
//! ```text
//! user objective
//!      + GroundedContext (Sprint 30B)
//!      + ResearchResult (Sprint 30C — what actually exists)
//!      + TestingResult (Sprint 30D — what actually works)
//! ```
//!
//! into an evidence-backed implementation plan:
//!
//! ```text
//! Planning objective
//!      ↓
//! LLM decides what repository information is missing
//!      ↓
//! read-only tool (targeted read — never a broad scan)
//!      ↓
//! observation
//!      ↓
//! next decision
//!      ↓
//! reserved plan synthesis
//!      ↓
//! PlanningResult
//!      ↓
//! main EngineeringContext → main LLM prompt
//! ```
//!
//! Planning is strictly READ-ONLY. A restricted registry only exposes the
//! allowlisted tools and an explicit [`PlanningPermissionHook`] deny-lists
//! anything else; `run_command` is not even registered — Testing already owns
//! command execution. Planning can never modify source files, execute
//! arbitrary commands or mutate git state.
//!
//! It must also NOT become Coding: it produces an evidence-backed plan with
//! concrete files, symbols, validation and risks. It never generates patches.
//!
//! Failure isolation: a planning failure produces a bounded error result; the
//! coordinator continues with the existing context.

pub mod contract;
pub mod limits;
pub mod permissions;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use contract::{
    PlanStep, PlanningEvidence, PlanningRequest, PlanningResult, PlanningRisk, PlanningTermination,
};
pub use limits::PlanningLimits;
pub use permissions::{
    build_planning_tool_registry, install_planning_permission_hook, PlanningPermissionHook,
    PlanningTooling,
};
pub use runtime::PlanningSubagent;

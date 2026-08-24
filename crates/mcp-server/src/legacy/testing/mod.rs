//! Sprint 30D — Autonomous Testing Subagent.
//!
//! The second genuinely autonomous specialist. Unlike Research (read-only),
//! Testing is allowed to execute **bounded validation commands**, but it can
//! never modify repository source state:
//!
//! ```text
//! Testing objective
//!      ↓
//! LLM decides validation action
//!      ↓
//! bounded command execution (policy-checked)
//!      ↓
//! real stdout + authoritative exit code
//!      ↓
//! observation
//!      ↓
//! next decision
//!      ↓
//! reserved synthesis
//!      ↓
//! TestingResult
//! ```
//!
//! The execution result belongs to the machine (exit code), the
//! interpretation belongs to the model. Never reversed.
//!
//! Safety: Testing is NOT unrestricted shell. `run_command` is the single
//! execution surface, gated by the explicit [`TestingCommandPolicy`] at the
//! permission layer and again before any process spawns. Mutating tools are
//! neither registered nor permitted, and the git state is snapshotted before
//! and after so any unexpected tracked-tree mutation is surfaced.

pub mod contract;
pub mod limits;
pub mod permissions;
pub mod policy;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use contract::{
    GitStateSnapshot, TestCommandResult, TestFailure, TestFailureKind, TestFinding,
    TestObservation, TestingRequest, TestingResult, TestingTermination,
};
pub use limits::TestingLimits;
pub use permissions::{
    build_testing_tool_registry, install_testing_permission_hook, TestingPermissionHook,
    TestingTooling,
};
pub use policy::{CommandDecision, TestingCommandPolicy};
pub use runtime::TestingSubagent;

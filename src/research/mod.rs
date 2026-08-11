//! Sprint 30C — Autonomous Research Subagent.
//!
//! One bounded, real executor that performs read-only repository research:
//!
//! ```text
//! GroundedContext (Sprint 30B)
//!      + real read-only tool execution (list_files / read_file / git_status)
//!      + bounded ReAct iteration (shared canonical provider primitive)
//!      + structured ResearchResult
//! ```
//!
//! Safety: research is READ-ONLY. A restricted registry only exposes the
//! allowlisted tools, and an explicit `ResearchPermissionHook` deny-lists
//! anything else. Research can never modify repository or git state.
//!
//! Failure isolation: a research failure produces a bounded error result;
//! the coordinator continues with the existing context.

pub mod contract;
pub mod limits;
pub mod permissions;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use contract::{
    truncate_chars, ResearchFinding, ResearchRequest, ResearchResult, ResearchTermination,
    ToolObservation,
};
pub use limits::ResearchLimits;
pub use permissions::{build_research_tool_registry, ResearchPermissionHook, ResearchTooling};
pub use runtime::ResearchSubagent;

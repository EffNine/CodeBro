//! Sprint 30F — Autonomous Coding Subagent.
//!
//! The fourth genuinely autonomous specialist: it turns
//!
//! ```text
//! user objective
//!      + GroundedContext (Sprint 30B)
//!      + ResearchResult (Sprint 30C — what actually exists)
//!      + TestingResult (Sprint 30D — what actually works)
//!      + PlanningResult (Sprint 30E — what must change and how to validate)
//! ```
//!
//! into verified, reversible repository mutation:
//!
//! ```text
//! Coding objective
//!      ↓
//! LLM decides which repository information is missing
//!      ↓
//! read-only tool (targeted read — never a broad scan)
//!      ↓
//! observation
//!      ↓
//! propose_change (mutation via ChangeEngine behind a permission boundary)
//!      ↓
//! verify (runtime-intercepted policy-checked command, authoritative exit code)
//!      ↓
//! bounded revision on explicit verify failure
//!      ↓
//! reserved final synthesis → completion gate auto-verifies unverified changes
//!      (no plan validation commands → VerificationUnavailable, never faked)
//!      ↓
//! CodingResult
//!      ↓
//! main EngineeringContext → main LLM prompt
//! ```
//!
//! Coding is the FIRST mutating subagent, which is why it carries the strictest
//! boundaries:
//!
//! - **No raw filesystem writes.** All mutation goes through the
//!   [`ChangeEngine`] behind an explicit permission boundary ([`permissions`]):
//!   existing-file writes ride [`ChangePlan`](crate::tools::ChangePlan) /
//!   [`PatchEngine`](crate::tools::PatchEngine), and file creation is the
//!   engine's documented controlled creation seam. The model surface is
//!   `propose_change` + `verify`, both runtime-intercepted.
//! - **Plan-driven.** Coding consumes the REAL `PlanningResult`, not rendered
//!   prose, and enforces plan adherence: changes outside the plan are recorded
//!   as unplanned (never silently added), and a strict mode denies them.
//! - **No blind overwrite.** `propose_change` requires a unique `old` match;
//!   ambiguous matches and stale content are refused, so pre-existing user
//!   changes are preserved.
//! - **Reversible.** A terminal failure (verification-failed or error) rolls
//!   back only the session's own changes, in reverse order; created files are
//!   removed only when their content still matches what the session wrote.
//! - **Honest verification.** `AppliedChange.verified` (and therefore
//!   [`CodingResult::all_verified`]) is true ONLY after an authoritative
//!   machine verification succeeded. A session that applied changes but had no
//!   validation commands terminates as `VerificationUnavailable` — never
//!   fabricated as verified.
//! - **Bounded.** Independent iteration/tool/model/timeout budgets plus a
//!   bounded revision budget ([`limits::MAX_REVISION_ATTEMPTS`]).
//! - **No `run_command`.** Coding has NO arbitrary execution surface;
//!   verification is runtime-driven through
//!   [`TestingTooling`](crate::testing::TestingTooling) and the
//!   [`TestingCommandPolicy`](crate::testing::TestingCommandPolicy), so exit
//!   codes stay authoritative.
//! - **Never mutates git history.** No commit, no checkout — only working-tree
//!   file edits through the change engine.
//!
//! Failure isolation: a coding failure produces a bounded error result with
//! the session's own changes rolled back; the coordinator continues.

pub mod contract;
pub mod limits;
pub mod permissions;
pub mod runtime;

#[cfg(test)]
mod tests;

pub use contract::{
    AppliedChange, CodingRequest, CodingResult, CodingTermination, VerificationRecord,
    VerificationSource,
};
pub use limits::CodingLimits;
pub use permissions::{
    build_coding_tool_registry, install_coding_permission_hook, ChangeEngine, CodingPermissionHook,
    CodingTooling, PreparedChange,
};
pub use runtime::CodingSubagent;

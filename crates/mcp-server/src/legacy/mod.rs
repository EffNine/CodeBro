//! LEGACY ARCHITECTURE — NOT PART OF THE CODEBRO MCP RUNTIME.
//!
//! Everything under this module belongs to retired product directions:
//!
//! - the pre-MCP TUI coding agent (agent loop, planner, subagents, tool
//!   platform, canonical runtime, prompt assembly),
//! - the Adaptive Developer Platform phase (intent/preference/recommendation
//!   engines, plugin SDK, service registry, workflow engine, ...).
//!
//! This code is compiled ONLY so its regression suite keeps passing while the
//! migration completes. It has no entry point from `main`, is never reached by
//! any MCP tool, and must not gain new functionality. The single approved
//! direction out of this module is deletion.
//!
//! Live runtime: see `src/mcp`, `src/fact_store`, `src/engineering_facts`,
//! `src/engineering_memory`, `src/memory_runtime`, `src/project_identity`,
//! `src/init`, `src/sandbox`, `src/coding` (change engine), `src/consultant`,
//! `src/impact`, `src/doctor`.
// Legacy is exempt from strict lints by policy (see docs/LEGACY_RETIREMENT.md):
// it compiles only under cfg(test) and its approved direction is deletion.
#![allow(
    dead_code,
    unused_imports,
    unused_variables,
    clippy::all,
    ambiguous_glob_reexports,
    private_interfaces,
    private_bounds
)]

pub mod adaptive_validation;
pub mod agent;
pub mod ai_runtime;
pub mod assembly;
pub mod canonical_runtime;
pub mod capability_discovery;
pub mod coding_contract;
pub mod coding_tests;
pub mod coding_limits;
pub mod coding_runtime;
pub mod coding_tooling;
pub mod dispatcher;
pub mod engineering_context;
pub mod engineering_objective;
pub mod integration_pipeline;
pub mod intent_engine;
pub mod metrics;
pub mod observability;
pub mod onboarding;
pub mod planning;
pub mod plugin_sdk;
pub mod preference_engine;
pub mod prompt_builder;
pub mod provider_manager;
pub mod provider_runtime;
pub mod providers;
pub mod recommendation_engine;
pub mod reliability;
pub mod research;
pub mod review;
pub mod runtime;
pub mod scanner;
pub mod service_registry;
pub mod session;
pub mod settings;
pub mod testing;
pub mod tools;
pub mod workflow_engine;
pub mod workspace_discovery;
pub mod workspace_runtime;

/// Legacy regression suite for the deprecated agent engine.
#[cfg(test)]
pub mod tests;

//! CodeBro MCP server — composition root of the engineering runtime.
//!
//! This crate hosts the public surface: the stdio MCP server (`mcp`), CLI
//! (`cli`), diagnostics (`doctor`), model-provider plumbing (`providers`,
//! `credentials`, `consultant`), and the binary entry point.
//!
//! All engineering domains live in dedicated workspace crates; this crate
//! constructs runtimes and delegates. The `pub use` aliases below keep
//! historical `crate::<module>` paths resolving during the workspace split.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

pub mod cli;
pub mod consultant;
pub mod credentials;
pub mod doctor;
pub mod mcp;
pub mod providers;

// ---- path-compatibility re-exports (workspace split) --------------------
pub use codebro_change_engine::coding;
pub use codebro_core::{cancellation, config, error, persistence, provenance, tools};
pub use codebro_fact_store::{engineering_facts, fact_store};
pub use codebro_identity_runtime::project_identity;
pub use codebro_impact_engine::impact;
pub use codebro_indexer::init;
pub use codebro_memory_runtime::{engineering_memory, memory_runtime};
pub use codebro_parsers::intelligence;
pub use codebro_sandbox_runtime::sandbox;

/// Process entry point: tracing init + CLI dispatch.
pub async fn run() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    tracing::info!("CodeBro starting...");
    cli::run().await?;
    tracing::info!("CodeBro session ended.");
    Ok(())
}

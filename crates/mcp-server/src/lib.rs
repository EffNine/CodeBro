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
pub mod debugging;
pub mod doctor;
pub mod engineering_brief;
pub mod engineering_context;
pub mod history_capture;
pub mod integration;
pub mod mcp;
pub mod providers;
pub mod workspace;
pub mod workspace_registry;

// ---- path-compatibility re-exports (workspace split) --------------------
pub use codebro_change_engine::coding;
pub use codebro_context_runtime::context_runtime;
pub use codebro_core::{cancellation, config, error, persistence, provenance, tools};
pub use codebro_fact_store::{engineering_facts, fact_store};
pub use codebro_identity_runtime::project_identity;
pub use codebro_impact_engine::impact;
pub use codebro_indexer::init;
pub use codebro_memory_runtime::{engineering_memory, memory_runtime};
pub use codebro_parsers::intelligence;
pub use codebro_sandbox_runtime::sandbox;

/// Process entry point: tracing init + CLI dispatch.
///
/// P8 protocol hygiene: stdout is the stdio MCP JSON-RPC channel when
/// `codebro serve` runs, so tracing MUST write to stderr. A log line on
/// stdout corrupts the framing (observed live before P8: an ERROR line
/// interleaved with JSON-RPC responses). Tests pin this.
///
/// P8 audit (stderr secret hygiene): the subscriber routes every event —
/// including `rmcp`'s own `response error` lines, which embed raw
/// tool-error messages — through a redacting writer so no caller-supplied
/// secret-shaped text can reach the stderr log verbatim. The single
/// redaction authority is `redact_secrets_public`.
pub async fn run() -> anyhow::Result<()> {
    use std::io::Write;
    /// Redacting stderr writer: tracing's `MakeWriter` extension point.
    /// Every formatted log line passes through the canonical secret
    /// redaction authority before it reaches the OS pipe.
    struct RedactingStderr;
    impl Write for RedactingStderr {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let lossy = String::from_utf8_lossy(buf);
            let redacted = crate::tools::shell::redact_secrets_public(&lossy);
            std::io::stderr().write_all(redacted.as_bytes())?;
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            std::io::stderr().flush()
        }
    }
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(move || RedactingStderr)
        .init();

    tracing::info!("CodeBro starting...");
    cli::run().await?;
    tracing::info!("CodeBro session ended.");
    Ok(())
}

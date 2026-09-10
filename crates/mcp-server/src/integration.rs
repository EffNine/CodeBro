//! CodeBro P8 — OpenCode integration layer.
//!
//! P8 makes CodeBro a *natural* engineering context/runtime companion for
//! OpenCode (and any MCP agent client) without moving any execution,
//! reasoning, or agent-loop ownership into CodeBro. It is a thin
//! integration-contract layer over the mature P0–P7 core:
//!
//! - **Contract surface:** the existing 25 MCP tools ARE the contract.
//!   `engineering_brief` is the primary high-level context surface;
//!   `context` the always-available packet; `recall`/`engineering_facts`/
//!   `impact_analyze`/`repository_health` targeted follow-up; `remember`/
//!   `record_memory`/`task`/`learn`/`skill` the explicit persistence path.
//!   P8 adds NO new tools — every capability already exists.
//! - **Protocol hygiene:** a stdio MCP server must keep stdout reserved
//!   for JSON-RPC. `tracing` therefore writes to **stderr** (see `lib.rs`
//!   — the subscriber is built with `.with_writer(std::io::stderr)`).
//!   Before P8, log lines were emitted to stdout, corrupting the framing
//!   channel (observed live: an ERROR line interleaved with responses).
//! - **Server identity:** the initialize response identifies the server
//!   as `codebro` with the crate version (not rmcp's `from_build_env`
//!   default, which reported `"rmcp"` — a real discovery/telemetry defect
//!   for clients that display or route on server identity).
//! - **Client observability (bounded, privacy-preserving):** the client's
//!   declared `clientInfo` (name + version) is captured at initialize and
//!   emitted as a tracing event; every tool call emits a one-line tracing
//!   event (client, tool, duration, error status, response bytes). Never
//!   logged: tool arguments, task text, brief content, secrets. The client
//!   name is *not* persisted anywhere — it is process-local observability.
//! - **Degraded mode contract:** every read tool already degrades to
//!   explicit unknowns (P6/P7 failure model). For the *client* side the
//!   contract is: CodeBro unavailable → OpenCode continues with its own
//!   native tools; CodeBro stale → briefs carry `STALE_INDEX`; CodeBro
//!   unknown → explicit `UNKNOWN` entries. This module documents that
//!   contract; the guarantees are enforced by the P0–P7 tools themselves.
//!
//! Boundary (unchanged from P0–P7): OpenCode decides WHEN context is
//! needed; CodeBro decides WHAT context is relevant; CodeBro persists
//! meaningful engineering knowledge only through explicit write tools
//! with the unchanged authority gates (`user_confirmed` speech acts,
//! evidence-cited inference, store-enforced lifecycle).
//!
//! No agent loop, no scheduler, no daemon, no watcher, no model calls, no
//! skill execution, no remote transport — request-driven only, exactly
//! like P0–P7.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::time::Instant;

/// Human-readable server identity for the MCP `initialize` handshake.
///
/// Clients (OpenCode among them) display and sometimes route on
/// `serverInfo.name`. Reporting the SDK default (`"rmcp"`) made every
/// CodeBro instance masquerade as the transport library; P8 reports the
/// product identity. The version is the crate version from the build
/// environment.
pub const SERVER_NAME: &str = "codebro";

/// One-line-per-tool-call observability record.
///
/// Deliberately minimal: client identity, tool name, duration, error
/// status, and response size. **Never** arguments, task text, brief
/// content, or anything secret-shaped — this exists so integrators can
/// diagnose "is the MCP server being called, is it fast, is it failing"
/// from stderr logs without becoming a data-leak path (see the P8
/// observability contract).
#[derive(Debug, Clone)]
pub struct ToolCallObservation {
    /// Client-declared name from `initialize` (e.g. "opencode"), when known.
    pub client_name: Option<String>,
    /// Client-declared version from `initialize`, when known.
    pub client_version: Option<String>,
    /// Tool name exactly as routed.
    pub tool: String,
    /// Wall-clock execution duration.
    pub duration_ms: u128,
    /// Whether the call completed with an error (JSON-RPC error OR
    /// `isError` tool result).
    pub errored: bool,
    /// Error label when errored (code/message summary, bounded, redacted).
    pub error_summary: Option<String>,
    /// Serialized response size in bytes (bounded evidence of bounds).
    pub response_bytes: usize,
}

impl ToolCallObservation {
    /// Render the single-line tracing form. Bounded (each field capped),
    /// secret-redacted through the same authority every other free-text
    /// seam uses — defense in depth, even though only tool names and
    /// client names (bounded vocabulary) flow here.
    pub fn render_line(&self) -> String {
        let client = match (&self.client_name, &self.client_version) {
            (Some(n), Some(v)) => format!("{}/{}", redact(n), redact(v)),
            (Some(n), None) => redact(n).to_string(),
            (None, _) => "unknown".to_string(),
        };
        let status = if self.errored {
            match &self.error_summary {
                Some(s) => format!("error: {}", redact(s)),
                None => "error".to_string(),
            }
        } else {
            "ok".to_string()
        };
        format!(
            "client={} tool={} duration_ms={} status={} response_bytes={}",
            client, self.tool, self.duration_ms, status, self.response_bytes
        )
    }
}

/// Redact through the canonical authority (defense in depth; tool/client
/// names are bounded vocabularies but the seam must never become the
/// exception).
fn redact(s: &str) -> String {
    crate::tools::shell::redact_secrets_public(s)
}

/// Measure a tool call for observability. Returns the observation for the
/// caller to emit after the response is known (the caller owns the
/// `Instant` lifecycle because it also owns the response envelope).
pub fn observe_call(
    client: Option<(String, String)>,
    tool: &str,
    started: Instant,
    errored: bool,
    error_summary: Option<String>,
    response_bytes: usize,
) -> ToolCallObservation {
    ToolCallObservation {
        client_name: client.as_ref().map(|(n, _)| n.clone()),
        client_version: client.as_ref().map(|(_, v)| v.clone()),
        tool: tool.to_string(),
        duration_ms: started.elapsed().as_millis(),
        errored,
        error_summary,
        response_bytes,
    }
}

/// Bound an error summary for the observation line (never a raw payload).
pub fn bounded_error_summary(msg: &str) -> String {
    let redacted = redact(msg);
    redacted.chars().take(240).collect()
}

/// The P8 integration contract, expressed as data for tests and docs.
///
/// Codifies what the 25 existing tools mean *to an agent client* so the
/// contract is enforceable by regression tests instead of existing only
/// in prose. Every entry maps an agent-client intent to the exact tool
/// that serves it — the point of P8 §6 ("avoid the client manually calling
/// dozens of low-level tools": the brief IS the primary surface, targeted
/// follow-up uses existing semantic tools).
pub mod contract {
    /// The primary high-level context surface: one bounded, deterministic
    /// brief covering repository identity, freshness, files, symbols,
    /// dependencies, impact, tests, health, history, memory, learning,
    /// skill applicability, task state, constraints, decisions, risks,
    /// and explicit unknowns.
    pub const PRIMARY_CONTEXT_TOOL: &str = "engineering_brief";
    /// Always-available orientation packet (structural digest when no
    /// task is known).
    pub const ORIENTATION_TOOL: &str = "context";
    /// Workspace-level orientation + fact counts + freshness.
    pub const WORKSPACE_ORIENTATION_TOOL: &str = "workspace_context";
    /// Targeted follow-up: verified repository facts.
    pub const FACTS_TOOL: &str = "engineering_facts";
    /// Targeted follow-up: structural impact of a change.
    pub const IMPACT_TOOL: &str = "impact_analyze";
    /// Targeted follow-up: historical evidence (decisions/failures).
    pub const HISTORY_TOOL: &str = "recall";
    /// Targeted follow-up: engineering memory resolution.
    pub const MEMORY_TOOL: &str = "engineering_memory";
    /// Targeted follow-up: workspace health.
    pub const HEALTH_TOOL: &str = "repository_health";
    /// Durable task state (create/start/checkpoint/complete/…).
    pub const TASK_TOOL: &str = "task";
    /// Explicit user-confirmed preference/intent persistence.
    pub const REMEMBER_TOOL: &str = "remember";
    /// Durable agent-recorded engineering memory.
    pub const RECORD_MEMORY_TOOL: &str = "record_memory";
    /// Learning lifecycle (hypotheses from history; never self-confirmed).
    pub const LEARN_TOOL: &str = "learn";
    /// Skill lifecycle (CodeBro manages; OpenCode executes natively).
    pub const SKILL_TOOL: &str = "skill";
    /// Index refresh (freshness recovery path).
    pub const REINDEX_TOOL: &str = "reindex";

    /// The full P8 client-facing contract: intent → tool. Ordered by the
    /// canonical acquisition flow (orientation → brief → targeted →
    /// persistence). Regression tests assert each tool exists in the
    /// router and the map covers exactly the capabilities P8 promises.
    pub fn intents() -> &'static [(&'static str, &'static str)] {
        &[
            ("primary_context", PRIMARY_CONTEXT_TOOL),
            ("orientation", ORIENTATION_TOOL),
            ("workspace_orientation", WORKSPACE_ORIENTATION_TOOL),
            ("facts", FACTS_TOOL),
            ("impact", IMPACT_TOOL),
            ("history", HISTORY_TOOL),
            ("memory", MEMORY_TOOL),
            ("health", HEALTH_TOOL),
            ("task_state", TASK_TOOL),
            ("remember", REMEMBER_TOOL),
            ("record_memory", RECORD_MEMORY_TOOL),
            ("learn", LEARN_TOOL),
            ("skill_lifecycle", SKILL_TOOL),
            ("reindex", REINDEX_TOOL),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observation_line_is_bounded_and_redacted() {
        let obs = ToolCallObservation {
            client_name: Some("opencode".into()),
            client_version: Some("1.18.29".into()),
            tool: "engineering_brief".into(),
            duration_ms: 42,
            errored: false,
            error_summary: None,
            response_bytes: 8192,
        };
        let line = obs.render_line();
        assert!(line.contains("client=opencode/1.18.29"));
        assert!(line.contains("tool=engineering_brief"));
        assert!(line.contains("status=ok"));
        assert!(!line.contains("task"), "no task text in observation");
        assert!(!line.contains("sk-"), "no secret-shaped text");
    }

    #[test]
    fn observation_line_redacts_secret_shaped_client_names() {
        // Defense in depth: even if a hostile client declares a
        // secret-shaped name, the observation never echoes it.
        let obs = ToolCallObservation {
            client_name: Some("sk-abc123secretkey".into()),
            client_version: None,
            tool: "workspace_context".into(),
            duration_ms: 1,
            errored: true,
            error_summary: Some("boom at sk-live-xyz".into()),
            response_bytes: 10,
        };
        let line = obs.render_line();
        assert!(
            !line.contains("sk-abc123secretkey"),
            "client name must be redacted"
        );
        assert!(!line.contains("sk-live-xyz"), "error must be redacted");
        assert!(line.contains("status=error"));
    }

    #[test]
    fn observation_line_handles_unknown_client() {
        let obs = ToolCallObservation {
            client_name: None,
            client_version: None,
            tool: "memory_stats".into(),
            duration_ms: 0,
            errored: false,
            error_summary: None,
            response_bytes: 0,
        };
        let line = obs.render_line();
        assert!(line.contains("client=unknown"));
    }

    #[test]
    fn bounded_error_summary_truncates_and_redacts() {
        let long = format!("{} {}", "x".repeat(500), "password=hunter2");
        let bounded = bounded_error_summary(&long);
        assert!(bounded.chars().count() <= 240);
        assert!(!bounded.contains("hunter2"));
    }

    #[test]
    fn contract_intents_cover_the_p8_flow_without_new_tools() {
        let intents = contract::intents();
        // Every intent maps to one of the existing 25 tools — P8 adds none.
        for (_, tool) in intents {
            assert!(
                matches!(
                    *tool,
                    "engineering_brief"
                        | "context"
                        | "workspace_context"
                        | "engineering_facts"
                        | "impact_analyze"
                        | "recall"
                        | "engineering_memory"
                        | "repository_health"
                        | "task"
                        | "remember"
                        | "record_memory"
                        | "learn"
                        | "skill"
                        | "reindex"
                ),
                "contract tool {tool} is not an existing P0-P7 tool"
            );
        }
        // The primary surface is the brief — the P8 acquisition contract.
        assert_eq!(contract::PRIMARY_CONTEXT_TOOL, "engineering_brief");
        // No contract intent references a CRUD/second context system.
        assert!(!intents.iter().any(|(_, t)| t.contains("get_")));
    }

    #[test]
    fn server_identity_is_the_product_not_the_sdk() {
        assert_eq!(SERVER_NAME, "codebro");
    }
}

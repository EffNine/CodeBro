//! P2 passive history capture: CodeBro records what happened while
//! OpenCode works normally.
//!
//! ```text
//! OpenCode works normally (remember/apply/test …)
//!         ↓
//! meaningful event occurs
//!         ↓
//! capture() — best-effort, redacted, bounded, session-linked
//!         ↓
//! SQLite events → events_fts
//! ```
//!
//! # Invariants
//!
//! - **Best-effort.** Capture never fails the tool it observes: every
//!   error is swallowed into a `tracing::debug!`. History is evidence,
//!   not a gate.
//! - **No recursion.** `recall` and `context` (read paths) capture
//!   nothing — recalling history must not write history.
//! - **Summaries, not transcripts.** Captured payloads are short metadata
//!   (paths, commands, classifications, namespaces); file contents and
//!   full outputs are never copied. Git stays the source of truth for
//!   repository content.
//! - **Structural only.** Capture records that an operation happened, in
//!   which project/task/session, with what outcome. No preference
//!   inference, no promotion — P3's job.
//! - **Subset by design.** Only high-value operations capture: user-context
//!   writes (`remember`/`forget`), applied changes (`apply_change(s)`),
//!   and validation runs (`sandbox_test`/`sandbox_build`). Reads,
//!   memory/identity writes, consults, and raw execs stay quiet; the MCP
//!   surface documents this so absence of an event is explicable.

#![allow(dead_code, unused_imports)] // deliberate product surface beyond current callers; revisit at legacy retirement

use std::path::Path;

use crate::context_runtime::{HistoryInput, HistoryKind};

/// One capturable moment: what happened, where, with what outcome.
pub struct HistoryCapture {
    pub kind: HistoryKind,
    /// Short human line (redacted + truncated at the store seam).
    pub summary: String,
    /// Originating tool (`remember`, `apply_change`, `sandbox_test`, …).
    pub tool: Option<String>,
    /// Affected path, if the event is about one file.
    pub path: Option<String>,
    /// Outcome label (`recorded`, `test_failure`, `success`, …).
    pub outcome: Option<String>,
    /// Small metadata detail (never file contents or full output).
    pub payload: Option<String>,
    /// Task viewpoint when the operation carried one.
    pub task_id: Option<String>,
}

/// Record a capture: resume (or open) the workspace/task session and store
/// the event. Never fails — errors are debug-logged and dropped.
pub fn capture(
    store: &crate::context_runtime::ContextStore,
    workspace_root: &Path,
    capture: HistoryCapture,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let ws = workspace_root.display().to_string();
    let session_id = match store.ensure_active_session(
        &ws,
        capture.task_id.as_deref(),
        Some("mcp-passive"),
        now,
    ) {
        Ok((session, _)) => Some(session.id),
        Err(e) => {
            tracing::debug!("history capture: session resume failed: {e}");
            None
        }
    };
    let mut input = HistoryInput::new(ws, capture.kind, capture.summary);
    input.session_id = session_id;
    input.task_id = capture.task_id;
    input.tool = capture.tool;
    input.path = capture.path;
    input.outcome = capture.outcome;
    input.payload = capture.payload;
    input.source = Some("mcp-passive".to_string());
    if let Err(e) = store.record_history(&input, now) {
        tracing::debug!("history capture: record failed: {e}");
    }
}

/// Bound a detail string for capture payloads (short metadata only).
pub fn detail(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    format!(
        "{}…[truncated for history budget]",
        trimmed.chars().take(max_chars).collect::<String>()
    )
}

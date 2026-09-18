//! Shared bounded context-packet assembly.
//!
//! The `context` MCP tool and the `codebro context` CLI verb (the host/hook
//! integration surface) must agree exactly on retrieval semantics, budgets,
//! and provenance labels. This module is the single implementation both
//! call. It is read-only by construction: it composes existing stores and
//! writes nothing.

use std::path::Path;

use crate::context_runtime::ContextStore;

/// Open the durable user-context store at the default state directory
/// (`CODEBRO_STATE_DIR` or `~/.codebro`). Opening is cheap; the database
/// itself is opened lazily on first use, and retrieval degrades to an empty
/// `records` section rather than failing composition.
pub fn open_default_store() -> ContextStore {
    ContextStore::at_state_dir(crate::mcp::default_state_dir())
}

/// Fetch bounded durable-context excerpts for a workspace.
///
/// Resolution (not concatenation): a generous fetch (keyword-less
/// importance order plus keyword matches when a task narrows relevance)
/// is reduced per (kind, namespace) to one winner by authority rank,
/// then scope specificity (task > project > global), decayed
/// confidence, recency, and id. Actionable intents surface alongside
/// fingerprint winners; losers stay in the store, queryable by id.
/// The user-context store is best-effort by design: if it cannot be
/// opened (e.g. an unwritable state dir), composition degrades to an
/// empty `records` section with a warning rather than failing the whole
/// packet — the engineering stores stay authoritative.
pub fn context_record_excerpts(
    store: &ContextStore,
    workspace_root: &Path,
    task_id: Option<&str>,
    keywords: &[String],
) -> Vec<crate::engineering_context::ContextRecordExcerpt> {
    use crate::context_runtime::{fingerprint, ContextRetriever};
    let root = workspace_root.display().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Fetch generously: resolution reduces, never expands. Keyword-less
    // first (the always-available fingerprint), keyword matches merged
    // in when a task narrows relevance.
    let mut merged: std::collections::BTreeMap<String, crate::context_runtime::RankedRecord> =
        std::collections::BTreeMap::new();
    let mut fetch = |kw: Vec<String>| {
        let query = crate::context_runtime::RecordQuery {
            workspace_root: Some(root.as_str()),
            task_id,
            kind: None,
            status: None,
            keywords: kw,
            limit: 100,
        };
        if let Ok(ranked) = ContextRetriever::search(store, &query, now) {
            for r in ranked {
                merged.insert(r.record.id.clone(), r);
            }
        }
    };
    fetch(Vec::new());
    if !keywords.is_empty() {
        fetch(keywords.to_vec());
    }
    if merged.is_empty() {
        tracing::debug!("context records: no rows visible for this viewpoint");
    }
    let scope = fingerprint::ResolutionScope {
        workspace_key: Some(root.as_str()),
        task_id,
    };
    let resolved = fingerprint::resolve_context(merged.into_values().collect(), &scope);
    resolved
        .intents
        .iter()
        .chain(resolved.fingerprint.iter())
        .chain(resolved.other.iter())
        .take(crate::engineering_context::MAX_CONTEXT_RECORDS)
        .map(crate::engineering_context::excerpt_from)
        .collect()
}

/// Compose the bounded context packet and return the bounded JSON document
/// as a string. An empty (or whitespace-only) `task` with no keywords
/// produces the structural session-start digest.
pub fn build_context_packet(
    store: &ContextStore,
    workspace_root: &Path,
    task: &str,
    keywords: &[String],
    task_id: Option<&str>,
) -> Result<String, String> {
    let request = crate::engineering_context::EngineeringContextRequest {
        task: task.to_string(),
        task_keywords: keywords.to_vec(),
        active_file_tags: Vec::new(),
    };
    let has_task = !request.is_empty();
    let records = context_record_excerpts(store, workspace_root, task_id, &request.keywords());
    let packet = if has_task {
        crate::engineering_context::compose(workspace_root, &request, &records)?
    } else {
        crate::engineering_context::compose_structural(workspace_root, &records)?
    };
    let value = serde_json::to_value(&packet).map_err(|e| e.to_string())?;
    crate::mcp::response_bounds::bounded_response(value)
}

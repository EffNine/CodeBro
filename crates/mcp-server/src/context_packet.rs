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

/// Guarded excerpts plus the intent-guard report explaining the guard's
/// decisions for this viewpoint.
pub struct GuardedExcerpts {
    pub records: Vec<crate::engineering_context::ContextRecordExcerpt>,
    pub report: crate::intent_guard::IntentGuardReport,
}

/// Fetch bounded durable-context excerpts and run the WS4 intent guard.
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
///
/// The guard (read-only, deterministic) then excludes global records that
/// reference another known workspace and flags cross-project records;
/// project/task-scoped records are never content-scanned because their
/// scope already binds them to this viewpoint.
pub fn context_record_excerpts_guarded(
    store: &ContextStore,
    workspace_root: &Path,
    task_id: Option<&str>,
    keywords: &[String],
    task_text: &str,
    project_name: Option<&str>,
    repository_url: Option<&str>,
) -> GuardedExcerpts {
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

    // Best-effort known-workspace list: the guard degrades to
    // identity/basename-only evaluation if the query fails, never fails
    // the packet.
    let known = store
        .distinct_project_workspace_roots(crate::intent_guard::MAX_GUARD_KNOWN_WORKSPACES)
        .unwrap_or_default();
    let view = crate::intent_guard::GuardView {
        workspace_root: &root,
        project_name,
        repository_url,
        known_workspace_roots: &known,
        task_text,
    };
    let ordered: Vec<crate::context_runtime::RankedRecord> = resolved
        .intents
        .into_iter()
        .chain(resolved.fingerprint)
        .chain(resolved.other)
        .collect();
    let guarded = crate::intent_guard::apply_guard(&view, ordered);
    let records = guarded
        .records
        .iter()
        .take(crate::engineering_context::MAX_CONTEXT_RECORDS)
        .map(|r| {
            crate::engineering_context::excerpt_from_guarded(
                r,
                guarded.notes.get(&r.record.id).cloned(),
            )
        })
        .collect();
    GuardedExcerpts {
        records,
        report: guarded.report,
    }
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
    // Identity is loaded read-only for the guard: an absent identity
    // simply means the guard cannot check it (verdict `unverified`), never
    // an error. The default snapshot carries the sentinel name "unknown",
    // which must never be treated as a declared identity.
    let (project_name, repository_url) = {
        let mut identity_rt = crate::project_identity::ProjectIdentityRuntime::new(workspace_root);
        let loaded = identity_rt.load().is_ok();
        let snapshot = identity_rt.snapshot();
        let name = if loaded
            && !snapshot.name.trim().is_empty()
            && !snapshot.name.eq_ignore_ascii_case("unknown")
        {
            Some(snapshot.name.clone())
        } else {
            None
        };
        (
            name,
            if loaded {
                snapshot.repository_url.clone()
            } else {
                None
            },
        )
    };
    let guarded = context_record_excerpts_guarded(
        store,
        workspace_root,
        task_id,
        &request.keywords(),
        &request.task,
        project_name.as_deref(),
        repository_url.as_deref(),
    );
    let mut packet = if has_task {
        crate::engineering_context::compose(workspace_root, &request, &guarded.records)?
    } else {
        crate::engineering_context::compose_structural(workspace_root, &guarded.records)?
    };
    if guarded.report.verdict == crate::intent_guard::GuardVerdict::Review {
        packet.notes.push(format!(
            "intent guard: review — {} finding(s); see `guard` for deterministic signals (wrong-project context may be present)",
            guarded.report.signals.len()
        ));
    }
    packet.guard = Some(guarded.report);
    let value = serde_json::to_value(&packet).map_err(|e| e.to_string())?;
    crate::mcp::response_bounds::bounded_response(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use codebro_context_runtime::{Authority, ContextRecord, RecordKind, RecordScope};

    fn put(
        store: &ContextStore,
        id: &str,
        kind: RecordKind,
        scope: RecordScope,
        content: &str,
        workspace: Option<&str>,
    ) {
        let mut record = ContextRecord::new(
            id,
            kind,
            format!("ns.{id}"),
            content,
            Authority::UserConfirmed,
        );
        record.scope = scope;
        record.workspace_root = workspace.map(str::to_string);
        store.put_record(&record, 1000).unwrap();
    }

    fn records_of(value: &serde_json::Value) -> Vec<String> {
        value["records"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn packet_excludes_foreign_global_record_and_reports() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let foreign_base = foreign
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        put(
            &store,
            "ctx::foreign",
            RecordKind::Preference,
            RecordScope::Project,
            "foreign project knowledge",
            Some(foreign.path().to_str().unwrap()),
        );
        put(
            &store,
            "ctx::leak",
            RecordKind::Preference,
            RecordScope::Global,
            &format!("In {foreign_base} always run the soak suite before merging"),
            None,
        );
        put(
            &store,
            "ctx::keep",
            RecordKind::Preference,
            RecordScope::Global,
            "Prefer the simplest reasonable implementation",
            None,
        );

        let out = build_context_packet(&store, ws.path(), "improve the parser", &[], None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ids = records_of(&value);
        assert!(
            ids.contains(&"ctx::keep".to_string()),
            "neutral kept: {ids:?}"
        );
        assert!(
            !ids.contains(&"ctx::leak".to_string()),
            "foreign record excluded: {ids:?}"
        );
        assert_eq!(value["guard"]["verdict"], "review");
        assert_eq!(value["guard"]["excluded_records"][0], "ctx::leak");
        assert!(value["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("intent guard")));
    }

    #[test]
    fn guard_never_writes_and_never_promotes() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        put(
            &store,
            "ctx::p",
            RecordKind::Preference,
            RecordScope::Project,
            "Prefer small diffs",
            Some(ws.path().to_str().unwrap()),
        );
        let before = store.get_record("ctx::p").unwrap().expect("record exists");
        let _ = build_context_packet(&store, ws.path(), "do the work", &[], None).unwrap();
        let after = store.get_record("ctx::p").unwrap().expect("record exists");
        assert_eq!(before, after, "guard must not mutate records");
        assert_eq!(after.authority, Authority::UserConfirmed);
    }

    #[test]
    fn cross_project_record_is_flagged_in_packet() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let foreign_base = foreign
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let ws_base = ws.path().file_name().unwrap().to_string_lossy().to_string();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        put(
            &store,
            "ctx::foreign",
            RecordKind::Experience,
            RecordScope::Project,
            "foreign project knowledge",
            Some(foreign.path().to_str().unwrap()),
        );
        put(
            &store,
            "ctx::cross",
            RecordKind::Experience,
            RecordScope::Global,
            &format!("The {ws_base} parser pattern also applies to {foreign_base}"),
            None,
        );
        let out = build_context_packet(&store, ws.path(), "improve the parser", &[], None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        let ids = records_of(&value);
        assert!(ids.contains(&"ctx::cross".to_string()), "kept: {ids:?}");
        let cross = value["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["id"] == "ctx::cross")
            .unwrap();
        assert_eq!(cross["guard"]["status"], "ambiguous_project_reference");
        assert_eq!(value["guard"]["verdict"], "review");
    }

    #[test]
    fn confirmed_project_intent_yields_aligned_verdict() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        put(
            &store,
            "ctx::intent",
            RecordKind::Intent,
            RecordScope::Project,
            "Ship the intent guard",
            Some(ws.path().to_str().unwrap()),
        );
        let out = build_context_packet(&store, ws.path(), "finish the guard", &[], None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            value["guard"]["verdict"], "aligned",
            "guard: {}",
            value["guard"]
        );
        assert_eq!(value["guard"]["intent_coverage"], "project");
    }

    #[test]
    fn no_intent_no_identity_is_unverified_not_failed() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        let out = build_context_packet(&store, ws.path(), "any task", &[], None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            value["guard"]["verdict"], "unverified",
            "guard: {}",
            value["guard"]
        );
        assert_eq!(value["guard"]["intent_coverage"], "none");
        assert!(!value["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("intent guard")));
    }

    #[test]
    fn legacy_packet_json_without_guard_still_parses() {
        // Packets composed by library callers (and every packet written
        // before WS4) omit `guard`; the new struct must accept them.
        let dir = tempfile::tempdir().unwrap();
        let request = crate::engineering_context::EngineeringContextRequest {
            task: "legacy packet".to_string(),
            task_keywords: Vec::new(),
            active_file_tags: Vec::new(),
        };
        let packet = crate::engineering_context::compose(dir.path(), &request, &[]).unwrap();
        let value = serde_json::to_value(&packet).unwrap();
        assert!(
            value.get("guard").is_none(),
            "unguarded packets omit the key entirely"
        );
        let reparsed: crate::engineering_context::EngineeringContextPacket =
            serde_json::from_value(value).unwrap();
        assert!(reparsed.guard.is_none());
        assert_eq!(reparsed.records.len(), 0);
    }

    #[test]
    fn structural_digest_carries_guard_without_task() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        let out = build_context_packet(&store, ws.path(), "", &[], None).unwrap();
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(value.get("guard").is_some(), "guard present on digest");
        assert!(value["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("structural digest")));
    }

    #[test]
    fn guard_output_is_deterministic_across_calls() {
        let state = tempfile::tempdir().unwrap();
        let ws = tempfile::tempdir().unwrap();
        let foreign = tempfile::tempdir().unwrap();
        let foreign_base = foreign
            .path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let store = ContextStore::at_state_dir(state.path().to_path_buf());
        put(
            &store,
            "ctx::foreign",
            RecordKind::Preference,
            RecordScope::Project,
            "foreign project knowledge",
            Some(foreign.path().to_str().unwrap()),
        );
        put(
            &store,
            "ctx::leak",
            RecordKind::Preference,
            RecordScope::Global,
            &format!("{foreign_base} convention"),
            None,
        );
        let first = build_context_packet(&store, ws.path(), "task", &[], None).unwrap();
        let second = build_context_packet(&store, ws.path(), "task", &[], None).unwrap();
        assert_eq!(first, second, "guard output must be byte-stable");
    }
}

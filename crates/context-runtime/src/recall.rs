//! P2 recall: "have we tried this before?" as a capability, not a query.
//!
//! OpenCode thinks "I need relevant previous work" — never "I need to
//! query FTS5". This module turns a question plus a scope into compact,
//! provenance-preserving historical evidence:
//!
//! ```text
//! query + scope
//!   → FTS5 candidates (derived index over summaries/payloads/kinds)
//!   → canonical workspace/task filtering (isolation is correctness, not rank)
//!   → deterministic ranking (documented below, no ML, no embeddings)
//!   → session grouping (representative excerpts, not search-result spam)
//!   → bounded excerpts → OpenCode
//! ```
//!
//! # Ranking (actual behavior, stated plainly)
//!
//! 1. FTS5 BM25 relevance (lower is better).
//! 2. Task match: with a task viewpoint, that task's events first.
//! 3. Event importance ([`HistoryKind::importance`]): decisions,
//!    validations, and errors outrank chatter.
//! 4. Recency (`created_at` desc).
//! 5. Row id desc (total order; stable across runs).
//!
//! BM25 does not understand intent; the order above is lexical relevance
//! plus structural priors, nothing more.
//!
//! # Scope
//!
//! Filtering happens on the canonical tables *before* ranking, so FTS can
//! never leak another project's history through a good rank:
//!
//! - `Project` (default): exactly one workspace.
//! - `Task`: one workspace plus an exact task id (required).
//! - `Global`: every workspace, explicit opt-in only, every hit tagged
//!   with its workspace so cross-project evidence is always visible as such.

use rusqlite::{params, OptionalExtension};

use crate::history::{HistoryKind, SessionRecord, HISTORY_TRUNCATION_MARKER};
use crate::store::{ContextError, ContextStore};
use crate::types::EventRecord;
use crate::workspace::canonical_workspace_key;

/// Recall scope: which history is even eligible, before ranking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecallScope {
    /// Exactly one workspace (the default).
    #[default]
    Project,
    /// One workspace plus an exact task id (required).
    Task,
    /// Every workspace. Explicit opt-in; hits carry their workspace.
    Global,
}

impl std::fmt::Display for RecallScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecallScope::Project => write!(f, "project"),
            RecallScope::Task => write!(f, "task"),
            RecallScope::Global => write!(f, "global"),
        }
    }
}

impl std::str::FromStr for RecallScope {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "project" => Ok(RecallScope::Project),
            "task" => Ok(RecallScope::Task),
            "global" => Ok(RecallScope::Global),
            other => Err(format!("unknown recall scope: {other}")),
        }
    }
}

/// A recall request: a question plus the viewpoint asking it.
#[derive(Debug, Clone, Default)]
pub struct RecallQuery<'a> {
    /// The question in the caller's words ("why are we using SQLite?").
    /// Must tokenize to at least one FTS token (≥3 alphanumeric chars).
    pub query: &'a str,
    /// Workspace viewpoint. Required for project/task scope; ignored for
    /// global scope.
    pub workspace_root: Option<&'a str>,
    /// Task viewpoint. Required for task scope; boosts ranking otherwise.
    pub task_id: Option<&'a str>,
    pub scope: RecallScope,
    /// Narrow to these event kinds (e.g. only `decision` + `validation`).
    pub kinds: Vec<HistoryKind>,
    /// Narrow to one session.
    pub session_id: Option<&'a str>,
    /// Maximum excerpts returned (0 = default 10, otherwise clamped
    /// 1..=50).
    pub limit: usize,
}

/// One bounded historical excerpt with full provenance.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallHit {
    pub event: EventRecord,
    /// FTS5 BM25 rank (lower is better).
    pub bm25: f64,
    /// Whether the event's task matched the query viewpoint.
    pub task_match: bool,
    /// The session this event belongs to, if linked and known.
    pub session: Option<SessionRecord>,
    /// Whether that session looks interrupted (stale active).
    pub session_stale: bool,
    /// Bounded excerpt (summary preferred, then payload, then kind line).
    pub excerpt: String,
}

/// One session's representative excerpts.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallGroup {
    /// Session id, or `None` for unlinked events (grouped together).
    pub session_id: Option<String>,
    pub session_title: Option<String>,
    pub session_status: Option<String>,
    pub session_stale: bool,
    pub workspace_root: String,
    pub task_id: Option<String>,
    /// Total matching events in this session (hits may be capped).
    pub total_in_session: usize,
    pub hits: Vec<RecallHit>,
}

/// Recall outcome: grouped evidence, bounded and provenance-tagged.
#[derive(Debug, Clone, PartialEq)]
pub struct RecallOutcome {
    pub groups: Vec<RecallGroup>,
    /// Matching events considered (bounded fetch, at most 500 candidates;
    /// per-session caps apply after). A lower bound on history, never an
    /// exact table count — recall is evidence, not analytics.
    pub total_matches: usize,
    /// Whether output was bounded (groups or excerpts capped).
    pub truncated: bool,
}

/// Maximum excerpts per session group: one session with 30 matches must
/// not drown out every other session.
pub const MAX_HITS_PER_SESSION: usize = 3;
/// Maximum characters per excerpt before the truncation marker.
pub const MAX_RECALL_EXCERPT_CHARS: usize = 240;
/// Default and maximum excerpt budgets.
pub const DEFAULT_RECALL_LIMIT: usize = 10;
pub const MAX_RECALL_LIMIT: usize = 50;

/// Build a bounded excerpt for an event: summary first, then payload,
/// then a structural kind line. Never the full transcript.
pub fn excerpt_of(event: &EventRecord) -> String {
    let raw = event
        .summary
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_string)
        .or_else(|| {
            event
                .payload
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .map(|p| p.chars().take(MAX_RECALL_EXCERPT_CHARS + 1).collect())
        })
        .unwrap_or_else(|| {
            let mut line = event.kind.clone();
            if let Some(tool) = event.tool.as_deref() {
                line.push_str(&format!(" via {tool}"));
            }
            if let Some(outcome) = event.outcome.as_deref() {
                line.push_str(&format!(" → {outcome}"));
            }
            line
        });
    if raw.chars().count() <= MAX_RECALL_EXCERPT_CHARS {
        return raw;
    }
    format!(
        "{}{}",
        raw.chars()
            .take(MAX_RECALL_EXCERPT_CHARS)
            .collect::<String>(),
        HISTORY_TRUNCATION_MARKER
    )
}

fn kind_importance(kind: &str) -> u8 {
    kind.parse::<HistoryKind>()
        .map(|k| k.importance())
        .unwrap_or(10)
}

impl ContextStore {
    /// Recall relevant previous work for a question + viewpoint.
    ///
    /// FTS5 supplies candidates; canonical workspace/task predicates
    /// filter *before* ranking (re-checked in Rust as defense in depth);
    /// ranking is the documented deterministic order; session grouping
    /// caps per-session excerpts; output is bounded.
    pub fn recall(&self, query: &RecallQuery<'_>, now: u64) -> Result<RecallOutcome, ContextError> {
        let tokens = crate::retrieval::query_tokens(&[query.query.to_string()]);
        if tokens.is_empty() {
            return Err(ContextError::Validation(
                "recall query must contain at least one searchable token (3+ alphanumeric characters)".to_string(),
            ));
        }
        let ws = query.workspace_root.map(canonical_workspace_key);
        let task = query
            .task_id
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        match query.scope {
            RecallScope::Project => {
                if ws.as_deref().unwrap_or("").is_empty() {
                    return Err(ContextError::Validation(
                        "project recall requires a workspace_root".to_string(),
                    ));
                }
            }
            RecallScope::Task => {
                if ws.as_deref().unwrap_or("").is_empty() {
                    return Err(ContextError::Validation(
                        "task recall requires a workspace_root".to_string(),
                    ));
                }
                if task.as_deref().unwrap_or("").is_empty() {
                    return Err(ContextError::Validation(
                        "task recall requires a task_id".to_string(),
                    ));
                }
            }
            RecallScope::Global => {}
        }
        let limit = if query.limit == 0 {
            DEFAULT_RECALL_LIMIT
        } else {
            query.limit.clamp(1, MAX_RECALL_LIMIT)
        };
        // Over-fetch for grouping headroom (per-session caps consume hits).
        // Bounded at 500 candidates: recall cost stays flat while grouping
        // still sees past the first page. total_matches counts these
        // candidates, not the whole table (see RecallOutcome docs).
        let fetch = (limit * 10).clamp(100, 500) as i64;
        // OR semantics: a question ("why are we using SQLite?") carries
        // stopwords no single event contains. BM25 + the deterministic
        // rank below restore precision; AND would refuse every natural
        // question that is not already a keyword list.
        let match_expr = tokens
            .iter()
            .map(|t| format!("\"{}\"", t.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let kind_list: Vec<String> = query.kinds.iter().map(|k| k.as_str().to_string()).collect();

        self.with_conn(|conn| {
            // Scope predicates on the canonical table (never on FTS alone).
            // Placeholders are positional (?1..?5); unused ones stay bound
            // but unreferenced, which rusqlite permits.
            let scope_sql: &str = match query.scope {
                RecallScope::Project => "e.workspace_root = ?1",
                RecallScope::Task => "e.workspace_root = ?1 AND e.task_id = ?2",
                RecallScope::Global => "1 = 1",
            };
            // Column order matches EVENT_COLUMNS so row_to_event applies;
            // bm25 rides along as the trailing column.
            let sql = format!(
                "SELECT e.id, e.session_id, e.workspace_root, e.task_id, e.kind, e.tool,
                        e.path, e.outcome, e.summary, e.payload_json, e.dedup_key,
                        e.source, e.digest, e.created_at, bm25(events_fts) AS rank
                 FROM events e
                 JOIN events_fts ON events_fts.event_id = e.id
                 WHERE {scope_sql}
                   AND (?3 IS NULL OR e.session_id = ?3)
                   AND events_fts MATCH ?4
                 ORDER BY rank, e.id DESC
                 LIMIT ?5"
            );
            let mut stmt = conn.prepare(&sql)?;
            let collected = stmt
                .query_map(
                    params![
                        ws.as_deref(),
                        task.as_deref(),
                        query.session_id,
                        match_expr,
                        fetch
                    ],
                    |row| {
                        let record = crate::store::row_to_event_pub(row)?;
                        let rank: f64 = row.get(14)?;
                        Ok((record, rank))
                    },
                )?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| ContextError::Decode(e.to_string()))?;

            // Defense in depth: re-apply scope predicates in Rust so an
            // FTS quirk can never bypass workspace/task isolation, then
            // apply the kind filter.
            let mut hits: Vec<(EventRecord, f64)> = Vec::new();
            for (event, rank) in collected {
                match query.scope {
                    RecallScope::Project => {
                        if Some(event.workspace_root.as_str()) != ws.as_deref() {
                            continue;
                        }
                    }
                    RecallScope::Task => {
                        if Some(event.workspace_root.as_str()) != ws.as_deref() {
                            continue;
                        }
                        if event.task_id.as_deref() != task.as_deref() {
                            continue;
                        }
                    }
                    RecallScope::Global => {}
                }
                if let Some(sid) = query.session_id {
                    if event.session_id.as_deref() != Some(sid) {
                        continue;
                    }
                }
                if !kind_list.is_empty() && !kind_list.iter().any(|k| k == &event.kind) {
                    continue;
                }
                hits.push((event, rank));
            }

            // Deterministic ranking: BM25 → task match → importance →
            // recency → id. (See module docs: BM25 is lexical, not intent.)
            hits.sort_by(|(a, ra), (b, rb)| {
                ra.partial_cmp(rb)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| {
                        let ta = a.task_id.as_deref() == task.as_deref() && task.is_some();
                        let tb = b.task_id.as_deref() == task.as_deref() && task.is_some();
                        tb.cmp(&ta)
                    })
                    .then_with(|| kind_importance(&b.kind).cmp(&kind_importance(&a.kind)))
                    .then_with(|| b.created_at.cmp(&a.created_at))
                    .then_with(|| b.id.cmp(&a.id))
            });

            let total_matches = hits.len();
            // Session grouping with per-session caps; overall excerpt cap.
            let mut groups: Vec<RecallGroup> = Vec::new();
            let mut index_by_session: std::collections::HashMap<Option<String>, usize> =
                std::collections::HashMap::new();
            let mut emitted = 0usize;
            let mut truncated = total_matches > limit;
            for (event, rank) in hits {
                if emitted >= limit {
                    truncated = true;
                    break;
                }
                let key = event.session_id.clone();
                let gi = match index_by_session.get(&key) {
                    Some(&i) => i,
                    None => {
                        let session = key.as_deref().and_then(|sid| {
                            conn.query_row(
                                "SELECT id, workspace_root, task_id, title, status, source,
                                        parent_session_id, opened_at, updated_at, closed_at,
                                        end_reason, event_count
                                 FROM sessions WHERE id = ?1",
                                [sid],
                                crate::history::row_to_session,
                            )
                            .optional()
                            .ok()
                            .flatten()
                        });
                        let stale = session.as_ref().is_some_and(|s| {
                            s.status == crate::history::SessionStatus::Active
                                && now.saturating_sub(s.updated_at)
                                    > crate::history::STALE_AFTER_SECS
                        });
                        groups.push(RecallGroup {
                            session_id: key.clone(),
                            session_title: session.as_ref().and_then(|s| s.title.clone()),
                            session_status: session.as_ref().map(|s| s.status.as_str().to_string()),
                            session_stale: stale,
                            workspace_root: event.workspace_root.clone(),
                            task_id: event.task_id.clone(),
                            total_in_session: 0,
                            hits: Vec::new(),
                        });
                        let i = groups.len() - 1;
                        index_by_session.insert(key.clone(), i);
                        i
                    }
                };
                groups[gi].total_in_session += 1;
                if groups[gi].hits.len() >= MAX_HITS_PER_SESSION {
                    truncated = true;
                    continue;
                }
                let task_match = task.is_some() && event.task_id.as_deref() == task.as_deref();
                let session = key.as_deref().and_then(|sid| {
                    conn.query_row(
                        "SELECT id, workspace_root, task_id, title, status, source,
                                parent_session_id, opened_at, updated_at, closed_at,
                                end_reason, event_count
                         FROM sessions WHERE id = ?1",
                        [sid],
                        crate::history::row_to_session,
                    )
                    .optional()
                    .ok()
                    .flatten()
                });
                let session_stale = session.as_ref().is_some_and(|s| {
                    s.status == crate::history::SessionStatus::Active
                        && now.saturating_sub(s.updated_at) > crate::history::STALE_AFTER_SECS
                });
                groups[gi].hits.push(RecallHit {
                    excerpt: excerpt_of(&event),
                    event,
                    bm25: rank,
                    task_match,
                    session,
                    session_stale,
                });
                emitted += 1;
            }
            // Drop groups left empty by per-session caps (their totals fed
            // total_matches already) and keep group order deterministic:
            // first-hit order.
            groups.retain(|g| !g.hits.is_empty());
            Ok(RecallOutcome {
                groups,
                total_matches,
                truncated,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::history::{HistoryInput, HistoryKind, OpenSession};

    fn store() -> (tempfile::TempDir, ContextStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        (dir, store)
    }

    fn input(ws: &str, kind: HistoryKind, summary: &str) -> HistoryInput {
        HistoryInput::new(ws, kind, summary)
    }

    fn seed_basic(store: &ContextStore) -> String {
        let s = store
            .open_session("/proj", &OpenSession::default(), 100)
            .unwrap();
        let mut d = input(
            "/proj",
            HistoryKind::Decision,
            "chose SQLite because this is a local persistent runtime",
        );
        d.session_id = Some(s.id.clone());
        d.outcome = Some("decided".to_string());
        store.record_history(&d, 110).unwrap();
        let mut v = input("/proj", HistoryKind::Validation, "cargo test passed clean");
        v.session_id = Some(s.id.clone());
        v.outcome = Some("passed".to_string());
        store.record_history(&v, 120).unwrap();
        s.id
    }

    #[test]
    fn recall_finds_relevant_decision() {
        let (_dir, store) = store();
        seed_basic(&store);
        let out = store
            .recall(
                &RecallQuery {
                    query: "why are we using SQLite",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                200,
            )
            .unwrap();
        assert!(out.total_matches >= 1);
        let kinds: Vec<_> = out
            .groups
            .iter()
            .flat_map(|g| g.hits.iter().map(|h| h.event.kind.as_str()))
            .collect();
        assert!(kinds.contains(&"decision"), "kinds: {kinds:?}");
        let hit = out
            .groups
            .iter()
            .flat_map(|g| &g.hits)
            .find(|h| h.event.kind == "decision")
            .unwrap();
        assert!(hit.excerpt.contains("SQLite"));
        assert_eq!(hit.event.workspace_root, "/proj");
        assert!(hit.session.is_some());
        assert!(!hit.session_stale);
    }

    #[test]
    fn recall_empty_query_is_refused() {
        let (_dir, store) = store();
        let err = store
            .recall(
                &RecallQuery {
                    query: "a?",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                1,
            )
            .unwrap_err();
        assert!(err.to_string().contains("searchable token"));
    }

    #[test]
    fn recall_no_result_is_empty_not_error() {
        let (_dir, store) = store();
        seed_basic(&store);
        let out = store
            .recall(
                &RecallQuery {
                    query: "quetzalcoatlus kubernetes operator",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                200,
            )
            .unwrap();
        assert_eq!(out.total_matches, 0);
        assert!(out.groups.is_empty());
    }

    #[test]
    fn recall_groups_cap_per_session_hits() {
        let (_dir, store) = store();
        let s = store
            .open_session("/proj", &OpenSession::default(), 1)
            .unwrap();
        for i in 0..30 {
            let mut e = input(
                "/proj",
                HistoryKind::Observation,
                &format!("repeated sqlite observation number {i}"),
            );
            e.session_id = Some(s.id.clone());
            store.record_history(&e, 10 + i as u64).unwrap();
        }
        let out = store
            .recall(
                &RecallQuery {
                    query: "sqlite observation",
                    workspace_root: Some("/proj"),
                    limit: 10,
                    ..RecallQuery::default()
                },
                1000,
            )
            .unwrap();
        assert_eq!(out.groups.len(), 1);
        assert_eq!(out.groups[0].hits.len(), MAX_HITS_PER_SESSION);
        assert_eq!(out.groups[0].total_in_session, 30);
        assert!(out.truncated);
    }

    #[test]
    fn recall_task_viewpoint_boosts_task_events() {
        let (_dir, store) = store();
        let mut a = input("/proj", HistoryKind::Decision, "task alpha chose sqlite");
        a.task_id = Some("alpha".to_string());
        store.record_history(&a, 10).unwrap();
        let b = input("/proj", HistoryKind::Decision, "project chose sqlite");
        store.record_history(&b, 20).unwrap();
        // Task scope sees only its own history.
        let out = store
            .recall(
                &RecallQuery {
                    query: "chose sqlite",
                    workspace_root: Some("/proj"),
                    task_id: Some("alpha"),
                    scope: RecallScope::Task,
                    ..RecallQuery::default()
                },
                30,
            )
            .unwrap();
        assert_eq!(out.total_matches, 1);
        assert_eq!(
            out.groups[0].hits[0].event.task_id.as_deref(),
            Some("alpha")
        );
        assert!(out.groups[0].hits[0].task_match);
        // Project scope sees both; the task hit is flagged task_match
        // (BM25 ranks first by design — lexical relevance before priors).
        let out = store
            .recall(
                &RecallQuery {
                    query: "chose sqlite",
                    workspace_root: Some("/proj"),
                    task_id: Some("alpha"),
                    ..RecallQuery::default()
                },
                30,
            )
            .unwrap();
        assert_eq!(out.total_matches, 2);
        let flagged: Vec<_> = out
            .groups
            .iter()
            .flat_map(|g| &g.hits)
            .filter(|h| h.task_match)
            .collect();
        assert_eq!(flagged.len(), 1);
        assert_eq!(flagged[0].event.task_id.as_deref(), Some("alpha"));
    }

    #[test]
    fn recall_cross_project_isolation_holds() {
        let (_dir, store) = store();
        store
            .record_history(
                &input(
                    "/proj-a",
                    HistoryKind::Decision,
                    "rejected architecture xylophone",
                ),
                10,
            )
            .unwrap();
        let out = store
            .recall(
                &RecallQuery {
                    query: "architecture xylophone",
                    workspace_root: Some("/proj-b"),
                    ..RecallQuery::default()
                },
                20,
            )
            .unwrap();
        assert_eq!(out.total_matches, 0, "project B must see nothing of A");
        // Explicit global opt-in surfaces it, tagged with its workspace.
        let out = store
            .recall(
                &RecallQuery {
                    query: "architecture xylophone",
                    scope: RecallScope::Global,
                    ..RecallQuery::default()
                },
                20,
            )
            .unwrap();
        assert_eq!(out.total_matches, 1);
        assert_eq!(out.groups[0].hits[0].event.workspace_root, "/proj-a");
    }

    #[test]
    fn recall_kind_filter_narrows_to_decisions() {
        let (_dir, store) = store();
        seed_basic(&store);
        // Both seeded events match ("sqlite" in the decision, "test" in
        // the validation); the kind filter keeps only validations.
        let out = store
            .recall(
                &RecallQuery {
                    query: "sqlite test",
                    workspace_root: Some("/proj"),
                    kinds: vec![HistoryKind::Validation],
                    ..RecallQuery::default()
                },
                200,
            )
            .unwrap();
        assert_eq!(out.total_matches, 1);
        assert_eq!(out.groups[0].hits[0].event.kind, "validation");
    }

    #[test]
    fn recall_excerpts_are_bounded_with_provenance() {
        let (_dir, store) = store();
        store
            .record_history(
                &input("/proj", HistoryKind::UserMessage, &"word ".repeat(500)),
                10,
            )
            .unwrap();
        let out = store
            .recall(
                &RecallQuery {
                    query: "word",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                20,
            )
            .unwrap();
        let hit = &out.groups[0].hits[0];
        assert!(hit.excerpt.ends_with(HISTORY_TRUNCATION_MARKER));
        assert!(hit.excerpt.chars().count() <= MAX_RECALL_EXCERPT_CHARS + 64);
        assert!(hit.event.id.is_some());
        assert!(!hit.event.workspace_root.is_empty());
    }

    #[test]
    fn recall_requires_scope_identities() {
        let (_dir, store) = store();
        assert!(store
            .recall(
                &RecallQuery {
                    query: "sqlite",
                    ..RecallQuery::default()
                },
                1
            )
            .is_err());
        assert!(store
            .recall(
                &RecallQuery {
                    query: "sqlite",
                    workspace_root: Some("/proj"),
                    scope: RecallScope::Task,
                    ..RecallQuery::default()
                },
                1,
            )
            .is_err());
    }

    #[test]
    fn recall_rebuild_keeps_searchability() {
        let (_dir, store) = store();
        seed_basic(&store);
        store.rebuild_history_fts().unwrap();
        let out = store
            .recall(
                &RecallQuery {
                    query: "SQLite runtime",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                200,
            )
            .unwrap();
        assert!(out.total_matches >= 1);
    }

    #[test]
    fn fts_corruption_is_recoverable_without_losing_history() {
        // The derived index is disposable: wipe it and recall goes blind
        // while canonical history stays intact; rebuild restores search.
        let (_dir, store) = store();
        seed_basic(&store);
        store
            .with_conn(|conn| {
                conn.execute("DELETE FROM events_fts", []).unwrap();
                Ok(())
            })
            .unwrap();
        let blind = store
            .recall(
                &RecallQuery {
                    query: "SQLite runtime",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                200,
            )
            .unwrap();
        assert_eq!(blind.total_matches, 0);
        // Canonical rows untouched by the index wipe.
        assert_eq!(store.list_events("/proj", 10).unwrap().len(), 2);
        let rebuilt = store.rebuild_history_fts().unwrap();
        assert_eq!(rebuilt, 2);
        let out = store
            .recall(
                &RecallQuery {
                    query: "SQLite runtime",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                200,
            )
            .unwrap();
        assert!(out.total_matches >= 1);
    }

    #[test]
    fn fts_cannot_bypass_canonical_isolation() {
        // The FTS table physically contains project A's text; recall from
        // B must still return zero A rows (filtering precedes ranking).
        let (_dir, store) = store();
        store
            .record_history(
                &input(
                    "/proj-a",
                    HistoryKind::Decision,
                    "xylophone architecture rejected",
                ),
                10,
            )
            .unwrap();
        let raw: i64 = store
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT count(*) FROM events_fts WHERE events_fts MATCH '\"xylophone\"'",
                    [],
                    |r| r.get(0),
                )
                .map_err(|e| crate::store::ContextError::Decode(e.to_string()))
            })
            .unwrap();
        assert_eq!(raw, 1, "precondition: FTS really does contain A's row");
        let out = store
            .recall(
                &RecallQuery {
                    query: "xylophone",
                    workspace_root: Some("/proj-b"),
                    ..RecallQuery::default()
                },
                20,
            )
            .unwrap();
        assert_eq!(out.total_matches, 0);
        assert!(out.groups.is_empty());
    }

    #[test]
    fn recall_stale_sessions_are_flagged_not_hidden() {
        let (_dir, store) = store();
        let s = store
            .open_session("/proj", &OpenSession::default(), 100)
            .unwrap();
        let mut e = input("/proj", HistoryKind::Error, "build failed on linker sqlite");
        e.session_id = Some(s.id.clone());
        store.record_history(&e, 110).unwrap();
        // updated_at is 110 (the event touch); go strictly past the horizon.
        let far = 110 + crate::history::STALE_AFTER_SECS + 10;
        let out = store
            .recall(
                &RecallQuery {
                    query: "linker sqlite",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                far,
            )
            .unwrap();
        assert_eq!(out.total_matches, 1);
        assert!(
            out.groups[0].session_stale,
            "interrupted session must say so"
        );
        assert_eq!(
            out.groups[0].session_status.as_deref(),
            Some("active"),
            "stale is derived, never auto-completed"
        );
    }

    #[test]
    fn recall_session_filter_narrows_to_one_session() {
        let (_dir, store) = store();
        let a = store
            .open_session("/proj", &OpenSession::default(), 1)
            .unwrap();
        let b = store
            .open_session("/proj", &OpenSession::default(), 2)
            .unwrap();
        for sid in [&a.id, &b.id] {
            let mut e = input("/proj", HistoryKind::Observation, "shared sqlite note");
            e.session_id = Some(sid.clone());
            store.record_history(&e, 10).unwrap();
        }
        let out = store
            .recall(
                &RecallQuery {
                    query: "shared sqlite",
                    workspace_root: Some("/proj"),
                    session_id: Some(a.id.as_str()),
                    ..RecallQuery::default()
                },
                20,
            )
            .unwrap();
        assert_eq!(out.total_matches, 1);
        assert_eq!(out.groups[0].session_id.as_deref(), Some(a.id.as_str()));
    }

    #[test]
    fn recall_over_synthetic_history_stays_fast() {
        let (_dir, store) = store();
        // Spread across sessions so grouping caps don't collapse the set.
        let sessions: Vec<String> = (0..20)
            .map(|_| {
                store
                    .open_session("/proj", &OpenSession::default(), 999)
                    .unwrap()
                    .id
            })
            .collect();
        for i in 0..1000 {
            let kind = match i % 5 {
                0 => HistoryKind::Decision,
                1 => HistoryKind::Validation,
                2 => HistoryKind::Error,
                3 => HistoryKind::ChangeApplied,
                _ => HistoryKind::Observation,
            };
            let summary = format!("synthetic event {i} about sqlite migrations and tooling");
            let mut e = input("/proj", kind, &summary);
            e.session_id = Some(sessions[i % sessions.len()].clone());
            store.record_history(&e, 1000 + i as u64).unwrap();
        }
        let start = std::time::Instant::now();
        let out = store
            .recall(
                &RecallQuery {
                    query: "sqlite migrations",
                    workspace_root: Some("/proj"),
                    limit: 10,
                    ..RecallQuery::default()
                },
                100_000,
            )
            .unwrap();
        let elapsed = start.elapsed();
        assert_eq!(out.groups.iter().map(|g| g.hits.len()).sum::<usize>(), 10);
        assert!(
            elapsed.as_secs() < 5,
            "recall over 1000 events took {elapsed:?}"
        );
    }
}

#[cfg(test)]
mod p9_outcome_tests {
    use super::*;
    use crate::db;
    use crate::tasks::{NewTask, OutcomeClassification, TaskOutcomeInput};

    #[test]
    fn recall_surfaces_task_outcome_evidence_by_keyword() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        let task = store
            .create_task(
                "/proj",
                &NewTask {
                    title: "migrate billing integration".to_string(),
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        store
            .record_task_outcome(
                "/proj",
                &task.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: "billing integration failed because API contract differs",
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        // A later related question finds the failed attempt: the feedback
        // loop closes through the existing recall ranking (no P9 ranker).
        let out = store
            .recall(
                &RecallQuery {
                    query: "billing integration API contract",
                    workspace_root: Some("/proj"),
                    ..RecallQuery::default()
                },
                300,
            )
            .unwrap();
        assert!(out.total_matches >= 1, "outcome must be recallable");
        let kinds: Vec<_> = out
            .groups
            .iter()
            .flat_map(|g| g.hits.iter().map(|h| h.event.kind.as_str()))
            .collect();
        assert!(kinds.contains(&"task_outcome"), "kinds: {kinds:?}");
    }

    #[test]
    fn outcome_evidence_stays_invisible_across_workspaces() {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
        let task = store
            .create_task(
                "/proj-a",
                &NewTask {
                    title: "secret billing migration".to_string(),
                    ..Default::default()
                },
                100,
            )
            .unwrap();
        store
            .record_task_outcome(
                "/proj-a",
                &task.task_id,
                &TaskOutcomeInput {
                    classification: OutcomeClassification::Failure,
                    summary: "billing migration failed on ledger schema",
                    ..Default::default()
                },
                200,
            )
            .unwrap();
        let out = store
            .recall(
                &RecallQuery {
                    query: "billing migration ledger schema",
                    workspace_root: Some("/proj-b"),
                    ..RecallQuery::default()
                },
                300,
            )
            .unwrap();
        assert_eq!(out.total_matches, 0, "no cross-workspace leakage");
    }
}

//! User fingerprint: durable collaboration preferences + deterministic
//! namespace resolution.
//!
//! A fingerprint is *not* a stereotype — it is the set of known patterns
//! about how the user prefers to collaborate ("prefer the simplest
//! reasonable implementation"), stored as semantic [`ContextRecord`]s, one
//! row per (kind, namespace, scope) with history preserved via supersede
//! chains.
//!
//! Fingerprint attribute kinds: `Preference`, `Style`, `Taste`,
//! `Constraint`, `Principle`, `Pattern`. Intents live in the same table
//! under `RecordKind::Intent` but resolve in their own lane (see
//! [`crate::intent`]); `Experience`/`Other` pass through untouched for the
//! later learning phase.
//!
//! # Resolution
//!
//! Retrieval for a (workspace, task) pair considers global rows, the
//! workspace's project rows, and — only with a matching `task_id` — that
//! task's rows. Within one (kind, namespace), contradictory rows are
//! reduced to a single winner by a deterministic precedence:
//!
//! ```text
//! authority rank  (user_confirmed > project_derived > system_derived
//!                  > imported > observed > ai_inferred)
//!   → scope specificity (task > project > global)
//!     → effective (decayed) confidence
//!       → recency (updated_at)
//!         → id (final tiebreak; total order, stable across runs)
//! ```
//!
//! Authority outranks specificity on purpose: a confirmed global
//! preference ("prefer concise responses") must not be silently overridden
//! by an inferred project guess. When authority ties — the common case,
//! e.g. a confirmed project override of a confirmed global — the more
//! specific scope wins. Nothing is concatenated blindly and nothing is
//! deleted: losers are reported as `suppressed` for auditability.

use crate::retrieval::RankedRecord;
use crate::types::{authority_rank, Authority, ContextRecord, RecordKind, RecordScope};

/// Record kinds that form the user fingerprint (durable collaboration
/// preferences across communication, engineering, product/design, and
/// working-style dimensions — all semantic records, no rigid columns).
pub const FINGERPRINT_KINDS: &[RecordKind] = &[
    RecordKind::Preference,
    RecordKind::Style,
    RecordKind::Taste,
    RecordKind::Constraint,
    RecordKind::Principle,
    RecordKind::Pattern,
];

/// Resolution lane of a record kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Durable collaboration preference (per-namespace winner).
    Fingerprint,
    /// Goal with rationale (actionable set, see `intent` module).
    Intent,
    /// Anything else (experiences, unclassified): passed through untouched.
    Other,
}

/// Which resolution lane a record kind belongs to.
pub fn lane_of(kind: RecordKind) -> Lane {
    if FINGERPRINT_KINDS.contains(&kind) {
        Lane::Fingerprint
    } else if kind == RecordKind::Intent {
        Lane::Intent
    } else {
        Lane::Other
    }
}

/// Scope specificity rank (higher wins within equal authority).
pub fn scope_rank(scope: RecordScope) -> u8 {
    match scope {
        RecordScope::Task => 3,
        RecordScope::Project => 2,
        RecordScope::Global => 1,
    }
}

/// The retrieval viewpoint: which workspace (canonical key) and task the
/// caller is working in. `None` workspace = global records only;
/// `None` task = task-scoped records are invisible.
#[derive(Debug, Clone, Default)]
pub struct ResolutionScope<'a> {
    pub workspace_key: Option<&'a str>,
    pub task_id: Option<&'a str>,
}

/// Whether a record applies to the viewpoint. Global rows apply
/// everywhere; project rows need a workspace match; task rows need a
/// workspace *and* task match. Records are stored with canonical workspace
/// keys, so comparison is exact string equality.
pub fn applies(record: &ContextRecord, scope: &ResolutionScope<'_>) -> bool {
    match record.scope {
        RecordScope::Global => true,
        RecordScope::Project => match (scope.workspace_key, record.workspace_root.as_deref()) {
            (Some(want), Some(have)) => want == have,
            _ => false,
        },
        RecordScope::Task => match (
            scope.workspace_key,
            scope.task_id,
            record.workspace_root.as_deref(),
            record.task_id.as_deref(),
        ) {
            (Some(want_ws), Some(want_task), Some(have_ws), Some(have_task)) => {
                want_ws == have_ws && want_task == have_task
            }
            _ => false,
        },
    }
}

/// Deterministic winner comparator for one (kind, namespace) group.
/// Authority first (confirmation beats inference), then specificity, then
/// decayed confidence, recency, and finally id for a total stable order.
pub fn winner_order(a: &RankedRecord, b: &RankedRecord) -> std::cmp::Ordering {
    authority_rank(a.record.authority)
        .cmp(&authority_rank(b.record.authority))
        .then_with(|| scope_rank(a.record.scope).cmp(&scope_rank(b.record.scope)))
        .then_with(|| {
            a.effective_confidence
                .partial_cmp(&b.effective_confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| a.record.updated_at.cmp(&b.record.updated_at))
        .then_with(|| b.record.id.cmp(&a.record.id))
}

/// Reverse of [`winner_order`] — strongest first.
pub fn strongest_first(a: &RankedRecord, b: &RankedRecord) -> std::cmp::Ordering {
    winner_order(a, b).reverse()
}

/// Resolved context for one viewpoint: per-namespace fingerprint winners,
/// actionable intents, and everything else passed through — plus the ids
/// that lost a namespace contest (audit trail, never silently dropped).
#[derive(Debug, Default)]
pub struct ResolvedContext {
    /// One winner per (kind, namespace), strongest first.
    pub fingerprint: Vec<RankedRecord>,
    /// Actionable intents (active rows with actionable intent status),
    /// most recently updated first.
    pub intents: Vec<RankedRecord>,
    /// Passthrough lane (experience / other kinds), strongest first.
    pub other: Vec<RankedRecord>,
    /// Ids that lost a namespace contest (still in the store, queryable by
    /// explicit status/id — just not winners for this viewpoint).
    pub suppressed: Vec<String>,
    /// Intent rows that could not be decoded (corrupt `extra_json`):
    /// excluded from the packet and counted so corruption stays visible.
    pub malformed_intents: usize,
}

/// Resolve ranked records into the bounded viewpoint described above.
///
/// Callers should search with a generous limit (resolution reduces, never
/// expands) and let the composer cap the packet afterwards.
pub fn resolve_context(records: Vec<RankedRecord>, scope: &ResolutionScope<'_>) -> ResolvedContext {
    use std::collections::BTreeMap;

    let mut out = ResolvedContext::default();
    let mut groups: BTreeMap<(String, String), Vec<RankedRecord>> = BTreeMap::new();

    for ranked in records {
        if !applies(&ranked.record, scope) {
            continue;
        }
        match lane_of(ranked.record.kind) {
            Lane::Fingerprint => {
                groups
                    .entry((
                        ranked.record.kind.to_string(),
                        ranked.record.namespace.clone(),
                    ))
                    .or_default()
                    .push(ranked);
            }
            Lane::Intent => {
                // Intent lane: only active rows with an actionable decoded
                // status surface; terminal rows are history, malformed rows
                // are counted, neither is silently promoted.
                if ranked.record.status != crate::types::RecordStatus::Active {
                    continue;
                }
                match crate::intent::IntentMetadata::read_from(&ranked.record) {
                    Ok(meta) if meta.intent_status.is_actionable() => out.intents.push(ranked),
                    Ok(_) => {}
                    Err(_) => out.malformed_intents += 1,
                }
            }
            Lane::Other => out.other.push(ranked),
        }
    }

    for (_, mut group) in groups {
        if group.len() == 1 {
            out.fingerprint.push(group.pop().unwrap());
            continue;
        }
        group.sort_by(strongest_first);
        let mut iter = group.into_iter();
        out.fingerprint.push(iter.next().expect("non-empty group"));
        out.suppressed.extend(iter.map(|r| r.record.id.clone()));
    }

    out.fingerprint.sort_by(strongest_first);
    out.intents.sort_by(|a, b| {
        b.record
            .updated_at
            .cmp(&a.record.updated_at)
            .then_with(|| a.record.id.cmp(&b.record.id))
    });
    out.other.sort_by(strongest_first);
    out.suppressed.sort();
    out
}

/// Convenience: the authorities involved, for tests and debugging.
#[allow(dead_code)]
pub fn authorities_present(records: &[RankedRecord]) -> Vec<Authority> {
    let mut out: Vec<Authority> = records.iter().map(|r| r.record.authority).collect();
    out.sort_by_key(|a| authority_rank(*a));
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Authority, ContextRecord, RecordKind, RecordScope};

    fn ranked(
        id: &str,
        kind: RecordKind,
        ns: &str,
        authority: Authority,
        scope: RecordScope,
        ws: Option<&str>,
        task: Option<&str>,
    ) -> RankedRecord {
        let mut r = ContextRecord::new(id, kind, ns, format!("content of {id}"), authority);
        r.scope = scope;
        r.workspace_root = ws.map(|s| s.to_string());
        r.task_id = task.map(|s| s.to_string());
        r.updated_at = 1000;
        RankedRecord {
            record: r,
            bm25: None,
            effective_confidence: 0.8,
        }
    }

    fn pref(id: &str, authority: Authority, scope: RecordScope) -> RankedRecord {
        let (ws, task) = match scope {
            RecordScope::Global => (None, None),
            RecordScope::Project => (Some("/work"), None),
            RecordScope::Task => (Some("/work"), Some("task-1")),
        };
        ranked(
            id,
            RecordKind::Preference,
            "fp.engineering.simplicity",
            authority,
            scope,
            ws,
            task,
        )
    }

    fn view<'a>(ws: Option<&'a str>, task: Option<&'a str>) -> ResolutionScope<'a> {
        ResolutionScope {
            workspace_key: ws,
            task_id: task,
        }
    }

    #[test]
    fn specificity_wins_within_equal_authority() {
        // Global "concise" vs project "detailed reasoning" vs task
        // "final plan only" — all confirmed: most specific wins.
        let records = vec![
            pref("ctx::g", Authority::UserConfirmed, RecordScope::Global),
            pref("ctx::p", Authority::UserConfirmed, RecordScope::Project),
            pref("ctx::t", Authority::UserConfirmed, RecordScope::Task),
        ];
        let resolved = resolve_context(records, &view(Some("/work"), Some("task-1")));
        assert_eq!(resolved.fingerprint.len(), 1);
        assert_eq!(resolved.fingerprint[0].record.id, "ctx::t");
        assert_eq!(
            resolved.suppressed,
            vec!["ctx::g".to_string(), "ctx::p".to_string()]
        );

        // Without a task viewpoint, the project record wins and the task
        // record is invisible (not merely suppressed).
        let records = vec![
            pref("ctx::g", Authority::UserConfirmed, RecordScope::Global),
            pref("ctx::p", Authority::UserConfirmed, RecordScope::Project),
            pref("ctx::t", Authority::UserConfirmed, RecordScope::Task),
        ];
        let resolved = resolve_context(records, &view(Some("/work"), None));
        assert_eq!(resolved.fingerprint.len(), 1);
        assert_eq!(resolved.fingerprint[0].record.id, "ctx::p");
        assert_eq!(resolved.suppressed, vec!["ctx::g".to_string()]);
    }

    #[test]
    fn confirmation_beats_inference_across_scopes() {
        // A confirmed global must not be silently overridden by an
        // inferred project guess.
        let records = vec![
            pref("ctx::g", Authority::UserConfirmed, RecordScope::Global),
            pref("ctx::p", Authority::AiInferred, RecordScope::Project),
        ];
        let resolved = resolve_context(records, &view(Some("/work"), None));
        assert_eq!(resolved.fingerprint.len(), 1);
        assert_eq!(resolved.fingerprint[0].record.id, "ctx::g");
        assert_eq!(resolved.suppressed, vec!["ctx::p".to_string()]);
    }

    #[test]
    fn different_namespaces_coexist() {
        let mut a = pref("ctx::a", Authority::UserConfirmed, RecordScope::Global);
        a.record.namespace = "fp.communication.verbosity".to_string();
        let mut b = pref("ctx::b", Authority::UserConfirmed, RecordScope::Global);
        b.record.namespace = "fp.engineering.simplicity".to_string();
        let resolved = resolve_context(vec![a, b], &view(None, None));
        assert_eq!(resolved.fingerprint.len(), 2);
        assert!(resolved.suppressed.is_empty());
    }

    #[test]
    fn task_isolation_between_tasks() {
        let mut t1 = pref("ctx::t1", Authority::UserConfirmed, RecordScope::Task);
        t1.record.task_id = Some("task-1".to_string());
        let mut t2 = pref("ctx::t2", Authority::UserConfirmed, RecordScope::Task);
        t2.record.task_id = Some("task-2".to_string());
        t2.record.workspace_root = Some("/work".to_string());
        let g = pref("ctx::g", Authority::UserConfirmed, RecordScope::Global);

        let resolved = resolve_context(vec![t1, t2, g], &view(Some("/work"), Some("task-1")));
        let ids: Vec<&str> = resolved
            .fingerprint
            .iter()
            .map(|r| r.record.id.as_str())
            .collect();
        assert!(ids.contains(&"ctx::t1"), "own task record wins: {ids:?}");
        assert!(
            !ids.contains(&"ctx::t2"),
            "other task's record excluded: {ids:?}"
        );
    }

    #[test]
    fn workspace_isolation_for_project_records() {
        let mut p = pref("ctx::p", Authority::UserConfirmed, RecordScope::Project);
        p.record.workspace_root = Some("/elsewhere".to_string());
        let g = pref("ctx::g", Authority::UserConfirmed, RecordScope::Global);
        let resolved = resolve_context(vec![p, g], &view(Some("/work"), None));
        assert_eq!(resolved.fingerprint.len(), 1);
        assert_eq!(resolved.fingerprint[0].record.id, "ctx::g");
    }

    #[test]
    fn global_view_sees_only_globals() {
        let records = vec![
            pref("ctx::g", Authority::UserConfirmed, RecordScope::Global),
            pref("ctx::p", Authority::UserConfirmed, RecordScope::Project),
        ];
        let resolved = resolve_context(records, &view(None, None));
        assert_eq!(resolved.fingerprint.len(), 1);
        assert_eq!(resolved.fingerprint[0].record.id, "ctx::g");
    }

    #[test]
    fn ordering_is_total_and_deterministic() {
        // Identical twins except id: lowest id wins, order stable.
        let mut a = pref("ctx::b", Authority::UserConfirmed, RecordScope::Global);
        a.effective_confidence = 0.5;
        let mut b = pref("ctx::a", Authority::UserConfirmed, RecordScope::Global);
        b.effective_confidence = 0.5;
        let r1 = resolve_context(vec![a.clone(), b.clone()], &view(None, None));
        let r2 = resolve_context(vec![b, a], &view(None, None));
        assert_eq!(r1.fingerprint[0].record.id, r2.fingerprint[0].record.id);
    }

    #[test]
    fn scope_ranks_task_above_project_above_global() {
        assert!(scope_rank(RecordScope::Task) > scope_rank(RecordScope::Project));
        assert!(scope_rank(RecordScope::Project) > scope_rank(RecordScope::Global));
    }
}

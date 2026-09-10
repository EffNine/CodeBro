//! Deterministic retrieval over the context store.
//!
//! Retrieval is deliberately simple and swappable: a [`ContextRetriever`]
//! trait over the store, structured filters, and — when keywords are given
//! — an FTS5 BM25 match. No embeddings, no external services. A future
//! semantic backend can implement the same trait without redesigning the
//! domain model.

use crate::types::{Authority, ContextRecord, RecordKind, RecordStatus};

/// One search hit: the record plus retrieval-time metadata.
#[derive(Debug, Clone)]
pub struct RankedRecord {
    pub record: ContextRecord,
    /// FTS5 BM25 rank (lower is better); `None` for keyword-less retrieval.
    pub bm25: Option<f64>,
    /// The record's confidence after evidence-decay at retrieval time.
    pub effective_confidence: f64,
}

/// Retrieval request. Status defaults to `Active` when unset: superseded,
/// expired, and rejected knowledge stays queryable only when explicitly
/// requested.
#[derive(Debug, Clone, Default)]
pub struct RecordQuery<'a> {
    /// Workspace to scope to. Visible records = global records, the
    /// workspace's project records, and — only with a matching `task_id` —
    /// that task's records. `None` = global records only.
    pub workspace_root: Option<&'a str>,
    /// Task identity for task-scoped resolution. `None` (the default) makes
    /// task-scoped records invisible; pass the current task to include its
    /// overrides. Ignored unless `workspace_root` matches the record.
    pub task_id: Option<&'a str>,
    pub kind: Option<RecordKind>,
    pub status: Option<RecordStatus>,
    /// Free-text keywords; FTS5-matched when present.
    pub keywords: Vec<String>,
    /// Hard-bounded result count (clamped to 1..=200).
    pub limit: usize,
}

/// Storage backends implement this; the MCP layer consumes only the trait.
pub trait ContextRetriever {
    fn search(
        &self,
        query: &RecordQuery<'_>,
        now: u64,
    ) -> Result<Vec<RankedRecord>, crate::store::ContextError>;
}

/// Monthly confidence decay rate per authority. Knowledge whose evidence
/// does not refresh must not stay permanently high-confidence; how fast it
/// decays reflects how much the user stood behind it.
pub fn decay_rate_per_month(authority: Authority) -> f64 {
    match authority {
        // User confirmation is the strongest standing: decays slowly.
        Authority::UserConfirmed => 0.98,
        // Project pipelines re-derive deterministically on reindex.
        Authority::ProjectDerived => 0.95,
        Authority::SystemDerived => 0.95,
        // Observations age faster once their events stop recurring.
        Authority::Observed => 0.90,
        Authority::Imported => 0.90,
        // Pure inference: without confirmation it fades fastest.
        Authority::AiInferred => 0.85,
    }
}

/// Effective confidence of a record at `now`, applying one decay step per
/// 30 days since `updated_at`. Clamped to [0, 1].
pub fn decayed_confidence(record: &ContextRecord, now: u64) -> f64 {
    let days_since_update = now.saturating_sub(record.updated_at) / 86_400;
    let months = days_since_update as f64 / 30.0;
    let multiplier = decay_rate_per_month(record.authority).powf(months);
    (record.confidence * multiplier).clamp(0.0, 1.0)
}

/// Split free-text keywords into FTS5-safe query tokens: alphanumeric runs
/// of at least three characters, lowercased (unicode61 folds case at index
/// time, so lowercase queries match).
pub fn query_tokens(keywords: &[String]) -> Vec<String> {
    let mut tokens: Vec<String> = Vec::new();
    for kw in keywords {
        for tok in kw.split(|c: char| !c.is_alphanumeric()) {
            let tok = tok.to_lowercase();
            if tok.chars().count() >= 3 && !tokens.contains(&tok) {
                tokens.push(tok);
            }
        }
    }
    tokens.sort();
    tokens
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(updated_at: u64, authority: Authority, confidence: f64) -> ContextRecord {
        let mut r = ContextRecord::new(
            "ctx::d",
            RecordKind::Preference,
            "fp.test",
            "content",
            authority,
        );
        r.confidence = confidence;
        r.updated_at = updated_at;
        r
    }

    #[test]
    fn confidence_decays_over_time_and_never_exceeds_base() {
        let now = 2_000_000_000;
        // One month after an inference at 0.9 → ~0.765; twelve months → low.
        let one_month = record(now - 30 * 86_400, Authority::AiInferred, 0.9);
        let decayed = decayed_confidence(&one_month, now);
        assert!((decayed - 0.9 * 0.85).abs() < 1e-9);
        assert!(decayed < 0.9);

        let one_year = record(now - 365 * 86_400, Authority::AiInferred, 0.9);
        let far = decayed_confidence(&one_year, now);
        assert!(far < 0.2, "a year without evidence must fade hard: {far}");
        assert!(far >= 0.0);

        // Fresh records keep their declared confidence.
        let fresh = record(now, Authority::AiInferred, 0.9);
        assert_eq!(decayed_confidence(&fresh, now), 0.9);
    }

    #[test]
    fn user_confirmed_decays_slower_than_inference() {
        let now = 2_000_000_000;
        let updated = now - 90 * 86_400;
        let confirmed = decayed_confidence(&record(updated, Authority::UserConfirmed, 1.0), now);
        let inferred = decayed_confidence(&record(updated, Authority::AiInferred, 1.0), now);
        assert!(
            confirmed > inferred,
            "user-confirmed knowledge must outlast bare inference"
        );
        assert!(confirmed > 0.9, "three months: confirmed ≈0.94");
    }

    #[test]
    fn decay_clamps_at_zero_and_stays_bounded() {
        let ancient = record(1000, Authority::AiInferred, 1.0);
        let value = decayed_confidence(&ancient, 3_000_000_000);
        assert!((0.0..=1.0).contains(&value));
    }

    #[test]
    fn query_tokens_are_normalized_deduplicated_and_sorted() {
        let tokens = query_tokens(&[
            "Manglish style".to_string(),
            "over-engineer!".to_string(),
            "ab".to_string(), // too short
            "style".to_string(),
        ]);
        assert_eq!(tokens, vec!["engineer", "manglish", "over", "style"]);
        assert!(query_tokens(&[]).is_empty());
    }

    #[test]
    fn query_tokens_escape_nothing_dangerous() {
        // Tokens are matched as quoted literals by the store; this just
        // guarantees the tokenizer never emits FTS5 syntax characters.
        let tokens = query_tokens(&["a\"b AND c".to_string()]);
        assert!(tokens
            .iter()
            .all(|t| !t.contains('"') && !t.contains("AND")));
    }

    #[test]
    fn decay_rates_are_stable_and_total() {
        // Every authority has an explicit, tested rate (regression guard:
        // adding an Authority variant without a rate would panic here).
        for a in [
            Authority::UserConfirmed,
            Authority::AiInferred,
            Authority::Observed,
            Authority::ProjectDerived,
            Authority::Imported,
            Authority::SystemDerived,
        ] {
            let rate = decay_rate_per_month(a);
            assert!((0.0..=1.0).contains(&rate), "rate for {a} out of range");
        }
    }
}

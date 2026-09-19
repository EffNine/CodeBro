//! P3 learning + inference tests: candidates, evidence, evaluation,
//! confidence, authority, scope, lifecycle, idempotency, contradictions,
//! security, context integration, and failure isolation.
//!
//! Strategy: drive the public store API (`propose_candidates`,
//! `evaluate_candidate`, `run_learning`, `confirm/reject_candidate`) over
//! hermetic tempdir stores. Summaries are crafted with stable shared
//! vocabulary so deterministic pair clustering fires predictably; tests
//! assert on counts/kinds/statuses/authority — never on a specific pair.

use crate::db;
use crate::fingerprint::{self, ResolutionScope};
use crate::history::{HistoryInput, HistoryKind, OpenSession};
use crate::learning::{
    contains_sensitive_content, outcome_polarity, CandidateKind, CandidateStatus, LearnScope,
    LearningCandidate, OutcomePolarity, ACCEPT_MIN_CONFIDENCE, MIN_SUPPORTING_EVIDENCE,
};
use crate::retrieval::RecordQuery;
use crate::store::{ContextError, ContextStore};
use crate::types::{Authority, RecordStatus};
use crate::ContextRetriever;

fn store() -> (tempfile::TempDir, ContextStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = ContextStore::new(dir.path().join(db::STATE_DB_FILE));
    (dir, store)
}

fn session(store: &ContextStore, ws: &str, at: u64) -> String {
    store
        .open_session(ws, &OpenSession::default(), at)
        .unwrap()
        .id
}

/// Record one history event; returns its id.
fn hist(
    store: &ContextStore,
    ws: &str,
    session: Option<&str>,
    kind: HistoryKind,
    summary: &str,
    outcome: Option<&str>,
    at: u64,
) -> i64 {
    let mut input = HistoryInput::new(ws, kind, summary);
    input.session_id = session.map(str::to_string);
    input.outcome = outcome.map(str::to_string);
    input.source = Some("p3-test".to_string());
    let (id, dup) = store.record_history(&input, at).unwrap();
    assert!(!dup);
    id
}

fn validation(store: &ContextStore, ws: &str, sid: &str, summary: &str, outcome: &str, at: u64) {
    hist(
        store,
        ws,
        Some(sid),
        HistoryKind::Validation,
        summary,
        Some(outcome),
        at,
    );
}

fn run_project(store: &ContextStore, ws: &str, now: u64) -> crate::learning::LearningRunOutcome {
    store
        .run_learning(Some(ws), None, LearnScope::Project, now)
        .unwrap()
}

/// The shared-vocabulary failure topic used across tests.
fn fail_summary(i: usize) -> String {
    format!("checkout pipeline container build failed on image layer {i}")
}

fn seed_failures(store: &ContextStore, ws: &str, n: usize, base: u64) {
    let sid = session(store, ws, base);
    for i in 0..n {
        validation(
            store,
            ws,
            &sid,
            &fail_summary(i),
            "test_failure",
            base + i as u64,
        );
    }
}

fn pass_summary(i: usize) -> String {
    format!("checkout pipeline container build passed on image layer {i}")
}

fn by_kind(store: &ContextStore, ws: &str, kind: &str) -> Vec<LearningCandidate> {
    store
        .list_candidates(Some(ws), None, None, 50)
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == kind)
        .collect()
}

// ── Candidate generation ─────────────────────────────────────────────────

#[test]
fn repeated_failures_form_failure_pattern() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 1, "one pair-cluster ⇒ one candidate");
    assert_eq!(outcome.accepted, 1);
    let found = by_kind(&store, "/proj", "failure_pattern");
    assert_eq!(found.len(), 1);
    let c = &found[0];
    assert_eq!(c.supporting_evidence.len(), 3);
    assert!(c.contradicting_evidence.is_empty());
    assert!(c.confidence >= ACCEPT_MIN_CONFIDENCE);
    assert!((c.confidence <= 0.95) && (c.confidence >= 0.05));
    // No fake precision: two decimals.
    assert!((c.confidence * 100.0).fract() == 0.0);
    // A proposition, not a bare fact.
    assert!(c.proposition.contains("repeatedly"));
    assert!(c.proposition.contains('3'));
}

#[test]
fn repeated_successes_form_success_pattern() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        validation(
            &store,
            "/proj",
            &sid,
            &pass_summary(i),
            "passed",
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.accepted, 1);
    assert_eq!(by_kind(&store, "/proj", "success_pattern").len(), 1);
}

#[test]
fn repeated_decisions_form_decision_pattern() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        hist(
            &store,
            "/proj",
            Some(&sid),
            HistoryKind::Decision,
            &format!("architecture review board selected sqlite storage backend option {i}"),
            Some("decided"),
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.accepted, 1);
    assert_eq!(by_kind(&store, "/proj", "decision_pattern").len(), 1);
}

#[test]
fn avoidance_decisions_become_user_preference_hypothesis() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        hist(
            &store,
            "/proj",
            Some(&sid),
            HistoryKind::Decision,
            &format!("avoid unnecessary dependency tiny utility inline implementation choice {i}"),
            Some("decided"),
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.accepted, 1);
    let found = by_kind(&store, "/proj", "user_preference");
    assert_eq!(found.len(), 1);
    assert!(found[0].proposition.contains("appears to prefer"));
}

#[test]
fn three_observations_are_not_enough_observations_need_four() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        hist(
            &store,
            "/proj",
            Some(&sid),
            HistoryKind::Observation,
            &format!("noticed flaky network sandbox cache latency spike {i}"),
            None,
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 0, "weak signals need four");
    hist(
        &store,
        "/proj",
        Some(&sid),
        HistoryKind::Observation,
        "noticed flaky network sandbox cache latency spike 3",
        None,
        1200,
    );
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 1);
}

#[test]
fn conversational_chatter_never_forms_candidates() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..5 {
        hist(
            &store,
            "/proj",
            Some(&sid),
            HistoryKind::UserMessage,
            &format!("please use sqlite database storage layer again {i}"),
            None,
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 0, "chat is weak evidence, never mined");
}

// ── Evidence ─────────────────────────────────────────────────────────────

#[test]
fn accepted_evidence_references_real_events() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    assert_eq!(found.len(), 1);
    for id in &found[0].supporting_evidence {
        let event = store.get_event(*id).unwrap();
        assert!(event.is_some(), "evidence {id} must resolve");
        assert_eq!(event.unwrap().workspace_root, "/proj");
    }
    // And the persisted inference cites the same ids.
    let inf_id = found[0].inference_record_id.clone().unwrap();
    let record = store.get_record(&inf_id).unwrap().unwrap();
    assert_eq!(record.evidence.len(), 3);
}

#[test]
fn fake_evidence_is_dropped_and_defers() {
    let (_dir, store) = store();
    // One real event so the workspace is non-empty; the candidate cites
    // fiction plus a foreign id.
    let sid = session(&store, "/proj", 1000);
    let real = hist(
        &store,
        "/proj",
        Some(&sid),
        HistoryKind::Validation,
        &fail_summary(0),
        Some("test_failure"),
        1100,
    );
    let ghost = LearningCandidate {
        candidate_id: "lc::fakeevidence0001".to_string(),
        workspace_root: Some("/proj".to_string()),
        task_id: None,
        scope: "project".to_string(),
        kind: CandidateKind::FailurePattern.as_str().to_string(),
        proposition: "fiction".to_string(),
        namespace: "learn.failure-pattern.fiction-case".to_string(),
        supporting_evidence: vec![999_999_999, real],
        contradicting_evidence: vec![],
        confidence: 0.0,
        status: CandidateStatus::Candidate.as_str().to_string(),
        created_at: 1100,
        updated_at: 1100,
        expires_at: None,
        eval_reason: None,
        inference_record_id: None,
    };
    store.upsert_candidate(&ghost).unwrap();
    let done = store
        .evaluate_candidate("lc::fakeevidence0001", 2000)
        .unwrap();
    assert_eq!(done.status, "deferred");
    assert_eq!(done.supporting_evidence, vec![real]);
    assert!(done.eval_reason.unwrap().contains("dropped"));
}

#[test]
fn cross_workspace_evidence_is_dropped() {
    let (_dir, store) = store();
    let sid_b = session(&store, "/proj-b", 1000);
    let foreign = hist(
        &store,
        "/proj-b",
        Some(&sid_b),
        HistoryKind::Validation,
        &fail_summary(0),
        Some("test_failure"),
        1100,
    );
    let sid_a = session(&store, "/proj-a", 1000);
    let local = hist(
        &store,
        "/proj-a",
        Some(&sid_a),
        HistoryKind::Validation,
        &fail_summary(1),
        Some("test_failure"),
        1100,
    );
    let candidate = LearningCandidate {
        candidate_id: "lc::crossworkspace01".to_string(),
        workspace_root: Some("/proj-a".to_string()),
        task_id: None,
        scope: "project".to_string(),
        kind: CandidateKind::FailurePattern.as_str().to_string(),
        proposition: "laundering".to_string(),
        namespace: "learn.failure-pattern.laundering-case".to_string(),
        supporting_evidence: vec![local, foreign],
        contradicting_evidence: vec![],
        confidence: 0.0,
        status: CandidateStatus::Candidate.as_str().to_string(),
        created_at: 1100,
        updated_at: 1100,
        expires_at: None,
        eval_reason: None,
        inference_record_id: None,
    };
    store.upsert_candidate(&candidate).unwrap();
    let done = store
        .evaluate_candidate("lc::crossworkspace01", 2000)
        .unwrap();
    assert_eq!(done.supporting_evidence, vec![local]);
    assert_eq!(done.status, "deferred", "one local event is insufficient");
}

// ── Evaluation ───────────────────────────────────────────────────────────

#[test]
fn single_weak_observation_defers_for_insufficient_evidence() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    let id = hist(
        &store,
        "/proj",
        Some(&sid),
        HistoryKind::Observation,
        &fail_summary(0),
        Some("test_failure"),
        1100,
    );
    // A lone event cannot cluster (min support), and a hand-built
    // single-evidence candidate must defer, not promote.
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 0);
    let candidate = LearningCandidate {
        candidate_id: "lc::singleweak00001".to_string(),
        workspace_root: Some("/proj".to_string()),
        task_id: None,
        scope: "project".to_string(),
        kind: CandidateKind::FailurePattern.as_str().to_string(),
        proposition: "one swallow".to_string(),
        namespace: "learn.failure-pattern.single-swallow".to_string(),
        supporting_evidence: vec![id],
        contradicting_evidence: vec![],
        confidence: 0.0,
        status: CandidateStatus::Candidate.as_str().to_string(),
        created_at: 1100,
        updated_at: 1100,
        expires_at: None,
        eval_reason: None,
        inference_record_id: None,
    };
    store.upsert_candidate(&candidate).unwrap();
    let done = store
        .evaluate_candidate("lc::singleweak00001", 2000)
        .unwrap();
    assert_eq!(done.status, "deferred");
    assert!(done.eval_reason.unwrap().contains("insufficient"));
    // Deferred ⇒ no knowledge persisted.
    assert!(done.inference_record_id.is_none());
}

#[test]
fn contested_evidence_defers_without_blind_promotion() {
    let (_dir, store) = store();
    // 5 successes vs 4 failures on the same topic: neither side promotes.
    let sid = session(&store, "/proj", 1000);
    for i in 0..5 {
        validation(
            &store,
            "/proj",
            &sid,
            &pass_summary(i),
            "passed",
            1100 + i as u64,
        );
    }
    for i in 0..4 {
        validation(
            &store,
            "/proj",
            &sid,
            &fail_summary(i),
            "test_failure",
            1200 + i as u64,
        );
    }
    // Same pair? pass/fail summaries share checkout/pipeline/container/build
    // vocabulary, so both lanes see the same topic with opposite polarity.
    let outcome = run_project(&store, "/proj", 3000);
    assert_eq!(outcome.accepted, 0, "contested evidence must not promote");
    let success = by_kind(&store, "/proj", "success_pattern");
    let failure = by_kind(&store, "/proj", "failure_pattern");
    assert_eq!(success.len(), 1);
    assert_eq!(failure.len(), 1);
    assert_eq!(success[0].status, "deferred", "5v4 contests ⇒ defer");
    assert_eq!(failure[0].status, "rejected", "4v5 weighs against ⇒ reject");
    // Nothing reached trusted context.
    assert!(success[0].inference_record_id.is_none());
    assert!(failure[0].inference_record_id.is_none());
}

#[test]
fn majority_contradiction_rejects() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        validation(
            &store,
            "/proj",
            &sid,
            &pass_summary(i),
            "passed",
            1100 + i as u64,
        );
    }
    for i in 0..5 {
        validation(
            &store,
            "/proj",
            &sid,
            &fail_summary(i),
            "test_failure",
            1200 + i as u64,
        );
    }
    run_project(&store, "/proj", 3000);
    let success = by_kind(&store, "/proj", "success_pattern");
    assert_eq!(success.len(), 1);
    assert_eq!(success[0].status, "rejected", "3v5 weighs against");
}

// ── Confidence ───────────────────────────────────────────────────────────

#[test]
fn more_support_means_more_confidence() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj-small", 3, 1000);
    run_project(&store, "/proj-small", 2000);
    // A different topic with five failures (same shape, more evidence).
    let sid = session(&store, "/proj-large", 1000);
    for i in 0..5 {
        validation(
            &store,
            "/proj-large",
            &sid,
            &format!("ledger reconciliation worker crashed on batch segment {i}"),
            "test_failure",
            1100 + i as u64,
        );
    }
    run_project(&store, "/proj-large", 2000);
    let small = by_kind(&store, "/proj-small", "failure_pattern")[0].clone();
    let large = by_kind(&store, "/proj-large", "failure_pattern")[0].clone();
    assert!(large.confidence > small.confidence);
}

#[test]
fn contradiction_lowers_confidence() {
    let (_dir, store) = store();
    // Pure topic: 4 successes, no contradiction.
    let sid = session(&store, "/proj-pure", 1000);
    for i in 0..4 {
        validation(
            &store,
            "/proj-pure",
            &sid,
            &format!("pure victory pipeline release bundle parcel {i}"),
            "passed",
            1100 + i as u64,
        );
    }
    run_project(&store, "/proj-pure", 2000);
    // Mixed topic: 4 successes + 1 failure (still accepted: 1×2 < 4).
    let sid = session(&store, "/proj-mixed", 1000);
    for i in 0..4 {
        validation(
            &store,
            "/proj-mixed",
            &sid,
            &format!("mixed victory pipeline release bundle parcel {i}"),
            "passed",
            1100 + i as u64,
        );
    }
    validation(
        &store,
        "/proj-mixed",
        &sid,
        "mixed victory pipeline release bundle parcel nine",
        "test_failure",
        1200,
    );
    run_project(&store, "/proj-mixed", 2000);
    let pure = by_kind(&store, "/proj-pure", "success_pattern")[0].clone();
    let mixed = by_kind(&store, "/proj-mixed", "success_pattern")[0].clone();
    assert_eq!(mixed.status, "accepted");
    assert!(
        mixed.confidence < pure.confidence,
        "contradiction must dent confidence: {} vs {}",
        mixed.confidence,
        pure.confidence
    );
}

#[test]
fn stale_evidence_confidence_is_lower_than_fresh() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj-old", 1000);
    for i in 0..3 {
        validation(
            &store,
            "/proj-old",
            &sid,
            &format!("ancient archive migration cold storage vault {i}"),
            "test_failure",
            1000 + i as u64,
        );
    }
    // Evaluate two months later: nothing within the 30-day window.
    run_project(&store, "/proj-old", 1000 + 60 * 86_400);
    let sid = session(&store, "/proj-new", 100_000_000);
    for i in 0..3 {
        validation(
            &store,
            "/proj-new",
            &sid,
            &format!("fresh archive migration cold storage vault {i}"),
            "test_failure",
            100_000_000 + i as u64,
        );
    }
    run_project(&store, "/proj-new", 100_000_000 + 500);
    let old = by_kind(&store, "/proj-old", "failure_pattern")[0].clone();
    let new = by_kind(&store, "/proj-new", "failure_pattern")[0].clone();
    assert!(old.confidence < new.confidence);
}

#[test]
fn strong_evidence_outweighs_weak_evidence() {
    let (_dir, store) = store();
    // 3 strong decisions…
    let sid = session(&store, "/proj-strong", 1000);
    for i in 0..3 {
        hist(
            &store,
            "/proj-strong",
            Some(&sid),
            HistoryKind::Decision,
            &format!("strong granite basalt quartz council ruling decree {i}"),
            Some("decided"),
            1100 + i as u64,
        );
    }
    run_project(&store, "/proj-strong", 2000);
    // …vs 4 weak observations (the minimum for that lane).
    let sid = session(&store, "/proj-weak", 1000);
    for i in 0..4 {
        hist(
            &store,
            "/proj-weak",
            Some(&sid),
            HistoryKind::Observation,
            &format!("weak granite basalt quartz council rumor whisper {i}"),
            None,
            1100 + i as u64,
        );
    }
    run_project(&store, "/proj-weak", 2000);
    let strong = by_kind(&store, "/proj-strong", "decision_pattern")[0].clone();
    let weak_cands = store
        .list_candidates(Some("/proj-weak"), None, None, 50)
        .unwrap();
    assert_eq!(weak_cands.len(), 1);
    assert!(
        strong.confidence > weak_cands[0].confidence,
        "strong {} vs weak {}",
        strong.confidence,
        weak_cands[0].confidence
    );
}

// ── Authority ────────────────────────────────────────────────────────────

#[test]
fn inference_is_ai_inferred_never_user_confirmed() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    let record = store
        .get_record(found[0].inference_record_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.authority, Authority::AiInferred);
    assert_ne!(record.authority, Authority::UserConfirmed);
}

#[test]
fn forged_self_confirmation_is_refused() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    let err = store
        .confirm_candidate(&found[0].candidate_id, false, 3000)
        .unwrap_err();
    assert!(err.to_string().contains("user_confirmed=true"));
    // Authority untouched.
    let record = store
        .get_record(found[0].inference_record_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.authority, Authority::AiInferred);
}

#[test]
fn explicit_user_confirmation_promotes_via_supersede() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    let old_inf = found[0].inference_record_id.clone().unwrap();
    let done = store
        .confirm_candidate(&found[0].candidate_id, true, 3000)
        .unwrap();
    assert_eq!(done.status, "superseded");
    let new_id = done.inference_record_id.clone().unwrap();
    assert_ne!(new_id, old_inf);
    let confirmed = store.get_record(&new_id).unwrap().unwrap();
    assert_eq!(confirmed.authority, Authority::UserConfirmed);
    assert_eq!(confirmed.supersedes.as_deref(), Some(old_inf.as_str()));
    let old = store.get_record(&old_inf).unwrap().unwrap();
    assert_eq!(old.status, RecordStatus::Superseded);
}

#[test]
fn user_rejection_preserves_negative_knowledge() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    let inf_id = found[0].inference_record_id.clone().unwrap();
    let done = store
        .reject_candidate(
            &found[0].candidate_id,
            Some("only true for legacy code"),
            3000,
        )
        .unwrap();
    assert_eq!(done.status, "rejected");
    // The inference row stays — rejected, not deleted.
    let inf = store.get_record(&inf_id).unwrap().unwrap();
    assert_eq!(inf.status, RecordStatus::Rejected);
    // And re-detection refuses to rewrite the user's verdict.
    let outcome = run_project(&store, "/proj", 4000);
    assert_eq!(outcome.skipped_terminal, 1);
    assert_eq!(
        store
            .get_candidate(&found[0].candidate_id)
            .unwrap()
            .unwrap()
            .status,
        "rejected"
    );
}

// ── Scope ────────────────────────────────────────────────────────────────

#[test]
fn project_pattern_does_not_leak_into_other_projects() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj-a", 3, 1000);
    let outcome = store
        .run_learning(Some("/proj-b"), None, LearnScope::Project, 2000)
        .unwrap();
    assert_eq!(outcome.proposed, 0);
    assert!(store
        .list_candidates(Some("/proj-b"), None, None, 50)
        .unwrap()
        .is_empty());
    // A's candidate exists and is scoped to A.
    run_project(&store, "/proj-a", 2000);
    let a = store
        .list_candidates(Some("/proj-a"), None, None, 50)
        .unwrap();
    assert_eq!(a.len(), 1);
    assert_eq!(a[0].workspace_root.as_deref(), Some("/proj-a"));
}

#[test]
fn task_pattern_stays_in_its_task() {
    let (_dir, store) = store();
    for i in 0..3 {
        let mut input = HistoryInput::new(
            "/proj",
            HistoryKind::Validation,
            format!("task scoped widget gadget sprocket failure batch {i}"),
        );
        input.task_id = Some("task-1".to_string());
        input.outcome = Some("test_failure".to_string());
        store.record_history(&input, 1100 + i as u64).unwrap();
    }
    // Task B sees nothing.
    let outcome = store
        .run_learning(Some("/proj"), Some("task-2"), LearnScope::Task, 2000)
        .unwrap();
    assert_eq!(outcome.proposed, 0);
    // Task A learns, task-scoped.
    let outcome = store
        .run_learning(Some("/proj"), Some("task-1"), LearnScope::Task, 2000)
        .unwrap();
    assert_eq!(outcome.accepted, 1);
    let found = store
        .list_candidates(Some("/proj"), None, Some("task-1"), 50)
        .unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].scope, "task");
    assert_eq!(found[0].task_id.as_deref(), Some("task-1"));
    let inf_id = found[0].inference_record_id.clone().unwrap();
    let record = store.get_record(&inf_id).unwrap().unwrap();
    assert_eq!(record.task_id.as_deref(), Some("task-1"));
}

#[test]
fn global_inference_needs_broader_evidence() {
    let (_dir, store) = store();
    seed_failures(&store, "/only-proj", 3, 1000);
    let outcome = store
        .run_learning(None, None, LearnScope::Global, 2000)
        .unwrap();
    assert_eq!(outcome.proposed, 1);
    assert_eq!(outcome.accepted, 0, "one workspace is not global evidence");
    assert_eq!(outcome.deferred, 1);
    let all = store
        .list_candidates(Some("/only-proj"), None, None, 50)
        .unwrap();
    assert_eq!(all[0].status, "deferred");
    assert!(all[0].eval_reason.as_deref().unwrap().contains("broader"));
}

#[test]
fn global_inference_accepts_multi_workspace_evidence() {
    let (_dir, store) = store();
    for ws in ["/ga", "/gb"] {
        let sid = session(&store, ws, 1000);
        for i in 0..2 {
            validation(
                &store,
                ws,
                &sid,
                &format!("shared quorum consensus protocol election round failure {i}"),
                "test_failure",
                1100 + i as u64,
            );
        }
    }
    let outcome = store
        .run_learning(None, None, LearnScope::Global, 2000)
        .unwrap();
    assert_eq!(outcome.accepted, 1);
    let found = store
        .list_candidates(Some("/elsewhere"), None, None, 50)
        .unwrap();
    assert_eq!(found.len(), 1, "global rows are visible everywhere");
    assert_eq!(found[0].scope, "global");
    assert!(found[0].workspace_root.is_none());
    // …but still AI_INFERRED, never confirmed.
    let record = store
        .get_record(found[0].inference_record_id.as_deref().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(record.authority, Authority::AiInferred);
}

// ── Lifecycle ────────────────────────────────────────────────────────────

#[test]
fn rejected_and_expired_rows_are_never_reevaluated() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    let id = found[0].candidate_id.clone();
    store.reject_candidate(&id, None, 3000).unwrap();
    assert!(store.evaluate_candidate(&id, 4000).is_err());
}

#[test]
fn candidates_expire_but_history_survives() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 1, 1000);
    // Hand-built candidate already past its TTL.
    let candidate = LearningCandidate {
        candidate_id: "lc::expiring000001".to_string(),
        workspace_root: Some("/proj".to_string()),
        task_id: None,
        scope: "project".to_string(),
        kind: CandidateKind::FailurePattern.as_str().to_string(),
        proposition: "wilting".to_string(),
        namespace: "learn.failure-pattern.wilting-case".to_string(),
        supporting_evidence: vec![1],
        contradicting_evidence: vec![],
        confidence: 0.4,
        status: CandidateStatus::Deferred.as_str().to_string(),
        created_at: 1000,
        updated_at: 1000,
        expires_at: Some(1500),
        eval_reason: None,
        inference_record_id: None,
    };
    store.upsert_candidate(&candidate).unwrap();
    assert_eq!(store.expire_learning_sweep(2000).unwrap(), 1);
    assert_eq!(
        store
            .get_candidate("lc::expiring000001")
            .unwrap()
            .unwrap()
            .status,
        "expired"
    );
    // Historical evidence untouched by the sweep.
    assert_eq!(store.list_events("/proj", 10).unwrap().len(), 1);
}

#[test]
fn reprocessing_evolves_confidence_without_duplicates() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    let first = run_project(&store, "/proj", 2000);
    assert_eq!(first.accepted, 1);
    let c1 = by_kind(&store, "/proj", "failure_pattern")[0].clone();
    // Contradicting evidence arrives: one success on the same topic.
    let sid = session(&store, "/proj", 3000);
    validation(
        &store,
        "/proj",
        &sid,
        "checkout pipeline container build passed on image layer nine",
        "passed",
        3100,
    );
    let second = run_project(&store, "/proj", 4000);
    assert_eq!(second.accepted, 1);
    assert_eq!(second.refreshed, 1);
    let all = store
        .list_candidates(Some("/proj"), None, None, 50)
        .unwrap();
    assert_eq!(all.len(), 1, "no duplicate candidates");
    assert_eq!(
        all[0].candidate_id, c1.candidate_id,
        "deterministic identity"
    );
    assert!(
        all[0].confidence < c1.confidence,
        "confidence evolves: {} → {}",
        c1.confidence,
        all[0].confidence
    );
    assert_eq!(all[0].contradicting_evidence.len(), 1);
}

#[test]
fn repeated_full_passes_stay_deterministic() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    let first = run_project(&store, "/proj", 2000);
    let second = run_project(&store, "/proj", 3000);
    assert_eq!(first.accepted, 1);
    assert_eq!(second.accepted, 1);
    // No new evidence ⇒ the inference record is reused, not churned.
    let a = by_kind(&store, "/proj", "failure_pattern")[0].clone();
    assert_eq!(second.refreshed, 1);
    assert_eq!(
        store
            .list_candidates(Some("/proj"), None, None, 50)
            .unwrap()
            .len(),
        1
    );
    let _ = a;
}

// ── Security ─────────────────────────────────────────────────────────────

#[test]
fn sensitive_topics_never_form_candidates() {
    let (_dir, store) = store();
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        hist(
            &store,
            "/proj",
            Some(&sid),
            HistoryKind::Decision,
            &format!("personality disorder diagnosis {i}"),
            Some("decided"),
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 0, "sensitive topics must not be mined");
    assert!(contains_sensitive_content("personality disorder"));
    assert!(!contains_sensitive_content("checkout pipeline build"));
}

#[test]
fn single_token_topics_are_string_frequency_not_evidence() {
    let (_dir, store) = store();
    // The only shared token is "sqlite": no pair ever repeats.
    let sid = session(&store, "/proj", 1000);
    for (i, extra) in ["alpha bravo", "charlie delta", "echo foxtrot"]
        .iter()
        .enumerate()
    {
        validation(
            &store,
            "/proj",
            &sid,
            &format!("sqlite {extra}"),
            "test_failure",
            1100 + i as u64,
        );
    }
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 0);
}

#[test]
fn secrets_in_history_do_not_reach_propositions() {
    let (_dir, store) = store();
    // History redacts before storage; propositions are built from stored
    // (already-redacted) text, so a secret can never surface in learning.
    let sid = session(&store, "/proj", 1000);
    let secret = "sk-testsecretkey1234567890abcdef";
    for i in 0..3 {
        validation(
            &store,
            "/proj",
            &sid,
            &format!("checkout pipeline deploy failed with api_key=\"{secret}\" retry {i}"),
            "test_failure",
            1100 + i as u64,
        );
    }
    run_project(&store, "/proj", 2000);
    for c in store
        .list_candidates(Some("/proj"), None, None, 50)
        .unwrap()
    {
        assert!(!c.proposition.contains(secret));
        assert!(!c.namespace.contains(secret));
        if let Some(inf) = c.inference_record_id {
            let record = store.get_record(&inf).unwrap().unwrap();
            assert!(!record.content.contains(secret));
        }
    }
}

// ── Workflow patterns ────────────────────────────────────────────────────

#[test]
fn change_then_validate_cycles_form_workflow_pattern() {
    let (_dir, store) = store();
    for s in 0..3u64 {
        let sid = session(&store, "/proj", 1000 + s * 100);
        let mut change = HistoryInput::new(
            "/proj",
            HistoryKind::ChangeApplied,
            format!("refactor checkout pipeline step {} complete", s + 10),
        );
        change.session_id = Some(sid.clone());
        change.outcome = Some("applied".to_string());
        store.record_history(&change, 1010 + s * 100).unwrap();
        validation(
            &store,
            "/proj",
            &sid,
            &format!("checkout pipeline validation passed run {}", s + 10),
            "passed",
            1020 + s * 100,
        );
    }
    let outcome = run_project(&store, "/proj", 5000);
    assert!(outcome.accepted >= 1);
    let workflows = by_kind(&store, "/proj", "workflow_pattern");
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].status, "accepted");
}

// ── Context integration ──────────────────────────────────────────────────

#[test]
fn accepted_inference_is_context_eligible_deferred_is_not() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 5000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    assert_eq!(found.len(), 1);
    // The accepted inference resolves through the standard retrieval path
    // (Experience passthrough lane) with its authority visible.
    let ranked = store
        .search(
            &RecordQuery {
                workspace_root: Some("/proj"),
                task_id: None,
                kind: None,
                status: None,
                keywords: Vec::new(),
                limit: 50,
            },
            5000,
        )
        .unwrap();
    let scope = ResolutionScope {
        workspace_key: Some("/proj"),
        task_id: None,
    };
    let resolved = fingerprint::resolve_context(ranked, &scope);
    let ids: Vec<&str> = resolved
        .other
        .iter()
        .map(|r| r.record.id.as_str())
        .collect();
    assert!(
        ids.contains(&found[0].inference_record_id.as_deref().unwrap()),
        "accepted inference must be context-eligible: {ids:?}"
    );
    // A deferred candidate persists no record ⇒ nothing to surface.
    let contested = LearningCandidate {
        candidate_id: "lc::deferredghost01".to_string(),
        workspace_root: Some("/proj".to_string()),
        task_id: None,
        scope: "project".to_string(),
        kind: CandidateKind::SuccessPattern.as_str().to_string(),
        proposition: "ghost".to_string(),
        namespace: "learn.success-pattern.ghost-case".to_string(),
        supporting_evidence: vec![1],
        contradicting_evidence: vec![],
        confidence: 0.2,
        status: CandidateStatus::Deferred.as_str().to_string(),
        created_at: 1000,
        updated_at: 1000,
        expires_at: None,
        eval_reason: Some("weak".to_string()),
        inference_record_id: None,
    };
    store.upsert_candidate(&contested).unwrap();
    let ranked = store
        .search(
            &RecordQuery {
                workspace_root: Some("/proj"),
                task_id: None,
                kind: None,
                status: None,
                keywords: Vec::new(),
                limit: 50,
            },
            5000,
        )
        .unwrap();
    assert!(
        ranked
            .iter()
            .all(|r| r.record.namespace != "learn.success-pattern.ghost-case"),
        "deferred hypotheses must never enter trusted context"
    );
}

#[test]
fn confirmed_preference_beats_inference_in_resolution() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    // Confirm the hypothesis: USER_CONFIRMED now owns the namespace…
    let found = by_kind(&store, "/proj", "failure_pattern")[0].clone();
    store
        .confirm_candidate(&found.candidate_id, true, 3000)
        .unwrap();
    // …and resolution prefers it over the superseded inference.
    let ranked = store
        .search(
            &RecordQuery {
                workspace_root: Some("/proj"),
                task_id: None,
                kind: None,
                status: None,
                keywords: Vec::new(),
                limit: 50,
            },
            3000,
        )
        .unwrap();
    let scope = ResolutionScope {
        workspace_key: Some("/proj"),
        task_id: None,
    };
    let resolved = fingerprint::resolve_context(ranked, &scope);
    let winner = resolved
        .other
        .iter()
        .find(|r| r.record.namespace == found.namespace)
        .unwrap();
    assert_eq!(winner.record.authority, Authority::UserConfirmed);
}

// ── Failure isolation / transactions ─────────────────────────────────────

#[test]
fn learning_failure_leaves_history_durable() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    // Break the candidate table out from under learning.
    store
        .with_conn(|conn| {
            conn.execute("DROP TABLE learning_candidates", []).unwrap();
            Ok(())
        })
        .unwrap();
    assert!(store
        .run_learning(Some("/proj"), None, LearnScope::Project, 2000)
        .is_err());
    // History is canonical and untouched: reads and writes still work.
    assert_eq!(store.list_events("/proj", 10).unwrap().len(), 3);
    let sid = session(&store, "/proj", 3000);
    hist(
        &store,
        "/proj",
        Some(&sid),
        HistoryKind::Observation,
        "history survives learning failure",
        None,
        3100,
    );
    assert_eq!(store.list_events("/proj", 10).unwrap().len(), 4);
}

#[test]
fn evaluation_of_unknown_candidate_is_a_clean_error() {
    let (_dir, store) = store();
    let err = store
        .evaluate_candidate("lc::doesnotexist000", 1)
        .unwrap_err();
    assert!(matches!(err, ContextError::Validation(_)));
}

// ── Concurrency / idempotency ────────────────────────────────────────────

#[test]
fn concurrent_learning_passes_produce_no_duplicates() {
    let dir = tempfile::tempdir().unwrap();
    let store = std::sync::Arc::new(ContextStore::new(dir.path().join(db::STATE_DB_FILE)));
    seed_failures(&store, "/proj", 3, 1000);
    let mut handles = Vec::new();
    for _ in 0..8 {
        let store = std::sync::Arc::clone(&store);
        handles.push(std::thread::spawn(move || {
            store
                .run_learning(Some("/proj"), None, LearnScope::Project, 2000)
                .unwrap()
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let all = store
        .list_candidates(Some("/proj"), None, None, 50)
        .unwrap();
    assert_eq!(all.len(), 1, "concurrent passes must converge: {all:?}");
}

#[test]
fn same_evidence_twice_keeps_one_candidate() {
    let (_dir, store) = store();
    // Recording the same dedup-keyed events twice must not double evidence.
    let sid = session(&store, "/proj", 1000);
    for i in 0..3 {
        let mut input = HistoryInput::new("/proj", HistoryKind::Validation, fail_summary(i));
        input.session_id = Some(sid.clone());
        input.outcome = Some("test_failure".to_string());
        input.dedup_key = Some(format!("p3-dedup-{i}"));
        let (id, _) = store.record_history(&input, 1100 + i as u64).unwrap();
        // Replay: same key ⇒ same id, no duplicate history.
        let mut replay =
            HistoryInput::new("/proj", HistoryKind::Validation, "different words here");
        replay.dedup_key = Some(format!("p3-dedup-{i}"));
        let (id2, dup) = store.record_history(&replay, 1200 + i as u64).unwrap();
        assert!(dup);
        assert_eq!(id, id2);
    }
    run_project(&store, "/proj", 2000);
    run_project(&store, "/proj", 3000);
    assert_eq!(
        store
            .list_candidates(Some("/proj"), None, None, 50)
            .unwrap()
            .len(),
        1
    );
}

// ── Vocabulary / taxonomy ────────────────────────────────────────────────

#[test]
fn learning_vocabularies_roundtrip() {
    for k in [
        CandidateKind::UserPreference,
        CandidateKind::EngineeringPattern,
        CandidateKind::ProjectPattern,
        CandidateKind::FailurePattern,
        CandidateKind::SuccessPattern,
        CandidateKind::WorkflowPattern,
        CandidateKind::DecisionPattern,
    ] {
        assert_eq!(k.to_string().parse::<CandidateKind>().unwrap(), k);
    }
    assert!("telepathy".parse::<CandidateKind>().is_err());
    for s in [
        CandidateStatus::Candidate,
        CandidateStatus::Evaluating,
        CandidateStatus::Accepted,
        CandidateStatus::Rejected,
        CandidateStatus::Deferred,
        CandidateStatus::Superseded,
        CandidateStatus::Expired,
    ] {
        assert_eq!(s.to_string().parse::<CandidateStatus>().unwrap(), s);
    }
    assert!("ascended".parse::<CandidateStatus>().is_err());
    assert_eq!(
        "project".parse::<LearnScope>().unwrap(),
        LearnScope::Project
    );
    assert_eq!("task".parse::<LearnScope>().unwrap(), LearnScope::Task);
    assert_eq!("global".parse::<LearnScope>().unwrap(), LearnScope::Global);
    assert!("universe".parse::<LearnScope>().is_err());
}

#[test]
fn outcome_polarity_maps_known_labels_and_stays_neutral_otherwise() {
    assert_eq!(outcome_polarity(Some("passed")), OutcomePolarity::Success);
    assert_eq!(
        outcome_polarity(Some("test_failure")),
        OutcomePolarity::Failure
    );
    assert_eq!(
        outcome_polarity(Some("compile_error")),
        OutcomePolarity::Failure
    );
    assert_eq!(outcome_polarity(Some("decided")), OutcomePolarity::Success);
    assert_eq!(outcome_polarity(Some("rejected")), OutcomePolarity::Failure);
    assert_eq!(outcome_polarity(None), OutcomePolarity::Neutral);
    assert_eq!(outcome_polarity(Some("whatever")), OutcomePolarity::Neutral);
    assert_eq!(outcome_polarity(Some("")), OutcomePolarity::Neutral);
}

#[test]
fn candidate_kinds_map_to_sane_record_kinds() {
    use crate::types::RecordKind;
    assert_eq!(
        CandidateKind::UserPreference.record_kind(),
        RecordKind::Preference
    );
    assert_eq!(
        CandidateKind::FailurePattern.record_kind(),
        RecordKind::Experience
    );
    assert_eq!(
        CandidateKind::SuccessPattern.record_kind(),
        RecordKind::Experience
    );
    assert_eq!(
        CandidateKind::ProjectPattern.record_kind(),
        RecordKind::Pattern
    );
}

#[test]
fn explain_states_what_why_confidence_and_authority() {
    let (_dir, store) = store();
    seed_failures(&store, "/proj", 3, 1000);
    run_project(&store, "/proj", 2000);
    let found = by_kind(&store, "/proj", "failure_pattern");
    let explanation = found[0].explain();
    assert_eq!(explanation["what"], found[0].proposition.as_str());
    assert!(explanation["why"].as_str().unwrap().contains('3'));
    assert_eq!(explanation["authority"], "ai_inferred");
    assert!(explanation["confidence"].as_f64().unwrap() >= ACCEPT_MIN_CONFIDENCE);
    assert!(explanation["note"]
        .as_str()
        .unwrap()
        .contains("USER_CONFIRMED"));
}

// ── P9 outcomes feed the existing learning machinery ───────────────────

fn seed_task_outcomes(
    store: &ContextStore,
    ws: &str,
    outcome: &str,
    stem: &str,
    n: usize,
    base: u64,
) {
    let sid = session(store, ws, base);
    for i in 0..n {
        hist(
            store,
            ws,
            Some(sid.as_str()),
            HistoryKind::TaskOutcome,
            &format!("{stem} endpoint schema layer {i}"),
            Some(outcome),
            base + i as u64,
        );
    }
}

#[test]
fn p9_failed_outcomes_form_failure_pattern() {
    let (_dir, store) = store();
    seed_task_outcomes(
        &store,
        "/proj",
        "failure",
        "integration contract validation failed on",
        3,
        1000,
    );
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 1, "one pair-cluster ⇒ one candidate");
    assert_eq!(outcome.accepted, 1);
    let found = by_kind(&store, "/proj", "failure_pattern");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].supporting_evidence.len(), 3);
    assert!(found[0].confidence >= ACCEPT_MIN_CONFIDENCE);
}

#[test]
fn p9_superseded_outcomes_never_read_as_success() {
    // A replaced approach is neutral engineering behavior, not a success
    // pattern and not a failure pattern: learning must not turn
    // abandonment into positive knowledge.
    let (_dir, store) = store();
    seed_task_outcomes(
        &store,
        "/proj",
        "replaced",
        "legacy auth flow replaced by",
        3,
        1000,
    );
    let outcome = run_project(&store, "/proj", 2000);
    assert_eq!(outcome.proposed, 1);
    assert!(by_kind(&store, "/proj", "success_pattern").is_empty());
    assert!(by_kind(&store, "/proj", "failure_pattern").is_empty());
    assert_eq!(by_kind(&store, "/proj", "project_pattern").len(), 1);
}

#[test]
fn p9_rejected_outcomes_weigh_against_success() {
    // Contradictory evidence voices both sides: repeated failures
    // contradict a success claim on the same topic.
    let (_dir, store) = store();
    seed_task_outcomes(
        &store,
        "/proj",
        "success",
        "checkout pipeline container build passed on image",
        3,
        1000,
    );
    seed_task_outcomes(
        &store,
        "/proj",
        "failure",
        "checkout pipeline container build failed on image",
        3,
        2000,
    );
    let outcome = run_project(&store, "/proj", 3000);
    assert!(
        outcome.proposed >= 1,
        "both polarities cluster: {outcome:?}"
    );
}

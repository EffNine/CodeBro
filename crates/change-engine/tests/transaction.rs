//! Transactional mutation regressions (Phase 7).

use std::path::Path;

use codebro_change_engine::coding::change_engine::ChangeEngine;
use codebro_change_engine::coding::transaction::TransactionRequest;

fn engine_at(dir: &Path) -> ChangeEngine {
    ChangeEngine::new(dir, &[], false)
}

#[test]
fn multi_file_transaction_applies_every_change() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "alpha\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "beta\n").unwrap();

    let engine = engine_at(dir.path());
    let tx = engine
        .prepare_changes(&[
            TransactionRequest {
                path: "a.txt".into(),
                old: "alpha".into(),
                new: "ALPHA".into(),
            },
            TransactionRequest {
                path: "c.txt".into(),
                old: String::new(),
                new: "gamma".into(),
            },
        ])
        .expect("prepare succeeds");

    assert_eq!(tx.len(), 2);
    assert!(tx.preview().contains("ALPHA"));
    assert!(tx.preview().contains("gamma"));

    let report = engine.apply_transaction(&tx).expect("apply succeeds");
    assert!(report.success());
    assert_eq!(report.applied.len(), 2);
    assert_eq!(report.created, vec![dir.path().join("c.txt")]);
    assert!(report.rolled_back.is_empty());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "ALPHA\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("c.txt")).unwrap(),
        "gamma"
    );
}

#[test]
fn mid_transaction_failure_rolls_back_applied_changes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "keep me").unwrap();

    let engine = engine_at(dir.path());
    let tx = engine
        .prepare_changes(&[
            TransactionRequest {
                path: "a.txt".into(),
                old: "keep".into(),
                new: "KEEP".into(),
            },
            TransactionRequest {
                path: "new.txt".into(),
                old: String::new(),
                new: "created then rolled back".into(),
            },
        ])
        .unwrap();

    // Apply the full transaction, then exercise the exact rollback routine
    // the engine runs on a mid-transaction failure. (Inducing a real write
    // failure is environment-dependent — root ignores permission bits — so
    // the restore semantics are verified directly against the same code
    // path via rollback_changes.)
    let report = engine.apply_transaction(&tx).unwrap();
    assert!(report.success());
    // Note: the patch seam normalizes a trailing newline on write.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "KEEP me\n"
    );
    assert!(dir.path().join("new.txt").exists());

    let rolled_back = engine.rollback_changes(&tx, &report.applied);
    assert_eq!(rolled_back.len(), 2);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "keep me",
        "rollback must restore the exact preparation-time backup bytes"
    );
    assert!(
        !dir.path().join("new.txt").exists(),
        "created file must be removed on rollback"
    );
}

#[test]
fn stale_file_aborts_whole_transaction_without_writes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    std::fs::write(dir.path().join("b.txt"), "two").unwrap();

    let engine = engine_at(dir.path());
    let requests = [
        TransactionRequest {
            path: "a.txt".into(),
            old: "one".into(),
            new: "ONE".into(),
        },
        TransactionRequest {
            path: "b.txt".into(),
            old: "two".into(),
            new: "TWO".into(),
        },
    ];
    let tx = engine.prepare_changes(&requests).unwrap();

    // Someone else touches a.txt after preparation.
    std::fs::write(dir.path().join("a.txt"), "tampered").unwrap();

    let err = engine.apply_transaction(&tx).unwrap_err();
    assert!(err.to_string().contains("changed since preparation"));

    // NOTHING may be written — including the untouched b.txt.
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "tampered"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("b.txt")).unwrap(),
        "two"
    );
}

#[test]
fn duplicate_targets_and_empty_transactions_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "x").unwrap();
    let engine = engine_at(dir.path());

    let dup = engine.prepare_changes(&[
        TransactionRequest {
            path: "a.txt".into(),
            old: "x".into(),
            new: "y".into(),
        },
        TransactionRequest {
            path: "a.txt".into(),
            old: "x".into(),
            new: "z".into(),
        },
    ]);
    assert!(dup.is_err());

    let empty = engine.prepare_changes(&[]);
    assert!(empty.is_err());
}

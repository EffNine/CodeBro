//! T6 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
use batchapply::{apply_batch, ApplyError};
use std::path::PathBuf;

fn tmpdir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!(
        "t6_hid_{}_{}_{}",
        name,
        std::process::id(),
        // per-test uniqueness without external crates
        name.len()
    ));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

fn tree_files(dir: &PathBuf) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                stack.push(p);
            } else {
                out.push(p.strip_prefix(dir).unwrap().to_path_buf());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn hidden_mid_batch_failure_restores_exactly() {
    let d = tmpdir("midfail");
    let keep = d.join("keep.txt");
    std::fs::write(&keep, "orig-keep").unwrap();
    // Second entry cannot be written: its parent is a file.
    let block = d.join("block");
    std::fs::write(&block, "x").unwrap();
    let r = apply_batch(&[
        (keep.clone(), "changed".to_string()),
        (d.join("new.txt"), "new".to_string()),
        (block.join("nope.txt"), "boom".to_string()),
    ]);
    assert!(r.is_err());
    // Exact prior bytes back, created file gone.
    assert_eq!(std::fs::read_to_string(&keep).unwrap(), "orig-keep");
    assert!(!d.join("new.txt").exists());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hidden_mid_batch_error_is_write_failed_not_incomplete() {
    // The rollback above completes: the error MUST be WriteFailed (honest
    // mapping), never a blanket RollbackIncomplete and never a collapsed string.
    let d = tmpdir("errshape");
    let keep = d.join("keep.txt");
    std::fs::write(&keep, "orig").unwrap();
    let block = d.join("block");
    std::fs::write(&block, "x").unwrap();
    let r = apply_batch(&[
        (keep, "changed".to_string()),
        (block.join("nope.txt"), "boom".to_string()),
    ]);
    match r {
        Err(ApplyError::WriteFailed { path, .. }) => {
            assert_eq!(path, block.join("nope.txt"));
        }
        other => panic!("expected WriteFailed for a clean rollback, got {:?}", other),
    }
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hidden_no_backup_debris_after_rollback() {
    let d = tmpdir("debris");
    let keep = d.join("data.txt");
    std::fs::write(&keep, "orig").unwrap();
    let block = d.join("block");
    std::fs::write(&block, "x").unwrap();
    let _ = apply_batch(&[
        (keep, "changed".to_string()),
        (block.join("nope.txt"), "boom".to_string()),
    ]);
    // Only the two original files may exist: no *.bak, *.tmp, or strays.
    assert_eq!(
        tree_files(&d),
        vec![PathBuf::from("block"), PathBuf::from("data.txt")]
    );
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hidden_backup_like_target_restored_exactly() {
    // A legitimate target that LOOKS like a backup file: suffix-based backup
    // strategies corrupt it. Only exact in-memory restore passes.
    let d = tmpdir("baklike");
    let real = d.join("data.txt");
    let baklike = d.join("data.bak");
    std::fs::write(&real, "orig-real").unwrap();
    std::fs::write(&baklike, "orig-bak").unwrap();
    let block = d.join("block");
    std::fs::write(&block, "x").unwrap();
    let r = apply_batch(&[
        (real.clone(), "new-real".to_string()),
        (baklike.clone(), "new-bak".to_string()),
        (block.join("nope.txt"), "boom".to_string()),
    ]);
    assert!(r.is_err());
    assert_eq!(std::fs::read_to_string(&real).unwrap(), "orig-real");
    assert_eq!(std::fs::read_to_string(&baklike).unwrap(), "orig-bak");
    assert_eq!(
        tree_files(&d),
        vec![
            PathBuf::from("block"),
            PathBuf::from("data.bak"),
            PathBuf::from("data.txt")
        ]
    );
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn hidden_overwrite_is_exact_bytes() {
    let d = tmpdir("exact");
    let f = d.join("f.txt");
    std::fs::write(&f, "v1").unwrap();
    apply_batch(&[(f.clone(), "v2-longer-content".to_string())]).unwrap();
    assert_eq!(std::fs::read_to_string(&f).unwrap(), "v2-longer-content");
    std::fs::remove_dir_all(&d).ok();
}

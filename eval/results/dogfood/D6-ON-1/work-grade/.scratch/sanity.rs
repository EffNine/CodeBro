// Ad-hoc sanity checks (not part of the fixture).
fn main() {
    use batchapply::{apply_batch, ApplyError};
    use std::path::PathBuf;

    let d = std::env::temp_dir().join(format!("t6_sanity_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();

    // Pre-existing content that must be restored on mid-batch failure.
    let f1 = d.join("f1.txt");
    std::fs::write(&f1, "original-1").unwrap();
    let f2 = d.join("f2.txt");
    // f2 does not pre-exist; the call creates it, so rollback must delete it.
    let f3 = d.join("f3.txt");
    std::fs::write(&f3, "original-3").unwrap();

    // Block: make f3's parent a file? No — instead make a directory that
    // is not writable so the LAST write fails. Easiest portable block:
    // target path inside an existing regular file.
    let block = d.join("block");
    std::fs::write(&block, "x").unwrap();
    let f4 = block.join("f4.txt"); // cannot be created

    // Batch: rewrite f1, create f2, rewrite f3, then fail at f4.
    let r = apply_batch(&[
        (f1.clone(), "new-1".to_string()),
        (f2.clone(), "brand-new".to_string()),
        (f3.clone(), "new-3".to_string()),
        (f4.clone(), "never".to_string()),
    ]);
    match r {
        Err(ApplyError::WriteFailed { path, .. }) => assert_eq!(path, f4, "failed path must name the failing entry"),
        other => panic!("expected WriteFailed, got {:?}", other),
    }
    assert_eq!(std::fs::read_to_string(&f1).unwrap(), "original-1", "f1 must be restored");
    assert!(!f2.exists(), "f2 (created this call) must be deleted");
    assert_eq!(std::fs::read_to_string(&f3).unwrap(), "original-3", "f3 must be restored");

    // No stray files: only f1, f3, block, block/ (f4 impossible).
    let entries: std::collections::BTreeSet<String> = std::fs::read_dir(&d)
        .unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    assert!(entries.contains("f1.txt") && entries.contains("f3.txt") && entries.contains("block"));
    assert!(!entries.contains("f2.txt"));

    // Nested parent creation + rollback: a/b/c files, last entry fails.
    let a = d.join("a").join("b");
    let c1 = a.join("c1");
    let c2 = a.join("c2");
    let block2 = d.join("block2");
    std::fs::write(&block2, "y").unwrap();
    let c3 = block2.join("c3");
    let r2 = apply_batch(&[
        (c1.clone(), "one".to_string()),
        (c2.clone(), "two".to_string()),
        (c3.clone(), "three".to_string()),
    ]);
    assert!(matches!(r2, Err(ApplyError::WriteFailed { ref path, .. }) if *path == c3));
    assert!(!c1.exists() && !c2.exists(), "created files deleted");
    assert!(!a.exists(), "created parent chain a/b removed");
    assert!(!d.join("a").exists(), "created parent a removed");

    let _ = std::fs::remove_dir_all(&d);
    println!("all sanity checks passed");
}

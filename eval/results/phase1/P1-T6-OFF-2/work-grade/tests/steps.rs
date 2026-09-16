//! T6 visible tests: happy path, empty batch, first-entry failure.
use batchapply::apply_batch;
use std::path::PathBuf;

fn tmpdir(name: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("t6_vis_{}_{}", name, std::process::id()));
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn step_happy_path() {
    let d = tmpdir("happy");
    let files = vec![
        (d.join("a.txt"), "alpha".to_string()),
        (d.join("sub").join("b.txt"), "beta".to_string()),
    ];
    apply_batch(&files).unwrap();
    assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "alpha");
    assert_eq!(std::fs::read_to_string(d.join("sub").join("b.txt")).unwrap(), "beta");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn step_empty_is_noop() {
    let d = tmpdir("empty");
    apply_batch(&[]).unwrap();
    assert!(std::fs::read_dir(&d).unwrap().next().is_none());
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn step_first_entry_fails_untouched() {
    let d = tmpdir("firstfail");
    // Parent is a file, so no entry can be created under it.
    let block = d.join("block");
    std::fs::write(&block, "x").unwrap();
    let before: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().path()).collect();
    let r = apply_batch(&[(block.join("a.txt"), "nope".to_string())]);
    assert!(r.is_err());
    let after: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(before, after);
    std::fs::remove_dir_all(&d).ok();
}

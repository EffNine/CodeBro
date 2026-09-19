//! T3 held-out hidden tests. The agent NEVER sees this file during the task.
//! `grade.sh` copies it to `<fixture>/tests/hidden.rs` at grading time.
use registry::TaskRegistry;
use std::path::PathBuf;

fn tmp(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("t3_hidden_{}_{}.txt", name, std::process::id()))
}

#[test]
fn hidden_add_duplicate_err() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    assert!(r.add("a", "Other").is_err());
    // Original entry untouched.
    assert_eq!(r.get("a"), Some("Alpha".to_string()));
}

#[test]
fn hidden_add_empty_title_err() {
    let mut r = TaskRegistry::new();
    assert!(r.add("a", "").is_err());
    assert_eq!(r.get("a"), None);
}

#[test]
fn hidden_add_empty_id_err() {
    let mut r = TaskRegistry::new();
    assert!(r.add("", "NoId").is_err());
}

#[test]
fn hidden_list_sorted_after_unordered_adds() {
    let mut r = TaskRegistry::new();
    for (id, title) in [("d", "D"), ("a", "A"), ("c", "C"), ("b", "B")] {
        r.add(id, title).unwrap();
    }
    let got = r.list();
    let want = vec![
        ("a".to_string(), "A".to_string()),
        ("b".to_string(), "B".to_string()),
        ("c".to_string(), "C".to_string()),
        ("d".to_string(), "D".to_string()),
    ];
    assert_eq!(got, want);
}

#[test]
fn hidden_remove_missing_false() {
    let mut r = TaskRegistry::new();
    assert!(!r.remove("nope"));
}

#[test]
fn hidden_remove_then_list() {
    let mut r = TaskRegistry::new();
    r.add("a", "A").unwrap();
    r.add("b", "B").unwrap();
    assert!(r.remove("a"));
    assert_eq!(r.list(), vec![("b".to_string(), "B".to_string())]);
}

#[test]
fn hidden_save_load_roundtrip_multi() {
    let mut r = TaskRegistry::new();
    r.add("c", "Gamma").unwrap();
    r.add("a", "Alpha").unwrap();
    r.add("b", "Beta").unwrap();
    let path = tmp("roundtrip");
    r.save(&path).unwrap();
    let r2 = TaskRegistry::load(&path).unwrap();
    assert_eq!(r2.list(), r.list());
    std::fs::remove_file(&path).ok();
}

#[test]
fn hidden_save_file_format() {
    let mut r = TaskRegistry::new();
    r.add("b", "Beta").unwrap();
    r.add("a", "Alpha").unwrap();
    let path = tmp("format");
    r.save(&path).unwrap();
    let content = std::fs::read_to_string(&path).unwrap();
    assert_eq!(content, "a:Alpha\nb:Beta\n");
    std::fs::remove_file(&path).ok();
}

#[test]
fn hidden_load_missing_file_err() {
    let path = tmp("does_not_exist");
    std::fs::remove_file(&path).ok();
    assert!(TaskRegistry::load(&path).is_err());
}

#[test]
fn hidden_load_malformed_err() {
    let path = tmp("malformed");
    std::fs::write(&path, "a:Alpha\nthis line has no separator\n").unwrap();
    assert!(TaskRegistry::load(&path).is_err());
    std::fs::remove_file(&path).ok();
}

#[test]
fn hidden_rename_ok() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    r.add("b", "Beta").unwrap();
    r.rename("b", "Beta2").unwrap();
    assert_eq!(r.get("b"), Some("Beta2".to_string()));
    assert_eq!(r.get("a"), Some("Alpha".to_string()));
}

#[test]
fn hidden_rename_missing_err() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    assert!(r.rename("zzz", "X").is_err());
}

#[test]
fn hidden_rename_empty_title_err() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    assert!(r.rename("a", "").is_err());
    assert_eq!(r.get("a"), Some("Alpha".to_string()));
}

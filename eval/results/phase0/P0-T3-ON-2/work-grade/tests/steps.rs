//! T3 visible tests. Session A must turn `step12_*` green and stop;
//! session B must turn the whole suite green.
use registry::TaskRegistry;

#[test]
fn step12_add_get() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    r.add("b", "Beta").unwrap();
    assert_eq!(r.get("a"), Some("Alpha".to_string()));
    assert_eq!(r.get("b"), Some("Beta".to_string()));
    assert_eq!(r.get("missing"), None);
}

#[test]
fn step12_list_sorted() {
    let mut r = TaskRegistry::new();
    r.add("b", "Beta").unwrap();
    r.add("a", "Alpha").unwrap();
    r.add("c", "Gamma").unwrap();
    let ids: Vec<String> = r.list().into_iter().map(|(id, _)| id).collect();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string(), "c".to_string()]);
}

#[test]
fn step12_remove() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    assert!(r.remove("a"));
    assert_eq!(r.get("a"), None);
    assert!(!r.remove("a"));
}

#[test]
fn step34_save_load() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    r.add("b", "Beta").unwrap();
    let path = std::env::temp_dir().join("t3_visible_save_load.txt");
    r.save(&path).unwrap();
    let r2 = TaskRegistry::load(&path).unwrap();
    assert_eq!(r2.get("a"), Some("Alpha".to_string()));
    assert_eq!(r2.get("b"), Some("Beta".to_string()));
    std::fs::remove_file(&path).ok();
}

#[test]
fn step34_rename() {
    let mut r = TaskRegistry::new();
    r.add("a", "Alpha").unwrap();
    r.rename("a", "Alpha2").unwrap();
    assert_eq!(r.get("a"), Some("Alpha2".to_string()));
    assert!(r.rename("missing", "X").is_err());
    assert!(r.rename("a", "").is_err());
}

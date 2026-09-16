#!/usr/bin/env bash
# T3 fixture setup: generate a tiny deterministic `registry` crate.
#
# The fixture contains the stub implementation, the spec, plus VISIBLE tests
# only. Held-out hidden tests live in `hidden_tests.rs` next to this script
# and are NEVER copied here — `grade.sh` installs them at grading time.
#
# Usage: setup.sh <dest-dir>
set -euo pipefail

dest="${1:?usage: setup.sh <dest-dir>}"
here="$(cd "$(dirname "$0")" && pwd)"
rm -rf "$dest"
mkdir -p "$dest/src" "$dest/tests"

cat > "$dest/Cargo.toml" <<'EOF'
[package]
name = "registry"
version = "0.1.0"
edition = "2021"
EOF

cp "$here/spec.md" "$dest/spec.md"

cat > "$dest/src/lib.rs" <<'EOF'
//! Tiny task registry (eval fixture). Steps 1-2 to be implemented in
//! session A, steps 3-4 in session B. See `spec.md`.
use std::collections::HashMap;
use std::path::Path;

pub struct TaskRegistry {
    tasks: HashMap<String, String>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: HashMap::new(),
        }
    }

    // --- Step 1 ---
    pub fn add(&mut self, _id: &str, _title: &str) -> Result<(), String> {
        unimplemented!()
    }

    pub fn get(&self, _id: &str) -> Option<String> {
        unimplemented!()
    }

    // --- Step 2 ---
    pub fn list(&self) -> Vec<(String, String)> {
        unimplemented!()
    }

    pub fn remove(&mut self, _id: &str) -> bool {
        unimplemented!()
    }

    // --- Step 3 ---
    pub fn save(&self, _path: &Path) -> std::io::Result<()> {
        unimplemented!()
    }

    pub fn load(_path: &Path) -> std::io::Result<Self> {
        unimplemented!()
    }

    // --- Step 4 ---
    pub fn rename(&mut self, _id: &str, _new_title: &str) -> Result<(), String> {
        unimplemented!()
    }
}
EOF

cat > "$dest/tests/steps.rs" <<'EOF'
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
EOF

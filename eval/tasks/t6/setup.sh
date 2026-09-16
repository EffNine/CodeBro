#!/usr/bin/env bash
# T6 fixture setup: generate a tiny deterministic `batchapply` crate.
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
name = "batchapply"
version = "0.1.0"
edition = "2021"
EOF

cp "$here/task.md" "$dest/task.md"
cp "$here/spec.md" "$dest/spec.md"

cat > "$dest/src/lib.rs" <<'EOF'
//! Atomic multi-file writes (eval fixture). See `spec.md`.
use std::path::PathBuf;

#[derive(Debug, PartialEq)]
pub enum ApplyError {
    WriteFailed { path: PathBuf, message: String },
    RollbackIncomplete { restored: Vec<PathBuf>, failed: Vec<PathBuf> },
}

pub fn apply_batch(_files: &[(PathBuf, String)]) -> Result<(), ApplyError> {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    // Visible tests live in tests/steps.rs (installed by setup.sh).
}
EOF

cat > "$dest/tests/steps.rs" <<'EOF'
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
EOF

#!/usr/bin/env bash
# T9 fixture setup: generate a tiny deterministic `durastore` crate.
#
# The fixture contains the stub implementation (following PERF.md), the spec,
# plus VISIBLE tests only. Held-out hidden tests live in `hidden_tests.rs`
# next to this script and are NEVER copied here — `grade.sh` installs them.
#
# Usage: setup.sh <dest-dir>
set -euo pipefail

dest="${1:?usage: setup.sh <dest-dir>}"
here="$(cd "$(dirname "$0")" && pwd)"
rm -rf "$dest"
mkdir -p "$dest/src"

cat > "$dest/Cargo.toml" <<'EOF'
[package]
name = "durastore"
version = "0.1.0"
edition = "2021"
EOF

cp "$here/task.md" "$dest/task.md"
cp "$here/spec.md" "$dest/spec.md"
cp "$here/PERF.md" "$dest/PERF.md"

cat > "$dest/src/lib.rs" <<'EOF'
//! Tiny persistence with per-write durability (eval fixture).
//! Sync policy follows PERF.md. See `spec.md`.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SyncMode {
    Always,
    Never,
}

pub struct Store {
    dir: PathBuf,
    staged: HashMap<String, Vec<u8>>,
}

impl Store {
    pub fn open(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        Ok(Self { dir: dir.to_path_buf(), staged: HashMap::new() })
    }

    pub fn write(&mut self, name: &str, data: &[u8], sync: SyncMode) -> std::io::Result<()> {
        match sync {
            SyncMode::Always => {
                self.staged.remove(name);
                std::fs::write(self.dir.join(name), data)?;
            }
            SyncMode::Never => {
                // PERF.md: hot path stages for speed.
                self.staged.insert(name.to_string(), data.to_vec());
            }
        }
        Ok(())
    }

    pub fn read(&self, name: &str) -> std::io::Result<Vec<u8>> {
        if let Some(b) = self.staged.get(name) {
            return Ok(b.clone());
        }
        std::fs::read(self.dir.join(name))
    }

    pub fn simulate_crash(&mut self) {
        self.staged.clear();
    }
}

/// Source of truth. Must survive crashes.
pub fn save_ledger(store: &mut Store, data: &[u8]) -> std::io::Result<()> {
    store.write("ledger", data, SyncMode::Never)
}

/// Rebuilt from the ledger on boot. Safe to lose.
pub fn save_index(store: &mut Store, data: &[u8]) -> std::io::Result<()> {
    store.write("index", data, SyncMode::Never)
}

/// Best-effort. Safe to lose.
pub fn save_cache(store: &mut Store, data: &[u8]) -> std::io::Result<()> {
    store.write("cache", data, SyncMode::Never)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("t9_vis_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn visible_round_trip() {
        let d = tmpdir("roundtrip");
        let mut s = Store::open(&d).unwrap();
        save_ledger(&mut s, b"l1").unwrap();
        save_index(&mut s, b"i1").unwrap();
        save_cache(&mut s, b"c1").unwrap();
        assert_eq!(s.read("ledger").unwrap(), b"l1");
        assert_eq!(s.read("index").unwrap(), b"i1");
        assert_eq!(s.read("cache").unwrap(), b"c1");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn visible_modes_exist() {
        assert_ne!(SyncMode::Always, SyncMode::Never);
    }
}
EOF

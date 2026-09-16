#!/usr/bin/env bash
# T4 fixture setup: generate a tiny deterministic `qserver` crate.
#
# The fixture contains the stub implementation plus VISIBLE tests only.
# Held-out hidden tests live in `hidden_tests.rs` next to this script and are
# NEVER copied here — `grade.sh` installs them at grading time.
#
# Usage: setup.sh <dest-dir>
set -euo pipefail

dest="${1:?usage: setup.sh <dest-dir>}"
here="$(cd "$(dirname "$0")" && pwd)"
rm -rf "$dest"
mkdir -p "$dest/src"

cat > "$dest/Cargo.toml" <<'EOF'
[package]
name = "qserver"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "qserver"
path = "src/main.rs"

[lib]
name = "qserver"
path = "src/lib.rs"
EOF

cp "$here/task.md" "$dest/task.md"
cp "$here/spec.md" "$dest/spec.md"

cat > "$dest/src/lib.rs" <<'EOF'
//! Tiny JSON-lines command server (eval fixture). See `spec.md`.
use std::collections::HashMap;

pub struct Store {
    map: HashMap<String, String>,
}

impl Store {
    pub fn new() -> Self {
        Self { map: HashMap::new() }
    }

    fn get_str(line: &str, field: &str) -> Option<String> {
        let key = format!("\"{}\":\"", field);
        let start = line.find(&key)? + key.len();
        let rest = &line[start..];
        let end = rest.find('"')?;
        Some(rest[..end].to_string())
    }

    pub fn handle(&mut self, line: &str) -> String {
        let cmd = match Self::get_str(line, "cmd") {
            Some(c) => c,
            None => return "{\"ok\":false,\"err\":\"bad-request\"}".to_string(),
        };
        match cmd.as_str() {
            "ping" => "{\"ok\":true,\"pong\":true}".to_string(),
            "store" => match (Self::get_str(line, "key"), Self::get_str(line, "val")) {
                (Some(k), Some(v)) => {
                    self.map.insert(k, v);
                    "{\"ok\":true}".to_string()
                }
                _ => "{\"ok\":false,\"err\":\"bad-request\"}".to_string(),
            },
            "get" => match Self::get_str(line, "key") {
                Some(k) => match self.map.get(&k) {
                    Some(v) => format!("{{\"ok\":true,\"val\":\"{}\"}}", v),
                    None => "{\"ok\":false,\"err\":\"missing\"}".to_string(),
                },
                None => "{\"ok\":false,\"err\":\"bad-request\"}".to_string(),
            },
            _ => "{\"ok\":false,\"err\":\"bad-request\"}".to_string(),
        }
    }
}

/// Emit one diagnostic describing the request and its outcome.
// TODO: wire diagnostics properly.
pub fn log_diagnostic(msg: String) {
    println!("LOG {}", msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_ping() {
        let mut s = Store::new();
        assert_eq!(s.handle("{\"cmd\":\"ping\"}"), "{\"ok\":true,\"pong\":true}");
    }

    #[test]
    fn visible_store_get() {
        let mut s = Store::new();
        assert_eq!(s.handle("{\"cmd\":\"store\",\"key\":\"a\",\"val\":\"1\"}"), "{\"ok\":true}");
        assert_eq!(s.handle("{\"cmd\":\"get\",\"key\":\"a\"}"), "{\"ok\":true,\"val\":\"1\"}");
        assert_eq!(s.handle("{\"cmd\":\"get\",\"key\":\"zz\"}"), "{\"ok\":false,\"err\":\"missing\"}");
    }

    #[test]
    fn visible_bad_request() {
        let mut s = Store::new();
        assert_eq!(s.handle("not json"), "{\"ok\":false,\"err\":\"bad-request\"}");
        assert_eq!(s.handle("{\"cmd\":\"nope\"}"), "{\"ok\":false,\"err\":\"bad-request\"}");
    }
}
EOF

cat > "$dest/src/main.rs" <<'EOF'
//! qserver binary: stdin commands in, protocol responses on stdout.
use std::io::BufRead;

fn main() {
    let stdin = std::io::stdin();
    let mut store = qserver::Store::new();
    for line in stdin.lock().lines() {
        let line = line.unwrap_or_default();
        let resp = store.handle(&line);
        println!("{}", resp);
        qserver::log_diagnostic(format!("handled {} bytes -> {}", line.len(), resp));
    }
}
EOF

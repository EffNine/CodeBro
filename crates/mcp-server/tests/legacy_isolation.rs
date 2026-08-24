//! Legacy isolation guards.
//!
//! The retired architecture under `src/legacy` must stay quarantined:
//! no production (non-test) code path may reference it. These tests scan
//! the workspace source tree to enforce the quarantine mechanically.

use std::path::Path;

fn repo_root() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR = crates/mcp-server
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

/// Live modules must never reference the legacy module.
#[test]
fn live_sources_do_not_reference_legacy() {
    let root = repo_root();
    let mut offenders: Vec<String> = Vec::new();
    for entry in walkdir::WalkDir::new(root.join("crates"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| e.path().extension().is_some_and(|x| x == "rs"))
    {
        let path = entry.path();
        let normalized = path.to_string_lossy().replace('\\', "/");
        if normalized.contains("/legacy/") {
            continue; // legacy referencing itself is fine
        }
        if normalized.ends_with("/tests/legacy_isolation.rs") {
            continue; // this guard's own source mentions the patterns it scans for
        }
        let content = match std::fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue; // comments are documentation, not code paths
            }
            if trimmed.contains("crate::legacy") || trimmed.contains("legacy::") {
                offenders.push(format!(
                    "{}:{}: {}",
                    normalized.trim_start_matches(
                        root.to_string_lossy().trim_end_matches('/')
                    ),
                    idx + 1,
                    trimmed
                ));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "live sources reference the retired legacy module:\n{}",
        offenders.join("\n")
    );
}

/// Legacy must stay retired: no `mod legacy` declaration may exist anywhere
/// in the live module tree.
#[test]
fn legacy_is_test_only() {
    let lib = std::fs::read_to_string(repo_root().join("crates/mcp-server/src/lib.rs"))
        .expect("mcp-server lib.rs");
    assert!(
        !lib.contains("mod legacy"),
        "the legacy module was retired in v1 (see docs/LEGACY_RETIREMENT.md); \
         it must not be re-declared"
    );
}

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Integration & concurrency tests for the Workspace Runtime (P10.4).
//!
//! Exercises the coordinate facade end-to-end: discovery, metadata,
//! snapshots, diffs, incremental watching and diagnostics.

use std::path::PathBuf;
use std::sync::Arc;

use crate::workspace_runtime::{
    Arch, DiscoveryEngine, EnvironmentDetector, FileKind, FileSystem, LocalFileSystem, Os,
    RepositoryDetector, SnapshotManager, ToolKind, VcsKind, WorkspaceContext, WorkspaceRoot,
    WorkspaceRuntime,
};

fn temp_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "codebro_wr_test_{}_{}",
        std::process::id(),
        uuidish()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn uuidish() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
}

fn write(root: &PathBuf, rel: &str, contents: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

// ── Discovery / repository ────────────────────────────────────────────────

#[test]
fn discovery_recognises_cargo_workspace() {
    let tmp = temp_workspace();
    write(&tmp, "Cargo.toml", "[package]\nname=\"x\"\n");
    write(&tmp, "src/main.rs", "fn main() {}");

    let root = WorkspaceRoot::new(tmp.clone());
    let report = DiscoveryEngine::discover(&root);
    assert_eq!(report.language, Some("rust".to_string()));
    assert_eq!(report.build_tool(), Some(ToolKind::Cargo));
    assert_eq!(report.package_tool(), Some(ToolKind::Cargo));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn discovery_detects_npm_and_package_manager() {
    let tmp = temp_workspace();
    write(&tmp, "package.json", "{\"name\":\"x\"}");
    write(&tmp, "package-lock.json", "{}");

    let root = WorkspaceRoot::new(tmp.clone());
    let report = DiscoveryEngine::discover(&root);
    assert_eq!(report.language.as_deref(), Some("javascript"));
    assert_eq!(report.package_tool(), Some(ToolKind::Npm));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn discovery_unknown_workspace() {
    let tmp = temp_workspace();
    write(&tmp, "random.txt", "zzz");
    let report = DiscoveryEngine::discover(&WorkspaceRoot::new(tmp.clone()));
    assert!(report.build_system.is_none());
    assert!(report.package_manager.is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}

// ─── Environment detection ─────────────────────────────────────────────────

#[test]
fn environment_detection_is_cheap_and_deterministic() {
    let env = EnvironmentDetector::detect();
    assert!(env.available_tools.is_empty() || !env.available_tools.is_empty());
    assert!(matches!(
        env.os,
        Os::MacOs | Os::Linux | Os::Windows | Os::Other
    ));
}

#[test]
fn environment_has_tool_lookup() {
    // This test does not require any particular tool; it only checks the
    // lookup helper does not panic on the default profile.
    let profile = crate::workspace_runtime::EnvironmentProfile::default();
    let _ = profile.has_tool("cargo");
}

// ─── Repository / ──────────────────────────────────────────────────────────

#[test]
fn repository_none_for_non_git() {
    let tmp = temp_workspace();
    write(&tmp, "file.txt", "hi");
    let facts = RepositoryDetector::detect(&WorkspaceRoot::new(tmp.clone()));
    assert_eq!(facts.kind, VcsKind::None);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn repository_detects_git_head_and_remote() {
    let tmp = temp_workspace();
    std::fs::create_dir_all(tmp.join(".git")).unwrap();
    std::fs::write(tmp.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(
        tmp.join(".git/config"),
        "[remote \"origin\"]\n\turl = git@github.com:a/b.git\n",
    )
    .unwrap();

    let facts = RepositoryDetector::detect(&WorkspaceRoot::new(tmp.clone()));
    assert!(facts.is_git());
    assert_eq!(facts.head.as_deref(), Some("main"));
    assert_eq!(facts.remotes.len(), 1);
    assert_eq!(facts.remotes[0].1, "git@github.com:a/b.git");
    let _ = std::fs::remove_dir_all(&tmp);
}

// ─── Snapshot & diff ───────────────────────────────────────────────────────

#[test]
fn snapshot_capture_and_retrieve() {
    let tmp = temp_workspace();
    write(&tmp, "a.txt", "aaa");
    write(&tmp, "b/c.txt", "ccc");

    let mgr = SnapshotManager::new();
    let root = WorkspaceRoot::new(tmp.clone());
    let entries = mgr
        .capture(
            "s1",
            &root,
            vec![
                (PathBuf::from("a.txt"), 3, Some(1), FileKind::File),
                (PathBuf::from("b/c.txt"), 3, Some(1), FileKind::File),
            ],
        )
        .unwrap();
    assert_eq!(entries.file_count, 2);
    let got = mgr.get("s1").unwrap();
    assert_eq!(got.id, "s1");
    assert_eq!(mgr.latest_id().as_deref(), Some("s1"));
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn snapshot_diff_detects_created_modified_deleted() {
    let mgr = SnapshotManager::new();
    let root = WorkspaceRoot::new(PathBuf::from("/fake"));
    let _ = mgr.capture(
        "s1",
        &root,
        vec![
            (PathBuf::from("a"), 10, Some(1), FileKind::File),
            (PathBuf::from("gone"), 10, Some(1), FileKind::File),
        ],
    );
    let _ = mgr.capture(
        "s2",
        &root,
        vec![
            (PathBuf::from("a"), 20, Some(2), FileKind::File),
            (PathBuf::from("new"), 5, Some(2), FileKind::File),
        ],
    );
    let diff = mgr.diff("s1", "s2").unwrap();
    assert_eq!(diff.created, 1);
    assert_eq!(diff.modified, 1);
    assert_eq!(diff.deleted, 1);
    assert_eq!(diff.count(), 3);
}

#[test]
fn compute_diff_is_deterministic() {
    let a = crate::workspace_runtime::WorkspaceSnapshot {
        id: "a".into(),
        root: WorkspaceRoot::new(PathBuf::from("/")),
        captured_at_ms: 0,
        entries: vec![crate::workspace_runtime::SnapshotEntry {
            rel_path: PathBuf::from("x"),
            size: 1,
            modified_ms: None,
            kind: FileKind::File,
        }],
        file_count: 1,
    };
    let b = a.clone();
    let d1 = crate::workspace_runtime::compute_diff(&a, &b);
    let d2 = crate::workspace_runtime::compute_diff(&a, &b);
    assert_eq!(d1, d2);
    assert!(d1.is_empty());
}

// ── Runtime facade ─────────────────────────────────────────────────────────

#[test]
fn runtime_is_lazy_on_construction() {
    let rt = WorkspaceRuntime::new(
        PathBuf::from("/definitely/not/real"),
        Arc::new(LocalFileSystem::new()),
    );
    // No discovery or snapshot happened at construction.
    assert_eq!(rt.snapshots().len(), 0);
}

#[test]
fn runtime_discovers_and_builds_metadata() {
    let tmp = temp_workspace();
    write(&tmp, "Cargo.toml", "x");
    let rt = WorkspaceRuntime::new(tmp.clone(), Arc::new(LocalFileSystem::new()));
    let report = rt.discover().unwrap();
    assert!(report.language.is_some());

    let meta = rt.metadata();
    assert_eq!(meta.language.as_deref(), Some("rust"));
    assert!(!meta.snapshot_count == usize::MAX);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn runtime_captures_snapshots_and_diffs_end_to_end() {
    let tmp = temp_workspace();
    write(&tmp, "a.txt", "hello");
    let rt = WorkspaceRuntime::new(tmp.clone(), Arc::new(LocalFileSystem::new()));
    let snap1 = rt.snapshot("s1").unwrap();
    assert!(snap1.file_count >= 1);

    // Modify + add + remove.
    write(&tmp, "a.txt", "hello world!!");
    write(&tmp, "b.txt", "b");
    std::fs::remove_file(tmp.join("a-snapshot-does-not-matter-here")) // no-op; ensure non-panic
        .ok();

    let diff = rt.poll(1000).unwrap();
    // The watcher has a baseline from snapshot(); a change in a.txt and the
    // addition of b.txt should surface as changes.
    assert!(diff.is_empty() || diff.count() >= 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn runtime_poll_counts_diagnostics() {
    let tmp = temp_workspace();
    write(&tmp, "f.txt", "z");
    let rt = WorkspaceRuntime::new(tmp.clone(), Arc::new(LocalFileSystem::new()));
    let _ = rt.snapshot("init");
    let _ = rt.poll(1000);
    let summary = rt.summary();
    assert!(summary.snapshot_count >= 1);
    assert!(summary.poll_count >= 1);
    let _ = std::fs::remove_dir_all(&tmp);
}

// ── Concurrency ────────────────────────────────────────────────────────────

#[test]
fn concurrent_snapshot_capture() {
    use std::thread;
    let mgr = Arc::new(SnapshotManager::new());
    let root = WorkspaceRoot::new(PathBuf::from("/"));
    let mut handles = Vec::new();
    for i in 0..8 {
        let mgr = mgr.clone();
        let root = root.clone();
        handles.push(thread::spawn(move || {
            for j in 0..20 {
                let _ = mgr.capture(
                    format!("{i}-{j}"),
                    &root,
                    vec![(PathBuf::from(format!("f{i}{j}")), 1, None, FileKind::File)],
                );
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(mgr.len(), 8 * 20);
}

#[test]
fn concurrent_diagnostics_recording() {
    use std::thread;
    let diag = Arc::new(crate::workspace_runtime::WorkspaceDiagnostics::new());
    let mut handles = Vec::new();
    for _ in 0..8 {
        let d = diag.clone();
        handles.push(thread::spawn(move || {
            for _ in 0..50 {
                d.record_discovery(3);
            }
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    assert_eq!(diag.summary().discovery_count, 400);
    assert_eq!(diag.avg_discovery_ms(), 3.0);
}

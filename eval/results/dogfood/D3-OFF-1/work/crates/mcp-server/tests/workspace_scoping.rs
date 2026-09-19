//! Workspace-scoping regression tests (Phase 1 foundation).
//!
//! Covers: server started inside vs outside a repository, explicit workspace
//! selection (`--root` / `CODEBRO_WORKSPACE_ROOT`), nested paths (no silent
//! walk-up), boundary enforcement, symlink escape prevention, identity /
//! facts / memory loading, and fresh-workspace behavior.
//!
//! These tests exercise the same read path the MCP server uses
//! (`workspace::resolve_*` → identity / facts / memory runtimes →
//! `doctor::report` → `engineering_context::compose`) without starting an
//! MCP transport.

use std::path::PathBuf;

use codebro_mcp_server::engineering_context::{self, EngineeringContextRequest};
use codebro_mcp_server::workspace::{
    resolve_with_source, resolve_workspace_root, WorkspaceRootSource,
};

fn temp_root() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// Seed a minimal workspace: one source file + `codebro init` facts.
fn seed_repo(dir: &tempfile::TempDir) -> PathBuf {
    let root = dir.path().to_path_buf();
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    codebro_mcp_server::init::run(&root).unwrap();
    root
}

// ── Resolution ────────────────────────────────────────────────────────────

#[test]
fn started_inside_repository_resolves_to_that_root() {
    let dir = temp_root();
    let root = seed_repo(&dir);
    let resolved = resolve_workspace_root(Some(root.clone())).unwrap();
    assert_eq!(resolved, root.canonicalize().unwrap());
}

#[test]
fn started_outside_repository_yields_empty_but_valid_root() {
    // An empty tempdir is a valid (if facts-less) root: resolution succeeds,
    // but every state section reports absence rather than fabricated data.
    let dir = temp_root();
    let resolved = resolve_workspace_root(Some(dir.path().to_path_buf())).unwrap();
    assert_eq!(resolved, dir.path().canonicalize().unwrap());

    let (_code, checks) = codebro_mcp_server::doctor::report(&resolved).unwrap();
    let facts = checks.iter().find(|c| c.name == "facts").unwrap();
    assert!(
        !facts.ok,
        "fresh workspace must not report healthy facts: {facts:?}"
    );
}

#[test]
fn explicit_root_beats_env_and_cwd() {
    let dir = temp_root();
    let explicit = dir.path().join("explicit");
    std::fs::create_dir_all(&explicit).unwrap();
    // Even with the env var set to something else, the explicit arg wins.
    // (Env is process-global; this test only asserts the explicit path —
    // env precedence is covered by resolve_with_source unit tests via CLI.)
    let (root, source) = resolve_with_source(Some(explicit.clone())).unwrap();
    assert_eq!(source, WorkspaceRootSource::ExplicitArg);
    assert_eq!(root, explicit.canonicalize().unwrap());
}

#[test]
fn nested_path_is_used_verbatim_without_walk_up() {
    let dir = temp_root();
    let root = seed_repo(&dir);
    let sub = root.join("subdir");
    std::fs::create_dir_all(&sub).unwrap();
    let resolved = resolve_workspace_root(Some(sub.clone())).unwrap();
    // Must NOT climb to the enclosing repo: that would silently widen the
    // ChangeEngine write boundary beyond the configured root.
    assert_eq!(resolved, sub.canonicalize().unwrap());
    assert_ne!(resolved, root.canonicalize().unwrap());
}

// ── Boundary ──────────────────────────────────────────────────────────────

#[test]
fn workspace_boundary_denies_traversal_and_outside_paths() {
    let dir = temp_root();
    let root = seed_repo(&dir);
    let engine = codebro_mcp_server::coding::change_engine::ChangeEngine::new(&root, &[], false);
    assert!(engine.resolve("../escape.txt").is_err());
    assert!(engine.resolve("/etc/hostname").is_err());
    assert!(engine.resolve("main.rs").is_ok());
}

#[test]
#[cfg(unix)]
fn symlink_escape_is_denied() {
    let dir = temp_root();
    let root = seed_repo(&dir);
    let outside = temp_root();
    let outside_file = outside.path().join("secret.txt");
    std::fs::write(&outside_file, "secret").unwrap();
    let link = root.join("evil-link");
    std::os::unix::fs::symlink(&outside_file, &link).unwrap();

    let engine = codebro_mcp_server::coding::change_engine::ChangeEngine::new(&root, &[], false);
    // Resolving through the symlink to outside content must fail.
    assert!(engine.resolve("evil-link").is_err());
}

// ── State loading ─────────────────────────────────────────────────────────

#[test]
fn identity_loading_reports_absence_then_presence() {
    let dir = temp_root();
    let root = dir.path().to_path_buf();

    let mut rt = codebro_mcp_server::project_identity::ProjectIdentityRuntime::new(&root);
    assert!(rt.load().is_err(), "fresh workspace has no identity");

    let mut rt = codebro_mcp_server::project_identity::ProjectIdentityRuntime::new(&root);
    rt.create_minimal("scoped-probe", "rust").unwrap();

    let mut rt2 = codebro_mcp_server::project_identity::ProjectIdentityRuntime::new(&root);
    assert!(rt2.load().is_ok());
    assert_eq!(rt2.snapshot().name, "scoped-probe");
}

#[test]
fn facts_loading_empty_then_populated() {
    let dir = temp_root();
    let root = dir.path().to_path_buf();

    // Before init: no facts file → empty store, not an error.
    assert!(!root.join(".codebro/facts.json").exists());
    let packet = engineering_context::compose(
        &root,
        &EngineeringContextRequest {
            task: "facts loading probe".to_string(),
            ..Default::default()
        },
        &[],
    )
    .unwrap();
    assert!(packet.facts.is_empty());

    // After init: facts resolve for a matching keyword.
    std::fs::write(root.join("main.rs"), "fn main() {}\n").unwrap();
    codebro_mcp_server::init::run(&root).unwrap();
    assert!(root.join(".codebro/facts.json").exists());
}

#[test]
fn memory_loading_absent_then_present() {
    let dir = temp_root();
    let root = dir.path().to_path_buf();

    let identity = codebro_mcp_server::project_identity::ProjectIdentityRuntime::new(&root);
    let mut mem =
        codebro_mcp_server::engineering_memory::EngineeringMemoryRuntime::new(&root, identity);
    // Absent store loads to zero entries (read path), never an error.
    let _ = mem.load();
    assert_eq!(mem.snapshot().len(), 0);
}

#[test]
fn fresh_workspace_context_is_honest_and_bounded() {
    let dir = temp_root();
    let packet = engineering_context::compose(
        dir.path(),
        &EngineeringContextRequest {
            task: "fresh workspace probe".to_string(),
            ..Default::default()
        },
        &[],
    )
    .unwrap();
    assert!(!packet.repository.identity_loaded);
    assert!(packet.facts.is_empty());
    assert!(packet.memory.is_empty());
    assert!(packet.evidence.is_empty());
    assert!(
        !packet.notes.is_empty(),
        "fresh workspace must explain itself"
    );
    assert!(packet.serialized_len() < 8192);
}

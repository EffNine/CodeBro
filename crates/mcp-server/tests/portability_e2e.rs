//! E2E: `codebro export` / `codebro import` — the explicit portability path.
//!
//! Pins that the CLI verbs round-trip durable memory through a directory,
//! that import is hermetic (`CODEBRO_STATE_DIR` / `CODEBRO_SKILLS_DIR`),
//! that dry runs write nothing, and that malformed input fails cleanly
//! without touching the store.

use std::path::Path;
use std::process::{Command, Output};

use codebro_mcp_server::context_runtime::{
    Authority, ContextRecord, ContextStore, HistoryInput, HistoryKind, OpenSession, RecordKind,
    RecordScope, STATE_DB_FILE,
};

fn run(args: &[&str], state_dir: &Path, skills_dir: &Path) -> Output {
    let bin = env!("CARGO_BIN_EXE_codebro");
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .env("CODEBRO_STATE_DIR", state_dir)
        .env("CODEBRO_SKILLS_DIR", skills_dir);
    cmd.output().expect("run codebro")
}

fn stdout_json(output: &Output) -> serde_json::Value {
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON document")
}

fn seed(store: &ContextStore, ws: &str) {
    let mut record = ContextRecord::new(
        "ctx::portable",
        RecordKind::Intent,
        "intent.portable",
        "Keep memory portable across devices",
        Authority::UserConfirmed,
    );
    record.scope = RecordScope::Project;
    record.workspace_root = Some(ws.to_string());
    record.created_at = 1000;
    record.updated_at = 1000;
    store.put_record(&record, 1000).unwrap();

    let session = store
        .open_session(
            ws,
            &OpenSession {
                task_id: None,
                title: Some("portable e2e".to_string()),
                source: Some("test".to_string()),
                parent_session_id: None,
            },
            900,
        )
        .unwrap();
    let mut input = HistoryInput::new(ws, HistoryKind::Validation, "cargo test passed");
    input.session_id = Some(session.id);
    input.created_at = Some(950);
    store.record_history(&input, 950).unwrap();
}

#[test]
fn export_then_import_round_trips_through_the_cli() {
    let source_state = tempfile::tempdir().unwrap();
    let source_ws = tempfile::tempdir().unwrap();
    let skills = tempfile::tempdir().unwrap();
    let ws = source_ws.path().display().to_string();
    seed(
        &ContextStore::at_state_dir(source_state.path().to_path_buf()),
        &ws,
    );

    let export_dir = tempfile::tempdir().unwrap();
    let export = run(
        &[
            "export",
            "--out",
            export_dir.path().to_str().unwrap(),
            "--json",
        ],
        source_state.path(),
        skills.path(),
    );
    assert!(
        export.status.success(),
        "export failed: {}",
        String::from_utf8_lossy(&export.stderr)
    );
    let report = stdout_json(&export);
    assert_eq!(report["counts"]["context_records"], 1);
    assert!(export_dir.path().join("manifest.json").is_file());
    assert!(export_dir.path().join("context_records.jsonl").is_file());

    let target_state = tempfile::tempdir().unwrap();
    let target_ws = tempfile::tempdir().unwrap();
    let import = run(
        &[
            "import",
            "--file",
            export_dir.path().to_str().unwrap(),
            "--root",
            target_ws.path().to_str().unwrap(),
            "--json",
        ],
        target_state.path(),
        skills.path(),
    );
    assert!(
        import.status.success(),
        "import failed: {}",
        String::from_utf8_lossy(&import.stderr)
    );
    let report = stdout_json(&import);
    assert_eq!(report["integrity"], "hash_verified");
    assert_eq!(report["format_version"], 1);
    assert_eq!(report["tables"]["context_records"]["inserted"], 1);

    let target_store = ContextStore::at_state_dir(target_state.path().to_path_buf());
    let imported = target_store
        .get_record("ctx::portable")
        .unwrap()
        .expect("record imported");
    assert_eq!(imported.authority, Authority::UserConfirmed);
    assert_eq!(imported.content, "Keep memory portable across devices");
    assert!(target_state.path().join(STATE_DB_FILE).is_file());

    // Duplicate import converges: nothing new is written.
    let again = run(
        &[
            "import",
            "--file",
            export_dir.path().to_str().unwrap(),
            "--root",
            target_ws.path().to_str().unwrap(),
            "--json",
        ],
        target_state.path(),
        skills.path(),
    );
    assert!(again.status.success());
    let report = stdout_json(&again);
    assert_eq!(report["tables"]["context_records"]["inserted"], 0);
    assert_eq!(report["tables"]["context_records"]["duplicates"], 1);
}

#[test]
fn import_dry_run_writes_nothing() {
    let source_state = tempfile::tempdir().unwrap();
    let source_ws = tempfile::tempdir().unwrap();
    let skills = tempfile::tempdir().unwrap();
    seed(
        &ContextStore::at_state_dir(source_state.path().to_path_buf()),
        &source_ws.path().display().to_string(),
    );
    let export_dir = tempfile::tempdir().unwrap();
    assert!(run(
        &["export", "--out", export_dir.path().to_str().unwrap()],
        source_state.path(),
        skills.path(),
    )
    .status
    .success());

    let target_state = tempfile::tempdir().unwrap();
    let target_ws = tempfile::tempdir().unwrap();
    let dry = run(
        &[
            "import",
            "--file",
            export_dir.path().to_str().unwrap(),
            "--root",
            target_ws.path().to_str().unwrap(),
            "--dry-run",
            "--json",
        ],
        target_state.path(),
        skills.path(),
    );
    assert!(dry.status.success());
    let report = stdout_json(&dry);
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["tables"]["context_records"]["inserted"], 1);
    let target_store = ContextStore::at_state_dir(target_state.path().to_path_buf());
    assert!(target_store.get_record("ctx::portable").unwrap().is_none());
}

#[test]
fn export_without_state_database_fails_cleanly() {
    let state = tempfile::tempdir().unwrap();
    let skills = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let result = run(
        &["export", "--out", out.path().to_str().unwrap()],
        state.path(),
        skills.path(),
    );
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("state database not found"),
        "unexpected stderr: {stderr}"
    );
}

#[test]
fn import_malformed_manifest_fails_cleanly() {
    let export_dir = tempfile::tempdir().unwrap();
    std::fs::write(export_dir.path().join("manifest.json"), "{oops").unwrap();
    let target_state = tempfile::tempdir().unwrap();
    let skills = tempfile::tempdir().unwrap();
    let result = run(
        &[
            "import",
            "--file",
            export_dir.path().to_str().unwrap(),
            "--json",
        ],
        target_state.path(),
        skills.path(),
    );
    assert!(!result.status.success());
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(
        stderr.contains("malformed manifest"),
        "unexpected stderr: {stderr}"
    );
    assert!(!target_state.path().join(STATE_DB_FILE).is_file());
}

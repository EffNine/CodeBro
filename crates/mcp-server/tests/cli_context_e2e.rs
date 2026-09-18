//! E2E: the `codebro context` CLI verb — the host/hook integration surface.
//!
//! Pins that the verb emits one bounded JSON document on stdout, that it
//! agrees with the MCP `context` packet shape (structural digest without
//! `--task`, task-ranked packet with `--task`), and that it is hermetic
//! under `CODEBRO_STATE_DIR` (never touches the developer's real store).

use std::path::Path;
use std::process::{Command, Output};

fn run_context(state_dir: &Path, ws: &Path, extra: &[&str]) -> Output {
    let bin = env!("CARGO_BIN_EXE_codebro");
    let mut cmd = Command::new(bin);
    cmd.arg("context")
        .arg("--root")
        .arg(ws)
        .env("CODEBRO_STATE_DIR", state_dir);
    for arg in extra {
        cmd.arg(arg);
    }
    cmd.output().expect("run codebro context")
}

fn json_stdout(output: &Output) -> serde_json::Value {
    assert!(
        output.status.success(),
        "context verb failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout.clone()).expect("utf8 stdout");
    serde_json::from_str(stdout.trim()).expect("stdout is exactly one JSON document")
}

#[test]
fn structural_digest_without_task_is_valid_json() {
    let ws = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();

    let value = json_stdout(&run_context(state.path(), ws.path(), &[]));

    assert!(value.get("repository").is_some(), "missing repository");
    assert!(value.get("records").is_some(), "missing records");
    let notes = value
        .get("notes")
        .and_then(|n| n.as_array())
        .expect("notes array");
    assert!(
        notes
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("structural digest")),
        "structural digest must self-label: {notes:?}"
    );
}

#[test]
fn task_packet_carries_context_tool_sections() {
    let ws = tempfile::tempdir().unwrap();
    std::fs::write(ws.path().join("lib.rs"), "pub fn alpha() {}\n").unwrap();
    let state = tempfile::tempdir().unwrap();

    let value = json_stdout(&run_context(
        state.path(),
        ws.path(),
        &["--task", "alpha function"],
    ));

    for key in [
        "repository",
        "facts",
        "facts_provenance",
        "decisions_provenance",
        "records",
        "records_provenance",
        "impact",
    ] {
        assert!(value.get(key).is_some(), "missing {key}");
    }
    let notes = value
        .get("notes")
        .and_then(|n| n.as_array())
        .expect("notes array");
    assert!(
        !notes
            .iter()
            .any(|n| n.as_str().unwrap_or("").contains("structural digest")),
        "task packet must not self-label structural: {notes:?}"
    );
}

#[test]
fn keywords_task_id_and_pretty_are_accepted() {
    let ws = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();

    let value = json_stdout(&run_context(
        state.path(),
        ws.path(),
        &[
            "--task",
            "wire the host surface",
            "--keyword",
            "context",
            "--task-id",
            "task::demo",
            "--pretty",
        ],
    ));

    assert!(value.get("repository").is_some());
}

#[test]
fn missing_workspace_root_fails_without_panicking() {
    let ws = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    let missing = ws.path().join("does-not-exist");

    let output = run_context(state.path(), &missing, &[]);

    assert!(!output.status.success(), "missing root must fail");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.trim().is_empty(),
        "failure must not emit a partial JSON document: {stdout:?}"
    );
}

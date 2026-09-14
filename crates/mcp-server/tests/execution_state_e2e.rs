//! Execution-state reliability E2E through the real `codebro` binary over
//! stdio: an applied edit is declared unverified; a failing test is
//! journaled and blocks `task complete` / `outcome=success`; a later
//! passing run of the same invocation resolves it; failed edits never mark
//! a task successful.
//!
//! Hermetic: tempdir workspace with its own git repo, isolated state dir.
//! Local sandbox backend (no OPEN_SANDBOX_URL configured).

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};

fn git(cwd: &Path, args: &[&str]) {
    let status = Proc::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

/// Minimal git-tracked cargo project whose single test can be flipped
/// between passing and failing by editing one line.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("exec-state")
            .tempdir()
            .unwrap();
        let p = dir.path();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(
            p.join("src/lib.rs"),
            "#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {\n        assert_eq!(2 + 2, 4);\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(p.join(".gitignore"), "/target\nCargo.lock\n.codebro/\n").unwrap();
        git(p, &["init", "-q"]);
        git(p, &["add", "-A"]);
        git(
            p,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-qm",
                "init",
            ],
        );
        Fixture { dir }
    }
}

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
    _state: tempfile::TempDir,
}

impl Server {
    fn start(root: &Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let state = tempfile::Builder::new()
            .prefix("exec-state-db")
            .tempdir()
            .expect("state tempdir");
        let mut child = Proc::new(bin)
            .args(["serve", "--root"])
            .arg(root)
            .env("CODEBRO_STATE_DIR", state.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codebro serve");
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let reader = BufReader::new(child.stdout.take().unwrap());
        let mut s = Server {
            child,
            stdin,
            reader,
            _state: state,
        };
        s.rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "exec-state-e2e", "version": "0"}
            }),
        );
        s.stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        s.stdin.flush().unwrap();
        s
    }

    fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let msg = serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.stdin.write_all(msg.to_string().as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf).expect("read");
            assert!(n > 0, "server closed");
            let trimmed = buf.trim();
            if !trimmed.starts_with('{') {
                continue;
            }
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
                if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                    return v;
                }
            }
        }
    }

    /// (ok, text): tool errors return (false, text) instead of panicking.
    fn call_raw(&mut self, tool: &str, args: serde_json::Value) -> (bool, String) {
        let r = self.rpc(
            "tools/call",
            serde_json::json!({"name":tool,"arguments":args}),
        );
        if r.get("error").is_some() {
            return (false, r["error"].to_string());
        }
        let is_error = r["result"]["isError"].as_bool().unwrap_or(false);
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        (!is_error, text)
    }

    fn call(&mut self, tool: &str, args: serde_json::Value) -> serde_json::Value {
        let (ok, text) = self.call_raw(tool, args);
        assert!(ok, "tool {tool} errored: {text}");
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

    fn call_err(&mut self, tool: &str, args: serde_json::Value) -> String {
        let (ok, text) = self.call_raw(tool, args);
        assert!(!ok, "tool {tool} unexpectedly succeeded: {text}");
        text
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Full loop: edit (unverified) → failing test (blocked) → fix → passing
/// test (verified) → completion allowed. Plus: a failed edit never marks a
/// task successful, and success cannot be claimed while failure evidence
/// stands.
#[test]
fn edit_verify_gate_loop_over_real_binary() {
    let fx = Fixture::new();
    let root = fx.dir.path();
    let mut s = Server::start(root);

    // ── 1. Apply a breaking edit: applied, explicitly unverified. ──────
    let applied = s.call(
        "apply_change",
        serde_json::json!({
            "path": "src/lib.rs",
            "old": "assert_eq!(2 + 2, 4);",
            "new": "assert_eq!(2 + 2, 5);",
        }),
    );
    assert_eq!(applied["applied"], true);
    assert_eq!(applied["status"], "applied_unverified");
    assert_eq!(applied["verification_status"], "unverified");
    assert_eq!(applied["edit_verification"]["content_matches_intent"], true);

    // ── 2. Task in flight; a failing test is journaled. ────────────────
    let created = s.call(
        "task",
        serde_json::json!({"action": "create", "title": "fix adds"}),
    );
    let task_id = created["task"]["task_id"].as_str().unwrap().to_string();
    s.call(
        "task",
        serde_json::json!({"action": "start", "task_id": task_id}),
    );

    let tested = s.call("sandbox_test", serde_json::json!({}));
    assert_eq!(tested["verification"]["verified"], false);
    assert_eq!(tested["verification"]["classification"], "test_failure");
    assert_eq!(tested["verification"]["execution_state"]["state"], "failed");
    assert_eq!(
        tested["verification"]["execution_state"]["unresolved_failures_total"],
        1
    );

    // ── 3. A passed validation record cannot override the evidence. ────
    s.call(
        "task",
        serde_json::json!({"action": "validate", "task_id": task_id, "what": "cargo test"}),
    );
    s.call(
        "task",
        serde_json::json!({
            "action": "validation_result", "task_id": task_id,
            "result": "passed", "what": "cargo test",
        }),
    );
    let err = s.call_err(
        "task",
        serde_json::json!({"action": "complete", "task_id": task_id, "reason": "done"}),
    );
    assert!(
        err.contains("unresolved build/test failure"),
        "completion must be refused: {err}"
    );

    // ── 4. A success outcome is refused too; failure reports remain. ───
    let err = s.call_err(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "success", "summary": "implemented",
        }),
    );
    assert!(
        err.contains("success outcome refused"),
        "success outcome must be refused: {err}"
    );
    let honest = s.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "failure", "summary": "test still failing",
        }),
    );
    assert_eq!(honest["classification"], "failure");
    assert_eq!(honest["execution_state"]["state"], "failed");

    // ── 5. Fix the edit, re-run the same invocation: verified. ─────────
    let fixed = s.call(
        "apply_change",
        serde_json::json!({
            "path": "src/lib.rs",
            "old": "assert_eq!(2 + 2, 5);",
            "new": "assert_eq!(2 + 2, 4);",
        }),
    );
    assert_eq!(fixed["status"], "applied_unverified");
    let retested = s.call("sandbox_test", serde_json::json!({}));
    assert_eq!(retested["verification"]["verified"], true);
    assert_eq!(
        retested["verification"]["execution_state"]["state"],
        "verified"
    );

    // ── 6. Completion now allowed, carrying the verified state. ────────
    let done = s.call(
        "task",
        serde_json::json!({"action": "complete", "task_id": task_id, "reason": "fixed"}),
    );
    assert_eq!(done["task"]["status"], "completed");
    assert_eq!(done["execution_state"]["state"], "verified");

    // ── 7. Failure path: an edit that cannot apply never marks success. ─
    let t2 = s.call(
        "task",
        serde_json::json!({"action": "create", "title": "failed edit task"}),
    );
    let t2_id = t2["task"]["task_id"].as_str().unwrap().to_string();
    s.call(
        "task",
        serde_json::json!({"action": "start", "task_id": t2_id}),
    );
    let err = s.call_err(
        "apply_change",
        serde_json::json!({
            "path": "src/lib.rs",
            "old": "this text does not exist anywhere",
            "new": "irrelevant",
        }),
    );
    assert!(err.contains("stale content"), "edit must fail: {err}");
    // Scenario 5: the failed edit leaves no verified completion behind —
    // the task cannot complete (still running), and the file is untouched.
    let _ = s.call_err(
        "task",
        serde_json::json!({"action": "complete", "task_id": t2_id, "reason": "trust me"}),
    );
    // The task is untouched: still running, never auto-completed.
    let inspected = s.call(
        "task",
        serde_json::json!({"action": "inspect", "task_id": t2_id}),
    );
    assert_eq!(inspected["snapshot"]["task"]["status"], "running");
    // The workspace is still verified (the failed edit wrote nothing).
    assert_eq!(inspected["execution_state"]["state"], "verified");
    // And the file content is unchanged.
    assert_eq!(
        std::fs::read_to_string(root.join("src/lib.rs")).unwrap(),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {\n        assert_eq!(2 + 2, 4);\n    }\n}\n"
    );

    // ── 8. Same gate is visible at session start. ──────────────────────
    let wc = s.call("workspace_context", serde_json::json!({}));
    assert_eq!(wc["execution_state"]["state"], "verified");
    assert_eq!(
        wc["execution_state"]["last_success"]["command"],
        "cargo test"
    );
}

/// An unresolved failure is surfaced by `workspace_context` and `context`
/// before any task action — the runtime cannot "forget" it.
#[test]
fn failure_state_is_visible_at_session_start() {
    let fx = Fixture::new();
    let root = fx.dir.path();
    let mut s = Server::start(root);

    // Break the code and run the test through CodeBro.
    s.call(
        "apply_change",
        serde_json::json!({
            "path": "src/lib.rs",
            "old": "assert_eq!(2 + 2, 4);",
            "new": "assert_eq!(2 + 2, 5);",
        }),
    );
    let tested = s.call("sandbox_test", serde_json::json!({}));
    assert_eq!(tested["verification"]["verified"], false);

    let wc = s.call("workspace_context", serde_json::json!({}));
    assert_eq!(wc["execution_state"]["state"], "failed");
    let failures = wc["execution_state"]["unresolved_failures"]
        .as_array()
        .expect("failures listed");
    assert!(!failures.is_empty());
    assert!(failures[0]["command"]
        .as_str()
        .unwrap()
        .contains("cargo test"));

    let ctx = s.call("context", serde_json::json!({"task": "finish the fix"}));
    assert_eq!(ctx["execution_state"]["state"], "failed");
    assert!(ctx["notes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|n| n.as_str().unwrap().contains("unresolved")));
}

/// A task whose failure was recorded in a DIFFERENT tree state is not
/// blocked: the evidence no longer applies once the tree changes. The new
/// tree inherits NEITHER the failure NOR any verification — completion is
/// possible only as `unverified`, never as `verified`.
#[test]
fn stale_tree_failure_does_not_block_after_edit() {
    let fx = Fixture::new();
    let root = fx.dir.path();
    let mut s = Server::start(root);

    // Failing run on the broken tree.
    s.call(
        "apply_change",
        serde_json::json!({
            "path": "src/lib.rs",
            "old": "assert_eq!(2 + 2, 4);",
            "new": "assert_eq!(2 + 2, 5);",
        }),
    );
    let tested = s.call("sandbox_test", serde_json::json!({}));
    assert_eq!(tested["verification"]["verified"], false);

    // Edit again (fix) without re-running the test: the old failure no
    // longer applies to the new tree state — the state is honest
    // `unverified`, not falsely verified and not falsely failed.
    s.call(
        "apply_change",
        serde_json::json!({
            "path": "src/lib.rs",
            "old": "assert_eq!(2 + 2, 5);",
            "new": "assert_eq!(2 + 2, 4);",
        }),
    );
    let wc = s.call("workspace_context", serde_json::json!({}));
    assert_eq!(wc["execution_state"]["state"], "unverified");
    assert_eq!(wc["execution_state"]["unresolved_failures_total"], 0);
    assert!(root.join(".codebro/execution_evidence.json").exists());

    // Scenario 4: the task can be completed on the new tree, but the
    // completion response must say UNVERIFIED — the new tree inherits no
    // verification from the pre-edit tree, and a prose validation record
    // never manufactures one.
    let created = s.call(
        "task",
        serde_json::json!({"action": "create", "title": "edited without retest"}),
    );
    let task_id = created["task"]["task_id"].as_str().unwrap().to_string();
    s.call(
        "task",
        serde_json::json!({"action": "start", "task_id": task_id}),
    );
    s.call(
        "task",
        serde_json::json!({"action": "validate", "task_id": task_id, "what": "cargo test"}),
    );
    s.call(
        "task",
        serde_json::json!({
            "action": "validation_result", "task_id": task_id,
            "result": "passed", "what": "cargo test",
        }),
    );
    let done = s.call(
        "task",
        serde_json::json!({"action": "complete", "task_id": task_id, "reason": "fixed"}),
    );
    assert_eq!(done["task"]["status"], "completed");
    assert_eq!(done["execution_state"]["state"], "unverified");
    assert_eq!(done["execution_state"]["evidence_records"], 0);
}

/// Phase 4: CodeBro observes workspace changes made OUTSIDE its own
/// mutation path through the working-tree hash — no filesystem watcher.
/// A recorded pass stops applying to a natively edited tree: the state
/// drops to `unverified` and completion can never claim `verified`.
#[test]
fn native_edit_invalidates_verification_without_polling() {
    let fx = Fixture::new();
    let root = fx.dir.path();
    let mut s = Server::start(root);

    // Passing run on the pristine tree.
    let tested = s.call("sandbox_test", serde_json::json!({}));
    assert_eq!(tested["verification"]["verified"], true);
    assert_eq!(
        tested["verification"]["execution_state"]["state"],
        "verified"
    );

    let created = s.call(
        "task",
        serde_json::json!({"action": "create", "title": "native edit"}),
    );
    let task_id = created["task"]["task_id"].as_str().unwrap().to_string();
    s.call(
        "task",
        serde_json::json!({"action": "start", "task_id": task_id}),
    );

    // Native edit through the filesystem (NOT apply_change), no re-run.
    std::fs::write(
        root.join("src/lib.rs"),
        "#[cfg(test)]\nmod tests {\n    #[test]\n    fn adds() {\n        assert_eq!(2 + 2, 4);\n    }\n}\n// native edit\n",
    )
    .unwrap();

    let wc = s.call("workspace_context", serde_json::json!({}));
    assert_eq!(wc["execution_state"]["state"], "unverified");
    assert_eq!(wc["execution_state"]["evidence_records"], 0);

    // Completion remains possible, but only as unverified.
    s.call(
        "task",
        serde_json::json!({"action": "validate", "task_id": task_id, "what": "cargo test"}),
    );
    s.call(
        "task",
        serde_json::json!({
            "action": "validation_result", "task_id": task_id,
            "result": "passed", "what": "cargo test",
        }),
    );
    let done = s.call(
        "task",
        serde_json::json!({"action": "complete", "task_id": task_id, "reason": "edited"}),
    );
    assert_eq!(done["task"]["status"], "completed");
    assert_eq!(done["execution_state"]["state"], "unverified");
}

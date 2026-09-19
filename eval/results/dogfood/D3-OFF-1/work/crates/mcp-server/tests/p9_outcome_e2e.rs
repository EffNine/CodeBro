//! P9 outcome & feedback-loop probes: the engineering-outcome contract
//! verified through the real `codebro` binary over stdio RPC.
//!
//! P9 adds no MCP tool and no schema change: one new `task` action
//! (`outcome`) records structured outcome evidence as task-bound history,
//! and the existing recall/learning/brief machinery carries it into
//! future context. These probes pin that loop end-to-end:
//!
//! 1. **Outcome ingestion** — orient → brief → task create/start → outcome
//!    (failure with test evidence) → no task transition occurs.
//! 2. **Idempotency** — redelivering the same (task, dedup_key) returns
//!    the original event; the same key on another task is distinct.
//! 3. **Completion + user confirmation** — validate → passed → complete →
//!    post-terminal `user_confirmed` outcome records `user_confirmed`
//!    authority while plain reports stay `observed`.
//! 4. **Restart persistence** — task, checkpoint, and outcome evidence
//!    survive a fresh server process over the same state.db.
//! 5. **Feedback into future context** — a new related brief surfaces the
//!    earlier failure through its history excerpts (recall-ranked, no P9
//!    ranker); repeated briefs agree byte-for-byte.
//! 6. **Security** — secret-shaped outcome text is redacted at write and
//!    in every projection; cross-workspace outcome injection is refused;
//!    stdout stays pure JSON-RPC throughout.
//! 7. **Concurrency** — two live server processes record distinct
//!    outcomes over shared state without deadlock or loss.
//! 8. **CodeBro executes nothing** — the flow never invokes a sandbox
//!    tool; outcomes are reported evidence, never CodeBro-run tests.
//!
//! Hermetic: tempdir workspaces, explicit CODEBRO_STATE_DIR +
//! CODEBRO_SKILLS_DIR, never touches ~/.codebro, the user's real
//! repository, or real skills.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
    stderr: Arc<Mutex<String>>,
}

impl Server {
    fn start(root: &Path, state: &Path, skills: &Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut child = Proc::new(bin)
            .args(["serve", "--root"])
            .arg(root)
            .env("CODEBRO_STATE_DIR", state)
            .env("CODEBRO_SKILLS_DIR", skills)
            .env("RUST_LOG", "info")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn codebro serve");
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let reader = BufReader::new(child.stdout.take().unwrap());
        let stderr_handle = child.stderr.take().unwrap();
        let stderr = Arc::new(Mutex::new(String::new()));
        let stderr_sink = Arc::clone(&stderr);
        std::thread::spawn(move || {
            let mut reader = BufReader::new(stderr_handle);
            let mut buf = String::new();
            loop {
                buf.clear();
                match reader.read_line(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if let Ok(mut sink) = stderr_sink.lock() {
                            sink.push_str(&buf);
                        }
                    }
                }
            }
        });
        let mut s = Server {
            child,
            stdin,
            reader,
            stderr,
        };
        s.rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "p9-probe", "version": "0"}
            }),
        );
        s.stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        s.stdin.flush().unwrap();
        s
    }

    fn stderr_so_far(&self) -> String {
        self.stderr.lock().expect("stderr lock").clone()
    }

    fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
        let msg = serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params});
        self.stdin.write_all(msg.to_string().as_bytes()).unwrap();
        self.stdin.write_all(b"\n").unwrap();
        self.stdin.flush().unwrap();
        self.read_rpc_response(id)
    }

    fn read_rpc_response(&mut self, id: u64) -> serde_json::Value {
        let mut buf = String::new();
        loop {
            buf.clear();
            let n = self.reader.read_line(&mut buf).expect("read");
            assert!(n > 0, "server closed before responding to id {id}");
            let trimmed = buf.trim();
            assert!(
                trimmed.starts_with('{'),
                "PROTOCOL VIOLATION: non-JSON line on stdout: {trimmed:?}"
            );
            let v: serde_json::Value =
                serde_json::from_str(trimmed).expect("stdout line is valid JSON-RPC");
            assert!(
                v.get("jsonrpc").is_some(),
                "PROTOCOL VIOLATION: stdout line without jsonrpc field: {trimmed:?}"
            );
            if v.get("id").and_then(|i| i.as_u64()) == Some(id) {
                return v;
            }
        }
    }

    fn call(&mut self, tool: &str, args: serde_json::Value) -> serde_json::Value {
        let r = self.rpc(
            "tools/call",
            serde_json::json!({"name":tool,"arguments":args}),
        );
        assert!(
            r.get("error").is_none(),
            "tool {tool} errored: {}",
            r["error"]
        );
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let is_error = r["result"]["isError"].as_bool().unwrap_or(false);
        assert!(!is_error, "tool {tool} returned isError: {text}");
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

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
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Env {
    _home: tempfile::TempDir,
    state: PathBuf,
    skills: PathBuf,
}

fn env_new() -> Env {
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let skills = home.path().join("skills");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&skills).unwrap();
    Env {
        _home: home,
        state,
        skills,
    }
}

fn seed_repo(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"probe-repo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
    )
    .unwrap();
    let run = |args: &[&str]| {
        let out = Proc::new("git")
            .args(args)
            .current_dir(root)
            .env("GIT_AUTHOR_NAME", "probe")
            .env("GIT_AUTHOR_EMAIL", "probe@invalid")
            .env("GIT_COMMITTER_NAME", "probe")
            .env("GIT_COMMITTER_EMAIL", "probe@invalid")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("git invocation");
        assert!(
            out.status.success(),
            "git {:?} failed: {}{}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["init", "-q"]);
    run(&["add", "."]);
    run(&["commit", "-q", "-m", "probe seed"]);
}

// ── Probe 1: the full P9 loop over the real binary ───────────────────────

#[test]
fn p9_outcome_loop_ingest_complete_restart_feedback() {
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let env = env_new();
    let mut server = Server::start(repo.path(), &env.state, &env.skills);

    // Orient: workspace context answers.
    let ctx = server.call("workspace_context", serde_json::json!({}));
    assert!(ctx.get("workspace_root").is_some(), "{ctx}");

    // Brief before work: honest unknowns, no fabrication.
    let brief = server.call(
        "engineering_brief",
        serde_json::json!({"task": "migrate billing integration to the new ledger"}),
    );
    assert!(brief.get("unknowns").is_some(), "{brief}");

    // Task identity: create + start (OpenCode owns what happens next).
    let created = server.call(
        "task",
        serde_json::json!({"action": "create", "title": "migrate billing integration"}),
    );
    let task_id = created["task"]["task_id"].as_str().unwrap().to_string();
    server.call(
        "task",
        serde_json::json!({"action": "start", "task_id": task_id}),
    );

    // Outcome: OpenCode reports what its own test run taught us.
    // CodeBro records the report — it runs no tests itself (no sandbox
    // tool is invoked anywhere in this flow).
    let outcome = server.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "failure",
            "summary": "billing integration failed because API contract differs",
            "what": "cargo test", "exit_code": 101,
            "reason": "integration_contract: expected 200 got 500",
            "changed_areas": ["src/api.rs"],
            "dedup_key": "attempt-1",
        }),
    );
    assert_eq!(outcome["classification"], "failure");
    assert_eq!(outcome["authority"], "observed");
    assert_eq!(outcome["duplicate"], false);
    let event_id = outcome["event_id"].as_i64().unwrap();
    assert!(event_id > 0);

    // No transition happened: still running, version untouched.
    let inspect = server.call(
        "task",
        serde_json::json!({"action": "inspect", "task_id": task_id}),
    );
    assert_eq!(inspect["snapshot"]["task"]["status"], "running");
    let events = inspect["snapshot"]["recent_events"].as_array().unwrap();
    assert!(
        events.iter().any(|e| e["event_id"] == event_id
            && e["summary"].as_str().unwrap().contains("API contract")),
        "outcome evidence must surface in the resume snapshot: {events:?}"
    );

    // Idempotency: redelivery returns the original event.
    let replay = server.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "failure",
            "summary": "billing integration failed because API contract differs",
            "dedup_key": "attempt-1",
        }),
    );
    assert_eq!(replay["duplicate"], true);
    assert_eq!(replay["event_id"], event_id);

    // Complete through the unchanged validation gate, then record the
    // user's own confirmation as user-confirmed evidence.
    server.call(
        "task",
        serde_json::json!({"action": "validate", "task_id": task_id, "what": "cargo test"}),
    );
    server.call("task", serde_json::json!({"action": "validation_result", "task_id": task_id, "result": "passed", "what": "cargo test"}));
    server.call("task", serde_json::json!({"action": "complete", "task_id": task_id, "reason": "contract adapter shipped"}));
    let confirm = server.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "success",
            "summary": "user confirmed the billing migration resolves the issue",
            "user_confirmed": true,
        }),
    );
    assert_eq!(confirm["authority"], "user_confirmed");

    // Recall finds the failure by its own vocabulary.
    let recall = server.call(
        "recall",
        serde_json::json!({"query": "billing integration API contract"}),
    );
    let recall_text = serde_json::to_string(&recall).unwrap();
    assert!(recall_text.contains("API contract"), "{recall_text}");

    // Restart: a fresh server process over the same state.db.
    drop(server);
    let mut server = Server::start(repo.path(), &env.state, &env.skills);
    let inspect = server.call(
        "task",
        serde_json::json!({"action": "inspect", "task_id": task_id}),
    );
    assert_eq!(inspect["snapshot"]["task"]["status"], "completed");
    let events = inspect["snapshot"]["recent_events"].as_array().unwrap();
    assert!(
        events.iter().any(|e| e["event_id"] == event_id),
        "outcome evidence must survive restart: {events:?}"
    );

    // Future context: a new related brief surfaces the earlier failure
    // through history excerpts — the loop closes without any P9 ranker.
    let brief_a = server.call(
        "engineering_brief",
        serde_json::json!({"task": "implement similar billing integration for refunds"}),
    );
    let history = brief_a["history"].as_array().unwrap();
    assert!(
        history.iter().any(|h| h["kind"] == "task_outcome"
            && h["excerpt"].as_str().unwrap().contains("API contract")),
        "validated outcome must appear in a related brief: {history:?}"
    );
    // Determinism: the same brief twice agrees byte-for-byte.
    let brief_b = server.call(
        "engineering_brief",
        serde_json::json!({"task": "implement similar billing integration for refunds"}),
    );
    assert_eq!(
        serde_json::to_string(&brief_a).unwrap(),
        serde_json::to_string(&brief_b).unwrap(),
        "briefs must be deterministic across repeats"
    );
}

// ── Probe 2: secrets, isolation, malformed input, stdout purity ──────────

#[test]
fn p9_outcome_adversarial_redaction_isolation_and_malformed() {
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let env = env_new();
    let mut server = Server::start(repo.path(), &env.state, &env.skills);
    let secret = "sk-P9PROBESECRETKEY1234567890ab";

    let created = server.call(
        "task",
        serde_json::json!({"action": "create", "title": "secret probe"}),
    );
    let task_id = created["task"]["task_id"].as_str().unwrap().to_string();

    // Secret-shaped outcome text must never persist verbatim…
    let outcome = server.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "failure",
            "summary": format!("deploy failed with api_key=\"{secret}\""),
            "reason": format!("token {secret}"),
        }),
    );
    assert!(!serde_json::to_string(&outcome).unwrap().contains(secret));

    // …nor surface in any projection (inspect, recall, brief) or stderr.
    let inspect = server.call(
        "task",
        serde_json::json!({"action": "inspect", "task_id": task_id}),
    );
    assert!(!serde_json::to_string(&inspect).unwrap().contains(secret));
    // The user-confirmed path shares the write seam: same guarantee with
    // a different secret-shaped value.
    let secret2 = "ghp_P9USERCONFIRMED0123456789abcd";
    server.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "success",
            "summary": format!("user confirmed with token {secret2}"),
            "user_confirmed": true,
        }),
    );
    let inspect = server.call(
        "task",
        serde_json::json!({"action": "inspect", "task_id": task_id}),
    );
    assert!(!serde_json::to_string(&inspect).unwrap().contains(secret2));
    let recall = server.call(
        "recall",
        serde_json::json!({"query": "deploy failed api key"}),
    );
    assert!(!serde_json::to_string(&recall).unwrap().contains(secret));
    let brief = server.call(
        "engineering_brief",
        serde_json::json!({"task": "deploy the service"}),
    );
    assert!(!serde_json::to_string(&brief).unwrap().contains(secret));
    // A redaction marker survives instead of the raw secret.
    let recall_text = serde_json::to_string(&recall).unwrap();
    assert!(recall_text.contains("[REDACTED]"), "{recall_text}");
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(
        !server.stderr_so_far().contains(secret),
        "stderr must never carry caller secrets"
    );

    // Malformed outcomes are rejected with bounded errors.
    for args in [
        serde_json::json!({"action": "outcome", "task_id": task_id}),
        serde_json::json!({"action": "outcome", "task_id": task_id, "summary": "x"}),
        serde_json::json!({"action": "outcome", "task_id": task_id, "classification": "triumph", "summary": "x"}),
        serde_json::json!({"action": "outcome", "task_id": task_id, "classification": "success", "summary": "   "}),
        serde_json::json!({"action": "outcome", "task_id": "task::0000000000000000", "classification": "success", "summary": "ghost"}),
    ] {
        let (ok, _) = server.call_raw("task", args);
        assert!(!ok, "malformed outcome must be rejected");
    }

    // Cross-workspace injection: another workspace's server refuses this
    // workspace's task id with no content leak.
    let repo_b = tempfile::tempdir().unwrap();
    seed_repo(repo_b.path());
    let mut server_b = Server::start(repo_b.path(), &env.state, &env.skills);
    let (ok, err) = server_b.call_raw(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "success", "summary": "injected",
        }),
    );
    assert!(!ok, "cross-workspace outcome must be refused");
    assert!(
        err.contains("workspace"),
        "refusal must not leak content: {err}"
    );
}

// ── Probe 3: concurrent outcome clients over shared state ────────────────

#[test]
fn p9_concurrent_outcome_clients_share_state_safely() {
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let env = env_new();
    let mut server = Server::start(repo.path(), &env.state, &env.skills);
    let created = server.call(
        "task",
        serde_json::json!({"action": "create", "title": "concurrent outcomes"}),
    );
    let task_id = created["task"]["task_id"].as_str().unwrap().to_string();
    drop(server);

    let root = repo.path().to_path_buf();
    let state = env.state.clone();
    let skills = env.skills.clone();
    let handles: Vec<_> = (0..4)
        .map(|i| {
            let (root, state, skills, task_id) =
                (root.clone(), state.clone(), skills.clone(), task_id.clone());
            std::thread::spawn(move || {
                let mut server = Server::start(&root, &state, &skills);
                server.call(
                    "task",
                    serde_json::json!({
                        "action": "outcome", "task_id": task_id,
                        "classification": "partial",
                        "summary": format!("worker progress report number {i}"),
                        "dedup_key": format!("worker-{i}"),
                    }),
                )["event_id"]
                    .as_i64()
                    .unwrap()
            })
        })
        .collect();
    let mut ids: Vec<i64> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(
        ids.len(),
        4,
        "four concurrent outcomes ⇒ four distinct events"
    );

    // Same-key redelivery after the race still resolves idempotently.
    let mut server = Server::start(&root, &state, &skills);
    let replay = server.call(
        "task",
        serde_json::json!({
            "action": "outcome", "task_id": task_id,
            "classification": "partial", "summary": "worker progress report number 0",
            "dedup_key": "worker-0",
        }),
    );
    assert_eq!(replay["duplicate"], true);
    let rid = replay["event_id"].as_i64().unwrap();
    assert!(ids.contains(&rid), "replay must resolve to a recorded id");
    drop(server);

    // Same-key concurrent race: two processes deliver the same (task,
    // key) at once — exactly one event exists afterwards, and both
    // callers agree on its id (first-write-wins, atomic under BEGIN
    // IMMEDIATE serialization).
    let (root2, state2, skills2, task_id2) =
        (root.clone(), state.clone(), skills.clone(), task_id.clone());
    let race: Vec<_> = (0..2)
        .map(|i| {
            let (root, state, skills, task_id) = (
                root2.clone(),
                state2.clone(),
                skills2.clone(),
                task_id2.clone(),
            );
            std::thread::spawn(move || {
                let mut server = Server::start(&root, &state, &skills);
                let out = server.call(
                    "task",
                    serde_json::json!({
                        "action": "outcome", "task_id": task_id,
                        "classification": "failure",
                        "summary": format!("simultaneous failure report {i}"),
                        "dedup_key": "race-key",
                    }),
                );
                (
                    out["event_id"].as_i64().unwrap(),
                    out["duplicate"].as_bool().unwrap(),
                )
            })
        })
        .collect();
    let race: Vec<(i64, bool)> = race.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(race[0].0, race[1].0, "racers must agree on one event");
    assert_eq!(
        race.iter().filter(|(_, d)| !d).count(),
        1,
        "exactly one racer creates, the other replays: {race:?}"
    );
}

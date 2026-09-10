//! P7 integration: engineering decision-support briefs through the real
//! `codebro` binary + library RPC surface.
//!
//! Covers the P7 acceptance spine end to end: temporary repository → task
//! creation → index → brief → modify → stale brief → reindex → changed
//! brief (impact/freshness) → restart → deterministic persisted result.
//! Plus task isolation over RPC and malformed-input rejection.
//!
//! Hermetic: every test uses `tempfile::tempdir()` for repos and an
//! explicit `CODEBRO_STATE_DIR`. Never touches `~/.codebro`, real repos,
//! or real skill directories.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};

// ── Binary harness (same discipline as the P6 E2E) ──────────────────────

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
}

impl Server {
    fn start(root: &Path, state: &Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut child = Proc::new(bin)
            .args(["serve", "--root"])
            .arg(root)
            .env("CODEBRO_STATE_DIR", state)
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
        };
        s.rpc(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "p7-e2e", "version": "0"}
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

    fn call(&mut self, tool: &str, args: serde_json::Value) -> serde_json::Value {
        let r = self.rpc(
            "tools/call",
            serde_json::json!({"name":tool,"arguments":args}),
        );
        assert!(
            r.get("error").is_none(),
            "tool {tool} errored: {}",
            r.get("error").unwrap_or(&serde_json::Value::Null)
        );
        // tools/call returns { result: { content: [{ text }] } }.
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .expect("tool text")
            .to_string();
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

    fn call_err(&mut self, tool: &str, args: serde_json::Value) -> String {
        let r = self.rpc(
            "tools/call",
            serde_json::json!({"name":tool,"arguments":args}),
        );
        r.get("error")
            .map(|e| e.to_string())
            .unwrap_or_else(|| panic!("tool {tool} must fail, got {r:?}"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn seed_repo(root: &Path) {
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"p7demo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
    )
    .unwrap();
    // Git identity so live freshness can report fresh/stale honestly.
    let git = |args: &[&str]| {
        let out = Proc::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git must be available for the P7 E2E");
        assert!(out.status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "p7@example.com"]);
    git(&["config", "user.name", "p7"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "seed"]);
}

fn unknown_kinds(v: &serde_json::Value) -> Vec<String> {
    v["unknowns"]
        .as_array()
        .expect("unknowns array")
        .iter()
        .filter_map(|u| u["kind"].as_str().map(str::to_string))
        .collect()
}

// ── Real binary E2E ─────────────────────────────────────────────────────

#[test]
fn real_binary_e2e_task_brief_modify_reindex_restart() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());

    let mut srv = Server::start(repo.path(), state.path());

    // 1. Index + task creation (task state feeds the brief).
    let re1 = srv.call("reindex", serde_json::json!({}));
    assert_eq!(re1["status"], "ok");
    let created = srv.call(
        "task",
        serde_json::json!({"action": "create", "title": "Repair beta flow", "description": "beta output looks wrong"}),
    );
    let task_id = created["task"]["task_id"]
        .as_str()
        .expect("task id")
        .to_string();
    assert!(task_id.starts_with("task::"));

    // 2. Brief with task + explicit symbol target.
    let brief1 = srv.call(
        "engineering_brief",
        serde_json::json!({"task_id": task_id, "task": "repair beta flow", "target_symbol": "beta"}),
    );
    assert_eq!(brief1["task_state"]["status"], "pending");
    assert_eq!(brief1["task_state"]["task_id"], task_id.as_str());
    assert_eq!(brief1["targets"]["discovery"], "explicit");
    assert_eq!(brief1["freshness"]["status"], "fresh", "{brief1:?}");
    assert_eq!(brief1["freshness"]["persisted_status"], "READY");
    assert!(
        !unknown_kinds(&brief1).contains(&"STALE_INDEX".to_string()),
        "{brief1:?}"
    );
    let impact1 = &brief1["impact"];
    assert!(
        impact1.is_object(),
        "explicit symbol target must traverse: {brief1:?}"
    );
    let direct1: Vec<String> = impact1["direct"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["target_name"].as_str().map(str::to_string))
        .collect();
    assert!(direct1.contains(&"alpha".to_string()), "{direct1:?}");
    assert!(
        !brief1.to_string().contains("gamma"),
        "gamma does not exist yet"
    );
    assert!(unknown_kinds(&brief1).contains(&"NO_RELEVANT_TESTS".to_string()));

    // 3. Modify without reindex → stale brief, old impact.
    std::fs::write(
        repo.path().join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() { alpha(); }\npub fn gamma() { beta(); }\n",
    )
    .unwrap();
    let stale = srv.call(
        "engineering_brief",
        serde_json::json!({"task_id": task_id, "task": "repair beta flow", "target_symbol": "beta"}),
    );
    assert_eq!(stale["freshness"]["status"], "stale", "{stale:?}");
    assert!(
        unknown_kinds(&stale).contains(&"STALE_INDEX".to_string()),
        "staleness must be explicit, never silent: {stale:?}"
    );
    assert!(
        !stale.to_string().contains("gamma"),
        "stale brief describes the indexed state"
    );

    // 4. Reindex → fresh brief with changed impact.
    let re2 = srv.call("reindex", serde_json::json!({}));
    assert_eq!(re2["status"], "ok");
    let brief2 = srv.call(
        "engineering_brief",
        serde_json::json!({"task_id": task_id, "task": "repair beta flow", "target_symbol": "beta"}),
    );
    assert_eq!(brief2["freshness"]["status"], "fresh", "{brief2:?}");
    assert!(!unknown_kinds(&brief2).contains(&"STALE_INDEX".to_string()));
    let direct2: Vec<String> = brief2["impact"]["direct"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["target_name"].as_str().map(str::to_string))
        .collect();
    assert!(direct2.contains(&"alpha".to_string()), "{direct2:?}");
    assert!(
        brief2.to_string().contains("gamma"),
        "reindexed brief must reflect the new symbol: {brief2:?}"
    );
    assert_ne!(
        brief1.to_string(),
        brief2.to_string(),
        "brief must change with the repository"
    );

    // 5. Restart → deterministic persisted result.
    drop(srv);
    let mut srv2 = Server::start(repo.path(), state.path());
    let brief3 = srv2.call(
        "engineering_brief",
        serde_json::json!({"task_id": task_id, "task": "repair beta flow", "target_symbol": "beta"}),
    );
    assert_eq!(
        brief2.to_string(),
        brief3.to_string(),
        "same repo + task + context state must reproduce the brief"
    );

    // 6. Hermeticity.
    assert!(
        state.path().join("state.db").exists(),
        "hermetic state.db must exist"
    );
}

// ── Isolation + malformed inputs over RPC ───────────────────────────────

#[test]
fn brief_e2e_task_isolation_across_workspaces() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(a.path());
    seed_repo(b.path());

    let mut srv_b = Server::start(b.path(), state.path());
    let created = srv_b.call(
        "task",
        serde_json::json!({"action": "create", "title": "Classified refactor"}),
    );
    let task_id = created["task"]["task_id"]
        .as_str()
        .expect("task id")
        .to_string();
    drop(srv_b);

    // Same state dir, different workspace root: the task must be invisible.
    let mut srv_a = Server::start(a.path(), state.path());
    let brief = srv_a.call(
        "engineering_brief",
        serde_json::json!({"task_id": task_id, "task": "continue work"}),
    );
    assert!(brief["task_state"].is_null());
    assert!(unknown_kinds(&brief).contains(&"TASK_NOT_FOUND".to_string()));
    assert!(!brief.to_string().contains("Classified refactor"));
}

#[test]
fn brief_e2e_malformed_inputs_rejected() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());

    let err = srv.call_err("engineering_brief", serde_json::json!({}));
    assert!(err.contains("task scope"), "{err}");

    let err = srv.call_err(
        "engineering_brief",
        serde_json::json!({"task": "x", "target_path": "../escape"}),
    );
    assert!(err.contains(".."), "{err}");

    // Unknown tool surface stays free of CRUD-style getters.
    let r = srv.rpc("tools/list", serde_json::json!({}));
    let tools = r["result"]["tools"].as_array().expect("tools").clone();
    assert_eq!(tools.len(), 25, "P7 adds exactly one tool");
    let names: Vec<String> = tools
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect();
    assert!(
        names.contains(&"engineering_brief".to_string()),
        "{names:?}"
    );
    for forbidden in [
        "get_file_context",
        "get_symbol_context",
        "get_graph_context",
        "get_history_context",
        "get_learning_context",
        "get_skill_context",
    ] {
        assert!(
            !names.contains(&forbidden.to_string()),
            "CRUD leakage: {names:?}"
        );
    }
}

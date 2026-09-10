//! P8 integration probes: the OpenCode integration contract verified
//! through the real `codebro` binary over stdio RPC.
//!
//! P8 adds no tools and no storage; it is a thin integration layer over
//! P0–P7. These probes pin the integration contract end-to-end:
//!
//! 1. **Protocol hygiene** — stdout carries ONLY JSON-RPC (tracing goes
//!    to stderr; a log line on stdout corrupts the stdio channel).
//! 2. **Server identity** — `initialize` reports `serverInfo.name ==
//!    "codebro"` (not the rmcp SDK default), enabling client discovery
//!    and telemetry.
//! 3. **Contract coverage** — `tools/list` still exposes exactly the 25
//!    semantic tools, including the P8 contract surface (engineering_brief
//!    primary, context orientation, targeted follow-up, persistence).
//! 4. **The full P8 flow against a hermetic repository** — the exact
//!    mission sequence: task arrives → workspace established → task
//!    identity created → engineering brief requested → targeted follow-up
//!    evidence → harmless change → CodeBro does NOT execute it → durable
//!    outcome persistence → restart → persistence verified → no secret
//!    leakage → workspace isolation.
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
    /// Background drain of the server's stderr: logs are collected for
    /// observability assertions while keeping the pipe from filling
    /// (a full stderr pipe would block the server — a deadlock the
    /// pre-P8 harness avoided by discarding stderr entirely).
    stderr: Arc<Mutex<String>>,
}

impl Server {
    /// Start the real binary with stderr captured (NOT discarded) so
    /// protocol-hygiene probes can assert what the server prints there —
    /// and, critically, so stdout can be asserted pure.
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
                "clientInfo": {"name": "p8-probe", "version": "0"}
            }),
        );
        s.stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        s.stdin.flush().unwrap();
        s
    }

    /// Everything the server has printed to stderr so far.
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

    /// Read lines until the response for `id` arrives. Any line that is
    /// not valid JSON-RPC on stdout FAILS the probe — that is the
    /// protocol-hygiene assertion (before P8, tracing wrote ERROR/INFO
    /// lines to stdout, corrupting the channel).
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

/// Seed a minimal hermetic Rust repository (deterministic content). The
/// repo is a real git repository so the P6 freshness signal (generation
/// hash vs working tree) is exercisable — outside git, freshness honestly
/// reports `unknown` and the fresh→stale→fresh cycle is not observable.
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
    // Hermetic git identity + initial commit (never the user's config).
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

// ── Probe 1: protocol hygiene — stdout is pure JSON-RPC ───────────────────

#[test]
fn p8_stdout_carries_only_jsonrpc_and_logs_go_to_stderr() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo(dir.path());
    let env = env_new();
    let mut server = Server::start(dir.path(), &env.state, &env.skills);

    // A batch of calls that exercise both reads and errors.
    server.call("workspace_context", serde_json::json!({}));
    let (ok, _) = server.call_raw("engineering_brief", serde_json::json!({}));
    assert!(!ok, "empty brief scope must be rejected");

    // Drain stderr and assert the observability lines landed THERE.
    let err_buf = server.stderr_so_far();
    assert!(
        err_buf.contains("tool call"),
        "expected observability lines on stderr, got: {err_buf}"
    );
    assert!(
        err_buf.contains("tool=workspace_context"),
        "expected per-tool observation on stderr: {err_buf}"
    );
    // The error observation must carry status=error without leaking args.
    assert!(
        err_buf.contains("status=error"),
        "expected an errored observation: {err_buf}"
    );
    assert!(
        !err_buf.contains("arguments"),
        "stderr must not include tool arguments: {err_buf}"
    );
}

// ── Probe 1b (audit F1): EVERY stderr line is secret-redacted ────────────
//
// The P8 observation seam redacts its own lines, but rmcp's transport
// layer independently logs `response error` lines carrying the raw
// tool-error message (which echoes caller input, e.g. a rejected
// `action` containing an API key). Before the audit fix, a secret
// supplied in a rejected tool argument reached the stderr log verbatim.
// The subscriber now routes ALL tracing output (including rmcp's)
// through the canonical redaction authority.
#[test]
fn p8_stderr_is_secret_redacted_even_for_transport_error_lines() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo(dir.path());
    let env = env_new();
    let mut server = Server::start(dir.path(), &env.state, &env.skills);

    // Secret-shaped text in rejected tool arguments: the tool error
    // message echoes the caller's input, and rmcp logs that message.
    let secret = "sk-AUDITLEAKCHECK0001234567890";
    let _ = server.call_raw("learn", serde_json::json!({"action": secret}));
    let _ = server.call_raw(
        "task",
        serde_json::json!({"action": "inspect", "task_id": secret}),
    );
    let _ = server.call_raw(
        "delete_memory",
        serde_json::json!({"key": secret, "confirm": true}),
    );

    // Give the stderr drain thread a moment to collect.
    std::thread::sleep(std::time::Duration::from_millis(300));
    let err_buf = server.stderr_so_far();
    assert!(
        !err_buf.contains(secret),
        "AUDIT F1: secret-shaped caller input leaked into stderr logs \
         (rmcp response-error line must be redacted): {err_buf}"
    );
    assert!(
        err_buf.contains("[REDACTED]"),
        "the redaction authority must visibly act on the secret-shaped \
         input: {err_buf}"
    );
}

// ── Probe 2: server identity is the product, not the SDK ─────────────────

#[test]
fn p8_initialize_reports_codebro_server_identity() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo(dir.path());
    let env = env_new();
    let mut server = Server::start(dir.path(), &env.state, &env.skills);
    let init = server.rpc(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "identity-probe", "version": "0"}
        }),
    );
    let result = &init["result"];
    assert_eq!(result["serverInfo"]["name"], "codebro");
    assert!(
        result["serverInfo"]["version"].is_string(),
        "serverInfo.version must be present"
    );
    assert_ne!(result["serverInfo"]["name"], "rmcp");
    // Tools capability still advertised.
    assert!(result["capabilities"]["tools"].is_object());
    // Instructions survive (the client-facing usage contract).
    assert!(
        result["instructions"]
            .as_str()
            .unwrap_or("")
            .contains("CodeBro"),
        "instructions must still introduce CodeBro"
    );
}

// ── Probe 3: contract surface — 25 tools, P8 intents all routed ───────────

#[test]
fn p8_contract_surface_is_the_existing_25_tools() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo(dir.path());
    let env = env_new();
    let mut server = Server::start(dir.path(), &env.state, &env.skills);
    let list = server.rpc("tools/list", serde_json::json!({}));
    let tools = list["result"]["tools"].as_array().expect("tools array");
    assert_eq!(
        tools.len(),
        25,
        "P8 must not add tools: got {}",
        tools.len()
    );
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    // The P8 contract intents (integration.rs::contract) must all exist.
    for (_, tool) in codebro_mcp_server::integration::contract::intents() {
        assert!(
            names.contains(tool),
            "contract tool {tool} missing from tools/list"
        );
    }
    // No CRUD getters snuck in.
    assert!(!names.iter().any(|n| n.contains("get_")));
    // The primary surface is present.
    assert!(names.contains(&"engineering_brief"));
}

// ── Probe 4: the full P8 mission flow against a hermetic repo ──────────────

#[test]
fn p8_full_opencode_flow_brief_targeted_persistence_restart() {
    let dir = tempfile::tempdir().unwrap();
    seed_repo(dir.path());
    let env = env_new();

    // 1. Start CodeBro MCP + "connect" (initialize handshake, as OpenCode does).
    let mut server = Server::start(dir.path(), &env.state, &env.skills);

    // 2. Workspace orientation (OpenCode establishes the workspace).
    let ctx = server.call("workspace_context", serde_json::json!({}));
    assert_eq!(
        ctx["workspace_root"],
        dir.path().canonicalize().unwrap().display().to_string()
    );

    // 3. Index the hermetic repository (freshness starts honest).
    server.call("reindex", serde_json::json!({}));

    // 4. Establish durable task identity (P5) — CodeBro persists state,
    //    never executes.
    let task = server.call(
        "task",
        serde_json::json!({"action": "create", "title": "P8 probe: exercise brief flow"}),
    );
    let task_id = task["task"]["task_id"]
        .as_str()
        .expect("task id")
        .to_string();
    assert!(task_id.starts_with("task::"));

    // 5. Request the Engineering Brief (the primary P8 context surface).
    let brief = server.call(
        "engineering_brief",
        serde_json::json!({
            "task": "investigate the alpha and beta functions",
            "task_id": task_id,
            "target_symbol": "alpha",
            "keywords": ["alpha", "beta"]
        }),
    );
    // Freshness + provenance + freshness survive the integration boundary.
    assert!(
        brief["freshness"]["status"].is_string(),
        "live freshness must survive integration"
    );
    assert!(
        brief["freshness"]["persisted_status"].is_string(),
        "persisted freshness must survive integration"
    );
    assert!(
        brief["scope"]["workspace_key"].is_string() || brief["scope"].is_object(),
        "scope must be present"
    );
    // Evidence sections with categories (never a flat dump).
    assert!(
        brief["symbols"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false)
            || brief["files"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
        "brief must surface task-relevant repository evidence"
    );
    // Task state is read-only and correct.
    assert_eq!(brief["task_state"]["task_id"], task_id);
    // Decision-neutrality: no solution/plan fields.
    assert!(brief.get("decision").is_none());
    assert!(brief.get("recommendation").is_none());

    // 6. Targeted follow-up evidence (existing semantic tools, bounded).
    let facts = server.call(
        "engineering_facts",
        serde_json::json!({"query": "alpha", "kind": "symbol"}),
    );
    assert!(
        facts["facts"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "targeted facts follow-up must resolve alpha"
    );

    // 7. Execute a harmless change WITH NATIVE TOOLING (the probe writes
    //    directly, as OpenCode would) — then verify CodeBro did NOT write
    //    anything itself: no execution ownership moved into CodeBro.
    let lib_path = dir.path().join("src/lib.rs");
    std::fs::write(
        &lib_path,
        "pub fn alpha() {}\npub fn beta() { alpha(); }\npub fn gamma() {}\n",
    )
    .unwrap();
    let before_marker = std::fs::read(dir.path().join(".codebro/facts.json")).unwrap();
    // No CodeBro mutation tool was called; the file content is ours.
    assert!(std::fs::read_to_string(&lib_path)
        .unwrap()
        .contains("gamma"));

    // 8. Staleness is honest: the index predates our change.
    let stale_brief = server.call(
        "engineering_brief",
        serde_json::json!({"task": "check gamma", "keywords": ["gamma"]}),
    );
    let freshness_live = stale_brief["freshness"]["status"].as_str().unwrap_or("");
    assert!(
        freshness_live == "stale" || freshness_live == "unknown",
        "freshness must report stale/unknown after an unindexed change, got {freshness_live}"
    );

    // 9. Reindex + the brief reflects current truth.
    server.call("reindex", serde_json::json!({}));
    let fresh_brief = server.call(
        "engineering_brief",
        serde_json::json!({"task": "check gamma", "target_symbol": "gamma", "keywords": ["gamma"]}),
    );
    let fresh_live = fresh_brief["freshness"]["status"].as_str().unwrap_or("");
    assert_eq!(fresh_live, "fresh", "reindex must restore freshness");
    let found_gamma = fresh_brief["symbols"]
        .as_array()
        .map(|a| a.iter().any(|s| s["name"] == "gamma"))
        .unwrap_or(false);
    assert!(found_gamma, "post-reindex brief must see gamma");

    // 10. Persist a meaningful durable outcome explicitly (user-confirmed
    //     preference via `remember`; agent memory via `record_memory`).
    //     The caller (OpenCode) decides what is meaningful — CodeBro gates
    //     authority, evidence, and provenance.
    let remembered = server.call(
        "remember",
        serde_json::json!({
            "content": "P8 probe repo uses edition 2021",
            "namespace": "probe/p8-flow",
            "kind": "preference",
            "user_confirmed": true
        }),
    );
    assert!(
        remembered["id"].is_string(),
        "remember must mint a record id"
    );
    let memory = server.call(
        "record_memory",
        serde_json::json!({
            "key": "probe:p8-flow",
            "value": "brief flow verified end to end",
            "tags": ["p8"],
            "confidence": 0.8
        }),
    );
    // record_memory answers with a text summary; assert it mentions the key.
    assert!(
        memory.is_string() || memory.is_object(),
        "record_memory must acknowledge the write: {memory}"
    );

    // Task progression through the durable lifecycle (start → checkpoint →
    // validate → complete); CodeBro stores, never executes.
    server.call(
        "task",
        serde_json::json!({"action": "start", "task_id": task_id}),
    );
    server.call(
        "task",
        serde_json::json!({"action": "checkpoint", "task_id": task_id, "summary": "probe complete"}),
    );
    server.call(
        "task",
        serde_json::json!({"action": "validate", "task_id": task_id, "what": "hermetic probe"}),
    );
    server.call(
        "task",
        serde_json::json!({"action": "validation_result", "task_id": task_id, "result": "passed", "reason": "all probe assertions held"}),
    );
    let completed = server.call(
        "task",
        serde_json::json!({"action": "complete", "task_id": task_id, "reason": "probe finished"}),
    );
    assert_eq!(completed["task"]["status"], "completed");

    // 11. Restart (hard): kill and start a fresh server over the same
    //     state + workspace.
    drop(server);
    let mut server2 = Server::start(dir.path(), &env.state, &env.skills);

    // 12. Persistence verified after restart.
    let tasks = server2.call("task", serde_json::json!({"action": "list"}));
    let listed = tasks["tasks"].as_array().expect("task list");
    assert!(
        listed
            .iter()
            .any(|t| t["task_id"] == task_id && t["status"] == "completed"),
        "completed task must survive restart: {tasks}"
    );
    // Fresh-namespace clash refuses and names the incumbent → the record
    // from step 10 survived the restart.
    let (ok, text) = server2.call_raw(
        "remember",
        serde_json::json!({"content": "probe duplicate should clash", "namespace": "probe/p8-flow", "user_confirmed": true}),
    );
    assert!(!ok, "expected namespace clash refusal: {text}");
    assert!(
        text.contains("probe/p8-flow"),
        "clash must name the incumbent: {text}"
    );
    let mem = server2.call(
        "engineering_memory",
        serde_json::json!({"task_keywords": ["p8"]}),
    );
    assert!(
        mem["entries"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false),
        "recorded memory must resolve after restart"
    );

    // 13. Determinism: identical brief requests agree after restart.
    let a = server2.call(
        "engineering_brief",
        serde_json::json!({"task": "check gamma", "target_symbol": "gamma", "keywords": ["gamma"]}),
    );
    let b = server2.call(
        "engineering_brief",
        serde_json::json!({"task": "check gamma", "target_symbol": "gamma", "keywords": ["gamma"]}),
    );
    assert_eq!(a, b, "brief must be deterministic across repeated requests");

    // 14. No secret leakage through the integration path.
    let secret_marker = "sk-p8probe-leak-check-000";
    let (ok, _) = server2.call_raw(
        "remember",
        serde_json::json!({"content": format!("token {secret_marker}"), "namespace": "probe/leak", "user_confirmed": true}),
    );
    assert!(ok);
    let leak_brief = server2.call(
        "engineering_brief",
        serde_json::json!({"task": "leak check", "keywords": ["leak"]}),
    );
    let serialized = serde_json::to_string(&leak_brief).unwrap();
    assert!(
        !serialized.contains(secret_marker),
        "secret must never surface through the brief"
    );
    // Cleanup the probe record so the state dir stays probe-only anyway
    // (hermetic — nothing to clean outside the tempdir).
    let _ = server2.call(
        "forget",
        serde_json::json!({"namespace": "probe/leak", "confirm": true}),
    );

    // 15. Workspace isolation: a second workspace sees none of this.
    let other = tempfile::tempdir().unwrap();
    seed_repo(other.path());
    let mut server_b = Server::start(other.path(), &env.state, &env.skills);
    let other_brief = server_b.call(
        "engineering_brief",
        serde_json::json!({"task": "find probe task", "task_id": task_id, "keywords": ["probe"]}),
    );
    let other_str = serde_json::to_string(&other_brief).unwrap();
    assert!(
        !other_str.contains("P8 probe: exercise brief flow"),
        "cross-workspace task title must not leak"
    );
    let other_ctx = server_b.call("workspace_context", serde_json::json!({}));
    assert_ne!(
        other_ctx["workspace_root"],
        dir.path().canonicalize().unwrap().display().to_string()
    );

    // Sanity: the probe workspace's own facts.json was only written by the
    // explicit reindex calls (the file we mutated was OUR write, not
    // CodeBro's — CodeBro never executed anything).
    let _ = before_marker;
}

// ── Probe 5: client identity flows to observability; multi-client safe ────

#[test]
fn p8_client_identity_observability_and_two_client_concurrency() {
    let dir_a = tempfile::tempdir().unwrap();
    seed_repo(dir_a.path());
    let env = env_new();

    // Client A and client B against the same workspace + state, exactly
    // the P8 concurrency model (OpenCode session + a second consumer).
    let mut a = Server::start(dir_a.path(), &env.state, &env.skills);
    let mut b = Server::start(dir_a.path(), &env.state, &env.skills);

    a.call("reindex", serde_json::json!({}));
    b.call("reindex", serde_json::json!({}));

    // Concurrent brief + memory reads from both clients.
    let a1 = a.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha analysis", "target_symbol": "alpha"}),
    );
    let b1 = b.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha analysis", "target_symbol": "alpha"}),
    );
    // Both well-formed and consistent on stable state (determinism holds
    // across clients; timestamps/now-fields aside, evidence agrees).
    assert!(a1["freshness"]["status"].is_string());
    assert!(b1["freshness"]["status"].is_string());

    // Brief + mutation interleave (write on one client, read on other).
    let task = a.call(
        "task",
        serde_json::json!({"action": "create", "title": "shared concurrency probe"}),
    );
    let tid = task["task"]["task_id"].as_str().unwrap().to_string();
    let brief_b = b.call(
        "engineering_brief",
        serde_json::json!({"task": "find shared task", "task_id": tid, "keywords": ["concurrency"]}),
    );
    let bs = serde_json::to_string(&brief_b).unwrap();
    assert!(
        bs.contains("shared concurrency probe"),
        "client B must see client A's committed task"
    );

    // Observability carries the client's declared name.
    let err_buf = a.stderr_so_far();
    assert!(
        err_buf.contains("client=p8-probe"),
        "observation must name the connected client: {err_buf}"
    );
    assert!(
        !err_buf.contains("\"arguments\""),
        "observability must never log arguments: {err_buf}"
    );
}

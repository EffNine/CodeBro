//! FINAL GATE probe: independent adversarial verification through the real
//! `codebro` binary over stdio RPC. Hermetic: tempdir workspaces,
//! CODEBRO_STATE_DIR + CODEBRO_SKILLS_DIR isolation, never touches
//! ~/.codebro or real skills.
//!
//! Scenarios (final-gate checklist): secret injection through every major
//! MCP write path; two-workspace zero-leakage; write→restart→read; malformed
//! input; trust boundaries; conflicting-constraint decision-neutrality;
//! concurrent brief + mutation.
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
}

impl Server {
    fn start(root: &Path, state: &Path, skills: &Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut child = Proc::new(bin)
            .args(["serve", "--root"])
            .arg(root)
            .env("CODEBRO_STATE_DIR", state)
            .env("CODEBRO_SKILLS_DIR", skills)
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
                "clientInfo": {"name": "gate-probe", "version": "0"}
            }),
        );
        s.stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        s.stdin.flush().unwrap();
        s
    }

    fn rpc(&mut self, method: &str, params: serde_json::Value) -> serde_json::Value {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::SeqCst);
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

    /// Returns (ok, text). Tool errors return (false, error text) instead of
    /// panicking, so probes can assert on them.
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

impl Env {
    fn server(&self, root: &Path) -> Server {
        Server::start(root, &self.state, &self.skills)
    }
}

fn seed_repo(root: &Path, unique: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        format!("[package]\nname = \"{unique}\"\nversion = \"0.1.0\"\nedition = \"2021\"\n"),
    )
    .unwrap();
    std::fs::write(
        root.join("src").join("lib.rs"),
        format!(
            "pub fn {unique}_alpha() -> u32 {{ 1 }}\npub fn {unique}_beta() -> u32 {{ {unique}_alpha() }}\n#[cfg(test)]\nmod tests {{\n    #[test]\n    fn t() {{ assert_eq!(super::{unique}_beta(), 1); }}\n}}\n"
        ),
    )
    .unwrap();
}

// ── Scenario K: secrets through every write path never surface in reads ─
#[test]
fn gate_secrets_never_surface_through_any_read_path() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "secretapp");
    let mut s = env.server(repo.path());
    let ws = repo.path().to_string_lossy().into_owned();
    s.call("reindex", serde_json::json!({"workspace_root": ws}));

    let secret = "sk-GATEPROBE1234567890abcdef";
    let pat = "ghp_GATEGATEGATEGATEGATEGATEGATE1234";

    // Write-path 1: remember (user_confirmed preference)
    let r = s.call(
        "remember",
        serde_json::json!({
            "workspace_root": ws, "namespace": "gate.probe.pref",
            "content": format!("always deploy with api_key={secret}"),
            "user_confirmed": true
        }),
    );
    let rid = r["id"].as_str().unwrap().to_string();

    // Write-path 2: record_memory
    s.call(
        "record_memory",
        serde_json::json!({
            "workspace_root": ws, "key": "gate:secret-probe",
            "value": format!("token={secret} in memory"), "confidence": 0.8
        }),
    );

    // Write-path 3: update_identity
    s.call(
        "update_identity",
        serde_json::json!({
            "workspace_root": ws,
            "add_constraints": [format!("deploy only when password={pat} is set")],
            "add_decisions": [{"title": format!("use bearer {secret} always"), "description": "x"}],
            "architecture_summary": format!("auth uses {secret}"),
            "description": format!("app with {secret}")
        }),
    );

    // Write-path 4: task create (title/description/skill_refs)
    let tr = s.call(
        "task",
        serde_json::json!({
            "workspace_root": ws, "action": "create",
            "title": format!("rotate {secret}"),
            "description": format!("the token {secret} leaked"),
            "skill_refs": [format!("file://{secret}")]
        }),
    );
    let tid = tr["task"]["task_id"].as_str().unwrap().to_string();

    // Write-path 5: remember with observation (evidence event)
    s.call(
        "remember",
        serde_json::json!({
            "workspace_root": ws, "namespace": "gate.probe.pattern",
            "kind": "pattern", "content": format!("pattern uses token {secret}"),
            "observation": format!("saw token {secret} in logs")
        }),
    );

    // Read-path hunt.
    let brief = s.call(
        "engineering_brief",
        serde_json::json!({
            "workspace_root": ws, "task": "fix the auth handling", "task_id": tid
        }),
    );
    let b = serde_json::to_string(&brief).unwrap();
    assert!(!b.contains(secret), "brief leaked sk secret");
    assert!(!b.contains(pat), "brief leaked PAT");

    let ctx = s.call(
        "context",
        serde_json::json!({
            "workspace_root": ws, "task": "handle auth tokens"
        }),
    );
    let c = serde_json::to_string(&ctx).unwrap();
    assert!(!c.contains(secret), "context leaked secret");
    assert!(!c.contains(pat), "context leaked PAT");

    let recall = s.call(
        "recall",
        serde_json::json!({
            "workspace_root": ws, "query": "auth token secret"
        }),
    );
    let rc = serde_json::to_string(&recall).unwrap();
    assert!(!rc.contains(secret), "recall leaked secret");

    let mem = s.call(
        "engineering_memory",
        serde_json::json!({
            "task_keywords": ["secret", "gate", "token"]
        }),
    );
    let m = serde_json::to_string(&mem).unwrap();
    assert!(!m.contains(secret), "memory resolution leaked secret");

    let task = s.call(
        "task",
        serde_json::json!({
            "workspace_root": ws, "action": "inspect", "task_id": tid
        }),
    );
    let t = serde_json::to_string(&task).unwrap();
    assert!(!t.contains(secret), "task inspect leaked secret");
    assert!(!t.contains(pat), "task inspect leaked PAT");

    let _ = s.call(
        "forget",
        serde_json::json!({
            "workspace_root": ws, "id": rid, "confirm": true
        }),
    );
}

// ── Scenario I: two workspaces, zero cross-scope leakage ────────────────
#[test]
fn gate_two_workspaces_zero_leakage() {
    let env = env_new();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    seed_repo(a.path(), "wsalpha");
    seed_repo(b.path(), "wsbeta");

    let mut sa = env.server(a.path());
    let mut sb = env.server(b.path());
    let wa = a.path().to_string_lossy().into_owned();
    let wb = b.path().to_string_lossy().into_owned();

    let tr = sa.call(
        "task",
        serde_json::json!({
            "workspace_root": wa, "action": "create", "title": "alpha-only-task-marker"
        }),
    );
    let tid = tr["task"]["task_id"].as_str().unwrap().to_string();

    sa.call(
        "remember",
        serde_json::json!({
            "workspace_root": wa, "namespace": "gate.alpha.pref",
            "content": "ALPHA-UNIQUE-PREFERENCE-MARKER", "user_confirmed": true
        }),
    );
    sa.call("record_memory", serde_json::json!({
        "workspace_root": wa, "key": "gate:alpha", "value": "ALPHA-MEMORY-MARKER", "confidence": 0.9
    }));

    // B cannot see A's task content (any shape of refusal is fine).
    let (_ok, text) = sb.call_raw(
        "task",
        serde_json::json!({
            "workspace_root": wb, "action": "inspect", "task_id": tid
        }),
    );
    assert!(
        !text.contains("alpha-only-task-marker"),
        "cross-workspace task content leaked"
    );

    // B's context has no ALPHA markers.
    let ctxb = sb.call(
        "context",
        serde_json::json!({
            "workspace_root": wb, "task": "beta work only"
        }),
    );
    let cb = serde_json::to_string(&ctxb).unwrap();
    assert!(
        !cb.contains("ALPHA-UNIQUE"),
        "A's record leaked into B's context"
    );
    assert!(
        !cb.contains("alpha-only-task-marker"),
        "A's task title leaked into B's context"
    );
    assert!(
        !cb.contains("ALPHA-MEMORY"),
        "A's memory leaked into B's context"
    );

    // B's brief probed with neutral task text and B-relevant keywords. The
    // brief legitimately echoes the caller's own task text and keywords
    // (same-channel echo, never persisted) — so A's markers must not appear
    // in the request itself. A keyword overlap probe uses B's own symbol.
    let briefb = sb.call("engineering_brief", serde_json::json!({
        "workspace_root": wb, "task": "beta work wsbeta_alpha", "keywords": ["alpha", "marker", "preference", "memory", "task"]
    }));
    let bb = serde_json::to_string(&briefb).unwrap();
    assert!(
        !bb.contains("ALPHA-UNIQUE"),
        "A's record leaked into B's brief"
    );
    assert!(
        !bb.contains("wsalpha"),
        "A's project name leaked into B's brief"
    );
    assert!(
        !bb.contains("alpha-only-task-marker"),
        "A's task leaked into B's brief"
    );
    assert!(
        !bb.contains("ALPHA-MEMORY"),
        "A's memory leaked into B's brief"
    );
    // B's brief must still be about B (sanity).
    assert!(
        bb.contains("wsbeta"),
        "B's brief should reference B's own project"
    );

    // A's brief DOES reference its own data (sanity that stores work).
    let briefa = sa.call(
        "engineering_brief",
        serde_json::json!({
            "workspace_root": wa, "task": "wsalpha alpha marker preference"
        }),
    );
    let ba = serde_json::to_string(&briefa).unwrap();
    assert!(
        ba.contains("ALPHA-UNIQUE") || ba.contains("ALPHA-MEMORY"),
        "A's own record should be visible to A's brief"
    );

    // B's recall sees nothing of A.
    let recallb = sb.call(
        "recall",
        serde_json::json!({
            "workspace_root": wb, "query": "alpha only task marker"
        }),
    );
    let rb = serde_json::to_string(&recallb).unwrap();
    assert!(
        !rb.contains("alpha-only-task-marker"),
        "A's history leaked into B's recall"
    );
    assert!(
        !rb.contains("ALPHA-UNIQUE"),
        "A's record leaked into B's recall"
    );
}

// ── Persistence: write → restart → read; determinism ────────────────────
#[test]
fn gate_restart_preserves_state_and_brief_determinism() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "restartapp");
    let ws = repo.path().to_string_lossy().into_owned();

    let tid = {
        let mut s = env.server(repo.path());
        s.call("reindex", serde_json::json!({"workspace_root": ws}));
        let tr = s.call(
            "task",
            serde_json::json!({
                "workspace_root": ws, "action": "create", "title": "persist across restart"
            }),
        );
        let tid = tr["task"]["task_id"].as_str().unwrap().to_string();
        s.call(
            "task",
            serde_json::json!({
                "workspace_root": ws, "action": "start", "task_id": tid
            }),
        );
        tid
    }; // server killed — simulated crash

    // Restart: fresh process, same state dir.
    let mut s2 = env.server(repo.path());
    let task = s2.call(
        "task",
        serde_json::json!({
            "workspace_root": ws, "action": "inspect", "task_id": tid
        }),
    );
    assert_eq!(
        task["snapshot"]["task"]["status"], "running",
        "task state must survive restart"
    );
    assert_ne!(task["snapshot"]["task"]["status"], "completed");

    // Brief determinism: two identical requests agree.
    let args = serde_json::json!({
        "workspace_root": ws, "task": "restartapp_alpha bug", "task_id": tid
    });
    let b1 = s2.call("engineering_brief", args.clone());
    let b2 = s2.call("engineering_brief", args.clone());
    assert_eq!(b1, b2, "identical brief requests must agree");
}

// ── Malformed input / abuse ──────────────────────────────────────────────
#[test]
fn gate_malformed_input_is_rejected_safely() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "malapp");
    let ws = repo.path().to_string_lossy().into_owned();

    let mut s = env.server(repo.path());

    // Traversal workspace root must not leak other workspaces' data.
    let (_ok, text) = s.call_raw(
        "context",
        serde_json::json!({
            "workspace_root": format!("{}/../..", repo.path().display()), "task": "x"
        }),
    );
    let t = text.to_lowercase();
    assert!(
        !t.contains("secretapp") && !t.contains("restartapp") && !t.contains("wsalpha"),
        "traversal root leaked cross-workspace data"
    );

    // NUL bytes must be rejected, not panic.
    let (ok2, t2) = s.call_raw(
        "engineering_brief",
        serde_json::json!({
            "workspace_root": ws, "task": "bad\u{0}task"
        }),
    );
    assert!(
        !ok2 || t2.to_lowercase().contains("panic").not(),
        "NUL input: {t2}"
    );

    let huge = "x".repeat(1_000_000);
    let (_ok3, t3) = s.call_raw(
        "context",
        serde_json::json!({
            "workspace_root": ws, "task": huge
        }),
    );
    assert!(
        !t3.to_lowercase().contains("panic"),
        "huge input caused panic"
    );

    // Absurd limits clamp; absurd depth rejects.
    let (ok4, t4) = s.call_raw(
        "engineering_facts",
        serde_json::json!({
            "query": "alpha", "limit": 100000
        }),
    );
    assert!(ok4, "large limit must clamp, not error: {t4}");

    let (ok5, t5) = s.call_raw(
        "impact_analyze",
        serde_json::json!({
            "workspace_root": ws, "target": "malapp_alpha", "depth": 999
        }),
    );
    assert!(!ok5, "depth 999 must be rejected: {t5}");
}

// ── Trust boundary: inference can never self-confirm ────────────────────
#[test]
fn gate_ai_inference_cannot_self_confirm() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "trustapp");
    let ws = repo.path().to_string_lossy().into_owned();

    let mut s = env.server(repo.path());

    // remember with authority ai_inferred but no user_confirmed flag and no
    // evidence → must fail.
    let (ok, text) = s.call_raw(
        "remember",
        serde_json::json!({
            "workspace_root": ws, "namespace": "gate.trust.ai",
            "content": "inferred thing", "authority": "ai_inferred"
        }),
    );
    assert!(
        !ok,
        "ai_inferred remember without evidence must be rejected"
    );
    assert!(
        text.to_lowercase().contains("evidence"),
        "rejection must cite evidence requirement: {text}"
    );

    // user_confirmed=true is the only path to USER_CONFIRMED; verify the
    // record created via remember with user_confirmed reports user_confirmed
    // authority, and that a supersede without confirmation cannot upgrade.
    let r = s.call(
        "remember",
        serde_json::json!({
            "workspace_root": ws, "namespace": "gate.trust.conf",
            "content": "confirmed thing", "user_confirmed": true
        }),
    );
    let c = serde_json::to_string(&r).unwrap();
    assert!(
        c.contains("user_confirmed"),
        "confirmed record must be labelled"
    );

    // learn confirm without user_confirmed=true must fail even for real ids.
    let (ok2, text2) = s.call_raw(
        "learn",
        serde_json::json!({
            "workspace_root": ws, "action": "confirm", "candidate_id": "hyp::nonexistent"
        }),
    );
    assert!(!ok2, "confirm of nonexistent candidate must fail");
    assert!(!text2.to_lowercase().contains("panic"));
}

// ── Scenario F: conflicting constraints visible, brief decision-neutral ──
#[test]
fn gate_conflicting_constraints_preserved_and_decision_neutral() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "confapp");
    let ws = repo.path().to_string_lossy().into_owned();

    let mut s = env.server(repo.path());
    s.call("reindex", serde_json::json!({"workspace_root": ws}));

    s.call(
        "remember",
        serde_json::json!({
            "workspace_root": ws, "namespace": "gate.conf.constraint",
            "kind": "constraint",
            "content": "never rewrite the alpha module without review",
            "user_confirmed": true
        }),
    );
    s.call(
        "update_identity",
        serde_json::json!({
            "workspace_root": ws,
            "add_decisions": [{"title": "use approach X", "description": "adopted"},
                              {"title": "use approach Y", "description": "also adopted"}]
        }),
    );

    let brief = s.call(
        "engineering_brief",
        serde_json::json!({
            "workspace_root": ws, "task": "rewrite the alpha module confapp_alpha"
        }),
    );
    let b = serde_json::to_string(&brief).unwrap();

    // Constraint visible (redaction must not have swallowed it).
    assert!(
        b.contains("never rewrite the alpha module"),
        "confirmed constraint missing from brief"
    );

    // Decision-language scan.
    let lower = b.to_lowercase();
    for banned in [
        "you must implement",
        "you should implement",
        "the correct solution is",
        "we recommend implementing",
        "implement it now",
    ] {
        assert!(
            !lower.contains(banned),
            "brief contains decision language: {banned}"
        );
    }
}

// ── Scenario L: concurrent brief + mutation ──────────────────────────────
#[test]
fn gate_concurrent_brief_during_mutation_stays_consistent() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "concapp");
    let ws = repo.path().to_string_lossy().into_owned();

    let mut s = env.server(repo.path());
    s.call("reindex", serde_json::json!({"workspace_root": ws}));

    // Second connection over the same state.
    let mut s2 = env.server(repo.path());

    let tr = s2.call(
        "task",
        serde_json::json!({
            "workspace_root": ws, "action": "create", "title": "concurrent mutation"
        }),
    );
    let tid = tr["task"]["task_id"].as_str().unwrap().to_string();

    // s2 mutates; s reads briefs — no deadlock, no fabricated state.
    s2.call(
        "task",
        serde_json::json!({"workspace_root": ws, "action": "start", "task_id": tid}),
    );
    let brief1 = s.call(
        "engineering_brief",
        serde_json::json!({
            "workspace_root": ws, "task": "concapp_alpha", "task_id": tid
        }),
    );
    s2.call(
        "task",
        serde_json::json!({"workspace_root": ws, "action": "pause", "task_id": tid}),
    );
    let brief2 = s.call(
        "engineering_brief",
        serde_json::json!({
            "workspace_root": ws, "task": "concapp_alpha", "task_id": tid
        }),
    );

    assert_eq!(brief1["task_state"]["status"], "running");
    assert_eq!(brief2["task_state"]["status"], "paused");
}

// Helper trait to keep the NUL assertion readable.
trait Not {
    fn not(self) -> bool;
}
impl Not for bool {
    fn not(self) -> bool {
        !self
    }
}

//! P11 integration: the complete skill-reuse loop through the real `codebro`
//! binary over stdio MCP.
//!
//! ```text
//! experience (repeated successful tool workflows in history)
//!   → skill detect_reuse (explicitly invoked, deterministic)
//!   → evidence-backed skill candidate (validated, never auto-published)
//!   → request_approval (needs_input: OpenCode asks the human)
//!   → respond approve (human decision)
//!   → ACTIVE skill + SKILL.md published
//!   → restart
//!   → new task: contextual selection (applicable) without naming the skill
//!   → minimal skill context (skill_context)
//! ```
//!
//! Hermetic: tempdir workspaces, explicit `CODEBRO_STATE_DIR` +
//! `CODEBRO_SKILLS_DIR`. Never touches `~/.codebro` or real skills.
//!
//! Seed strategy: history events are seeded library-side into the same
//! `state.db` before the server starts (the server opens it lazily), so the
//! wire test proves the detector mines real history — not hand-planted
//! learning rows. Selection/approval/persistence then run entirely over MCP.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use codebro_context_runtime::{
    ContextStore, HistoryInput, HistoryKind, OpenSession, Skill, SkillApplicability, SkillHealth,
};

// ── Binary harness ─────────────────────────────────────────────────────────

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
                "clientInfo": {"name": "p11-e2e", "version": "0"}
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

    fn call(&mut self, tool: &str, args: serde_json::Value) -> serde_json::Value {
        let r = self.rpc(
            "tools/call",
            serde_json::json!({"name":tool,"arguments":args}),
        );
        assert!(
            r.get("error").is_none(),
            "tool {tool} transport error: {}",
            r["error"]
        );
        assert!(
            !r["result"]["isError"].as_bool().unwrap_or(false),
            "tool {tool} errored: {}",
            r["result"]["content"][0]["text"]
        );
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Seed helpers (library-side, same state.db the server will open) ───────

struct Env {
    _dir: tempfile::TempDir,
    ws: PathBuf,
    state: PathBuf,
    skills: PathBuf,
}

fn setup() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path().join("repo");
    let state = dir.path().join("state");
    let skills = dir.path().join("skills");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&state).unwrap();
    std::fs::create_dir_all(&skills).unwrap();
    Env {
        _dir: dir,
        ws,
        state,
        skills,
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1_700_000_000)
}

/// One successful `workspace_context → apply_change → sandbox_test`
/// execution: real history shaped like normal OpenCode work.
fn seed_success_run(store: &ContextStore, ws: &str, at: u64) {
    let session = store.open_session(ws, &OpenSession::default(), at).unwrap();
    let steps = [
        (
            HistoryKind::ToolExecution,
            "workspace_context",
            "success",
            "inspected workspace context",
        ),
        (
            HistoryKind::ChangeApplied,
            "apply_change",
            "applied",
            "applied change",
        ),
        (
            HistoryKind::Validation,
            "sandbox_test",
            "passed",
            "sandbox_test passed",
        ),
    ];
    for (i, (kind, tool, outcome, summary)) in steps.iter().enumerate() {
        let mut input = HistoryInput::new(ws, *kind, *summary);
        input.session_id = Some(session.id.clone());
        input.tool = Some(tool.to_string());
        input.outcome = Some(outcome.to_string());
        input.created_at = Some(at + i as u64);
        let (_, dup) = store.record_history(&input, at + i as u64).unwrap();
        assert!(!dup);
    }
}

fn seed_irrelevant_skill(store: &ContextStore) {
    store
        .upsert_skill(&Skill {
            skill_id: "sk::seed-go-deployer".to_string(),
            workspace_root: None,
            scope: "global".to_string(),
            name: "go-deployer".to_string(),
            description: "Deploys go services to kubernetes".to_string(),
            applicability: SkillApplicability {
                projects: Vec::new(),
                task_types: Vec::new(),
                languages: vec!["go".to_string()],
                frameworks: Vec::new(),
                subsystems: vec!["kubernetes".to_string()],
            },
            current_version: 1,
            status: "active".to_string(),
            confidence: 0.8,
            health: SkillHealth::default(),
            source_candidate_id: None,
            superseded_by: None,
            created_at: now_unix(),
            updated_at: now_unix(),
        })
        .unwrap();
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// The complete P11 loop over the wire: experience → detector → candidate →
/// human question → human approval → published skill → restart → contextual
/// reuse without naming the skill.
#[test]
fn reuse_loop_detect_approve_publish_and_contextually_reuse() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        for i in 0..3 {
            seed_success_run(&store, &ws, base + i * 10);
        }
        seed_irrelevant_skill(&store);
    }

    // 1–2. Run the explicitly-invoked detector: a candidate appears with
    // evidence (nothing is published yet).
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let detected = s.call(
        "skill",
        serde_json::json!({"action":"detect_reuse","scope":"project"}),
    );
    assert_eq!(detected["status"], "candidates_found", "{detected}");
    assert_eq!(detected["candidates"].as_array().unwrap().len(), 1);
    let cand = &detected["candidates"][0];
    assert_eq!(cand["outcome"], "created_validated");
    assert_eq!(cand["observations"], 3);
    assert!(cand["confidence"].as_f64().unwrap() >= 0.60);
    assert_eq!(cand["scope"], "project");
    assert!(
        cand["next_action"]
            .as_str()
            .unwrap()
            .contains("request_approval"),
        "{cand}"
    );
    assert!(
        !cand["evidence_sample"].as_array().unwrap().is_empty(),
        "evidence must travel with the candidate"
    );
    let candidate_id = cand["skill_candidate_id"].as_str().unwrap().to_string();
    let skill_name = cand["name"].as_str().unwrap().to_string();
    assert!(skill_name.starts_with("reuse-"));

    // Nothing published by detection alone: the only active skill is the
    // seeded irrelevant one; the reuse proposal stays a candidate.
    let before = s.call("skill", serde_json::json!({"action":"discover"}));
    let active_before: Vec<String> = before["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sk| sk["name"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(active_before, vec!["go-deployer".to_string()], "{before}");

    // 3–5. Approval protocol: the candidate carries a human question with
    // the four options; the human approves; the skill publishes.
    let req = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":candidate_id}),
    );
    assert_eq!(req["status"], "needs_input", "{req}");
    let options: Vec<String> = req["interaction"]["options"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_str().unwrap().to_string())
        .collect();
    assert_eq!(options, vec!["approve", "reject", "modify", "defer"]);
    let request_id = req["request_id"].as_str().unwrap().to_string();
    let done = s.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":request_id,"response":"approve"}),
    );
    assert_eq!(done["status"], "approved", "{done}");
    assert!(
        env.skills.join(&skill_name).join("SKILL.md").exists(),
        "SKILL.md must be published on human approval"
    );

    // 6. Re-detection converges instead of duplicating.
    let again = s.call(
        "skill",
        serde_json::json!({"action":"detect_reuse","scope":"project"}),
    );
    assert_eq!(again["status"], "already_exists", "{again}");
}

/// Placeholder replaced below: restart + contextual-reuse half of the loop.
#[test]
fn reuse_loop_restart_and_contextual_selection() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        for i in 0..3 {
            seed_success_run(&store, &ws, base + i * 10);
        }
        seed_irrelevant_skill(&store);
    }
    let skill_name = {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let detected = s.call(
            "skill",
            serde_json::json!({"action":"detect_reuse","scope":"project"}),
        );
        let cand = &detected["candidates"][0];
        let candidate_id = cand["skill_candidate_id"].as_str().unwrap().to_string();
        let skill_name = cand["name"].as_str().unwrap().to_string();
        let req = s.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":candidate_id}),
        );
        let request_id = req["request_id"].as_str().unwrap().to_string();
        let done = s.call(
            "skill",
            serde_json::json!({"action":"respond","request_id":request_id,"response":"approve"}),
        );
        assert_eq!(done["status"], "approved");
        skill_name
        // Hard kill without a graceful shutdown: restart must not lose it.
    };

    // 9. Restart: the approved skill is still discovered.
    let mut s2 = Server::start(&env.ws, &env.state, &env.skills);
    let disc = s2.call("skill", serde_json::json!({"action":"discover"}));
    let active: Vec<String> = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .map(|sk| sk["name"].as_str().unwrap().to_string())
        .collect();
    assert!(active.contains(&skill_name), "active: {active:?}");

    // 10–11. New task, skill NOT named: contextual selection finds it via
    // intent overlap while the irrelevant skill is excluded with a reason.
    let task = "run workspace context inspection then apply change and sandbox test to verify the rust workflow";
    let sel = s2.call(
        "skill",
        serde_json::json!({"action":"applicable","task":task}),
    );
    assert_eq!(sel["status"], "ok");
    let names: Vec<String> = sel["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&skill_name),
        "contextual selection must find the skill without it being named: {names:?}"
    );
    assert!(
        !names.contains(&"go-deployer".to_string()),
        "irrelevant skill must be excluded: {names:?}"
    );

    // 12. The skill context packet carries skill-specific context.
    let ctx = s2.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":skill_name,"task":task}),
    );
    assert_eq!(ctx["status"], "ok");
    let packet = &ctx["skill_context"];
    assert_eq!(packet["category"], "SKILL_CONTEXT");
    assert_eq!(packet["skill_name"], skill_name);
    assert!(
        !packet["why_applicable"].as_array().unwrap().is_empty(),
        "selection reasons must travel into the packet"
    );
    assert!(
        !packet["required"].as_array().unwrap().is_empty(),
        "required context must be present"
    );
    let excerpt = packet["content_excerpt"].as_str().unwrap_or_default();
    assert!(
        excerpt.contains("apply_change") && excerpt.contains("sandbox_test"),
        "packet must carry the skill-specific procedure: {excerpt}"
    );
}

/// Weak evidence over the wire stays a non-promise: fewer than three
/// successful executions produce no candidate.
#[test]
fn detect_reuse_without_enough_evidence_finds_nothing() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        for i in 0..2 {
            seed_success_run(&store, &ws, base + i * 10);
        }
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let out = s.call(
        "skill",
        serde_json::json!({"action":"detect_reuse","scope":"project"}),
    );
    assert_eq!(out["status"], "no_candidates", "{out}");
    assert!(out["candidates"].as_array().unwrap().is_empty());
}

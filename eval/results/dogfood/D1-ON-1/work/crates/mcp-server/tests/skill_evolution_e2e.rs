//! P13 integration: the complete skill-evolution loop through the real
//! `codebro` binary over stdio MCP.
//!
//! ```text
//! ACTIVE skill v1 (+ published SKILL.md)
//!   → real executions via `skill health` (skill-linked history evidence)
//!   → recurring weakness seeded (repeated verification failures)
//!   → skill detect_evolution (explicitly invoked, deterministic)
//!   → evidence-backed v2 candidate (validated, never auto-published)
//!   → request_approval (needs_input: OpenCode asks the human)
//!   → respond approve (human decision)
//!   → ACTIVE skill v2 (v1 rows + file history immutable)
//!   → restart
//!   → fresh task: contextual selection finds v2 (skill_context → v2)
//!   → v2 regression seeded
//!   → rollback → v1 content restored as a new version (v2 preserved)
//! ```
//!
//! Hermetic: tempdir workspaces, explicit `CODEBRO_STATE_DIR` +
//! `CODEBRO_SKILLS_DIR`. Never touches `~/.codebro` or real skills.
//!
//! Seed strategy: the v1 skill row + version row + SKILL.md file are seeded
//! library-side into the same `state.db` / skills dir before the server
//! starts (the server opens them lazily). Executions, detection, approval,
//! publication, selection, and rollback then run entirely over MCP — the
//! wire test proves the loop mines real execution evidence, not hand-planted
//! candidates.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use codebro_context_runtime::{ContextStore, Skill, SkillApplicability, SkillHealth, SkillVersion};

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
                "clientInfo": {"name": "p13-e2e", "version": "0"}
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

    fn call_err(&mut self, tool: &str, args: serde_json::Value) -> String {
        let r = self.rpc(
            "tools/call",
            serde_json::json!({"name":tool,"arguments":args}),
        );
        if let Some(e) = r.get("error") {
            return e.to_string();
        }
        if r["result"]["isError"].as_bool().unwrap_or(false) {
            return r["result"]["content"][0]["text"]
                .as_str()
                .unwrap_or_default()
                .to_string();
        }
        panic!("tool {tool} unexpectedly succeeded");
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── Seed helpers (library-side, same state.db the server will open) ───────

const NOW: u64 = 1_700_000_000;
const SKILL_NAME: &str = "evolve-verify-demo";
const FAILURE_REASON: &str =
    "sandbox verify test run failed: verification step skipped, checks not confirmed";

fn v1_content() -> String {
    format!(
        "---\nname: {SKILL_NAME}\ndescription: Seeded evolve verify demo workflow\n---\n\n\
         # Purpose\n\nFollow inspect, modify, verify.\n\n\
         # Procedure\n\n1. Inspect.\n2. Modify.\n3. Verify.\n"
    )
}

fn seed_v1(env: &Env) -> String {
    let ws = env.ws.to_string_lossy().to_string();
    let content = v1_content();
    let store = ContextStore::at_state_dir(env.state.clone());
    // The seeded row must own the deterministic lineage identity the
    // lifecycle computes (mint_skill_id): a foreign id would trip the
    // approve-time name-conflict gate, which exists precisely to refuse
    // clobbering another identity.
    let skill_id = codebro_context_runtime::skills::mint_skill_id(
        &codebro_context_runtime::skills::SkillScope::Project,
        Some(&ws),
        SKILL_NAME,
    );
    let skill = Skill {
        skill_id: skill_id.clone(),
        workspace_root: Some(ws),
        scope: "project".to_string(),
        name: SKILL_NAME.to_string(),
        description: "Seeded evolve verify demo workflow".to_string(),
        applicability: SkillApplicability {
            subsystems: vec!["verify".to_string(), "demo".to_string()],
            ..Default::default()
        },
        current_version: 1,
        status: "active".to_string(),
        confidence: 0.8,
        health: SkillHealth::default(),
        source_candidate_id: None,
        superseded_by: None,
        created_at: NOW,
        updated_at: NOW,
    };
    store.upsert_skill(&skill).unwrap();
    store
        .insert_skill_version(&SkillVersion {
            version_id: codebro_context_runtime::skills::mint_version_id(&skill_id, 1),
            skill_id: skill.skill_id.clone(),
            version_number: 1,
            content: content.clone(),
            content_hash: codebro_context_runtime::skills::content_hash(&content),
            source_candidate_id: None,
            supporting_evidence: Vec::new(),
            validation: None,
            author: "p13-seed".to_string(),
            status: "active".to_string(),
            created_at: NOW,
            parent_version: None,
        })
        .unwrap();
    // The published artifact: approval guards refuse to publish over a hole,
    // so the seeded lineage owns a real file from the start.
    let dir = env.skills.join(SKILL_NAME);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("SKILL.md"), &content).unwrap();
    skill.skill_id
}

fn seed_irrelevant_skill(env: &Env) {
    let store = ContextStore::at_state_dir(env.state.clone());
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
            created_at: NOW,
            updated_at: NOW,
        })
        .unwrap();
}

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

fn read_skill_file(env: &Env) -> String {
    std::fs::read_to_string(env.skills.join(SKILL_NAME).join("SKILL.md")).unwrap()
}

/// Execute v1 `n` times over the wire with failing outcomes (real execution
/// evidence: `skill health` records counters AND skill-linked history).
fn execute_failures(s: &mut Server, skill_id: &str, n: usize) {
    for _ in 0..n {
        let out = s.call(
            "skill",
            serde_json::json!({"action":"health","skill_id":skill_id,"success":false,"reason":FAILURE_REASON}),
        );
        assert_eq!(out["recorded"], "failure", "{out}");
    }
}

fn active_skill_id(s: &mut Server) -> String {
    let disc = s.call("skill", serde_json::json!({"action":"discover"}));
    disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sk| sk["name"] == SKILL_NAME)
        .unwrap()["skill_id"]
        .as_str()
        .unwrap()
        .to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

/// The complete P13 loop over the wire: v1 → executions → recurring
/// weakness → evolution candidate → human question → human approval →
/// v2 → restart → contextual reuse of v2 → v2 regression → rollback → v1.
#[test]
fn evolution_loop_detect_approve_publish_reuse_and_rollback() {
    let env = setup();
    seed_v1(&env);
    seed_irrelevant_skill(&env);
    let v1_body = read_skill_file(&env);

    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let skill_id = active_skill_id(&mut s);

    // 1–2. Execute v1 three times with failures: real execution evidence.
    execute_failures(&mut s, &skill_id, 3);

    // 3–4. Explicitly-invoked detection finds the recurring weakness and
    // mints a validated candidate — publishing nothing.
    let detected = s.call(
        "skill",
        serde_json::json!({"action":"detect_evolution","skill_id":skill_id}),
    );
    assert_eq!(detected["status"], "candidate_found", "{detected}");
    assert_eq!(detected["candidates"].as_array().unwrap().len(), 1);
    let cand = &detected["candidates"][0];
    assert_eq!(cand["outcome"], "created_validated", "{cand}");
    assert_eq!(cand["weakness_kind"], "verification_gap", "{cand}");
    assert_eq!(cand["source_version"], 1);
    assert_eq!(cand["proposed_version"], 2);
    assert_eq!(cand["observations"], 3);
    assert!(cand["confidence"].as_f64().unwrap() >= 0.60);
    assert!(
        !cand["evidence_sample"].as_array().unwrap().is_empty(),
        "evidence must travel with the candidate"
    );
    // The proposal answers the five evolution questions for the human.
    let candidate_id = cand["skill_candidate_id"].as_str().unwrap().to_string();
    let inspected = s.call(
        "skill",
        serde_json::json!({"action":"inspect","candidate_id":candidate_id}),
    );
    let purpose = inspected["candidate"]["purpose"]
        .as_str()
        .unwrap_or_default();
    assert!(purpose.contains("What is wrong"), "{purpose}");
    assert!(purpose.contains("What evidence"), "{purpose}");
    assert!(purpose.contains("contradicts"), "{purpose}");
    let question = cand["suggested_question"].as_str().unwrap().to_string();
    assert!(
        question.contains(SKILL_NAME) && question.contains("v1"),
        "{question}"
    );

    // Detection alone publishes nothing: file and version untouched.
    assert_eq!(read_skill_file(&env), v1_body);
    let before = s.call("skill", serde_json::json!({"action":"discover"}));
    let v_before = before["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sk| sk["name"] == SKILL_NAME)
        .unwrap()["current_version"]
        .as_u64()
        .unwrap();
    assert_eq!(v_before, 1);

    // 5–7. Approval protocol with the evolution framing: needs_input with
    // the four options; the human approves; v2 publishes.
    let req = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":candidate_id,"question":question}),
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

    // 8–9. v2 is a complete successor: v1 preserved plus hardening. v1 rows
    // stay immutable (new version row, not an edit).
    let v2_body = read_skill_file(&env);
    assert!(
        v2_body.starts_with(v1_body.trim_end()),
        "v1 must be preserved verbatim"
    );
    assert!(
        v2_body.contains("Evolution hardening (v1 → v2"),
        "{v2_body}"
    );
    assert!(
        v2_body.contains("Mandatory read-back verification"),
        "{v2_body}"
    );
    let after = s.call("skill", serde_json::json!({"action":"discover"}));
    let v_after = after["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sk| sk["name"] == SKILL_NAME)
        .unwrap()["current_version"]
        .as_u64()
        .unwrap();
    assert_eq!(v_after, 2);
    let insp = s.call(
        "skill",
        serde_json::json!({"action":"inspect","skill_id":skill_id}),
    );
    let versions = insp["versions"].as_array().unwrap();
    assert_eq!(versions.len(), 2, "{insp}");
    assert!(versions.iter().any(|v| v["version_number"] == 1));
    assert!(versions.iter().any(|v| v["version_number"] == 2));

    // Re-detection converges: the v1-window failures cannot re-trigger.
    let again = s.call(
        "skill",
        serde_json::json!({"action":"detect_evolution","skill_id":skill_id}),
    );
    assert_eq!(again["status"], "no_candidates", "{again}");

    // 10. Restart: v2 survives a hard kill.
    drop(s);
    let mut s2 = Server::start(&env.ws, &env.state, &env.skills);
    let disc = s2.call("skill", serde_json::json!({"action":"discover"}));
    let v_restart = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sk| sk["name"] == SKILL_NAME)
        .unwrap()["current_version"]
        .as_u64()
        .unwrap();
    assert_eq!(v_restart, 2);

    // 11. Fresh task, skill NOT named: contextual selection finds v2, the
    // irrelevant skill stays excluded, and the context packet references v2.
    let task = "run the evolve verify demo rust workflow with inspection and checks";
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
    assert!(names.contains(&SKILL_NAME.to_string()), "{names:?}");
    assert!(!names.contains(&"go-deployer".to_string()), "{names:?}");
    let ranked = sel["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == SKILL_NAME)
        .unwrap();
    assert_eq!(ranked["version"], 2, "{ranked}");
    let ctx = s2.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":SKILL_NAME,"task":task}),
    );
    assert_eq!(ctx["status"], "ok");
    assert_eq!(ctx["skill_context"]["skill_version"], 2);
    assert_eq!(ctx["skill_context"]["skill_name"], SKILL_NAME);
    let excerpt = ctx["skill_context"]["content_excerpt"]
        .as_str()
        .unwrap_or_default();
    assert!(
        excerpt.contains("Mandatory read-back verification"),
        "{excerpt}"
    );

    // 12–14. v2 demonstrates regression → rollback restores v1 content as a
    // new version; v2 rows are preserved, never deleted.
    for _ in 0..3 {
        let out = s2.call(
            "skill",
            serde_json::json!({"action":"health","skill_id":skill_id,"success":false,"reason":FAILURE_REASON}),
        );
        assert_eq!(out["recorded"], "failure");
    }
    let rolled = s2.call(
        "skill",
        serde_json::json!({"action":"rollback","skill_id":skill_id,"version":1}),
    );
    assert_eq!(rolled["skill"]["current_version"], 3, "{rolled}");
    assert_eq!(
        read_skill_file(&env),
        v1_body,
        "rollback must restore the v1 content byte-for-byte"
    );
    let insp2 = s2.call(
        "skill",
        serde_json::json!({"action":"inspect","skill_id":skill_id}),
    );
    let versions2 = insp2["versions"].as_array().unwrap();
    assert_eq!(versions2.len(), 3, "{insp2}");
    // v2 still exists in history (rollback appends, never deletes).
    let v2_row = versions2.iter().find(|v| v["version_number"] == 2).unwrap();
    assert!(v2_row["content"]
        .as_str()
        .unwrap()
        .contains("Evolution hardening"));
    // The rolled-back skill is still reusable contextually.
    let sel2 = s2.call(
        "skill",
        serde_json::json!({"action":"applicable","task":task}),
    );
    let names2: Vec<String> = sel2["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names2.contains(&SKILL_NAME.to_string()), "{names2:?}");
}

/// Modify path: the human says "add timeout handling too" — the system
/// captures it, mints a successor lineage, revalidates, requires fresh
/// approval, and only then publishes. Restart with the pending successor
/// still resolves.
#[test]
fn evolution_modify_produces_revalidated_successor_and_survives_restart() {
    let env = setup();
    seed_v1(&env);
    let v1_body = read_skill_file(&env);

    let candidate_id: String;
    let request_id: String;
    {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let skill_id = active_skill_id(&mut s);
        execute_failures(&mut s, &skill_id, 3);
        let detected = s.call(
            "skill",
            serde_json::json!({"action":"detect_evolution","skill_id":skill_id}),
        );
        assert_eq!(detected["status"], "candidate_found", "{detected}");
        candidate_id = detected["candidates"][0]["skill_candidate_id"]
            .as_str()
            .unwrap()
            .to_string();
        let req = s.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":candidate_id}),
        );
        request_id = req["request_id"].as_str().unwrap().to_string();

        // Human modifies: OpenCode reads the on-disk SKILL.md natively and
        // supplies a complete replacement carrying both the detected fix
        // and the requested addition (inspect exposes metadata, not content
        // — the file is the content source, by design).
        let v1 = read_skill_file(&env);
        let modified = format!(
            "{v1}\n## Human-approved addition\n\n\
             Timeout handling: bound long steps with a deadline.\n\n\
             Mandatory read-back verification: confirm checks pass before claiming done.\n"
        );
        let out = s.call(
            "skill",
            serde_json::json!({"action":"respond","request_id":request_id,"response":"modify",
                "modification":"Add timeout handling too.","modified_content":modified}),
        );
        assert_eq!(out["status"], "superseded", "{out}");
        let new_id = out["outcome"]["new_candidate_id"]
            .as_str()
            .unwrap()
            .to_string();
        assert_ne!(new_id, candidate_id, "modify must mint a fresh lineage");
        let succ_id = out["outcome"]["successor_request_id"]
            .as_str()
            .unwrap()
            .to_string();
        // Nothing published by a modify: still v1 on disk.
        assert_eq!(read_skill_file(&env), v1_body);
        // Replay on the consumed request is refused.
        let err = s.call_err(
            "skill",
            serde_json::json!({"action":"respond","request_id":request_id,"response":"approve"}),
        );
        assert!(err.contains("already consumed"), "{err}");
        // Hard kill with the successor approval pending.
        drop(s);
        let mut s2 = Server::start(&env.ws, &env.state, &env.skills);
        let done = s2.call(
            "skill",
            serde_json::json!({"action":"respond","request_id":succ_id,"response":"approve"}),
        );
        assert_eq!(done["status"], "approved", "{done}");
        let body = read_skill_file(&env);
        assert!(body.contains("Mandatory read-back verification"), "{body}");
        assert!(body.contains("Timeout handling"), "{body}");
        let _ = new_id;
    }
}

/// Reject and defer paths: v1 remains active and untouched in both cases.
#[test]
fn evolution_reject_and_defer_leave_v1_active() {
    for (response, terminal) in [("reject", "rejected"), ("defer", "deferred")] {
        let env = setup();
        seed_v1(&env);
        let v1_body = read_skill_file(&env);
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let skill_id = active_skill_id(&mut s);
        execute_failures(&mut s, &skill_id, 3);
        let detected = s.call(
            "skill",
            serde_json::json!({"action":"detect_evolution","skill_id":skill_id}),
        );
        assert_eq!(detected["status"], "candidate_found", "{detected}");
        let candidate_id = detected["candidates"][0]["skill_candidate_id"]
            .as_str()
            .unwrap()
            .to_string();
        let req = s.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":candidate_id}),
        );
        let request_id = req["request_id"].as_str().unwrap().to_string();
        let done = s.call(
            "skill",
            serde_json::json!({"action":"respond","request_id":request_id,"response":response}),
        );
        assert_eq!(done["status"], terminal, "{done}");
        // v1 untouched: version, file, and selectability all intact.
        assert_eq!(read_skill_file(&env), v1_body);
        let disc = s.call("skill", serde_json::json!({"action":"discover"}));
        let v = disc["active_skills"]
            .as_array()
            .unwrap()
            .iter()
            .find(|sk| sk["name"] == SKILL_NAME)
            .unwrap()["current_version"]
            .as_u64()
            .unwrap();
        assert_eq!(v, 1, "{response}: v1 must remain active");
    }
}

/// Negative wire behavior: below-threshold evidence, unknown selectors,
/// duplicate convergence, and no version explosion under repeated runs.
#[test]
fn evolution_negative_paths_and_long_horizon_stability() {
    let env = setup();
    seed_v1(&env);
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let skill_id = active_skill_id(&mut s);

    // Two failures: below threshold, no candidate.
    execute_failures(&mut s, &skill_id, 2);
    let weak = s.call(
        "skill",
        serde_json::json!({"action":"detect_evolution","skill_id":skill_id}),
    );
    assert_eq!(weak["status"], "no_candidates", "{weak}");

    // Unknown selector is a caller error, not a silent empty result.
    let err = s.call_err(
        "skill",
        serde_json::json!({"action":"detect_evolution","skill_id":"sk::does-not-exist"}),
    );
    assert!(err.contains("skill not found"), "{err}");

    // Third failure crosses the threshold: exactly one candidate.
    execute_failures(&mut s, &skill_id, 1);
    let found = s.call(
        "skill",
        serde_json::json!({"action":"detect_evolution","skill_id":skill_id}),
    );
    assert_eq!(found["status"], "candidate_found", "{found}");
    let first_id = found["candidates"][0]["skill_candidate_id"]
        .as_str()
        .unwrap()
        .to_string();

    // Long horizon: repeated detection without new evidence converges —
    // same lineage, no duplicates, no version explosion, no auto-publish.
    for _ in 0..5 {
        let again = s.call("skill", serde_json::json!({"action":"detect_evolution"}));
        assert_eq!(again["status"], "already_exists", "{again}");
        assert_eq!(
            again["candidates"][0]["skill_candidate_id"]
                .as_str()
                .unwrap(),
            first_id.as_str()
        );
    }
    let disc = s.call("skill", serde_json::json!({"action":"discover"}));
    let v = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sk| sk["name"] == SKILL_NAME)
        .unwrap()["current_version"]
        .as_u64()
        .unwrap();
    assert_eq!(v, 1, "no automatic evolution loop may publish");
    let insp = s.call(
        "skill",
        serde_json::json!({"action":"inspect","skill_id":skill_id}),
    );
    assert_eq!(
        insp["versions"].as_array().unwrap().len(),
        1,
        "no version explosion"
    );
}

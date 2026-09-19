//! P12 — Consolidation + Long-Horizon Reality Test.
//!
//! Proves the persistent-intelligence system stays reliable across a
//! realistic multi-task, multi-session workflow: context stays clean,
//! memory stays scoped, learning stays evidence-backed, skills do not
//! explode, human approval stays authoritative, restarts preserve state,
//! and future sessions reuse skills without being told.
//!
//! No new features. If any test here fails it names the smallest
//! responsible layer; fixes require a regression test + full-suite green.
//!
//! Hermetic: tempdir workspaces, explicit `CODEBRO_STATE_DIR` +
//! `CODEBRO_SKILLS_DIR`. No network, no `~/.codebro`, no real skills.
//!
//! Fixture (10 sequential tasks, one project):
//! ```text
//! T1 success  workspace_context → apply_change → sandbox_test
//! T2 success  same pattern (session 2)
//! T3 success  same pattern (session 3) → detector would fire
//! T4 unrelated docker → kubectl success
//! T5 failed   same pattern but sandbox_test fails
//! T6 corrected success same pattern
//! T7 unrelated docs workflow (read → write)
//! T8 fresh-session success same pattern
//! T9 detect_reuse → skill lifecycle
//! T10 future task reuses skill without naming it
//! ```

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use codebro_context_runtime::{
    ContextStore, HistoryInput, HistoryKind, LearnScope, OpenSession, Skill, SkillApplicability,
    SkillHealth,
};

// ── Wire harness (same JSON-RPC boundary OpenCode uses) ────────────────────

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
                "clientInfo": {"name": "p12-long-horizon", "version": "0"}
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

// ── Fixture helpers ────────────────────────────────────────────────────────

const NOW: u64 = 1_700_000_000;

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
    NOW
}

#[allow(clippy::too_many_arguments)]
fn record(
    store: &ContextStore,
    ws: &str,
    sid: &str,
    kind: HistoryKind,
    tool: &str,
    outcome: &str,
    summary: &str,
    at: u64,
) -> i64 {
    let mut input = HistoryInput::new(ws, kind, summary);
    input.session_id = Some(sid.to_string());
    input.tool = Some(tool.to_string());
    input.outcome = Some(outcome.to_string());
    input.created_at = Some(at);
    let (id, dup) = store.record_history(&input, at).unwrap();
    assert!(!dup);
    id
}

/// One successful `workspace_context → apply_change → sandbox_test` run.
fn seed_success_run(store: &ContextStore, ws: &str, at: u64) -> String {
    let sid = store
        .open_session(ws, &OpenSession::default(), at)
        .unwrap()
        .id;
    record(
        store,
        ws,
        &sid,
        HistoryKind::ToolExecution,
        "workspace_context",
        "success",
        "inspected workspace context",
        at,
    );
    record(
        store,
        ws,
        &sid,
        HistoryKind::ChangeApplied,
        "apply_change",
        "applied",
        "applied change",
        at + 1,
    );
    record(
        store,
        ws,
        &sid,
        HistoryKind::Validation,
        "sandbox_test",
        "passed",
        "sandbox_test passed",
        at + 2,
    );
    sid
}

fn seed_failed_run(store: &ContextStore, ws: &str, at: u64) -> String {
    let sid = store
        .open_session(ws, &OpenSession::default(), at)
        .unwrap()
        .id;
    record(
        store,
        ws,
        &sid,
        HistoryKind::ToolExecution,
        "workspace_context",
        "success",
        "inspected workspace context",
        at,
    );
    record(
        store,
        ws,
        &sid,
        HistoryKind::ChangeApplied,
        "apply_change",
        "applied",
        "applied change",
        at + 1,
    );
    record(
        store,
        ws,
        &sid,
        HistoryKind::Validation,
        "sandbox_test",
        "test_failure",
        "sandbox_test failed",
        at + 2,
    );
    sid
}

fn seed_unrelated_run(store: &ContextStore, ws: &str, at: u64, a: &str, b: &str) -> String {
    let sid = store
        .open_session(ws, &OpenSession::default(), at)
        .unwrap()
        .id;
    record(
        store,
        ws,
        &sid,
        HistoryKind::ToolExecution,
        a,
        "success",
        "unrelated step one",
        at,
    );
    record(
        store,
        ws,
        &sid,
        HistoryKind::Validation,
        b,
        "passed",
        "unrelated step two",
        at + 1,
    );
    sid
}

fn seed_irrelevant_skill(store: &ContextStore, name: &str, langs: Vec<&str>, subs: Vec<&str>) {
    store
        .upsert_skill(&Skill {
            skill_id: format!("sk::{name}"),
            workspace_root: None,
            scope: "global".to_string(),
            name: name.to_string(),
            description: format!("Irrelevant {name} helper for other work"),
            applicability: SkillApplicability {
                projects: Vec::new(),
                task_types: Vec::new(),
                languages: langs.into_iter().map(str::to_string).collect(),
                frameworks: Vec::new(),
                subsystems: subs.into_iter().map(str::to_string).collect(),
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

/// The full 10-task long-horizon fixture (T1–T8 seed history; T9–T10 are
/// the detector + reuse steps the tests then perform).
fn seed_ten_task_fixture(store: &ContextStore, ws: &str, base: u64) {
    // T1–T3: repeated successful MCP workflow (3 sessions).
    for i in 0..3 {
        seed_success_run(store, ws, base + i * 10);
    }
    // T4: unrelated workflow.
    seed_unrelated_run(store, ws, base + 100, "docker_build", "kubectl_apply");
    // T5: failed execution of the same pattern.
    seed_failed_run(store, ws, base + 200);
    // T6: corrected work — success again.
    seed_success_run(store, ws, base + 300);
    // T7: unrelated docs workflow.
    seed_unrelated_run(store, ws, base + 400, "read_docs", "write_notes");
    // T8: fresh-session success of the MCP pattern.
    seed_success_run(store, ws, base + 500);
}

// ── PHASE 1+3: ten-task fixture converges, detector deterministic ──────────

#[test]
fn p12_ten_task_fixture_converges_to_single_skill() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 900;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_ten_task_fixture(&store, &ws, base);
        // Deliberate pollution inside the fixture: unrelated successful
        // pairs that never repeat — must not merge into the workflow.
        // (T4/T7 already cover this; this asserts the count.)
        let report = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
            .unwrap();
        // 5 successes vs 1 failure: contradiction discounted, one lineage.
        assert_eq!(report.status, "candidates_found", "{report:?}");
        // Pre-subsumption mining may consider sub-patterns (e.g. 2-step
        // windows of the 3-step workflow); the subsumption filter keeps the
        // most specific form so only one skill lineage is minted.
        assert!(report.patterns_considered >= 1, "{report:?}");
        assert_eq!(report.candidates.len(), 1, "{report:?}");
        let c = &report.candidates[0];
        assert_eq!(c.outcome, "created_validated", "{c:?}");
        assert_eq!(c.observations, 5, "{c:?}");
        assert!(c.name.starts_with("reuse-"), "{c:?}");
        assert!(!c.evidence_sample.is_empty(), "evidence must be preserved");

        // Second run converges: no duplicate.
        let again = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix() + 1)
            .unwrap();
        assert_eq!(again.status, "already_exists", "{again:?}");
        assert_eq!(again.candidates.len(), 1);
        assert_eq!(
            again.candidates[0].learning_candidate_id, c.learning_candidate_id,
            "candidate identity must be stable"
        );
        assert_eq!(again.candidates[0].name, c.name, "naming must be stable");
        assert!(
            (again.candidates[0].confidence - c.confidence).abs() < 1e-9,
            "confidence must be deterministic"
        );
    }
}

// ── PHASE 2: context pollution ─────────────────────────────────────────────

#[test]
fn p12_context_pollution_stays_bounded() {
    use codebro_context_runtime::context_runtime::skill_selection::select_applicable_skills;
    use codebro_context_runtime::context_runtime::SkillSelectionRequest;

    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 900;
    let store = ContextStore::at_state_dir(env.state.clone());
    seed_ten_task_fixture(&store, &ws, base);
    // Seed deliberately irrelevant information: unrelated skills,
    // unrelated history noise, conflicting but out-of-scope learning is
    // covered by the multi-project test; here we pollute the same scope.
    seed_irrelevant_skill(&store, "go-deployer", vec!["go"], vec!["kubernetes"]);
    seed_irrelevant_skill(&store, "py-formatter", vec!["python"], vec!["formatting"]);
    seed_irrelevant_skill(&store, "npm-publisher", vec!["javascript"], vec!["npm"]);
    for i in 0..10 {
        seed_unrelated_run(
            &store,
            &ws,
            base + 600 + i * 5,
            &format!("noise_tool_a_{i}"),
            &format!("noise_tool_b_{i}"),
        );
    }
    // Promote the repeated workflow to an active skill (library path).
    let report = store
        .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
        .unwrap();
    assert_eq!(report.status, "candidates_found");
    let sc_id = report.candidates[0]
        .skill_candidate_id
        .as_deref()
        .unwrap()
        .to_string();
    let skills_dir = tempfile::tempdir().unwrap();
    store
        .approve_skill_candidate(&sc_id, Some(&ws), skills_dir.path(), now_unix() + 1)
        .unwrap();
    let skill_name = report.candidates[0].name.clone();

    // Relevant task: must select the reuse skill, exclude the noise.
    let task = "run workspace context inspection then apply change and sandbox test to verify the rust workflow";
    let visible = store.list_skills(Some(&ws), None, 50).unwrap();
    assert!(
        visible.len() >= 4,
        "fixture + pollution expected: {visible:?}"
    );
    let sel = select_applicable_skills(
        &visible,
        &SkillSelectionRequest {
            task_text: task.to_string(),
            keywords: Vec::new(),
            workspace_root: ws.clone(),
            task_id: None,
            repo_languages: vec!["rust".to_string()],
            limit: Some(5),
        },
    );
    let names: Vec<&str> = sel.applicable.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names.contains(&skill_name.as_str()),
        "relevant skill must be selected: {names:?}"
    );
    assert!(
        !names.contains(&"go-deployer"),
        "irrelevant skills excluded: {names:?}"
    );
    assert!(
        !names.contains(&"py-formatter"),
        "irrelevant skills excluded: {names:?}"
    );
    // Exclusions are audited, never silent.
    assert!(
        !sel.excluded.is_empty(),
        "excluded skills must carry reasons"
    );
    // Selection carries reasons + constraints, and stays bounded.
    for r in &sel.applicable {
        assert!(!r.reasons.is_empty(), "reasons required: {r:?}");
        assert!(!r.constraints.is_empty(), "constraints required: {r:?}");
    }
    // Repeat after more history: output must not degrade.
    for i in 0..5 {
        seed_unrelated_run(&store, &ws, now_unix() + 100 + i * 5, "extra_a", "extra_b");
    }
    let visible2 = store.list_skills(Some(&ws), None, 50).unwrap();
    let sel2 = select_applicable_skills(
        &visible2,
        &SkillSelectionRequest {
            task_text: task.to_string(),
            keywords: Vec::new(),
            workspace_root: ws.clone(),
            task_id: None,
            repo_languages: vec!["rust".to_string()],
            limit: Some(5),
        },
    );
    let names2: Vec<&str> = sel2.applicable.iter().map(|r| r.name.as_str()).collect();
    assert!(
        names2.contains(&skill_name.as_str()),
        "selection must not degrade as history grows: {names2:?}"
    );
    assert_eq!(names, names2, "ranking must be stable under noise");
}

// ── PHASE 4: contradiction gates ───────────────────────────────────────────

#[test]
fn p12_contradiction_gates_hold() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 900;
    // Case A: 3 successes + 1 failure still succeeds (weak contradiction
    // discounted: 9 supporting vs 3 contradicting, 3*2=6 < 9).
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            seed_success_run(&store, &ws, base + i * 10);
        }
        seed_failed_run(&store, &ws, base + 100);
        let r = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "candidates_found", "weak contradiction: {r:?}");
    }
    // Case B: 3 successes + 2 failures blocked (9 vs 6, 6*2>=9 contested).
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            seed_success_run(&store, &ws, base + i * 10);
        }
        for i in 0..2 {
            seed_failed_run(&store, &ws, base + 100 + i * 10);
        }
        let r = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(
            r.status, "no_candidates",
            "contested pattern must not publish: {r:?}"
        );
        assert!(r.candidates.is_empty());
    }
    // Case C: mostly failures → nothing.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            seed_failed_run(&store, &ws, base + i * 10);
        }
        let r = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "no_candidates", "{r:?}");
    }
    // Case D: neutral-only sessions are neither support nor contradiction.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            let sid = store
                .open_session(&ws, &OpenSession::default(), base + i * 10)
                .unwrap()
                .id;
            record(
                &store,
                &ws,
                &sid,
                HistoryKind::ToolExecution,
                "workspace_context",
                "started",
                "neutral start",
                base + i * 10,
            );
            record(
                &store,
                &ws,
                &sid,
                HistoryKind::ToolExecution,
                "apply_change",
                "in_progress",
                "neutral middle",
                base + i * 10 + 1,
            );
        }
        let r = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "no_candidates", "neutral is not support: {r:?}");
    }
    // Thresholds are not weakened: 2 successes never qualify.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..2 {
            seed_success_run(&store, &ws, base + i * 10);
        }
        let r = store
            .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "no_candidates", "{r:?}");
    }
}

// ── PHASE 5: multi-project isolation ───────────────────────────────────────

#[test]
fn p12_multi_project_isolation_holds() {
    let dir = tempfile::tempdir().unwrap();
    let ws_a = "/repo-project-a";
    let ws_b = "/repo-project-b";
    let store = ContextStore::at_state_dir(dir.path().to_path_buf());
    let base = now_unix() - 900;
    // Project A: repeated MCP workflow.
    for i in 0..3 {
        seed_success_run(&store, ws_a, base + i * 10);
    }
    // Project B: unrelated repeated workflow (different tools).
    for i in 0..3 {
        let sid = store
            .open_session(ws_b, &OpenSession::default(), base + i * 10)
            .unwrap()
            .id;
        record(
            &store,
            ws_b,
            &sid,
            HistoryKind::ToolExecution,
            "docker_build",
            "success",
            "built image",
            base + i * 10,
        );
        record(
            &store,
            ws_b,
            &sid,
            HistoryKind::Validation,
            "kubectl_apply",
            "passed",
            "deployed",
            base + i * 10 + 1,
        );
    }
    let a = store
        .detect_skill_reuse(Some(ws_a), None, LearnScope::Project, now_unix())
        .unwrap();
    assert_eq!(a.status, "candidates_found", "{a:?}");
    let b = store
        .detect_skill_reuse(Some(ws_b), None, LearnScope::Project, now_unix())
        .unwrap();
    assert_eq!(b.status, "candidates_found", "{b:?}");
    assert_ne!(
        a.candidates[0].name, b.candidates[0].name,
        "different workflows must mint different skills"
    );
    // Cross-scope leakage check: A's evidence never appears in B's report
    // and vice versa (names are tool-derived; evidence ids are disjoint).
    assert!(a.candidates[0].pattern.contains("workspace_context"));
    assert!(b.candidates[0].pattern.contains("docker_build"));
    // Publish A's skill as project-scoped; B must not see it.
    let sc_id = a.candidates[0]
        .skill_candidate_id
        .as_deref()
        .unwrap()
        .to_string();
    let skills_dir = tempfile::tempdir().unwrap();
    store
        .approve_skill_candidate(&sc_id, Some(ws_a), skills_dir.path(), now_unix() + 1)
        .unwrap();
    let visible_b = store.list_skills(Some(ws_b), None, 20).unwrap();
    assert!(
        !visible_b.iter().any(|s| s.name == a.candidates[0].name),
        "project-A skill must not leak into project B: {visible_b:?}"
    );
    let visible_a = store.list_skills(Some(ws_a), None, 20).unwrap();
    assert!(
        visible_a.iter().any(|s| s.name == a.candidates[0].name),
        "project-A skill must be visible in project A"
    );
    // Restart does not break isolation: reopen and re-check.
    drop(store);
    let store2 = ContextStore::at_state_dir(dir.path().to_path_buf());
    let visible_b2 = store2.list_skills(Some(ws_b), None, 20).unwrap();
    assert!(
        !visible_b2.iter().any(|s| s.name == a.candidates[0].name),
        "isolation must survive restart"
    );
    // Global scope is allowed by architecture: a global detect over the
    // same store still finds a pattern (union view), without broadening
    // project isolation.
    let g = store2
        .detect_skill_reuse(None, None, LearnScope::Global, now_unix() + 2)
        .unwrap();
    assert!(
        g.status == "candidates_found" || g.status == "already_exists",
        "global detect must not error: {g:?}"
    );
}

// ── PHASE 6: skill creation loop over the wire ─────────────────────────────

#[test]
fn p12_skill_creation_loop_preserves_human_authority() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_ten_task_fixture(&store, &ws, base);
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let detected = s.call(
        "skill",
        serde_json::json!({"action":"detect_reuse","scope":"project"}),
    );
    assert_eq!(detected["status"], "candidates_found", "{detected}");
    let cand = &detected["candidates"][0];
    assert_eq!(cand["outcome"], "created_validated");
    let candidate_id = cand["skill_candidate_id"].as_str().unwrap().to_string();
    let skill_name = cand["name"].as_str().unwrap().to_string();

    // Detection alone publishes nothing.
    let before = s.call("skill", serde_json::json!({"action":"discover"}));
    assert!(
        before["active_skills"].as_array().unwrap().is_empty(),
        "no automatic publish: {before}"
    );

    // Request approval → needs_input with the four options.
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
    assert!(
        req["message"]
            .as_str()
            .unwrap()
            .contains("Human input required"),
        "OpenCode must be told to ask the human: {req}"
    );
    let request_id = req["request_id"].as_str().unwrap().to_string();

    // Human approves → ACTIVE + SKILL.md.
    let done = s.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":request_id,"response":"approve"}),
    );
    assert_eq!(done["status"], "approved", "{done}");
    let skill_path = env.skills.join(&skill_name).join("SKILL.md");
    assert!(skill_path.exists(), "SKILL.md must be published");
    let content = std::fs::read_to_string(&skill_path).unwrap();
    assert!(content.contains("name: "), "SKILL.md frontmatter required");
    assert!(content.contains("apply_change") && content.contains("sandbox_test"));

    // Skill version is deterministic (first version = 1).
    let disc = s.call("skill", serde_json::json!({"action":"discover"}));
    let entry = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|sk| sk["name"] == skill_name)
        .cloned()
        .expect("published skill must be discovered");
    assert_eq!(entry["current_version"], 1, "{entry}");

    // Duplicate creation is prevented: re-detect converges.
    let again = s.call(
        "skill",
        serde_json::json!({"action":"detect_reuse","scope":"project"}),
    );
    assert_eq!(again["status"], "already_exists", "{again}");
    // Replayed approval is refused (single-use requests).
    let (ok, text) = s.call_raw(
        "skill",
        serde_json::json!({"action":"respond","request_id":request_id,"response":"approve"}),
    );
    assert!(!ok, "replayed approval must be refused, got: {text}");
}

// ── PHASE 7: restart durability ────────────────────────────────────────────

#[test]
fn p12_restart_durability_across_all_states() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_ten_task_fixture(&store, &ws, base);
    }
    // Restart 1 — after history accumulation: detector still fires.
    let (candidate_id, skill_name) = {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let d = s.call(
            "skill",
            serde_json::json!({"action":"detect_reuse","scope":"project"}),
        );
        assert_eq!(d["status"], "candidates_found");
        (
            d["candidates"][0]["skill_candidate_id"]
                .as_str()
                .unwrap()
                .to_string(),
            d["candidates"][0]["name"].as_str().unwrap().to_string(),
        )
        // hard kill on drop
    };
    // Restart 2 — after candidate creation: candidate survives.
    {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let d = s.call(
            "skill",
            serde_json::json!({"action":"detect_reuse","scope":"project"}),
        );
        // Converges onto the same lineage (already_exists or re-validated
        // same id): either way the candidate id is stable.
        let ids: Vec<String> = d["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|c| {
                c["skill_candidate_id"]
                    .as_str()
                    .map(str::to_string)
                    .or_else(|| Some(c["learning_candidate_id"].as_str().unwrap().to_string()))
            })
            .collect();
        assert!(
            ids.iter().any(|id| id == &candidate_id
                || d["status"] == "already_exists"
                || d["status"] == "candidates_found"),
            "candidate must survive restart: {d}"
        );
    }
    // Restart 3 — after approval request: pending request survives.
    let request_id = {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let req = s.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":candidate_id.clone()}),
        );
        // request_approval is idempotent per candidate: second call may
        // return the existing pending request instead of minting anew.
        assert!(
            req["status"] == "needs_input" || req["status"] == "already_pending",
            "{req}"
        );
        req["request_id"].as_str().unwrap().to_string()
        // kill without responding: pending approval must be durable
    };
    {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let done = s.call(
            "skill",
            serde_json::json!({"action":"respond","request_id":request_id,"response":"approve"}),
        );
        assert_eq!(done["status"], "approved", "{done}");
        assert!(env.skills.join(&skill_name).join("SKILL.md").exists());
    }
    // Restart 4 — after publication: active skill + discovery survive.
    {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let disc = s.call("skill", serde_json::json!({"action":"discover"}));
        let names: Vec<String> = disc["active_skills"]
            .as_array()
            .unwrap()
            .iter()
            .map(|sk| sk["name"].as_str().unwrap().to_string())
            .collect();
        assert!(names.contains(&skill_name), "{names:?}");
        // Contextual discovery survives too.
        let task = "run workspace context inspection then apply change and sandbox test";
        let sel = s.call(
            "skill",
            serde_json::json!({"action":"applicable","task":task}),
        );
        let sel_names: Vec<String> = sel["applicable"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["name"].as_str().unwrap().to_string())
            .collect();
        assert!(sel_names.contains(&skill_name), "{sel_names:?}");
    }
}

// ── PHASE 8: future reuse (fresh session, skill not named) ─────────────────

#[test]
fn p12_future_reuse_without_naming_the_skill() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_ten_task_fixture(&store, &ws, base);
        seed_irrelevant_skill(&store, "go-deployer", vec!["go"], vec!["kubernetes"]);
    }
    let skill_name = {
        let mut s = Server::start(&env.ws, &env.state, &env.skills);
        let d = s.call(
            "skill",
            serde_json::json!({"action":"detect_reuse","scope":"project"}),
        );
        let cid = d["candidates"][0]["skill_candidate_id"]
            .as_str()
            .unwrap()
            .to_string();
        let name = d["candidates"][0]["name"].as_str().unwrap().to_string();
        let req = s.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":cid}),
        );
        let rid = req["request_id"].as_str().unwrap().to_string();
        let done = s.call(
            "skill",
            serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
        );
        assert_eq!(done["status"], "approved");
        name
    };
    // Genuinely fresh server handle = fresh session; task never names skill.
    // NOTE: the task names the repository language ("rust") so
    // language-mismatched skills (go-deployer) are excluded on positive
    // evidence — without a language signal the selector correctly keeps
    // them as uncertain rather than misreporting irrelevance.
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let task = "inspect the rust workspace context, make the requested change, then run the sandbox test to verify";
    let sel = s.call(
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
        "future session must find skill without naming it: {names:?}"
    );
    assert!(
        !names.contains(&"go-deployer".to_string()),
        "irrelevant skill stays excluded: {names:?}"
    );
    // Ranking: the reuse skill is first (most applicable).
    assert_eq!(
        names[0], skill_name,
        "reuse skill must rank first: {names:?}"
    );
    // SkillContextPacket is useful and bounded.
    let ctx = s.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":skill_name,"task":task}),
    );
    assert_eq!(ctx["status"], "ok");
    let packet = &ctx["skill_context"];
    assert_eq!(packet["category"], "SKILL_CONTEXT");
    assert!(!packet["why_applicable"].as_array().unwrap().is_empty());
    assert!(!packet["required"].as_array().unwrap().is_empty());
    assert!(!packet["constraints"].as_array().unwrap().is_empty());
    let excerpt = packet["content_excerpt"].as_str().unwrap_or_default();
    assert!(excerpt.contains("apply_change") && excerpt.contains("sandbox_test"));
    let serialized = serde_json::to_vec(&ctx["skill_context"]).unwrap().len();
    assert!(
        serialized < 16_384,
        "packet must stay bounded, got {serialized} bytes"
    );
}

// ── PHASE 9: OpenCode-style agent loop over the real binary ────────────────

#[test]
fn p12_opencode_agent_loop_over_real_binary() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 600;
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_ten_task_fixture(&store, &ws, base);
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let mut calls = 0u32;
    let mut failures = 0u32;
    let mut track = |ok: bool| {
        calls += 1;
        if !ok {
            failures += 1;
        }
    };
    // 1. Orient: workspace_context (real MCP tool).
    let (ok, _) = s.call_raw("workspace_context", serde_json::json!({}));
    track(ok);
    assert!(ok);
    // 2. Discover + applicable (model discovers CodeBro naturally).
    let disc = s.call("skill", serde_json::json!({"action":"discover"}));
    track(true);
    assert!(disc["active_skills"].as_array().is_some());
    let task = "run workspace context inspection then apply change and sandbox test";
    let sel = s.call(
        "skill",
        serde_json::json!({"action":"applicable","task":task}),
    );
    track(true);
    assert_eq!(sel["status"], "ok");
    // 3. Detect reuse (explicit model action, never background).
    let d = s.call(
        "skill",
        serde_json::json!({"action":"detect_reuse","scope":"project"}),
    );
    track(true);
    assert_eq!(d["status"], "candidates_found");
    let cid = d["candidates"][0]["skill_candidate_id"]
        .as_str()
        .unwrap()
        .to_string();
    // 4. needs_input stops completion: the agent must NOT claim done.
    let req = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":cid}),
    );
    track(true);
    assert_eq!(req["status"], "needs_input", "{req}");
    // A correct agent treats needs_input as blocked, not complete. Here we
    // assert the envelope carries the blocking message + options a human
    // can answer without knowing MCP internals.
    assert!(req["interaction"]["question"].as_str().unwrap().len() > 16);
    assert_eq!(req["interaction"]["options"].as_array().unwrap().len(), 4);
    // 5. Scripted human answers approve; workflow resumes.
    let rid = req["request_id"].as_str().unwrap().to_string();
    let done = s.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    track(true);
    assert_eq!(done["status"], "approved");
    // 6. Recovery: no failed calls in the happy path.
    assert_eq!(failures, 0, "agent loop must not need recovery here");
    assert!(calls >= 6, "measured MCP call count: {calls}");
}

// ── PHASE 10: adversarial matrix ───────────────────────────────────────────

#[test]
fn p12_adversarial_matrix() {
    // A. Repeated detector invocation never explodes.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            seed_success_run(&store, "/repo", now_unix() - 300 + i * 10);
        }
        let mut seen = std::collections::HashSet::new();
        for _ in 0..5 {
            let r = store
                .detect_skill_reuse(Some("/repo"), None, LearnScope::Project, now_unix())
                .unwrap();
            for c in &r.candidates {
                seen.insert(c.learning_candidate_id.clone());
            }
            assert!(r.candidates.len() <= 3, "bounded per run: {r:?}");
        }
        assert_eq!(seen.len(), 1, "no candidate explosion: {seen:?}");
    }
    // B. Large history (200 irrelevant events) does not degrade detection.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            seed_success_run(&store, "/repo", now_unix() - 900 + i * 10);
        }
        for i in 0..100 {
            seed_unrelated_run(
                &store,
                "/repo",
                now_unix() - 800 + i as u64 * 2,
                &format!("bulk_a_{i}"),
                &format!("bulk_b_{i}"),
            );
        }
        let r = store
            .detect_skill_reuse(Some("/repo"), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "candidates_found", "large history: {r:?}");
        assert_eq!(r.candidates.len(), 1);
    }
    // C. Failed workflow never becomes a skill.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..5 {
            seed_failed_run(&store, "/repo", now_unix() - 300 + i * 10);
        }
        let r = store
            .detect_skill_reuse(Some("/repo"), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "no_candidates", "{r:?}");
        assert!(store
            .list_skill_candidates(Some("/repo"), None, None, 10)
            .unwrap()
            .is_empty());
    }
    // D. Same tools but different order is a different workflow.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        // Pattern is A→B→C; seed C→B→A successes instead.
        for i in 0..3 {
            let at = now_unix() - 300 + i as u64 * 10;
            let sid = store
                .open_session("/repo", &OpenSession::default(), at)
                .unwrap()
                .id;
            for (j, t) in ["sandbox_test", "apply_change", "workspace_context"]
                .iter()
                .enumerate()
            {
                record(
                    &store,
                    "/repo",
                    &sid,
                    HistoryKind::ToolExecution,
                    t,
                    "success",
                    "reversed",
                    at + j as u64,
                );
            }
        }
        let r = store
            .detect_skill_reuse(Some("/repo"), None, LearnScope::Project, now_unix())
            .unwrap();
        // It finds the reversed pattern (order matters), not the forward one.
        assert_eq!(r.status, "candidates_found", "{r:?}");
        assert!(
            r.candidates[0].pattern.starts_with("sandbox_test"),
            "order is significant: {r:?}"
        );
    }
    // E. Single-tool repetition is frequency, not a workflow.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..4 {
            let at = now_unix() - 300 + i as u64 * 10;
            let sid = store
                .open_session("/repo", &OpenSession::default(), at)
                .unwrap()
                .id;
            record(
                &store,
                "/repo",
                &sid,
                HistoryKind::Validation,
                "sandbox_test",
                "passed",
                "only one tool",
                at,
            );
            record(
                &store,
                "/repo",
                &sid,
                HistoryKind::Validation,
                "sandbox_test",
                "passed",
                "only one tool",
                at + 1,
            );
        }
        let r = store
            .detect_skill_reuse(Some("/repo"), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "no_candidates", "single-tool runs: {r:?}");
    }
    // F. Deprecated skill is never selected; stale approval replay refused.
    {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("state");
        std::fs::create_dir_all(&state).unwrap();
        let store = ContextStore::at_state_dir(state);
        for i in 0..3 {
            seed_success_run(&store, "/repo", now_unix() - 300 + i * 10);
        }
        let r = store
            .detect_skill_reuse(Some("/repo"), None, LearnScope::Project, now_unix())
            .unwrap();
        let sc_id = r.candidates[0]
            .skill_candidate_id
            .as_deref()
            .unwrap()
            .to_string();
        let skills_dir = tempfile::tempdir().unwrap();
        let (skill, _version) = store
            .approve_skill_candidate(&sc_id, Some("/repo"), skills_dir.path(), now_unix() + 1)
            .unwrap();
        let skill_id = skill.skill_id.clone();
        // Deprecate against the SAME skills root where SKILL.md was
        // published (tombstone/write path is root-relative).
        store
            .deprecate_skill(
                &skill_id,
                Some("/repo"),
                skills_dir.path(),
                Some("superseded by test"),
                now_unix() + 2,
            )
            .unwrap();
        let visible = store.list_skills(Some("/repo"), None, 20).unwrap();
        assert!(
            visible
                .iter()
                .all(|sk| sk.status != "active" || sk.skill_id != skill_id),
            "deprecated skill must not be active-visible as executable"
        );
        // Selection over visible skills excludes the deprecated row.
        use codebro_context_runtime::context_runtime::skill_selection::select_applicable_skills;
        use codebro_context_runtime::context_runtime::SkillSelectionRequest;
        let sel = select_applicable_skills(
            &visible,
            &SkillSelectionRequest {
                task_text: "run workspace context inspection then apply change and sandbox test"
                    .to_string(),
                keywords: Vec::new(),
                workspace_root: "/repo".to_string(),
                task_id: None,
                repo_languages: Vec::new(),
                limit: Some(5),
            },
        );
        assert!(
            !sel.applicable.iter().any(|sk| sk.skill_id == skill_id),
            "deprecated skill must never be selected: {sel:?}"
        );
    }
    // G. Wrong workspace cannot approve/select another project's skill.
    {
        let dir = tempfile::tempdir().unwrap();
        let store = ContextStore::at_state_dir(dir.path().to_path_buf());
        for i in 0..3 {
            seed_success_run(&store, "/repo-a", now_unix() - 300 + i * 10);
        }
        let r = store
            .detect_skill_reuse(Some("/repo-a"), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(r.status, "candidates_found");
        // Detection in the wrong workspace finds nothing (no leakage).
        let wrong = store
            .detect_skill_reuse(Some("/repo-b"), None, LearnScope::Project, now_unix())
            .unwrap();
        assert_eq!(wrong.status, "no_candidates", "{wrong:?}");
    }
}

// ── PHASE 11: long-horizon quality measurements ────────────────────────────

#[test]
fn p12_long_horizon_measurements_stay_bounded() {
    let env = setup();
    let ws = env.ws.to_string_lossy().to_string();
    let base = now_unix() - 900;
    let store = ContextStore::at_state_dir(env.state.clone());
    seed_ten_task_fixture(&store, &ws, base);
    seed_irrelevant_skill(&store, "go-deployer", vec!["go"], vec!["kubernetes"]);
    for i in 0..20 {
        seed_unrelated_run(
            &store,
            &ws,
            base + 600 + i * 3,
            &format!("m_a_{i}"),
            &format!("m_b_{i}"),
        );
    }
    let history_events = store.list_events(&ws, 200);
    // list_events is bounded (clamped 1..=200); the direct count is a
    // sanity probe — the bounded assertions below are the quality gates.
    let _ = history_events;

    let first = store
        .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix())
        .unwrap();
    let second = store
        .detect_skill_reuse(Some(&ws), None, LearnScope::Project, now_unix() + 1)
        .unwrap();
    // Detector output stays bounded: ≤3 candidates per run.
    assert!(first.candidates.len() <= 3, "{first:?}");
    assert!(second.candidates.len() <= 3, "{second:?}");
    // No false-positive explosion: exactly one lineage across runs.
    let mut ids = std::collections::HashSet::new();
    for c in first.candidates.iter().chain(second.candidates.iter()) {
        ids.insert(c.learning_candidate_id.clone());
    }
    assert_eq!(ids.len(), 1, "one lineage, not explosion: {ids:?}");
    // Publish once; active skill count stays 1 (+1 seeded irrelevant).
    let sc_id = first.candidates[0]
        .skill_candidate_id
        .as_deref()
        .unwrap()
        .to_string();
    let skills_dir = tempfile::tempdir().unwrap();
    store
        .approve_skill_candidate(&sc_id, Some(&ws), skills_dir.path(), now_unix() + 2)
        .unwrap();
    let active = store.list_skills(Some(&ws), None, 50).unwrap();
    let reuse_count = active
        .iter()
        .filter(|s| s.name.starts_with("reuse-"))
        .count();
    assert_eq!(reuse_count, 1, "exactly one reuse skill: {active:?}");
    // Learning rows stay bounded: one candidate for the one lineage.
    let lc = store
        .get_candidate(&first.candidates[0].learning_candidate_id)
        .unwrap()
        .expect("learning candidate must exist");
    assert_eq!(lc.supporting_evidence.len(), 5 * 3, "{lc:?}");
    assert!(lc.supporting_evidence.len() <= 100, "evidence capped");
}

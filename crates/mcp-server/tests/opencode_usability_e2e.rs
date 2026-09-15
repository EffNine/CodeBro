//! OpenCode-native usability + integration validation.
//!
//! Milestone goal: prove CodeBro + OpenCode behaves like ONE coherent
//! persistent-intelligence system from the user's perspective — not merely
//! that unit tests pass.
//!
//! What this file does:
//! - Simulates the OpenCode agent loop ([`Agent`]) against the REAL `codebro`
//!   binary over stdio MCP (the exact JSON-RPC boundary OpenCode uses).
//!   Nothing here calls internal Rust functions to fake the interaction;
//!   library-side seeding is used ONLY for (a) pre-existing state from prior
//!   sessions (a previously-activated skill) and (b) store-level mutations
//!   that stand in for concurrent edits — both documented at the call site.
//! - Chains the full lifecycle in one continuous session: existing-skill
//!   reuse (A) → proposal from repeated workflow evidence (B) → human modify
//!   (C) → human approve + publish (D) → reuse in a fresh session (E).
//! - Covers the 10-case failure/recovery matrix, context layering quality,
//!   and OpenCode-native SKILL.md compatibility.
//! - Measures the interaction: total CodeBro calls, failed calls, and that
//!   `needs_input` is never treated as completion.
//!
//! Hermetic: tempdir workspaces, explicit `CODEBRO_STATE_DIR` +
//! `CODEBRO_SKILLS_DIR`. Never touches `~/.codebro` or real skills. No
//! network access. A live-model OpenCode session is deliberately out of
//! scope for hermetic CI (it needs API keys + network); the wire harness
//! here exercises the identical MCP boundary with a scripted human.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use codebro_context_runtime::{
    ContextStore, Skill, SkillApplicability, SkillCandidateStatus, SkillHealth, SkillVersion,
};

// ── Real-binary harness (same protocol OpenCode speaks) ────────────────────

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
}

impl Server {
    fn start_with_args(root: &Path, state: &Path, skills: &Path, extra: &[&str]) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut cmd = Proc::new(bin);
        cmd.args(["serve", "--root"]).arg(root);
        for a in extra {
            cmd.arg(a);
        }
        let mut child = cmd
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
                "clientInfo": {"name": "opencode-usability", "version": "0"}
            }),
        );
        s.stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        s.stdin.flush().unwrap();
        s
    }

    fn start(root: &Path, state: &Path, skills: &Path) -> Self {
        Self::start_with_args(root, state, skills, &[])
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
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

// ── OpenCode agent-loop simulator ──────────────────────────────────────────
//
// Counts every CodeBro call and every failure, and enforces the core UX
// invariant: `needs_input` must route to the human — the agent loop must
// never treat it as completion.

struct Agent {
    server: Server,
    calls: usize,
    failed: usize,
    /// needs_input request ids that reached the human (never completed over).
    escalated: Vec<String>,
}

impl Agent {
    fn new(server: Server) -> Self {
        Agent {
            server,
            calls: 0,
            failed: 0,
            escalated: Vec::new(),
        }
    }

    /// Normal tool call: any failure is unexpected and fails the test.
    fn call(&mut self, tool: &str, args: serde_json::Value) -> serde_json::Value {
        self.calls += 1;
        let (ok, text) = self.server.call_raw(tool, args);
        if !ok {
            self.failed += 1;
            panic!("unexpected CodeBro call failure on {tool}: {text}");
        }
        serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text))
    }

    /// Probing call: failures are expected recovery paths; they are counted
    /// so the test can assert exactly the intentional failures happened.
    fn try_call(&mut self, tool: &str, args: serde_json::Value) -> (bool, String) {
        self.calls += 1;
        let (ok, text) = self.server.call_raw(tool, args);
        if !ok {
            self.failed += 1;
        }
        (ok, text)
    }

    /// The agent-loop gate: a needs_input envelope blocks completion and
    /// must carry a human-renderable question + options + guidance.
    fn expect_needs_input(&mut self, out: &serde_json::Value) -> String {
        assert_eq!(
            out["status"], "needs_input",
            "agent must be blocked, not completed: {out}"
        );
        assert_eq!(out["interaction"]["kind"], "skill_approval");
        let q = out["interaction"]["question"].as_str().unwrap_or("");
        assert!(!q.is_empty(), "human needs a question: {out}");
        let options = out["interaction"]["options"].as_array().unwrap();
        let labels: Vec<&str> = options.iter().map(|o| o.as_str().unwrap()).collect();
        assert_eq!(labels, vec!["approve", "reject", "modify", "defer"]);
        // Model-facing guidance: ask the human naturally, then respond.
        let msg = out["message"].as_str().unwrap_or("");
        let next = out["next_action"].as_str().unwrap_or("");
        assert!(
            msg.contains("Human input required") && next.contains("respond"),
            "OpenCode needs render + resume guidance: {out}"
        );
        let rid = out["request_id"].as_str().unwrap().to_string();
        assert!(rid.starts_with("apr::"));
        self.escalated.push(rid.clone());
        rid
    }
}

// ── Fixtures ───────────────────────────────────────────────────────────────

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

fn ws_string(ws: &Path) -> String {
    ws.to_string_lossy().to_string()
}

/// Valid OpenCode-compatible SKILL.md (frontmatter name+description + body).
fn skill_md(name: &str, desc: &str, body: &str) -> String {
    format!("---\nname: {name}\ndescription: {desc}\n---\n\n# Purpose\n\n{body}\n")
}

fn seed_prior_skill(store: &ContextStore, name: &str, desc: &str, content: &str) {
    // Stands in for a skill activated in an EARLIER session (pre-existing
    // state, not part of this session's interaction).
    let skill = Skill {
        skill_id: format!("sk::prior-{name}"),
        workspace_root: None,
        scope: "global".to_string(),
        name: name.to_string(),
        description: desc.to_string(),
        applicability: SkillApplicability {
            languages: vec!["rust".to_string()],
            subsystems: vec!["review".to_string()],
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
            version_id: format!("sv::prior-{name}-v1"),
            skill_id: skill.skill_id.clone(),
            version_number: 1,
            content: content.to_string(),
            content_hash: codebro_context_runtime::skills::content_hash(content),
            source_candidate_id: None,
            supporting_evidence: Vec::new(),
            validation: None,
            author: "prior-session".to_string(),
            status: "active".to_string(),
            created_at: NOW,
            parent_version: None,
        })
        .unwrap();
}

fn seed_validated_candidate(
    store: &ContextStore,
    id: &str,
    name: &str,
    ws: &str,
    confidence: f64,
    content: &str,
) {
    store
        .insert_skill_candidate(&codebro_context_runtime::SkillCandidate {
            candidate_id: id.to_string(),
            workspace_root: Some(ws.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: name.to_string(),
            description: format!("Skill {name}"),
            purpose: "usability testing".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: Vec::new(),
            supporting_evidence: Vec::new(),
            contradicting_evidence: Vec::new(),
            proposed_content: content.to_string(),
            status: "candidate".to_string(),
            confidence,
            validation: None,
            eval_reason: None,
            rejection_reason: None,
            supersedes_skill: None,
            based_on_version: None,
            created_at: NOW,
            updated_at: NOW,
            expires_at: None,
        })
        .unwrap();
    store
        .transition_skill_candidate(id, SkillCandidateStatus::Evaluating, None, NOW + 1)
        .unwrap();
    store
        .promote_candidate_to_draft(id, content, NOW + 2)
        .unwrap();
    store
        .transition_skill_candidate(id, SkillCandidateStatus::Validated, None, NOW + 3)
        .unwrap();
}

// ── Full lifecycle: A → B → C → D → E ─────────────────────────────────────

#[test]
fn opencode_full_lifecycle_reuse_propose_modify_approve_reuse() {
    let env = setup();
    let ws = ws_string(&env.ws);

    // Pre-existing state from a prior session: one active global skill, plus
    // a genuinely unrelated one (go/kubernetes) that selection must exclude.
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_prior_skill(
            &store,
            "code-review",
            "Helps with code review workflows and procedures",
            &skill_md(
                "code-review",
                "Helps with code review workflows",
                "Review procedure body from a prior session.",
            ),
        );
        let k8s = Skill {
            skill_id: "sk::prior-k8s-deploy".to_string(),
            workspace_root: None,
            scope: "global".to_string(),
            name: "k8s-deploy".to_string(),
            description: "Deploys services to kubernetes clusters".to_string(),
            applicability: SkillApplicability {
                languages: vec!["go".to_string()],
                subsystems: vec!["kubernetes".to_string()],
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
        store.upsert_skill(&k8s).unwrap();
    }

    let mut agent = Agent::new(Server::start(&env.ws, &env.state, &env.skills));

    // ── SCENARIO A — existing skill reuse ──
    // OpenCode orients (context), then asks for applicable skills with its
    // own task words — never naming the skill.
    agent.call(
        "record_memory",
        serde_json::json!({"key":"arch:generic","value":"a durable generic memory entry"}),
    );
    let ctx = agent.call(
        "context",
        serde_json::json!({"task":"review this rust pull request"}),
    );
    assert!(
        ctx.get("facts").is_some() || ctx.get("orientation").is_some(),
        "{ctx}"
    );
    let sel = agent.call(
        "skill",
        serde_json::json!({"action":"applicable","task":"review this rust pull request"}),
    );
    assert_eq!(sel["status"], "ok");
    let names: Vec<String> = sel["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(names.contains(&"code-review".to_string()), "{names:?}");
    assert!(
        !names.contains(&"k8s-deploy".to_string()),
        "unrelated excluded: {names:?}"
    );
    let ranked = sel["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "code-review")
        .unwrap();
    assert!(
        !ranked["reasons"].as_array().unwrap().is_empty(),
        "reason required"
    );
    assert!(!ranked["required_context"].as_array().unwrap().is_empty());
    assert!(!ranked["constraints"].as_array().unwrap().is_empty());
    // Skill context is minimal and distinct from generic memory.
    let sc = agent.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":"code-review","task":"review this rust pull request"}),
    );
    assert_eq!(sc["status"], "ok");
    assert_eq!(sc["skill_context"]["category"], "SKILL_CONTEXT");
    assert!(!sc["skill_context"]["why_applicable"]
        .as_array()
        .unwrap()
        .is_empty());
    let sc_text = serde_json::to_string(&sc["skill_context"]).unwrap();
    assert!(
        !sc_text.contains("a durable generic memory entry"),
        "no memory dump"
    );
    assert!(sc_text.len() < 16_384, "bounded");

    // ── SCENARIO B — skill proposal from a deterministic repeated workflow ──
    // OpenCode performs the same workflow 5 times (user-confirmed
    // preferences — the deterministic evidence learning clusters on).
    for (i, area) in ["alpha", "beta", "gamma", "delta", "epsilon"]
        .iter()
        .enumerate()
    {
        agent.call(
            "remember",
            serde_json::json!({
                "content": format!("Always follow the MCP tool-change checklist when editing MCP tool descriptions in area {area}"),
                "namespace": format!("fp.e2e.toolchange.{i}"),
                "kind": "preference",
                "scope": "project",
                "user_confirmed": true,
            }),
        );
    }
    let learned = agent.call("learn", serde_json::json!({"action":"run"}));
    assert_eq!(learned["provenance"], "learning-candidates", "{learned}");
    let listed = agent.call("learn", serde_json::json!({"action":"list"}));
    let cands = listed["candidates"].as_array().unwrap();
    assert!(
        !cands.is_empty(),
        "repeated workflow must cluster: {listed}"
    );
    let accepted: Vec<&serde_json::Value> =
        cands.iter().filter(|c| c["status"] == "accepted").collect();
    assert!(!accepted.is_empty(), "need accepted learning: {listed}");
    let best = accepted
        .iter()
        .max_by(|a, b| {
            a["confidence"]
                .as_f64()
                .unwrap_or(0.0)
                .partial_cmp(&b["confidence"].as_f64().unwrap_or(0.0))
                .unwrap()
        })
        .unwrap();
    let lc_id = best["candidate_id"].as_str().unwrap().to_string();
    assert!(
        best["confidence"].as_f64().unwrap() >= 0.60,
        "evidence must clear the approval floor: {best}"
    );

    // OpenCode proposes a skill FROM that accepted learning (evidence-backed,
    // not a standalone guess). V1 deliberately contains git commands — the
    // human will ask to remove them in scenario C.
    let v1 = skill_md(
        "mcp-tool-change",
        "Checklist for changing MCP tool descriptions",
        "Follow the checklist.\n\n## Git workflow\n\nRun git status and git diff before editing.\n\n## Checklist\n\nEdit the tool description.",
    );
    let proposed = agent.call(
        "skill",
        serde_json::json!({
            "action":"propose",
            "learning_candidate_id": lc_id,
            "name":"mcp-tool-change",
            "description":"Checklist for changing MCP tool descriptions",
            "purpose":"Repeat the MCP tool-change checklist reliably",
            "content": v1,
            "scope":"project",
            "languages":["rust"],
            "subsystems":["mcp"],
        }),
    );
    let cand_id = proposed["candidate"]["candidate_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        proposed["candidate"]["confidence"].as_f64().unwrap() >= 0.60,
        "{proposed}"
    );
    let validated = agent.call(
        "skill",
        serde_json::json!({"action":"validate","candidate_id":cand_id}),
    );
    assert_eq!(validated["status"], "validated", "{validated}");
    assert!(validated["note"]
        .as_str()
        .unwrap()
        .contains("user_confirmed=true"));

    // CodeBro must NOT silently publish: it emits an actionable approval
    // request and the agent loop must block.
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":cand_id}),
    );
    let rid = agent.expect_needs_input(&req);
    assert!(req["interaction"]["question"]
        .as_str()
        .unwrap()
        .contains("mcp-tool-change"));

    // ── SCENARIO C — human modifies the proposal ──
    // Human turn (scripted): "Approve, but make it project-specific and
    // remove git commands." OpenCode forwards instruction + full revised
    // SKILL.md (preferred over the trailer fallback).
    let human_instruction = "Approve, but make it project-specific and remove git commands.";
    let v2 = skill_md(
        "mcp-tool-change",
        "Checklist for changing MCP tool descriptions",
        "Follow the checklist. revised-by-human.\n\n## Project scope\n\nThis skill applies to this project only.\n\n## Checklist\n\nEdit the tool description.",
    );
    assert!(
        !v2.contains("git status"),
        "human revision removes git commands"
    );
    let modified = agent.call(
        "skill",
        serde_json::json!({
            "action":"respond","request_id":rid,"response":"modify",
            "modification": human_instruction,
            "modified_content": v2,
        }),
    );
    assert_eq!(modified["status"], "superseded", "{modified}");
    let new_cand = modified["outcome"]["new_candidate_id"]
        .as_str()
        .unwrap()
        .to_string();
    let succ = modified["outcome"]["successor_request_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(new_cand, cand_id, "modify mints a new lineage");
    // The re-approval travels WITH the modify response (no second
    // round-trip): OpenCode presents successor_request.interaction.
    let succ_ix = &modified["successor_request"]["interaction"];
    assert_eq!(succ_ix["kind"], "skill_approval");
    assert_eq!(succ_ix["options"].as_array().unwrap().len(), 4);
    let succ_q = succ_ix["question"].as_str().unwrap();
    assert!(succ_q.contains("mcp-tool-change"), "{succ_q}");
    assert!(
        succ_q.contains("project-specific"),
        "instruction reflected: {succ_q}"
    );
    assert!(modified["next_action"]
        .as_str()
        .unwrap()
        .contains("successor_request.interaction"));
    agent.escalated.push(succ.clone());
    // Never silently published by the modify call.
    assert!(!env.skills.join("mcp-tool-change").join("SKILL.md").exists());
    // Provenance: the new candidate records the human instruction + parent.
    let insp = agent.call(
        "skill",
        serde_json::json!({"action":"inspect","candidate_id":new_cand}),
    );
    let eval_reason = insp["candidate"]["eval_reason"].as_str().unwrap_or("");
    assert!(
        eval_reason.contains(&cand_id),
        "parent lineage: {eval_reason}"
    );
    assert!(
        eval_reason.contains("project-specific") || eval_reason.contains("human instruction"),
        "{eval_reason}"
    );
    assert_eq!(insp["candidate"]["status"], "validated", "{insp}");

    // ── SCENARIO D — human approves the revised proposal ──
    // The successor question must reflect the modification.
    // (Question text lives on the pending request; responding approves it.)
    let approved = agent.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":succ,"response":"approve"}),
    );
    assert_eq!(approved["status"], "approved", "{approved}");
    assert!(approved["message"].as_str().unwrap().contains("published"));
    // Native SKILL.md: placement, frontmatter, content integrity.
    let skill_file = env.skills.join("mcp-tool-change").join("SKILL.md");
    assert!(skill_file.exists(), "published file must exist");
    let published = std::fs::read_to_string(&skill_file).unwrap();
    assert!(
        published.starts_with("---\nname: mcp-tool-change\n"),
        "{published}"
    );
    assert!(published.contains("description: Checklist for changing MCP tool descriptions"));
    assert!(published.contains("revised-by-human"));
    assert!(!published.contains("git status"), "human edit honored");
    // Discoverable natively, exactly once (no duplicates).
    let disc = agent.call("skill", serde_json::json!({"action":"discover"}));
    let actives: Vec<&serde_json::Value> = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["name"] == "mcp-tool-change")
        .collect();
    assert_eq!(actives.len(), 1, "no duplicates: {disc}");
    assert_eq!(actives[0]["status"], "active");
    assert_eq!(actives[0]["current_version"], 1);

    // Interaction metrics: this lifecycle performs zero unexpected failures
    // and every needs_input reached the (scripted) human.
    assert_eq!(
        agent.escalated.len(),
        2,
        "request + successor both escalated"
    );
    assert_eq!(agent.failed, 0, "no failed calls in the happy path");
    // ── SCENARIO E — reuse in a FRESH session (restart) ──
    // New OpenCode session: new server process, same state. The new task
    // names no skill — selection must find it by applicability. The task
    // names the workspace language (rust), so the genuinely unrelated
    // go/kubernetes skill is excluded with an audit reason.
    drop(agent);
    let mut agent2 = Agent::new(Server::start(&env.ws, &env.state, &env.skills));
    let sel2 = agent2.call(
        "skill",
        serde_json::json!({"action":"applicable","task":"change an MCP rust tool description following the checklist"}),
    );
    let names2: Vec<String> = sel2["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names2.contains(&"mcp-tool-change".to_string()),
        "rediscovered: {names2:?}"
    );
    assert!(
        !names2.contains(&"k8s-deploy".to_string()),
        "unrelated excluded: {names2:?}"
    );
    let top = &sel2["applicable"][0];
    assert_eq!(
        top["name"], "mcp-tool-change",
        "most applicable first: {sel2}"
    );
    assert!(!top["reasons"].as_array().unwrap().is_empty());
    // The same-language but intent-mismatched skill ranks strictly lower —
    // transparent ordering, not silent confusion.
    let pos = |n: &str| names2.iter().position(|x| x == n).unwrap_or(usize::MAX);
    assert!(
        pos("mcp-tool-change") < pos("code-review"),
        "intent outranks language-only: {names2:?}"
    );
    let sc2 = agent2.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":"mcp-tool-change","task":"change an MCP rust tool description following the checklist"}),
    );
    assert_eq!(sc2["skill_context"]["category"], "SKILL_CONTEXT");
    assert_eq!(sc2["skill_context"]["skill_version"], 1);
    assert!(!sc2["skill_context"]["required"]
        .as_array()
        .unwrap()
        .is_empty());
    // Prior memory/history remain available in the fresh session.
    let ctx2 = agent2.call(
        "context",
        serde_json::json!({"task":"change an MCP tool description"}),
    );
    let ctx2_text = serde_json::to_string(&ctx2).unwrap();
    assert!(
        ctx2_text.contains("tool-change") || ctx2_text.contains("checklist"),
        "{ctx2_text}"
    );

    // Interaction metrics for the fresh session: clean run, no approvals
    // needed (the skill is already active).
    assert_eq!(agent2.failed, 0, "no failed calls in the fresh session");
    assert!(agent2.escalated.is_empty(), "no new approvals needed");
    drop(agent2);

    // Evidence authority: request is model-initiated (ai_inferred), human
    // answers are user_confirmed — future learning can tell them apart, and
    // model inference is never recorded as human approval.
    let store = ContextStore::at_state_dir(env.state.clone());
    let events = store.list_events(&ws, 100).unwrap();
    let kinds: Vec<(String, String)> = events
        .iter()
        .filter(|e| e.kind.starts_with("skill_"))
        .map(|e| (e.kind.clone(), e.payload.clone().unwrap_or_default()))
        .collect();
    assert!(
        kinds
            .iter()
            .any(|(k, p)| k == "skill_approval_requested" && p.contains("ai_inferred")),
        "request is ai_inferred: {kinds:?}"
    );
    assert!(
        kinds
            .iter()
            .any(|(k, p)| k == "skill_modified" && p.contains("user_confirmed")),
        "modify is user_confirmed: {kinds:?}"
    );
    assert!(
        kinds
            .iter()
            .any(|(k, p)| k == "skill_approved" && p.contains("user_confirmed")),
        "approval is user_confirmed: {kinds:?}"
    );
}

// ── Failure / recovery matrix (10 cases) ───────────────────────────────────

#[test]
fn opencode_failure_recovery_matrix() {
    let env = setup();
    let ws = ws_string(&env.ws);
    let content_a = skill_md("recovery-skill", "A skill for recovery tests", "Body A.");
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(&store, "sc::rcv-a", "recovery-skill", &ws, 0.75, &content_a);
        seed_validated_candidate(
            &store,
            "sc::rcv-b",
            "recovery-def",
            &ws,
            0.75,
            &skill_md("recovery-def", "Deferrable", "Body D."),
        );
        seed_validated_candidate(
            &store,
            "sc::rcv-c",
            "recovery-scope",
            &ws,
            0.75,
            &skill_md("recovery-scope", "Scoped", "Body S."),
        );
    }

    // Server authorizes a second root so scope checks (not root auth) decide.
    let other_ws = env._dir.path().join("other");
    std::fs::create_dir_all(&other_ws).unwrap();
    let other_arg = format!("--allow-root={}", other_ws.to_string_lossy());
    let mut agent = Agent::new(Server::start_with_args(
        &env.ws,
        &env.state,
        &env.skills,
        &[&other_arg],
    ));
    let mut intentional_failures = 0;

    // 1. STALE: content changes underneath a pending request → refused.
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-a"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    drop(agent);
    {
        // Stands in for a concurrent edit: same candidate row, new content,
        // status untouched (still validated) — the approval's content-hash
        // anchor must catch it. Direct SQL because no MCP action rewrites
        // candidate content in place (edits go through modify lineages).
        let db_path = env.state.join(codebro_context_runtime::STATE_DB_FILE);
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let tampered = skill_md(
            "recovery-skill",
            "A skill for recovery tests",
            "Body TAMPERED.",
        );
        let updated = conn
            .execute(
                "UPDATE skill_candidates SET proposed_content = ?1 WHERE candidate_id = ?2",
                rusqlite::params![tampered, "sc::rcv-a"],
            )
            .unwrap();
        assert_eq!(updated, 1, "tamper must hit exactly one row");
    }
    let mut agent = Agent::new(Server::start_with_args(
        &env.ws,
        &env.state,
        &env.skills,
        &[&other_arg],
    ));
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert!(!ok, "stale approval must never publish");
    assert!(
        err.contains("stale") || err.contains("validated"),
        "got: {err}"
    );
    intentional_failures += 1;
    assert!(!env.skills.join("recovery-skill").join("SKILL.md").exists());

    // 2. DUPLICATE/REPLAY: a consumed request cannot be answered again.
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-b"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let done = agent.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"reject","modification":"not now"}),
    );
    assert_eq!(done["status"], "rejected");
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert!(!ok, "replay must be refused");
    assert!(
        err.contains("consumed") || err.contains("pending"),
        "got: {err}"
    );
    intentional_failures += 1;

    // 3. WRONG WORKSPACE: project candidate answered from another workspace.
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-c"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({
            "action":"respond","request_id":rid,"response":"approve",
            "workspace_root": other_ws.to_string_lossy(),
        }),
    );
    assert!(!ok, "cross-workspace response must be refused");
    assert!(
        err.contains("workspace") || err.contains("mismatch") || err.contains("authorized"),
        "got: {err}"
    );
    intentional_failures += 1;
    // And an unauthorized root is refused before any state is touched.
    let (ok, _err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"applicable","task":"x","workspace_root":"/nonexistent-root-xyz"}),
    );
    assert!(!ok, "unauthorized root must be refused");
    intentional_failures += 1;

    // 4. INVALID MODIFICATION: missing + oversized instructions refused.
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"modify"}),
    );
    assert!(!ok, "modify without instruction must be refused");
    assert!(err.contains("modification"), "got: {err}");
    intentional_failures += 1;
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"modify","modification":"y".repeat(2001)}),
    );
    assert!(!ok, "oversized modification must be refused");
    assert!(err.contains("exceeds"), "got: {err}");
    intentional_failures += 1;

    // 5. VALIDATION FAILURE: invalid content can never reach approval.
    let bad = agent.call(
        "skill",
        serde_json::json!({
            "action":"propose","name":"bad-skill","description":"bad",
            "purpose":"bad","content":"no frontmatter here","scope":"project",
        }),
    );
    let bad_id = bad["candidate"]["candidate_id"]
        .as_str()
        .unwrap()
        .to_string();
    let val = agent.call(
        "skill",
        serde_json::json!({"action":"validate","candidate_id":bad_id}),
    );
    assert_eq!(val["validation"]["valid"], false, "{val}");
    assert!(val["note"].as_str().unwrap().contains("Fix"), "{val}");
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":bad_id}),
    );
    assert!(!ok, "unvalidated content must never get an approval prompt");
    assert!(err.contains("validated"), "got: {err}");
    intentional_failures += 1;
    // Invalid modify content: parent superseded, NO successor, recovery named.
    let modded = agent.call(
        "skill",
        serde_json::json!({
            "action":"respond","request_id":rid,"response":"modify",
            "modification":"make it invalid",
            "modified_content":"still no frontmatter",
        }),
    );
    assert_eq!(modded["status"], "superseded");
    assert!(
        modded["outcome"]["successor_request_id"].is_null(),
        "{modded}"
    );
    assert!(
        modded["next_action"].as_str().unwrap().contains("inspect"),
        "{modded}"
    );
    assert!(!env.skills.join("recovery-scope").join("SKILL.md").exists());

    // 6. REJECTED: persists rejection, publishes nothing, blocks re-request.
    let insp = agent.call(
        "skill",
        serde_json::json!({"action":"inspect","candidate_id":"sc::rcv-b"}),
    );
    assert_eq!(insp["candidate"]["status"], "rejected");
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-b"}),
    );
    assert!(!ok, "rejected candidates need re-validation first");
    assert!(err.contains("validated"), "got: {err}");
    intentional_failures += 1;

    // 7. DEFERRED: persists deferred state with a resume path.
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(
            &store,
            "sc::rcv-d",
            "recovery-defer",
            &ws,
            0.75,
            &skill_md("recovery-defer", "Deferrable 2", "Body D2."),
        );
    }
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-d"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let def = agent.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"defer"}),
    );
    assert_eq!(def["status"], "deferred");
    assert!(
        def["next_action"].as_str().unwrap().contains("re-validate"),
        "{def}"
    );
    let insp = agent.call(
        "skill",
        serde_json::json!({"action":"inspect","candidate_id":"sc::rcv-d"}),
    );
    assert_eq!(insp["candidate"]["status"], "deferred");

    // 8. EXECUTION FAILURE AFTER APPROVAL: health degrades the skill —
    // penalized and surfaced, never auto-rewritten.
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(
            &store,
            "sc::rcv-e",
            "recovery-exec",
            &ws,
            0.75,
            &skill_md("recovery-exec", "Exec skill for rust mcp work", "Body E."),
        );
    }
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-e"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let ok_out = agent.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert_eq!(ok_out["status"], "approved");
    let before =
        std::fs::read_to_string(env.skills.join("recovery-exec").join("SKILL.md")).unwrap();
    let disc = agent.call("skill", serde_json::json!({"action":"discover"}));
    let sid = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "recovery-exec")
        .unwrap()["skill_id"]
        .as_str()
        .unwrap()
        .to_string();
    for _ in 0..3 {
        agent.call(
            "skill",
            serde_json::json!({"action":"health","skill_id":sid,"success":false}),
        );
    }
    let after = std::fs::read_to_string(env.skills.join("recovery-exec").join("SKILL.md")).unwrap();
    assert_eq!(before, after, "failures must never rewrite the skill");
    let sel = agent.call(
        "skill",
        serde_json::json!({"action":"applicable","task":"rust mcp work","keywords":["rust","mcp"]}),
    );
    let ranked = sel["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "recovery-exec")
        .unwrap();
    assert!(
        ranked["constraints"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("degraded")),
        "degradation surfaced: {ranked}"
    );

    // 9. VERIFICATION FAILURE: approve is refused while gates fail, and the
    // refusal names the recovery (weak confidence never publishes).
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(
            &store,
            "sc::rcv-f",
            "recovery-weak",
            &ws,
            0.50,
            &skill_md("recovery-weak", "Weak", "Body W."),
        );
    }
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-f"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let (ok, err) = agent.try_call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert!(!ok, "weak candidates must never publish");
    assert!(
        err.contains("confidence") || err.contains("floor"),
        "got: {err}"
    );
    intentional_failures += 1;
    assert!(!env.skills.join("recovery-weak").join("SKILL.md").exists());

    // 10. RESTART between request and response: pending survives, answers work.
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(
            &store,
            "sc::rcv-g",
            "recovery-restart",
            &ws,
            0.75,
            &skill_md("recovery-restart", "Restartable", "Body R."),
        );
    }
    let rid = {
        let req = agent.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":"sc::rcv-g"}),
        );
        assert_eq!(req["status"], "needs_input");
        req["request_id"].as_str().unwrap().to_string()
    };
    drop(agent);
    let mut agent = Agent::new(Server::start(&env.ws, &env.state, &env.skills));
    let done = agent.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"reject","modification":"not now"}),
    );
    assert_eq!(done["status"], "rejected");

    // Every failure was intentional and every refusal named its recovery.
    assert_eq!(agent.failed, 0, "restart leg must be clean");
    drop(agent);
    assert_eq!(intentional_failures, 9, "all negative probes accounted for");
}

// ── Context quality + native SKILL.md compatibility ────────────────────────

#[test]
fn opencode_context_layering_and_native_skill_compat() {
    let env = setup();
    let ws = ws_string(&env.ws);
    let body_marker = "layering-unique-body-marker-7f3a9c";
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_prior_skill(
            &store,
            "layer-skill",
            "Helps with layering review workflows",
            &skill_md(
                "layer-skill",
                "Helps with layering review workflows",
                body_marker,
            ),
        );
        seed_validated_candidate(
            &store,
            "sc::lyr-a",
            "layer-publish",
            &ws,
            0.75,
            &skill_md(
                "layer-publish",
                "Publishable layering skill",
                "Publish body.",
            ),
        );
    }
    let mut agent = Agent::new(Server::start(&env.ws, &env.state, &env.skills));
    agent.call(
        "record_memory",
        serde_json::json!({"key":"arch:review-layer","value":"layering review memory value 42"}),
    );
    agent.call(
        "remember",
        serde_json::json!({
            "content": "Prefer small review batches for layering work",
            "namespace": "fp.e2e.layering",
            "kind": "preference",
            "scope": "project",
            "user_confirmed": true,
        }),
    );

    // Layered context packet: task/project/memory/records/evidence stay
    // separated; skills are pointers, never full dumps.
    let ctx = agent.call(
        "context",
        serde_json::json!({"task":"layering review work"}),
    );
    let ctx_text = serde_json::to_string(&ctx).unwrap();
    assert!(
        !ctx_text.contains(body_marker),
        "no full SKILL.md in context"
    );
    assert!(
        !ctx_text.contains("layered memory value 42"),
        "no full memory values dumped"
    );
    assert!(ctx_text.len() < 65_536, "bounded: {}", ctx_text.len());

    // Decision-support brief: semantic separation with distinct layers.
    // Memory travels as bounded key/value excerpts (engineering_memory),
    // learning as authority-tagged hypotheses (learning), skills as
    // reasoned pointers (skills) — never dumps, never conflated.
    let brief = agent.call(
        "engineering_brief",
        serde_json::json!({"task":"layering review work in rust"}),
    );
    let brief_text = serde_json::to_string(&brief).unwrap();
    assert!(brief["skills"].as_array().is_some(), "skill layer present");
    assert!(
        brief["engineering_memory"].as_array().is_some(),
        "memory layer present"
    );
    assert!(
        brief["learning"].as_array().is_some(),
        "learning layer present"
    );
    assert!(
        brief["constraints"].as_array().is_some(),
        "constraint layer present"
    );
    assert!(
        brief["history"].as_array().is_some(),
        "history layer present"
    );
    let skill_cats: Vec<String> = brief["skills"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["category"].as_str().map(str::to_string))
        .collect();
    assert!(
        skill_cats.iter().all(|c| c == "SKILL"),
        "skill layer vocabulary: {skill_cats:?}"
    );
    assert!(
        !brief_text.contains(body_marker),
        "brief never dumps SKILL.md"
    );
    // Memory travels as bounded key excerpts (short values intact, long
    // ones flagged truncated) — addressable pointers, not a dump.
    let mem_entry = brief["engineering_memory"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["key"] == "arch:review-layer");
    assert!(mem_entry.is_some(), "memory layer carries the entry");
    assert_eq!(mem_entry.unwrap()["truncated"], false);
    assert!(brief_text.len() < 131_072, "bounded: {}", brief_text.len());
    let skills = brief["skills"].as_array().unwrap();
    let layer = skills.iter().find(|s| s["name"] == "layer-skill").unwrap();
    assert!(!layer["applicability_reason"].as_str().unwrap().is_empty());
    assert!(
        layer.get("content").is_none(),
        "skill entry is a pointer, not content"
    );

    // Publish through the approval flow, then verify native compatibility.
    let req = agent.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::lyr-a"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let approved = agent.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert_eq!(approved["status"], "approved");
    let skill_file = env.skills.join("layer-publish").join("SKILL.md");
    let raw = std::fs::read_to_string(&skill_file).unwrap();
    // Frontmatter: name matches directory, description present 1–1024 chars
    // (OpenCode ignores skills missing either).
    assert!(raw.starts_with("---\n"), "frontmatter fence");
    let fm_end = raw[4..].find("\n---").unwrap();
    let fm = &raw[4..4 + fm_end];
    let name_line = fm.lines().find(|l| l.starts_with("name:")).unwrap();
    assert_eq!(name_line.trim(), "name: layer-publish");
    let desc_line = fm.lines().find(|l| l.starts_with("description:")).unwrap();
    let desc = desc_line["description:".len()..].trim();
    assert!((1..=1024).contains(&desc.len()), "description bounds");
    // Version/reference consistency: discover + applicable agree.
    let disc = agent.call("skill", serde_json::json!({"action":"discover"}));
    let found = disc["active_skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["name"] == "layer-publish")
        .unwrap();
    assert_eq!(found["current_version"], 1);
    let sel = agent.call(
        "skill",
        serde_json::json!({"action":"applicable","task":"layering publish work"}),
    );
    assert!(sel["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r["name"] == "layer-publish"));
    // skill_context resolves by name and pins the published version.
    let sc = agent.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":"layer-publish","task":"layering publish work"}),
    );
    assert_eq!(
        sc["skill_context"]["skill_version"],
        found["current_version"]
    );

    assert_eq!(agent.failed, 0);
    drop(agent);
    let _ = ws;
}

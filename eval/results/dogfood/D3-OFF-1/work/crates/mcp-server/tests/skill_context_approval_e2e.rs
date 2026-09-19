//! P10 integration: context-aware skill selection, first-class skill
//! context, and real human-in-the-loop approval through the real `codebro`
//! binary over stdio MCP.
//!
//! Hermetic: tempdir workspaces, explicit `CODEBRO_STATE_DIR` +
//! `CODEBRO_SKILLS_DIR`. Never touches `~/.codebro` or real skills.
//!
//! Seed strategy: active skills and validated candidates are seeded
//! library-side into the same `state.db` before the server starts (the
//! server opens it lazily), so wire tests exercise the MCP contract —
//! selection ranking, skill-context shape, needs_input envelopes, response
//! validation, gates, restart persistence, and evidence authority — without
//! depending on P3 learning clustering at the wire level (the
//! evidence-backed publish path itself is covered by context-runtime unit
//! tests).

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use codebro_context_runtime::{
    ContextStore, Skill, SkillApplicability, SkillCandidate, SkillCandidateStatus, SkillHealth,
    SkillVersion,
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
                "clientInfo": {"name": "p10-e2e", "version": "0"}
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

// ── Seed helpers (library-side, same state.db the server will open) ───────

const NOW: u64 = 1_700_000_000;

fn skill_content(name: &str, marker: &str) -> String {
    format!("---\nname: {name}\ndescription: Seeded test skill {marker}\n---\n\n# Purpose\n\nSeeded body {marker}.")
}

fn active_skill(name: &str, ws: Option<&str>, languages: &[&str], subsystems: &[&str]) -> Skill {
    Skill {
        skill_id: format!("sk::seed-{name}"),
        workspace_root: ws.map(str::to_string),
        scope: if ws.is_some() {
            "project".to_string()
        } else {
            "global".to_string()
        },
        name: name.to_string(),
        description: format!("Helps with {name} review workflows"),
        applicability: SkillApplicability {
            projects: Vec::new(),
            task_types: Vec::new(),
            languages: languages.iter().map(|s| s.to_string()).collect(),
            frameworks: Vec::new(),
            subsystems: subsystems.iter().map(|s| s.to_string()).collect(),
        },
        current_version: 1,
        status: "active".to_string(),
        confidence: 0.8,
        health: SkillHealth::default(),
        source_candidate_id: None,
        superseded_by: None,
        created_at: NOW,
        updated_at: NOW,
    }
}

fn seed_active_skill(store: &ContextStore, skill: &Skill, content: &str) {
    store.upsert_skill(skill).unwrap();
    store
        .insert_skill_version(&SkillVersion {
            version_id: format!("sv::seed-{}-v1", skill.name),
            skill_id: skill.skill_id.clone(),
            version_number: 1,
            content: content.to_string(),
            content_hash: codebro_context_runtime::skills::content_hash(content),
            source_candidate_id: None,
            supporting_evidence: Vec::new(),
            validation: None,
            author: "p10-seed".to_string(),
            status: "active".to_string(),
            created_at: NOW,
            parent_version: None,
        })
        .unwrap();
}

fn seed_validated_candidate(store: &ContextStore, id: &str, name: &str, ws: &str, confidence: f64) {
    let content = skill_content(name, "candidate");
    store
        .insert_skill_candidate(&SkillCandidate {
            candidate_id: id.to_string(),
            workspace_root: Some(ws.to_string()),
            task_id: None,
            scope: "project".to_string(),
            name: name.to_string(),
            description: format!("Skill {name}"),
            purpose: "P10 e2e testing".to_string(),
            applicability: SkillApplicability::default(),
            source_learning_candidates: Vec::new(),
            supporting_evidence: Vec::new(),
            contradicting_evidence: Vec::new(),
            proposed_content: content.clone(),
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
        .promote_candidate_to_draft(id, &content, NOW + 2)
        .unwrap();
    store
        .transition_skill_candidate(id, SkillCandidateStatus::Validated, None, NOW + 3)
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

fn ws_string(ws: &Path) -> String {
    ws.to_string_lossy().to_string()
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[test]
fn applicable_selects_relevant_and_excludes_irrelevant_bounded() {
    let env = setup();
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_active_skill(
            &store,
            &active_skill("code-review-e2e", None, &["rust"], &["review"]),
            &skill_content("code-review-e2e", "review"),
        );
        seed_active_skill(
            &store,
            &active_skill("k8s-deploy-e2e", None, &["go"], &["kubernetes"]),
            &skill_content("k8s-deploy-e2e", "deploy"),
        );
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let out = s.call(
        "skill",
        serde_json::json!({"action":"applicable","task":"review this rust pull request"}),
    );
    assert_eq!(out["status"], "ok");
    let names: Vec<String> = out["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"code-review-e2e".to_string()),
        "names: {names:?}"
    );
    assert!(
        !names.contains(&"k8s-deploy-e2e".to_string()),
        "names: {names:?}"
    );
    let ranked = &out["applicable"][0];
    assert!(
        !ranked["reasons"].as_array().unwrap().is_empty(),
        "reasons required"
    );
    assert!(!ranked["required_context"].as_array().unwrap().is_empty());
    assert!(!ranked["constraints"].as_array().unwrap().is_empty());
    assert!(ranked["version"].as_u64().unwrap() >= 1);
    // Bounded.
    assert!(out["applicable"].as_array().unwrap().len() <= 8);
}

#[test]
fn applicable_workspace_isolation_and_deprecated_exclusion() {
    let env = setup();
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        let mut foreign = active_skill("foreign-only", Some("/elsewhere"), &[], &[]);
        foreign.description = "Foreign workspace skill for rust work".to_string();
        store.upsert_skill(&foreign).unwrap();
        let mut old = active_skill("retired-skill", None, &[], &[]);
        old.status = "deprecated".to_string();
        old.description = "Retired rust skill".to_string();
        store.upsert_skill(&old).unwrap();
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let out = s.call(
        "skill",
        serde_json::json!({"action":"applicable","task":"rust work"}),
    );
    let names: Vec<String> = out["applicable"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        !names.contains(&"foreign-only".to_string()),
        "leak: {names:?}"
    );
    assert!(
        !names.contains(&"retired-skill".to_string()),
        "deprecated: {names:?}"
    );
    let excluded: Vec<String> = out["excluded"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap().to_string())
        .collect();
    // The foreign project skill never reaches selection (the store's
    // visibility query confines it); the deprecated row reaches selection
    // and is excluded there with an audit reason.
    assert!(
        excluded.contains(&"retired-skill".to_string()),
        "audit: {excluded:?}"
    );
}

#[test]
fn skill_context_is_minimal_and_distinct_from_memory() {
    let env = setup();
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_active_skill(
            &store,
            &active_skill("ctx-skill", None, &["rust"], &["review"]),
            &skill_content("ctx-skill", "ctx"),
        );
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    // Seed generic memory for contrast (record_memory answers plain text).
    let (ok, text) = s.call_raw(
        "record_memory",
        serde_json::json!({"key":"p10:ctx","value":"a durable test memory entry"}),
    );
    assert!(ok, "record_memory must answer: {text}");
    let out = s.call(
        "skill",
        serde_json::json!({"action":"skill_context","name":"ctx-skill","task":"review rust code"}),
    );
    assert_eq!(out["status"], "ok");
    let ctx = &out["skill_context"];
    assert_eq!(ctx["category"], "SKILL_CONTEXT");
    assert_eq!(ctx["skill_name"], "ctx-skill");
    assert!(!ctx["why_applicable"].as_array().unwrap().is_empty());
    assert!(!ctx["required"].as_array().unwrap().is_empty());
    assert!(!ctx["constraints"].as_array().unwrap().is_empty());
    // Minimal: bounded excerpt, not the memory value, not a memory shape.
    assert!(
        ctx.get("key").is_none(),
        "must not look like a memory entry"
    );
    assert!(
        ctx.get("value").is_none(),
        "must not carry full memory values"
    );
    let text = serde_json::to_string(ctx).unwrap();
    assert!(
        !text.contains("a durable test memory entry"),
        "no memory dump"
    );
    assert!(text.len() < 16_384, "bounded");
}

#[test]
fn approval_request_emits_needs_input_with_question_and_options() {
    let env = setup();
    let ws = ws_string(&env.ws);
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(&store, "sc::e2e-q001", "e2e-quest", &ws, 0.75);
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let out = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-q001"}),
    );
    assert_eq!(
        out["status"], "needs_input",
        "must block false completion: {out}"
    );
    assert_eq!(out["interaction"]["kind"], "skill_approval");
    assert!(
        out["interaction"]["question"]
            .as_str()
            .unwrap()
            .contains("e2e-quest"),
        "{out}"
    );
    let options = out["interaction"]["options"].as_array().unwrap();
    let labels: Vec<&str> = options.iter().map(|o| o.as_str().unwrap()).collect();
    assert_eq!(labels, vec!["approve", "reject", "modify", "defer"]);
    assert!(out["request_id"].as_str().unwrap().starts_with("apr::"));
    assert!(out["message"]
        .as_str()
        .unwrap()
        .contains("Human input required"));
}

#[test]
fn weak_candidate_approve_is_refused_at_the_gate() {
    let env = setup();
    let ws = ws_string(&env.ws);
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        // Standalone-style confidence below the approval floor.
        seed_validated_candidate(&store, "sc::e2e-weak001", "e2e-weak", &ws, 0.5);
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let req = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-weak001"}),
    );
    let rid = req["request_id"].as_str().unwrap();
    let (ok, err) = s.call_raw(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert!(!ok, "weak candidates must never publish");
    assert!(
        err.contains("confidence") || err.contains("floor"),
        "got: {err}"
    );
    // Nothing published.
    assert!(!env.skills.join("e2e-weak").join("SKILL.md").exists());
}

#[test]
fn reject_defer_replay_and_scope_are_enforced() {
    let env = setup();
    let ws = ws_string(&env.ws);
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(&store, "sc::e2e-r001", "e2e-rejectable", &ws, 0.75);
        seed_validated_candidate(&store, "sc::e2e-d001", "e2e-deferrable", &ws, 0.75);
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    // Reject prevents publish.
    let req = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-r001"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let done = s.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"reject","modification":"not useful"}),
    );
    assert_eq!(done["status"], "rejected");
    assert!(!env.skills.join("e2e-rejectable").join("SKILL.md").exists());
    // Replay refused.
    let (ok, err) = s.call_raw(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"approve"}),
    );
    assert!(!ok, "replay must be refused");
    assert!(
        err.contains("consumed") || err.contains("pending"),
        "got: {err}"
    );
    // Defer persists deferred state.
    let req2 = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-d001"}),
    );
    let rid2 = req2["request_id"].as_str().unwrap().to_string();
    let deferred = s.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid2,"response":"defer"}),
    );
    assert_eq!(deferred["status"], "deferred");
    let insp = s.call(
        "skill",
        serde_json::json!({"action":"inspect","candidate_id":"sc::e2e-d001"}),
    );
    assert_eq!(insp["candidate"]["status"], "deferred");
    // Invalid response option refused.
    let (ok, err) = s.call_raw(
        "skill",
        serde_json::json!({"action":"respond","request_id":"apr::doesnotexist","response":"approve"}),
    );
    assert!(!ok);
    assert!(err.contains("not found"), "got: {err}");
    // A deferred candidate is no longer validated: requesting again must
    // fail loudly (only validated candidates are approvable) — no silent
    // interaction.
    let (ok, err) = s.call_raw(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-d001"}),
    );
    assert!(!ok);
    assert!(err.contains("validated"), "got: {err}");
}

#[test]
fn modify_requires_reapproval_and_never_publishes_silently() {
    let env = setup();
    let ws = ws_string(&env.ws);
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(&store, "sc::e2e-m001", "e2e-modifiable", &ws, 0.75);
    }
    let mut s = Server::start(&env.ws, &env.state, &env.skills);
    let req = s.call(
        "skill",
        serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-m001"}),
    );
    let rid = req["request_id"].as_str().unwrap().to_string();
    let new_content = skill_content("e2e-modifiable", "revised-by-human");
    let done = s.call(
        "skill",
        serde_json::json!({
            "action":"respond","request_id":rid,"response":"modify",
            "modification":"make it project-specific",
            "modified_content": new_content,
        }),
    );
    assert_eq!(done["status"], "superseded");
    assert!(done["outcome"]["successor_request_id"].is_string());
    // Never silently published by the modify call.
    assert!(!env.skills.join("e2e-modifiable").join("SKILL.md").exists());
    // Successor approval publishes the modified content.
    let succ = done["outcome"]["successor_request_id"]
        .as_str()
        .unwrap()
        .to_string();
    let approved = s.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":succ,"response":"approve"}),
    );
    assert_eq!(approved["status"], "approved");
    let published =
        std::fs::read_to_string(env.skills.join("e2e-modifiable").join("SKILL.md")).unwrap();
    assert!(published.contains("revised-by-human"), "{published}");
}

#[test]
fn restart_preserves_pending_approval_and_evidence_keeps_authority() {
    let env = setup();
    let ws = ws_string(&env.ws);
    {
        let store = ContextStore::at_state_dir(env.state.clone());
        seed_validated_candidate(&store, "sc::e2e-p001", "e2e-persistent", &ws, 0.75);
    }
    let rid = {
        let mut s1 = Server::start(&env.ws, &env.state, &env.skills);
        let req = s1.call(
            "skill",
            serde_json::json!({"action":"request_approval","candidate_id":"sc::e2e-p001"}),
        );
        assert_eq!(req["status"], "needs_input");
        req["request_id"].as_str().unwrap().to_string()
        // Hard kill: drop without responding.
    };
    let mut s2 = Server::start(&env.ws, &env.state, &env.skills);
    let done = s2.call(
        "skill",
        serde_json::json!({"action":"respond","request_id":rid,"response":"reject","modification":"not now"}),
    );
    assert_eq!(done["status"], "rejected");
    drop(s2);
    // Evidence: the request (model-initiated) vs the decision (human) carry
    // different authority so future learning can tell them apart.
    let store = ContextStore::at_state_dir(env.state.clone());
    let events = store.list_events(&ws, 50).unwrap();
    let kinds: Vec<(String, String)> = events
        .iter()
        .filter(|e| e.kind.starts_with("skill_"))
        .map(|e| (e.kind.clone(), e.payload.clone().unwrap_or_default()))
        .collect();
    assert!(
        kinds
            .iter()
            .any(|(k, p)| k == "skill_approval_requested" && p.contains("ai_inferred")),
        "request evidence must be ai_inferred: {kinds:?}"
    );
    assert!(
        kinds
            .iter()
            .any(|(k, p)| k == "skill_rejected" && p.contains("user_confirmed")),
        "human decision must be user_confirmed: {kinds:?}"
    );
}

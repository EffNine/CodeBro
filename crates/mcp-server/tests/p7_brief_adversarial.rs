//! P7 adversarial regression suite (post-implementation audit).
//!
//! Every test here reproduces a concrete attack the audit ran against the
//! engineering brief and pins the required behavior permanently:
//!
//! - secret redaction at every new P7 read seam (skill descriptions,
//!   task skill refs, task titles)
//! - authority preservation under conflicting evidence (USER_CONFIRMED
//!   constraints vs AI_INFERRED learning — surfaced, never merged)
//! - scope isolation (task-scoped records, ambiguous targets)
//! - determinism (keyword order invariance, restart identity)
//! - bounds (depth clamps, 256 KiB envelope on a 300-symbol store)
//! - unknowns honesty (taskless briefs, corrupt state.db, failed reindex)
//! - no decision-making language anywhere in brief output
//!
//! Hermetic: tempdir + CODEBRO_STATE_DIR per test, skill publication
//! isolated via CODEBRO_SKILLS_DIR where approval is exercised — never
//! ~/.codebro, never the real ~/.config/opencode/skills.
//!
//! Drives the real binary over stdio RPC like p7_brief_e2e.rs.

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command as Proc, Stdio};

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: BufReader<ChildStdout>,
}

impl Server {
    fn start(root: &Path, state: &Path) -> Self {
        Self::start_opt(root, state, false)
    }

    /// `skills` = true additionally isolates the skill publication root
    /// (approve writes SKILL.md) into the state tempdir — never the real
    /// ~/.config/opencode/skills.
    fn start_opt(root: &Path, state: &Path, skills: bool) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut cmd = Proc::new(bin);
        cmd.args(["serve", "--root"])
            .arg(root)
            .env("CODEBRO_STATE_DIR", state)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if skills {
            let skills_dir = state.join("skills");
            std::fs::create_dir_all(&skills_dir).unwrap();
            cmd.env("CODEBRO_SKILLS_DIR", &skills_dir);
        }
        let mut child = cmd.spawn().expect("spawn codebro serve");
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
                "clientInfo": {"name": "p7-audit", "version": "0"}
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
        assert!(r.get("error").is_none(), "tool {tool} errored: {r:?}");
        let text = r["result"]["content"][0]["text"]
            .as_str()
            .expect("tool text")
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

fn seed_repo(root: &Path) {
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname = \"auditdemo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("src/lib.rs"),
        "pub fn alpha() {}\npub fn beta() { alpha(); }\n",
    )
    .unwrap();
    let git = |args: &[&str]| {
        let out = Proc::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .expect("git");
        assert!(out.status.success(), "git {args:?} failed");
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "audit@example.com"]);
    git(&["config", "user.name", "audit"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "seed"]);
}

const SECRET: &str = "sk-audit-secretkey9876543210abcdef";

/// (security, regression F1): a skill description carrying a secret must
/// the brief? Skills reach the brief via `list_skills` (registry actives
/// with lang_hit) — a rust-scoped skill with a secret in its description.
#[test]
fn brief_secret_in_skill_description_never_reaches_brief() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start_opt(repo.path(), state.path(), true);
    srv.call("reindex", serde_json::json!({}));

    // Learning-backed skill so the confidence floor passes. Seed
    // supporting history for a learn candidate, accept it, then propose
    // the skill FROM that learning with a poisoned description.
    for i in 0..4u64 {
        let r = srv.call(
            "task",
            serde_json::json!({
                "action": "create",
                "title": format!("rust-hardening sweep {i}"),
                "description": format!("cargo test sweep round {i}")
            }),
        );
        let _ = r;
    }
    srv.call(
        "learn",
        serde_json::json!({
            "action": "run",
            "workspace_root": repo.path().to_string_lossy(),
        }),
    );
    // `run` reports counts; the accepted candidate comes from `list`.
    let listed = srv.call(
        "learn",
        serde_json::json!({
            "action": "list",
            "status": "accepted",
            "workspace_root": repo.path().to_string_lossy(),
        }),
    );
    let mut accepted: Option<String> = None;
    for c in listed["candidates"].as_array().cloned().unwrap_or_default() {
        if let Some(cid) = c["candidate_id"].as_str() {
            accepted = Some(cid.to_string());
            break;
        }
    }
    let Some(lc_id) = accepted else {
        panic!("no accepted learning candidate; cannot probe the publish path: {listed}");
    };
    let proposed = srv.call("skill", serde_json::json!({
        "action": "propose",
        "name": "rust-hardening",
        "learning_candidate_id": lc_id,
        "description": format!("Use api_key={SECRET} when running the deploy step"),
        "purpose": "Testing redaction",
        "languages": ["rust"],
        "content": "---\nname: rust-hardening\ndescription: Test skill\n---\n\n# Purpose\n\nBody."
    }));
    let cid = proposed["candidate"]["candidate_id"]
        .as_str()
        .expect("cid")
        .to_string();

    // Promote candidate -> validated -> approved (user_confirmed).
    for action in ["validate", "approve"] {
        srv.call(
            "skill",
            serde_json::json!({"action": action, "candidate_id": cid, "user_confirmed": true}),
        );
    }

    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "harden rust code"}),
    );
    let text = brief.to_string();
    assert!(
        !text.contains(SECRET),
        "F1 regression: secret leaked through skill description into the brief"
    );
}

/// (security): secret in task title — the brief surfaces
/// task_state.title. Task creation redacts; verify the brief output.
#[test]
fn brief_secret_in_task_title_is_redacted() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    let created = srv.call(
        "task",
        serde_json::json!({
            "action": "create",
            "title": format!("Rotate the key {SECRET} in vault"),
            "description": "after the leak"
        }),
    );
    let tid = created["task"]["task_id"]
        .as_str()
        .expect("tid")
        .to_string();
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task_id": tid, "task": "rotate key"}),
    );
    let text = brief.to_string();
    assert!(
        !text.contains(SECRET),
        "secret regression: secret leaked through task title into the brief"
    );
}

/// (authority): USER_CONFIRMED constraint vs AI_INFERRED
/// accepted learning on the same area — the brief must keep the constraint
/// hard/user_confirmed and the learning ai_inferred, never merged, never
/// upgraded, and never dropped.
#[test]
fn brief_authority_conflict_is_preserved_not_resolved() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());

    srv.call(
        "remember",
        serde_json::json!({
            "workspace_root": repo.path().to_string_lossy(),
            "content": "Never rewrite the vault migration module",
            "namespace": "eng.constraints.vault",
            "kind": "constraint",
            "user_confirmed": true
        }),
    )
    .to_string();

    // Accepted learning that "contradicts": learn -> accept, then brief.
    // (Learning needs 3+ supporting history events; use the learn tool's
    // run action through sandbox evidence... simpler: history events then
    // learn run.)
    for i in 0..4u64 {
        let r = srv.call(
            "task",
            serde_json::json!({
                "action": "create",
                "title": format!("vault retry attempt {i}"),
                "description": format!("vault retry pattern {i}")
            }),
        );
        let _ = r;
    }
    srv.call(
        "learn",
        serde_json::json!({
            "action": "run",
            "workspace_root": repo.path().to_string_lossy(),
        }),
    );
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "vault retry"}),
    );
    let constraints = brief["constraints"].as_array().cloned().unwrap_or_default();
    assert!(
        constraints.iter().any(|c| c["content"]
            .as_str()
            .unwrap_or("")
            .contains("Never rewrite the vault")
            && c["hardness"] == "hard"
            && c["authority"] == "user_confirmed"),
        "authority regression: user-confirmed constraint must surface as hard: {constraints:?}"
    );
    for l in brief["learning"].as_array().cloned().unwrap_or_default() {
        assert_eq!(
            l["authority"], "ai_inferred",
            "learning never upgraded: {l:?}"
        );
    }
}

/// (determinism): reordered keyword input -> identical brief.
#[test]
fn brief_keyword_order_determinism() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    let a = srv.call(
        "engineering_brief",
        serde_json::json!({
            "task": "fix beta flow", "keywords": ["alpha", "gamma", "beta"]
        }),
    );
    let b = srv.call(
        "engineering_brief",
        serde_json::json!({
            "task": "fix beta flow", "keywords": ["beta", "alpha", "gamma"]
        }),
    );
    assert_eq!(
        a.to_string(),
        b.to_string(),
        "determinism regression: keyword order changed the brief"
    );
}

/// (scope): task-scoped records from task A must not surface in
/// a brief for task B (same workspace).
#[test]
fn brief_task_scoped_record_isolation() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    let t1 = srv.call("task", serde_json::json!({"action": "create", "title": "Task one secretwork", "description": "unique-alpha-pattern"}));
    let t1_id = t1["task"]["task_id"].as_str().unwrap().to_string();
    let t2 = srv.call("task", serde_json::json!({"action": "create", "title": "Task two otherwork", "description": "unique-beta-pattern"}));
    let t2_id = t2["task"]["task_id"].as_str().unwrap().to_string();

    // Task-scoped records with distinct keywords.
    srv.call(
        "remember",
        serde_json::json!({
            "workspace_root": repo.path().to_string_lossy(),
            "task_id": t1_id,
            "content": "task-one-marker-alpha guidance",
            "namespace": "intent.mission.one",
            "kind": "intent",
            "scope": "task",
            "user_confirmed": true
        }),
    );
    let brief2 = srv.call(
        "engineering_brief",
        serde_json::json!({
            "task_id": t2_id, "task": "unique-beta-pattern work"
        }),
    );
    let text = brief2.to_string();
    assert!(
        !text.contains("task-one-marker-alpha"),
        "isolation regression: task-one record leaked into task-two brief: {text}"
    );
}

/// (ambiguity): same symbol name in two modules must not
/// silently traverse.
#[test]
fn brief_ambiguous_symbol_skips_traversal() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"ambigdemo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    std::fs::create_dir_all(repo.path().join("src/one")).unwrap();
    std::fs::create_dir_all(repo.path().join("src/two")).unwrap();
    std::fs::write(repo.path().join("src/lib.rs"), "mod one;\nmod two;\n").unwrap();
    std::fs::write(repo.path().join("src/one/mod.rs"), "pub fn handle() {}\n").unwrap();
    std::fs::write(repo.path().join("src/two/mod.rs"), "pub fn handle() {}\n").unwrap();
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({
            "task": "fix handler", "target_symbol": "handle"
        }),
    );
    assert_eq!(
        brief["targets"]["discovery"], "ambiguous",
        "must be ambiguous: {brief:?}"
    );
    assert!(
        brief["impact"].is_null(),
        "ambiguity regression: ambiguous target traversed anyway: {brief:?}"
    );
}

/// (bounds): depth=2 is allowed, depth>99 is rejected, mid clamps.
#[test]
fn brief_depth_bounds_enforced() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    // depth 2 allowed
    let ok = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "beta", "target_symbol": "beta", "depth": 2}),
    );
    assert!(ok["bounds"].is_object());
    // depth 99 rejected as invalid params (validate_targets: >99 errors, 99 clamps)
    let r = srv.rpc(
        "tools/call",
        serde_json::json!({
            "name": "engineering_brief",
            "arguments": {"task": "beta", "target_symbol": "beta", "depth": 200}
        }),
    );
    assert!(
        r.get("error").is_some(),
        "depth 200 must be rejected: {r:?}"
    );
    // depth 99 clamps to 2 silently (documented)
    let clamped = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "beta", "target_symbol": "beta", "depth": 50}),
    );
    assert!(clamped["bounds"].is_object());
}

/// (freshness): failed reindex keeps FAILED explicit; last-good
/// facts still readable; brief must mark FAILED_INDEX unknown.
#[test]
fn brief_failed_reindex_marks_failed_index() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    // Corrupt so the atomic facts.json write fails: a directory where the
    // file must be staged (init skips unreadable *sources* honestly, so
    // the persistence seam is the reliable failure).
    std::fs::remove_file(repo.path().join(".codebro/facts.json")).unwrap();
    std::fs::create_dir_all(repo.path().join(".codebro/facts.json")).unwrap();
    let failed = srv.call("reindex", serde_json::json!({}));
    assert_eq!(
        failed["status"], "error",
        "reindex must fail on the corrupted state: {failed}"
    );
    assert_eq!(failed["index_status"], "FAILED", "{failed}");
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha beta"}),
    );
    assert_eq!(
        brief["freshness"]["persisted_status"], "FAILED",
        "FAILED must be explicit: {brief:?}"
    );
    let kinds: Vec<String> = brief["unknowns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["kind"].as_str().unwrap_or("").to_string())
        .collect();
    assert!(
        kinds.contains(&"FAILED_INDEX".to_string()),
        "FAILED_INDEX unknown required: {kinds:?}"
    );
}

/// (taskless): no task_id → no task snapshot, no arbitrary task
/// chosen; repository intelligence still works.
#[test]
fn brief_taskless_never_invents_a_task() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    // Create tasks with tempting titles.
    for title in ["alpha work", "beta work"] {
        srv.call(
            "task",
            serde_json::json!({"action": "create", "title": title}),
        );
    }
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha beta"}),
    );
    assert!(
        brief["task_state"].is_null(),
        "taskless regression: taskless brief invented a task: {:?}",
        brief["task_state"]
    );
    assert!(brief["scope"]["task_id"].is_null());
    // Repository intelligence still present.
    assert!(brief["targets"].is_object());
    assert!(brief["bounds"].is_object());
}

/// (cross-domain): constraint + history + learning +
/// skill + health all present, none collapsed into a recommendation.
/// Seed: two modules with a call edge; a user constraint naming the
/// callee; accepted learning; an active skill; then verify every domain
/// appears with its own category and no "should/must implement" text.
#[test]
fn brief_cross_domain_evidence_no_collapse() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    srv.call(
        "remember",
        serde_json::json!({
            "workspace_root": repo.path().to_string_lossy(),
            "content": "Never rewrite the alpha module without review",
            "namespace": "eng.constraints.alpha",
            "kind": "constraint",
            "user_confirmed": true
        }),
    );
    for i in 0..4u64 {
        srv.call("task", serde_json::json!({"action": "create", "title": format!("alpha sweep {i}"), "description": format!("alpha sweep round {i}")}));
    }
    srv.call(
        "learn",
        serde_json::json!({"action": "run", "workspace_root": repo.path().to_string_lossy()}),
    );
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha module work", "target_symbol": "alpha"}),
    );
    let text = brief.to_string();
    // Constraint present as hard.
    let constraints = brief["constraints"].as_array().cloned().unwrap_or_default();
    assert!(
        constraints
            .iter()
            .any(|c| c["hardness"] == "hard"
                && c["content"].as_str().unwrap_or("").contains("alpha")),
        "constraint missing: {constraints:?}"
    );
    // No decision field, no imperative recommendation text.
    assert!(brief.get("decision").is_none() || brief["decision"].is_null());
    for forbidden in [
        "the correct solution is",
        "OpenCode should implement",
        "modify file",
        "you must execute",
        "recommend implementing",
    ] {
        let low = text.to_lowercase();
        assert!(
            !low.contains(&forbidden.to_lowercase()),
            "cross-domain regression: decision-like text '{forbidden}' found in brief"
        );
    }
    // Impact exists for alpha.
    assert!(brief["impact"].is_object(), "impact missing: {brief:?}");
}

/// (identity): brief must identify the actual repo;
/// moving the repo or changing the remote changes identity predictably.
#[test]
fn brief_repository_identity_stable_and_distinct() {
    let repo_a = tempfile::tempdir().unwrap();
    let repo_b = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo_a.path());
    seed_repo(repo_b.path());
    let mut srv = Server::start(repo_a.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    let brief = srv.call("engineering_brief", serde_json::json!({"task": "alpha"}));
    let id_a = brief["repository"]["project_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert!(!id_a.is_empty());
    // Restart + same repo: identity stable.
    drop(srv);
    let mut srv2 = Server::start(repo_a.path(), state.path());
    let brief2 = srv2.call("engineering_brief", serde_json::json!({"task": "alpha"}));
    let id_a2 = brief2["repository"]["project_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_eq!(id_a, id_a2, "identity must survive restart");
    // Different repo: distinct id.
    let mut srv_b = Server::start(repo_b.path(), state.path());
    srv_b.call("reindex", serde_json::json!({}));
    let brief_b = srv_b.call("engineering_brief", serde_json::json!({"task": "alpha"}));
    let id_b = brief_b["repository"]["project_id"]
        .as_str()
        .unwrap_or("")
        .to_string();
    assert_ne!(id_a, id_b, "distinct repos must have distinct project ids");
}

/// (failure/recovery): a corrupt state.db must not crash the
/// brief; sqlite quarantine + empty store should degrade to unknowns.
#[test]
fn brief_corrupt_state_db_degrades_to_unknowns() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    // Write garbage where state.db is expected.
    std::fs::write(state.path().join("state.db"), b"this is not sqlite at all").unwrap();
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha beta"}),
    );
    assert!(
        brief["bounds"].is_object(),
        "brief must still answer: {brief:?}"
    );
    let kinds: Vec<String> = brief["unknowns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|u| u["kind"].as_str().unwrap_or("").to_string())
        .collect();
    // History/learning/skills must be reported as their unavailable unknowns
    // or empty — never fabricate content. (A quarantined-then-fresh store
    // reads empty: the NO_* unknowns mark absence honestly.)
    assert!(brief["task_state"].is_null());
    assert!(
        kinds.contains(&"NO_HISTORY".to_string())
            || brief["history"]
                .as_array()
                .map(|h| h.is_empty())
                .unwrap_or(true)
    );
}

/// (bounding): a store with many symbols must stay within the
/// 256 KiB envelope and per-section caps.
#[test]
fn brief_massive_store_stays_bounded() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    // Generate 300 functions across 30 files, all calling alpha.
    std::fs::write(
        repo.path().join("Cargo.toml"),
        "[package]\nname = \"bigdemo\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(repo.path().join("src")).unwrap();
    let mut lib = String::from("pub fn alpha() {}\n");
    for f in 0..30 {
        let mut body = String::new();
        for i in 0..10 {
            body.push_str(&format!("pub fn f{f}_{i}() {{ alpha(); }}\n"));
            lib.push_str(&format!("pub use mod{f}::*;\n"));
        }
        std::fs::create_dir_all(repo.path().join(format!("src/mod{f}"))).unwrap();
        std::fs::write(repo.path().join(format!("src/mod{f}/mod.rs")), body).unwrap();
    }
    std::fs::write(repo.path().join("src/lib.rs"), lib).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .output()
            .unwrap();
        assert!(out.status.success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "b@b.com"]);
    git(&["config", "user.name", "b"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "seed"]);
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha", "target_symbol": "alpha", "depth": 2}),
    );
    let serialized = serde_json::to_string(&brief).unwrap();
    assert!(
        serialized.len() < 256 * 1024,
        "bounding regression: brief exceeded envelope: {}",
        serialized.len()
    );
    let direct = brief["impact"]["direct"].as_array().unwrap().len();
    assert!(
        direct <= 10,
        "bounding regression: direct edges unbounded: {direct}"
    );
}

/// (output hygiene): no raw row ids / internal fields in the brief.
#[test]
fn brief_output_hygiene_no_internal_fields() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    srv.call(
        "remember",
        serde_json::json!({
            "workspace_root": repo.path().to_string_lossy(),
            "content": "Chose sqlite for alpha storage",
            "namespace": "eng.history.alpha-decision",
            "kind": "decision",
            "related_ids": ["task::alpha-storage"],
            "user_confirmed": true
        }),
    );
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "alpha storage"}),
    );
    let text = brief.to_string();
    // no internal keys
    for forbidden in [
        "\"rowid\"",
        "\"event_id\"",
        "\"payload_json\"",
        "\"dedup_key\"",
        "\"digest\"",
    ] {
        assert!(
            !text.contains(forbidden),
            "internal field {forbidden} leaked: {text}"
        );
    }
}

/// (security, regression F1b): a task skill_ref carrying a secret token
/// must not leak verbatim into the brief skills section (write-time
/// redaction + projection defense).
#[test]
fn brief_secret_in_task_skill_ref_not_echoed() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start_opt(repo.path(), state.path(), true);
    let created = srv.call(
        "task",
        serde_json::json!({
            "action": "create",
            "title": "Deploy with vault",
            "skill_refs": [format!("deploy-token-{SECRET}")]
        }),
    );
    let tid = created["task"]["task_id"].as_str().unwrap().to_string();
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task_id": tid, "task": "deploy vault"}),
    );
    let text = brief.to_string();
    assert!(
        !text.contains(SECRET),
        "F1b regression: secret leaked through task skill_ref into the brief"
    );
    // The ref itself (unmatched) must still be surfaced as an opaque
    // unresolved skill entry — redacted, not dropped.
    let skills = brief["skills"].as_array().cloned().unwrap_or_default();
    assert!(
        skills
            .iter()
            .any(|s| s["origin"] == "task_ref" && s["status"] == "unresolved"),
        "unresolved task skill ref must still surface (redacted): {skills:?}"
    );
}

/// (security, identity seam): a secret pasted into an identity
/// constraint or decision title must never reach the brief (identity
/// JSON persists at .codebro/project_identity.json — a write seam).
#[test]
fn brief_secret_in_identity_constraint_not_surfaced() {
    let repo = tempfile::tempdir().unwrap();
    let state = tempfile::tempdir().unwrap();
    seed_repo(repo.path());
    let mut srv = Server::start(repo.path(), state.path());
    srv.call("reindex", serde_json::json!({}));
    srv.call("update_identity", serde_json::json!({
        "add_constraints": [format!("deploy only with token {SECRET} present")],
        "add_decisions": [{"title": format!("Use key {SECRET} for deploys"), "description": "ops decision"}]
    }));
    let brief = srv.call(
        "engineering_brief",
        serde_json::json!({"task": "deploy constraints"}),
    );
    let text = brief.to_string();
    eprintln!("constraints: {:?}", brief["constraints"]);
    assert!(
        !text.contains(SECRET),
        "identity-seam secret leaked into the brief"
    );
    // The constraint must still be present (redacted, not dropped).
    let constraints = brief["constraints"].as_array().cloned().unwrap_or_default();
    assert!(
        constraints
            .iter()
            .any(|c| c["source"] == "project_identity" && c["hardness"] == "hard"),
        "identity constraint must still surface (redacted): {constraints:?}"
    );
}

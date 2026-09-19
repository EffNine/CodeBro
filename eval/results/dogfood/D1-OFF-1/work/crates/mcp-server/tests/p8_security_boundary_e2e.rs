//! P8 security boundary closure probes: workspace-root authorization
//! verified through the real `codebro` binary over stdio RPC.
//!
//! These probes close audit finding F3 (MEDIUM): the P6-era multi-root
//! registry opened ANY existing host directory passed via a tool's
//! `workspace_root` argument, so an MCP client could direct
//! `reindex`/`apply_change` (and every read tool) outside the server's
//! configured root.
//!
//! The authorization model under test (P8 boundary closure):
//!
//! - The server's configured root (`--root` / `CODEBRO_WORKSPACE_ROOT` /
//!   cwd) is always authorized.
//! - Additional roots are authorized ONLY by the operator at launch:
//!   repeatable `--allow-root <path>` flags and/or the
//!   `CODEBRO_ALLOW_ROOTS` environment variable (path-separated).
//! - A per-call `workspace_root` argument is DISCOVERY (which authorized
//!   workspace the call addresses), never AUTHORIZATION. It must
//!   canonicalize exactly to an authorized root; anything else is a
//!   bounded refusal with zero filesystem side effects in the target.
//! - The authorized set is immutable for the process lifetime; no tool
//!   call, task id, skill, memory entry, or history row can widen it.
//!
//! Hermetic: tempdir workspaces and victims only, explicit
//! CODEBRO_STATE_DIR + CODEBRO_SKILLS_DIR, never touches ~/.codebro, the
//! user's real repositories, /etc contents, or real skills. The `/etc`
//! probe asserts only refusal (a safe existence check of the denial,
//! never a read).

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
    /// Start the real binary with explicit launch-time authorized roots.
    fn start_with(root: &Path, allow_roots: &[&Path], state: &Path, skills: &Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut cmd = Proc::new(bin);
        cmd.args(["serve", "--root"]).arg(root);
        for extra in allow_roots {
            cmd.arg("--allow-root").arg(extra);
        }
        let mut child = cmd
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
                "clientInfo": {"name": "p8-boundary-probe", "version": "0"}
            }),
        );
        s.stdin
            .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
            .unwrap();
        s.stdin.flush().unwrap();
        s
    }

    fn start(root: &Path, state: &Path, skills: &Path) -> Self {
        Self::start_with(root, &[], state, skills)
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

    /// Strict stdout purity: every line between responses must be valid
    /// JSON-RPC with a `jsonrpc` field (the P8 protocol-hygiene hook).
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
            "tool {tool} errored at protocol level: {}",
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

    /// Raw call: returns (ok, text) where ok=false covers both protocol
    /// errors and isError results.
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

/// Seed a plain victim directory (NOT a codebro workspace): if CodeBro
/// ever serves it, reindex would leave `.codebro/` and apply_change
/// could edit `secret.txt`.
fn seed_victim(root: &Path) {
    std::fs::create_dir_all(root).unwrap();
    std::fs::write(root.join("victim_file.txt"), "VICTIM-SECRET-CONTENT\n").unwrap();
}

/// Assert a directory shows no signs of CodeBro having touched it.
fn assert_untouched(dir: &Path) {
    assert!(
        !dir.join(".codebro").exists(),
        "CodeBro wrote .codebro state into the victim directory {dir:?}"
    );
    assert!(
        !dir.join("victim_file.txt").join(".codebro").exists(),
        "unexpected nested state"
    );
    if let Ok(content) = std::fs::read_to_string(dir.join("victim_file.txt")) {
        assert_eq!(
            content, "VICTIM-SECRET-CONTENT\n",
            "victim file was modified"
        );
    }
}

// ── Probe 1: unauthorized workspace_root is refused across the tool
//    surface with zero filesystem side effects (the F3 regression) ──────

#[test]
fn p8_unauthorized_workspace_root_refused_with_zero_side_effects() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "boundary_a");
    let victim = tempfile::tempdir().unwrap();
    seed_victim(victim.path());
    let victim_ws = victim.path().to_string_lossy().into_owned();

    let mut s = Server::start(repo.path(), &env.state, &env.skills);
    s.call("reindex", serde_json::json!({}));

    // Sweep representative tools across read/write surfaces. Every call
    // addressing the victim root must be refused.
    let probes: Vec<(&str, serde_json::Value)> = vec![
        (
            "workspace_context",
            serde_json::json!({"workspace_root": victim_ws}),
        ),
        (
            "engineering_facts",
            serde_json::json!({"workspace_root": victim_ws, "query": "victim"}),
        ),
        ("reindex", serde_json::json!({"workspace_root": victim_ws})),
        (
            "repository_health",
            serde_json::json!({"workspace_root": victim_ws}),
        ),
        (
            "engineering_brief",
            serde_json::json!({"workspace_root": victim_ws, "task": "victim probe"}),
        ),
        (
            "record_memory",
            serde_json::json!({"workspace_root": victim_ws, "key": "victim-key", "value": "x"}),
        ),
        (
            "task",
            serde_json::json!({"workspace_root": victim_ws, "action": "create", "title": "victim task"}),
        ),
        (
            "sandbox_exec",
            serde_json::json!({"workspace_root": victim_ws, "command": "ls"}),
        ),
    ];

    let mut refusals = Vec::new();
    for (tool, args) in &probes {
        let (ok, text) = s.call_raw(tool, args.clone());
        assert!(
            !ok,
            "{tool} served an unauthorized workspace_root — F3 not closed: {text}"
        );
        refusals.push(text);
    }

    // Refusals are bounded semantic failures naming authorization, not
    // host-filesystem disclosure.
    for text in &refusals {
        let lower = text.to_lowercase();
        assert!(
            lower.contains("not authorized") || lower.contains("authorized"),
            "refusal must state the authorization failure: {text}"
        );
    }

    // Determinism: repeated refusal is identical.
    let (ok1, t1) = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": victim_ws}),
    );
    let (ok2, t2) = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": victim_ws}),
    );
    assert!(!ok1 && !ok2);
    assert_eq!(t1, t2, "authorization refusal must be deterministic");

    // Zero filesystem side effects in the victim directory.
    assert_untouched(victim.path());

    // The authorized workspace still works afterwards (the server did not
    // degrade or lose state).
    let ctx = s.call("workspace_context", serde_json::json!({}));
    assert_eq!(
        ctx["workspace_root"],
        repo.path().canonicalize().unwrap().display().to_string()
    );
    assert_untouched(victim.path());
}

// ── Probe 2: apply_change cannot edit files in an unauthorized root ────

#[test]
fn p8_apply_change_cannot_target_unauthorized_root() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "boundary_b");
    let victim = tempfile::tempdir().unwrap();
    seed_victim(victim.path());
    let victim_file = victim.path().join("victim_file.txt");
    let victim_path = victim_file.to_string_lossy().into_owned();

    let mut s = Server::start(repo.path(), &env.state, &env.skills);

    // Absolute path inside apply_change targets the victim file while the
    // workspace_root itself is the victim root: both the root gate and the
    // ChangeEngine path gate must refuse.
    let (ok_root, text_root) = s.call_raw(
        "apply_change",
        serde_json::json!({
            "path": "victim_file.txt",
            "old": "VICTIM-SECRET-CONTENT\n",
            "new": "PWNED\n",
            "workspace_root": victim.path().to_string_lossy().to_string()
        }),
    );
    assert!(
        !ok_root,
        "apply_change served an unauthorized workspace_root: {text_root}"
    );

    // And when workspace_root is the authorized repo but the edit path is
    // an absolute path into the victim directory, the ChangeEngine
    // workspace boundary must refuse (existing guarantee, re-pinned here).
    let (ok_abs, text_abs) = s.call_raw(
        "apply_change",
        serde_json::json!({
            "path": victim_path,
            "old": "VICTIM-SECRET-CONTENT\n",
            "new": "PWNED\n"
        }),
    );
    assert!(
        !ok_abs,
        "apply_change escaped its workspace via absolute path: {text_abs}"
    );

    // Traversal-shaped workspace_root into the victim must also be
    // refused (canonicalization happens before authorization).
    let victim2 = tempfile::tempdir().unwrap();
    seed_victim(victim2.path());
    // Build the traversal from a directory in the same parent as the
    // victim so the `..` canonicalizes onto the (unauthorized) victim.
    let staging = tempfile::tempdir().unwrap();
    let traversal = format!(
        "{}/../{}",
        staging.path().display(),
        victim2.path().file_name().unwrap().to_string_lossy()
    );
    let (ok_trav, text_trav) =
        s.call_raw("reindex", serde_json::json!({"workspace_root": traversal}));
    assert!(
        !ok_trav,
        "traversal workspace_root reached an unauthorized directory: {text_trav}"
    );

    assert_eq!(
        std::fs::read_to_string(&victim_file).unwrap(),
        "VICTIM-SECRET-CONTENT\n"
    );
    assert_untouched(victim.path());
    assert_untouched(victim2.path());
}

// ── Probe 3: symlink roots cannot smuggle authorization ────────────────

#[test]
fn p8_symlink_roots_cannot_escape_authorization() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "boundary_c");
    let victim = tempfile::tempdir().unwrap();
    seed_victim(victim.path());

    // A symlink INSIDE the authorized repo pointing at the victim: using
    // it as workspace_root must be denied (it canonicalizes to the victim
    // root, which is not authorized).
    #[cfg(unix)]
    {
        let link = repo.path().join("victim-link");
        std::os::unix::fs::symlink(victim.path(), &link).unwrap();
        let mut s = Server::start(repo.path(), &env.state, &env.skills);
        let (ok, text) = s.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": link.to_string_lossy().to_string()}),
        );
        assert!(
            !ok,
            "symlink workspace_root smuggled an unauthorized root: {text}"
        );
        assert_untouched(victim.path());

        // A symlink pointing AT the authorized root itself is fine: it
        // canonicalizes to the authorized root (discovery, not escape).
        let repo_link = repo.path().parent().unwrap().join("repo-link-c");
        std::os::unix::fs::symlink(repo.path(), &repo_link).unwrap();
        let (ok2, text2) = s.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": repo_link.to_string_lossy().to_string()}),
        );
        assert!(
            ok2,
            "symlink to the authorized root must resolve as the same workspace: {text2}"
        );
        let _ = std::fs::remove_file(&repo_link);
    }
}

// ── Probe 4: /etc is refused (safe refusal check, never a read) ────────

#[test]
fn p8_host_system_directories_are_refused() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "boundary_d");

    let mut s = Server::start(repo.path(), &env.state, &env.skills);

    // These are safe existence/refusal checks: we assert the server
    // refuses; we never read or write anything under these paths.
    for hostile in ["/etc", "/root", "/var/log"] {
        let (ok, text) = s.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": hostile}),
        );
        assert!(
            !ok,
            "host directory {hostile} was served as a workspace — F3 not closed: {text}"
        );
        assert!(!text.to_lowercase().contains("passwd"));
    }
}

// ── Probe 5: operator allowlist (--allow-root) authorizes extra roots ──

#[test]
fn p8_allow_root_flag_authorizes_explicit_extra_roots() {
    let env = env_new();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let c = tempfile::tempdir().unwrap();
    seed_repo(a.path(), "wsallow_a");
    seed_repo(b.path(), "wsallow_b");
    seed_repo(c.path(), "wsallow_c");

    // Launch with a as the server root and b explicitly authorized.
    let mut s = Server::start_with(a.path(), &[b.path()], &env.state, &env.skills);

    // A → A PASS (default root).
    let ctx_a = s.call(
        "workspace_context",
        serde_json::json!({"workspace_root": a.path().to_string_lossy().to_string()}),
    );
    assert_eq!(
        ctx_a["workspace_root"],
        a.path().canonicalize().unwrap().display().to_string()
    );

    // B → B PASS (operator-authorized).
    let ctx_b = s.call(
        "workspace_context",
        serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
    );
    assert_eq!(
        ctx_b["workspace_root"],
        b.path().canonicalize().unwrap().display().to_string()
    );
    // B is independently usable.
    s.call(
        "reindex",
        serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
    );
    let facts_b = s.call(
        "engineering_facts",
        serde_json::json!({
            "workspace_root": b.path().to_string_lossy().to_string(),
            "query": "wsallow_b_alpha"
        }),
    );
    assert!(
        facts_b["returned"].as_u64().unwrap_or(0) > 0,
        "authorized sibling workspace must be indexable and queryable"
    );

    // C (exists, not authorized) → FAIL with zero side effects.
    let (ok_c, text_c) = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": c.path().to_string_lossy().to_string()}),
    );
    assert!(!ok_c, "unauthorized sibling C was served: {text_c}");
    assert!(
        !c.path().join(".codebro").exists(),
        "no state may be written into unauthorized C"
    );

    // B's own calls are isolated from A (registry identity, not scope
    // expansion): B's facts do not contain A's symbols.
    let facts_a_via_b = s.call(
        "engineering_facts",
        serde_json::json!({
            "workspace_root": b.path().to_string_lossy().to_string(),
            "query": "wsallow_a_alpha"
        }),
    );
    assert_eq!(
        facts_a_via_b["returned"].as_u64().unwrap_or(0),
        0,
        "A's symbols must not resolve in B's workspace"
    );
}

// ── Probe 6: CODEBRO_ALLOW_ROOTS env var authorizes extra roots ────────

#[test]
fn p8_allow_roots_env_var_authorizes_extra_roots() {
    let env = env_new();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    seed_repo(a.path(), "wsenv_a");
    seed_repo(b.path(), "wsenv_b");

    let bin = env!("CARGO_BIN_EXE_codebro");
    let allow = b.path().to_string_lossy().into_owned();
    let mut child = Proc::new(bin)
        .args(["serve", "--root"])
        .arg(a.path())
        .env("CODEBRO_STATE_DIR", &env.state)
        .env("CODEBRO_SKILLS_DIR", &env.skills)
        .env("CODEBRO_ALLOW_ROOTS", &allow)
        .env("RUST_LOG", "off")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn codebro serve");
    let mut s = Server {
        stdin: BufWriter::new(child.stdin.take().unwrap()),
        reader: BufReader::new(child.stdout.take().unwrap()),
        stderr: Arc::new(Mutex::new(String::new())),
        child,
    };
    s.rpc(
        "initialize",
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "p8-boundary-probe-env", "version": "0"}
        }),
    );
    s.stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();
    s.stdin.flush().unwrap();

    // B is authorized via the env var.
    let ctx_b = s.call(
        "workspace_context",
        serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
    );
    assert_eq!(
        ctx_b["workspace_root"],
        b.path().canonicalize().unwrap().display().to_string()
    );
}

// ── Probe 7: nested + overlapping authorized roots keep exact-root
//    ownership and deterministic refusals ───────────────────────────────

#[test]
fn p8_nested_and_overlapping_roots_are_explicit_and_isolated() {
    let env = env_new();
    let parent = tempfile::tempdir().unwrap();
    let outer = parent.path().join("outer");
    let inner = outer.join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    seed_repo(&outer, "nested_outer");
    seed_repo(&inner, "nested_inner");

    // Authorize both the outer root and the nested inner root.
    let mut s = Server::start_with(&outer, &[inner.as_path()], &env.state, &env.skills);

    // Both resolve; each owns its exact canonical root.
    let ctx_outer = s.call(
        "workspace_context",
        serde_json::json!({"workspace_root": outer.to_string_lossy().to_string()}),
    );
    assert_eq!(
        ctx_outer["workspace_root"],
        outer.canonicalize().unwrap().display().to_string()
    );
    let ctx_inner = s.call(
        "workspace_context",
        serde_json::json!({"workspace_root": inner.to_string_lossy().to_string()}),
    );
    assert_eq!(
        ctx_inner["workspace_root"],
        inner.canonicalize().unwrap().display().to_string()
    );

    // A sibling of outer that is NOT authorized (despite living under the
    // same parent) is refused.
    let stranger = parent.path().join("stranger");
    std::fs::create_dir_all(&stranger).unwrap();
    let (ok, text) = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": stranger.to_string_lossy().to_string()}),
    );
    assert!(!ok, "unauthorized stranger served: {text}");
    assert!(
        !stranger.join(".codebro").exists(),
        "no state written into the unauthorized stranger directory"
    );

    // Authorization is not prefix-based: the outer root being authorized
    // does NOT authorize its subdirectories that were not explicitly
    // allow-listed.
    let other_sub = outer.join("other-sub");
    std::fs::create_dir_all(&other_sub).unwrap();
    let (ok_sub, text_sub) = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": other_sub.to_string_lossy().to_string()}),
    );
    assert!(
        !ok_sub,
        "authorization leaked from outer root to an un-listed subdirectory: {text_sub}"
    );
}

// ── Probe 8: restart + hard-kill determinism of authorization ──────────

#[test]
fn p8_authorization_survives_restart_and_hard_kill() {
    let env = env_new();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    seed_repo(a.path(), "restart_auth_a");
    seed_repo(b.path(), "restart_auth_b");

    // Phase 1: plain server (a only). b refused.
    {
        let mut s = Server::start(a.path(), &env.state, &env.skills);
        let (ok, _) = s.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
        );
        assert!(!ok, "b must be refused pre-authorization");
        // Hard kill (SIGKILL semantics via Drop kill).
    }

    // Phase 2: restart with the same launch config. Authorization must
    // not broaden: b still refused.
    {
        let mut s2 = Server::start(a.path(), &env.state, &env.skills);
        let (ok, _) = s2.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
        );
        assert!(!ok, "authorization broadened after restart");
        // a still authorized.
        let ctx = s2.call("workspace_context", serde_json::json!({}));
        assert_eq!(
            ctx["workspace_root"],
            a.path().canonicalize().unwrap().display().to_string()
        );
    }

    // Phase 3: restart WITH the operator allowlist. b now authorized —
    // authorization only changes through operator launch config, never
    // through prior tool traffic.
    {
        let mut s3 = Server::start_with(a.path(), &[b.path()], &env.state, &env.skills);
        let (ok, text) = s3.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
        );
        assert!(ok, "operator allowlist must authorize b: {text}");
    }

    // Phase 4: drop the allowlist again — b is refused again (config-
    // derived, no sticky authorization).
    {
        let mut s4 = Server::start(a.path(), &env.state, &env.skills);
        let (ok, _) = s4.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": b.path().to_string_lossy().to_string()}),
        );
        assert!(!ok, "authorization must not persist past reconfiguration");
    }
}

// ── Probe 9: stdout purity + stderr redaction on authorization errors ──

#[test]
fn p8_authorization_errors_keep_stdout_pure_and_stderr_redacted() {
    let env = env_new();
    let repo = tempfile::tempdir().unwrap();
    seed_repo(repo.path(), "purity_auth");
    let victim = tempfile::tempdir().unwrap();
    seed_victim(victim.path());

    let mut s = Server::start(repo.path(), &env.state, &env.skills);

    // Secret-shaped workspace_root: the refusal text echoes the caller's
    // own input on the same channel (documented convention), but the
    // stderr log must carry it only redacted.
    let secret_path = format!("{}/sk-BOUNDARYSECRET1234567890", victim.path().display());
    let (ok, _) = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": secret_path}),
    );
    assert!(!ok, "secret-shaped root must still be refused (not usable)");

    let err_buf = s.stderr_so_far();
    assert!(
        !err_buf.contains("sk-BOUNDARYSECRET1234567890"),
        "raw secret reached stderr: {err_buf}"
    );

    // And an authorized-root refusal sweep keeps every stdout line
    // JSON-RPC (the strict reader above already enforces this for every
    // call in this probe — a few more error shapes to be thorough).
    for bad in ["", "not-a-root", "/definitely/not/a/real/dir"] {
        let _ = s.call_raw(
            "workspace_context",
            serde_json::json!({"workspace_root": bad}),
        );
    }
    // File-instead-of-directory root.
    let file_root = repo.path().join("Cargo.toml");
    let _ = s.call_raw(
        "workspace_context",
        serde_json::json!({"workspace_root": file_root.to_string_lossy().to_string()}),
    );
}

// ── Probe 10: authorized multi-root sessions stay isolated end to end ──

#[test]
fn p8_authorized_multi_root_sessions_stay_isolated() {
    let env = env_new();
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    seed_repo(a.path(), "multi_a");
    seed_repo(b.path(), "multi_b");

    let mut s = Server::start_with(a.path(), &[b.path()], &env.state, &env.skills);

    let wa = a.path().to_string_lossy().into_owned();
    let wb = b.path().to_string_lossy().into_owned();

    // Seed distinct durable state in each.
    s.call("reindex", serde_json::json!({"workspace_root": wa}));
    s.call("reindex", serde_json::json!({"workspace_root": wb}));
    let tr = s.call(
        "task",
        serde_json::json!({
            "workspace_root": wa, "action": "create",
            "title": "MULTI-A-TASK-MARKER"
        }),
    );
    let tid = tr["task"]["task_id"].as_str().unwrap().to_string();
    s.call(
        "record_memory",
        serde_json::json!({
            "workspace_root": wa, "key": "multi:a", "value": "MULTI-A-MEMORY-MARKER"
        }),
    );

    // B's surfaces carry none of A's markers.
    let (ok_inspect, text_inspect) = s.call_raw(
        "task",
        serde_json::json!({"workspace_root": wb, "action": "inspect", "task_id": tid}),
    );
    assert!(!ok_inspect, "cross-workspace task inspect must fail");
    assert!(
        !text_inspect.contains("MULTI-A-TASK-MARKER"),
        "A's task title leaked into B's refusal: {text_inspect}"
    );

    let brief_b = s.call(
        "engineering_brief",
        serde_json::json!({"workspace_root": wb, "task": "multi_b_alpha work"}),
    );
    let bb = serde_json::to_string(&brief_b).unwrap();
    assert!(
        !bb.contains("MULTI-A-TASK-MARKER") && !bb.contains("MULTI-A-MEMORY-MARKER"),
        "A's durable state leaked into B's brief"
    );
    assert!(bb.contains("multi_b"), "B's brief must be about B");

    // A's own brief still surfaces A's state (sanity).
    let brief_a = s.call(
        "engineering_brief",
        serde_json::json!({"workspace_root": wa, "task": "multi_a_alpha work"}),
    );
    let ba = serde_json::to_string(&brief_a).unwrap();
    assert!(
        ba.contains("MULTI-A-MEMORY-MARKER") || ba.contains("multi_a"),
        "A's own brief must reference A"
    );
}

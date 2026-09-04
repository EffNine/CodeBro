//! Phase-12 integration: the Execution Evidence Journal wired through the
//! real MCP server. Covers record-on-run, prior-evidence surfacing,
//! tree-hash distinction, restart persistence, isolation from facts/memory,
//! and response boundedness.

use std::io::{BufRead, BufWriter, Write as _};
use std::path::Path;
use std::process::{Child, Command as Proc, Stdio};

fn git(cwd: &Path, args: &[&str]) {
    let status = Proc::new("git")
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("git");
    assert!(status.success(), "git {args:?} failed");
}

/// A minimal git-tracked cargo project whose single test can be flipped
/// between passing and failing by editing one line.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(name: &str, expect: &str) -> Self {
        let dir = tempfile::Builder::new().prefix(name).tempdir().unwrap();
        let p = dir.path();
        std::fs::write(
            p.join("Cargo.toml"),
            "[package]\nname = \"fx\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::create_dir_all(p.join("src")).unwrap();
        std::fs::write(
            p.join("src/lib.rs"),
            format!(
                "#[cfg(test)]\nmod tests {{\n    #[test]\n    fn adds() {{\n        assert_eq!(2 + 2, {expect});\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(p.join(".gitignore"), "/target\nCargo.lock\n.codebro/\n").unwrap();
        git(p, &["init", "-q"]);
        git(p, &["add", "-A"]);
        git(
            p,
            &[
                "-c",
                "user.email=t@t",
                "-c",
                "user.name=t",
                "commit",
                "-qm",
                "init",
            ],
        );
        Fixture { dir }
    }

    fn flip_expectation(&self, expect: u32) {
        // Distinct content => distinct tree hash.
        let line = format!("        assert_eq!(2 + 2, {expect});");
        let src = std::fs::read_to_string(self.dir.path().join("src/lib.rs")).unwrap();
        let updated = src
            .lines()
            .map(|l| {
                if l.trim_start().starts_with("assert_eq!") {
                    line.clone()
                } else {
                    l.to_string()
                }
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(self.dir.path().join("src/lib.rs"), updated).unwrap();
    }
}

struct Server {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    reader: std::io::BufReader<ChildStdout>,
}

type ChildStdin = std::process::ChildStdin;
type ChildStdout = std::process::ChildStdout;

impl Server {
    fn start(root: &Path) -> Self {
        let bin = env!("CARGO_BIN_EXE_codebro");
        let mut child = Proc::new(bin)
            .args(["serve", "--root"])
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn codebro serve");
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let reader = std::io::BufReader::new(child.stdout.take().unwrap());
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
                "clientInfo": {"name": "p12", "version": "0"}
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
        let text = r["result"]["content"][0]["text"].as_str().unwrap_or("");
        serde_json::from_str(text).unwrap_or(serde_json::json!({}))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 1/2. Executions are journaled; 5/7. same-tree prior evidence surfaces on
/// a repeat run; 15. survives process restart (second server instance).
#[test]
fn journal_records_and_surfaces_same_tree_prior_run() {
    let fx = Fixture::new("evj-pass", "4");
    let root = fx.dir.path();

    let mut s1 = Server::start(root);
    let first = s1.call("sandbox_test", serde_json::json!({}));
    let v1 = &first["verification"];
    // Empty history: clean behavior — no prior_evidence field at all.
    assert!(
        v1.get("prior_evidence").is_none(),
        "{}",
        json_dump(v1.get("prior_evidence"))
    );
    assert!(root.join(".codebro/execution_evidence.json").exists());

    // Restart: a brand-new server process must see the persisted record.
    drop(s1);
    let mut s2 = Server::start(root);
    let second = s2.call("sandbox_test", serde_json::json!({}));
    let v2 = &second["verification"];
    let prior = v2
        .get("prior_evidence")
        .expect("same-tree prior must surface");
    let st = &prior["same_tree"];
    assert_eq!(st["outcome"], "success", "{}", json_dump(Some(prior)));
    assert!(st["age_seconds"].is_u64());
    drop(s2);
}

fn json_dump(v: impl std::fmt::Debug) -> String {
    format!("{v:?}")
}

/// Tree-hash distinction: after modifying the repository, the same-tree
/// lookup no longer matches the old tree's record.
#[test]
fn modified_tree_gets_fresh_journal_context() {
    let fx = Fixture::new("evj-mod", "4");
    let root = fx.dir.path();

    let mut s1 = Server::start(root);
    let _ = s1.call("sandbox_test", serde_json::json!({}));
    drop(s1);

    fx.flip_expectation(5); // changes working tree => different tree hash

    let mut s2 = Server::start(root);
    let resp = s2.call("sandbox_test", serde_json::json!({}));
    let v = &resp["verification"];
    if let Some(prior) = v.get("prior_evidence") {
        assert!(
            prior.get("same_tree").is_none(),
            "old tree's record must not match the new tree: {}",
            json_dump(Some(prior))
        );
    }
}

/// Failures are journaled; repeated failures across DISTINCT tree states
/// surface as deterministic repeated_failure patterns.
#[test]
fn repeated_failures_across_trees_surface() {
    let fx = Fixture::new("evj-fail", "5"); // failing baseline
    let root = fx.dir.path();

    let mut s1 = Server::start(root);
    let r1 = s1.call("sandbox_test", serde_json::json!({}));
    assert_eq!(r1["verification"]["classification"], "test_failure");
    drop(s1);

    fx.flip_expectation(6); // still failing, different tree
    let mut s2 = Server::start(root);
    let r2 = s2.call("sandbox_test", serde_json::json!({}));
    assert_eq!(r2["verification"]["classification"], "test_failure");
    drop(s2);

    // The pattern surfaces on the NEXT run: by then two failures across two
    // distinct tree states are journaled.
    fx.flip_expectation(7);
    let mut s3 = Server::start(root);
    let r3 = s3.call("sandbox_test", serde_json::json!({}));
    assert_eq!(r3["verification"]["classification"], "test_failure");
    let prior = r3["verification"]
        .get("prior_evidence")
        .expect("two failures across two trees must surface");
    let reps = prior
        .get("repeated_failures")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        reps.iter().any(|p| p["pattern"] == "repeated_failure"
            && p["failures"] == 2
            && p["distinct_trees"] == 2),
        "{}",
        json_dump(Some(prior))
    );
}

/// Isolation: running validations never modifies fact-store or memory state.
#[test]
fn validation_runs_never_touch_facts_or_memory() {
    let fx = Fixture::new("evj-iso", "4");
    let root = fx.dir.path();

    // Seed recognizable state files.
    std::fs::create_dir_all(root.join(".codebro")).unwrap();
    std::fs::write(root.join(".codebro/facts.json"), br#"{"seed":true}"#).unwrap();
    std::fs::write(
        root.join(".codebro/engineering_memory.json"),
        br#"{"seed":true}"#,
    )
    .unwrap();

    let mut s = Server::start(root);
    let _ = s.call("sandbox_test", serde_json::json!({}));
    let _ = s.call("sandbox_test", serde_json::json!({}));
    drop(s);

    assert_eq!(
        std::fs::read(root.join(".codebro/facts.json")).unwrap(),
        b"{\"seed\":true}".as_slice()
    );
    assert_eq!(
        std::fs::read(root.join(".codebro/engineering_memory.json")).unwrap(),
        b"{\"seed\":true}".as_slice()
    );
}

/// Prior-evidence payloads stay compact (< 4KB serialized section).
#[test]
fn prior_evidence_section_stays_bounded() {
    let fx = Fixture::new("evj-bound", "4");
    let root = fx.dir.path();
    let mut s = Server::start(root);
    let _ = s.call("sandbox_test", serde_json::json!({}));
    drop(s);
    let mut s2 = Server::start(root);
    let resp = s2.call("sandbox_test", serde_json::json!({}));
    if let Some(prior) = resp["verification"].get("prior_evidence") {
        let size = serde_json::to_string(prior).unwrap().len();
        assert!(size < 4096, "{size}");
    }
}

/// sandbox_build records and surfaces same-tree prior evidence, using its
/// own (command, empty-filter) invocation identity — isolated from the
/// sandbox_test history by design.
#[test]
fn journal_records_and_surfaces_for_sandbox_build() {
    let fx = Fixture::new("evj-build", "4");
    let root = fx.dir.path();

    let mut s1 = Server::start(root);
    let first = s1.call("sandbox_build", serde_json::json!({}));
    assert!(
        first["verification"].get("prior_evidence").is_none(),
        "first build has no history: {:?}",
        first["verification"].get("prior_evidence")
    );
    assert!(root.join(".codebro/execution_evidence.json").exists());
    drop(s1);

    // Restart: a brand-new server process must see the persisted build record.
    let mut s2 = Server::start(root);
    let second = s2.call("sandbox_build", serde_json::json!({}));
    let prior = second["verification"]
        .get("prior_evidence")
        .expect("same-tree prior build must surface");
    assert_eq!(prior["same_tree"]["outcome"], "success", "{prior:?}");
    assert!(prior["same_tree"]["age_seconds"].is_u64());
    drop(s2);
}

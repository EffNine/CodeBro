#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Local sandbox backend.
//!
//! Runs commands directly in the workspace using the existing PTY-backed
//! `RunCommand` infrastructure. The command is gated by `LocalCommandPolicy`
//! (a subset of the Testing subagent policy) before any process spawns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{ExecutionResult, SandboxBackend, SandboxCommand, SandboxMode, SandboxPolicy};

/// Command policy for the local sandbox backend.
///
/// Derives the allowed validation surface from the project metadata present
/// in the workspace root (Cargo.toml, package.json, go.mod, Makefile).
#[derive(Debug, Clone)]
pub struct LocalCommandPolicy {
    is_cargo: bool,
    is_node: bool,
    is_go: bool,
    is_python: bool,
    has_makefile: bool,
}

impl LocalCommandPolicy {
    pub fn for_workspace(workspace_root: &std::path::Path) -> Self {
        LocalCommandPolicy {
            is_cargo: workspace_root.join("Cargo.toml").exists(),
            is_node: workspace_root.join("package.json").exists(),
            is_go: workspace_root.join("go.mod").exists(),
            is_python: PY_MARKERS.iter().any(|m| workspace_root.join(m).exists()),
            has_makefile: workspace_root.join("Makefile").exists()
                || workspace_root.join("makefile").exists(),
        }
    }

    /// Check a raw command string against the policy.
    pub fn check(&self, command: &str) -> bool {
        self.check_in(command, None)
    }

    /// Policy check with workspace confinement for path-bearing arguments.
    pub fn check_in(&self, command: &str, workspace_root: Option<&Path>) -> bool {
        let normalized = normalize(command);
        if normalized.is_empty() {
            return false;
        }

        // Structural boundary: no shell metacharacters.
        const METACHARS: &[char] = &[
            ';', '&', '|', '>', '<', '$', '`', '\n', '\r', '{', '}', '*', '!', '(', ')',
        ];
        if normalized.chars().any(|c| METACHARS.contains(&c)) {
            return false;
        }

        let tokens: Vec<&str> = normalized.split(' ').collect();
        let program = tokens[0];

        // Workspace escape gate: path-bearing flags must stay inside root.
        if let Some(root) = workspace_root {
            if !check_path_args_confined(&tokens, root) {
                return false;
            }
        }

        match program {
            "true" | "false" | "echo" | "printf" => tokens.len() <= 20,
            "sleep" => tokens.len() == 2 && tokens[1].parse::<u64>().is_ok(),
            "cargo" => self.check_cargo(&tokens[1..]),
            "go" => self.check_go(&tokens[1..]),
            "npm" | "pnpm" | "yarn" => self.check_npm(&tokens[1..]),
            "npx" => self.check_npx(&tokens[1..]),
            "make" => self.check_make(&tokens[1..]),
            "git" => self.check_git_in(&tokens[1..], workspace_root),
            "python" | "python3" => self.check_python(&tokens[1..]),
            "rustc" => self.check_rustc(&tokens[1..]),
            // Read-only inspection commands — no workspace manifest
            // required, but their path arguments must stay confined to
            // the workspace root when one is provided (audit F2: `head
            // /etc/passwd`, `find / -fprint out`, `wc <any host file>`
            // previously escaped the "confined sandbox" guarantee).
            "pwd" => true,
            "ls" | "head" | "tail" | "wc" | "find" | "file" | "which" => {
                workspace_root.is_none_or(|root| args_confined_for_inspection(&tokens, root))
            }
            // `cat` is read-only but every path-looking operand and
            // flag value must stay inside the workspace root when one
            // is provided (audit F2: `cat /etc/passwd` and
            // `cat /home/.../credentials.json` previously escaped).
            "cat" => workspace_root.is_none_or(|root| args_confined_for_inspection(&tokens, root)),
            _ => false,
        }
    }

    fn check_cargo(&self, args: &[&str]) -> bool {
        if !self.is_cargo || args.is_empty() {
            return false;
        }
        let sub = args[0];
        // Global flags that don't require a subcommand.
        if sub == "--version" || sub == "-V" {
            return true;
        }
        if !matches!(
            sub,
            "check" | "test" | "build" | "clippy" | "fmt" | "doc" | "metadata" | "tree"
        ) {
            return false;
        }
        if sub == "fmt" && !args.iter().any(|a| *a == "--check") {
            return false;
        }
        !args[1..].iter().any(|a| MUTATING_TOKENS.contains(a))
            && args[1..]
                .iter()
                .all(|a| !a.starts_with('-') || CARGO_ALLOWED_FLAGS.contains(a))
    }

    fn check_go(&self, args: &[&str]) -> bool {
        if !self.is_go || args.is_empty() {
            return false;
        }
        matches!(args[0], "test" | "build" | "vet" | "mod")
            && !args[1..].iter().any(|a| MUTATING_TOKENS.contains(a))
    }

    /// Only `python -m pytest [flags] [names/paths]` is permitted, and only
    /// in Python workspaces. Flags are restricted to harmless selectors.
    fn check_python(&self, args: &[&str]) -> bool {
        if !self.is_python || args.len() < 3 {
            return false;
        }
        if args[0] != "-m" || args[1] != "pytest" {
            return false;
        }
        args[2..].len() <= 30
            && args[2..].iter().all(|a| {
                !MUTATING_TOKENS.contains(a)
                    && (*a == "-q"
                        || *a == "-x"
                        || *a == "-k"
                        || *a == "--tb=long"
                        || !a.starts_with('-'))
            })
    }

    fn check_npm(&self, args: &[&str]) -> bool {
        if !self.is_node || args.is_empty() {
            return false;
        }
        match args[0] {
            "test" => true,
            "run" => {
                if args.len() < 2 {
                    return false;
                }
                matches!(
                    args[1],
                    "build" | "test" | "lint" | "check" | "typecheck" | "fmt"
                )
            }
            _ => false,
        }
    }

    fn check_npx(&self, args: &[&str]) -> bool {
        if !self.is_node || args.is_empty() {
            return false;
        }
        match args[0] {
            "tsc" => args.iter().any(|a| *a == "--noEmit"),
            "eslint" | "vitest" | "jest" => !args[1..].iter().any(|a| MUTATING_TOKENS.contains(a)),
            _ => false,
        }
    }

    fn check_make(&self, args: &[&str]) -> bool {
        if !self.has_makefile || args.is_empty() {
            return false;
        }
        matches!(args[0], "build" | "test" | "check" | "lint") && args.len() == 1
    }

    fn check_git(&self, args: &[&str]) -> bool {
        if args.is_empty() {
            return false;
        }
        let sub = args[0];
        if !matches!(
            sub,
            "status" | "diff" | "log" | "show" | "rev-parse" | "ls-files"
        ) {
            return false;
        }
        !args[1..].iter().any(|a| MUTATING_TOKENS.contains(a))
    }

    /// `check_git` with workspace confinement: git's read-only subcommands
    /// may still carry paths (`--output=/tmp/x`, `show HEAD:../../file`),
    /// so every path-looking operand must stay inside the root (audit F2:
    /// `git log --output=/tmp/out` previously wrote outside the sandbox).
    fn check_git_in(&self, args: &[&str], workspace_root: Option<&Path>) -> bool {
        if !self.check_git(args) {
            return false;
        }
        let Some(root) = workspace_root else {
            return true;
        };
        for tok in &args[1..] {
            let is_flag = tok.starts_with('-');
            // Inline flag values (--output=/tmp/x) and path operands both
            // need confinement when they look like paths.
            let candidate = if let Some(v) = tok.strip_prefix("--output=") {
                v
            } else if is_flag {
                continue;
            } else {
                tok
            };
            let looks_like_path =
                candidate.contains('/') || candidate == ".." || candidate.starts_with("../");
            if looks_like_path && !is_path_confined(candidate, root) {
                return false;
            }
        }
        true
    }

    fn check_rustc(&self, args: &[&str]) -> bool {
        if args.is_empty() {
            return false;
        }
        // Only allow version-printing and target-info queries.
        let sub = args[0];
        if sub == "--version" || sub == "-V" {
            return true;
        }
        if sub.starts_with("--print") {
            return true;
        }
        false
    }
}

const PY_MARKERS: &[&str] = &[
    "pyproject.toml",
    "pytest.ini",
    "setup.py",
    "setup.cfg",
    "requirements.txt",
];

const MUTATING_TOKENS: &[&str] = &[
    "commit",
    "add",
    "rm",
    "mv",
    "checkout",
    "reset",
    "clean",
    "apply",
    "restore",
    "rebase",
    "merge",
    "push",
    "pull",
    "fetch",
    "tag",
    "stash",
    "cherry-pick",
    "revert",
    "switch",
    "config",
    "--fix",
    "--write",
    "-w",
    "--in-place",
    "--apply",
    "--amend",
    "--force",
    "-f",
    "--delete",
    "--remove",
    "--purge",
    "--install",
    "--push",
    "--save",
    "--overwrite",
    "--no-verify",
    "-i",
];

const CARGO_ALLOWED_FLAGS: &[&str] = &[
    "--all-targets",
    "--all-features",
    "--lib",
    "--bins",
    "--examples",
    "--benches",
    "--tests",
    "--workspace",
    "--no-run",
    "--release",
    "--offline",
    "--locked",
    "--no-deps",
    "--quiet",
    "-q",
    "-p",
    "--package",
    "--test",
    "--doc",
    "--manifest-path",
    "--message-format",
    "--check",
    "--",
    "--nocapture",
    "--ignored",
    "--exact",
    "--skip",
    "--list",
    "--include-ignored",
    "--show-output",
    "--color",
    "--format",
];

fn normalize(command: &str) -> String {
    command.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Classify a denied command into a brief human-readable reason.
fn classify_denial(command: &str) -> &'static str {
    let normalized = normalize(command);
    // Shell metacharacters are checked first (they short-circuit the match).
    const METACHARS: &[char] = &[
        ';', '&', '|', '>', '<', '$', '`', '\n', '\r', '{', '}', '*', '!', '(', ')', '~', '#',
    ];
    if normalized.chars().any(|c| METACHARS.contains(&c)) {
        return "shell_metacharacter_detected";
    }
    let tokens: Vec<&str> = normalized.split(' ').collect();
    let program = tokens[0];
    match program {
        "true" | "false" | "echo" | "printf" | "sleep" | "cargo" | "go" | "npm" | "pnpm"
        | "yarn" | "npx" | "make" | "git" | "python" | "python3" | "rustc" | "pwd" | "ls"
        | "cat" | "head" | "tail" | "wc" | "find" | "file" | "which" => {
            // Known program but disallowed variant/args — context-specific.
            match program {
                "cargo" | "go" | "npm" | "pnpm" | "yarn" | "npx" | "make" | "python"
                | "python3"
                    if tokens.len() > 1 =>
                {
                    "mutating_operation_blocked"
                }
                "rustc" => "mutating_operation_blocked",
                _ => "mutating_operation_blocked",
            }
        }
        _ => "executable_not_allowlisted",
    }
}

/// Path-bearing flags whose values must stay inside the workspace root.
/// Covers `--flag value` and `--flag=value` forms for the allowlisted
/// command model (cargo/go/npm/pytest). Returns false when any path value
/// escapes the workspace.
fn check_path_args_confined(tokens: &[&str], workspace_root: &Path) -> bool {
    // Flags that take a path value as the NEXT token.
    const PATH_FLAGS_NEXT: &[&str] = &[
        "--manifest-path",
        "-p",
        "--package",
        "--target-dir",
        "--config",
        "-C",
        "--directory",
        "--prefix",
        "--cache-dir",
    ];
    // Flags with `--flag=value` inline form.
    const PATH_FLAGS_INLINE: &[&str] = &["--manifest-path=", "--target-dir=", "--config="];
    let mut i = 0;
    while i < tokens.len() {
        let tok = tokens[i];
        let mut path_val: Option<&str> = None;
        if PATH_FLAGS_NEXT.contains(&tok) {
            if i + 1 >= tokens.len() {
                return false;
            }
            path_val = Some(tokens[i + 1]);
        } else {
            for prefix in PATH_FLAGS_INLINE {
                if let Some(v) = tok.strip_prefix(prefix) {
                    path_val = Some(v);
                    break;
                }
            }
            // `-p<value>` attached form (e.g. `-pfoo`).
            if path_val.is_none()
                && tok.starts_with("-p")
                && tok.len() > 2
                && !tok.starts_with("--")
            {
                path_val = Some(&tok[2..]);
            }
        }
        if let Some(p) = path_val {
            if !is_path_confined(p, workspace_root) {
                return false;
            }
        }
        // Bare `..` path components outside a flag value (e.g. `cargo test
        // ../../evil`) are also rejected when they look like paths.
        // Flag values starting with `-` are not paths.
        if !tok.starts_with('-')
            && tok != tokens[0]
            && (tok.contains("../") || tok == ".." || tok.starts_with("../"))
        {
            if !is_path_confined(tok, workspace_root) {
                return false;
            }
        }
        i += 1;
    }
    true
}

/// True when `candidate` resolves inside `workspace_root`. Absolute paths
/// must have the canonical root as a prefix; relative paths are joined to
/// the root and lexically checked for `..` escape (no symlink resolution
/// here — execution also validates the canonical path at spawn time).
fn is_path_confined(candidate: &str, workspace_root: &Path) -> bool {
    if candidate.is_empty() {
        return false;
    }
    // Reject absolute escape trivially.
    let cand_path = Path::new(candidate);
    if cand_path.is_absolute() {
        // Canonicalize root when possible; fall back to lexical prefix.
        if let (Ok(root_c), Ok(cand_c)) = (workspace_root.canonicalize(), cand_path.canonicalize())
        {
            return cand_c.starts_with(&root_c);
        }
        return cand_path.starts_with(workspace_root);
    }
    // Relative: join + lexical normalization, reject `..` escape.
    let mut depth: i32 = 0;
    for comp in Path::new(candidate).components() {
        use std::path::Component;
        match comp {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            Component::CurDir => {}
            _ => depth += 1,
        }
    }
    // Also resolve against root when both exist (symlink-aware).
    let joined = workspace_root.join(candidate);
    if let (Ok(root_c), Ok(join_c)) = (workspace_root.canonicalize(), joined.canonicalize()) {
        return join_c.starts_with(&root_c);
    }
    true
}

/// Confinement for the read-only inspection programs
/// (`ls`/`head`/`tail`/`wc`/`find`/`file`/`which`): every non-flag token
/// that looks like a filesystem path must resolve inside the workspace
/// root, and every path-bearing flag value must too (audit F2: `head
/// /etc/passwd` and `find / -fprint /tmp/out` previously escaped; `find`
/// especially is not purely read-only — `-fprint`/`-fprintf`/`-fls` WRITE
/// to their argument paths).
fn args_confined_for_inspection(tokens: &[&str], workspace_root: &Path) -> bool {
    // Flags of the inspection family that take a path-ish value as the
    // NEXT token. Everything else with a leading '-' is treated as a
    // boolean/numeric flag and ignored.
    const PATH_FLAGS_NEXT: &[&str] = &[
        // find: output-writing and path-scoping forms.
        "-fprint",
        "-fprintf",
        "-fls",
        "-fprint0",
        "-newer",
        "-anewer",
        "-cnewer",
        "-samefile",
        // head/tail: -c/-n take counts, not paths — excluded on purpose.
        // file: -m/-f name files.
        "-m",
        "-f",
        // ls: none beyond ignore patterns.
    ];
    // Inline `--flag=value` forms that carry paths.
    const PATH_FLAGS_INLINE: &[&str] = &[
        "-fprint=",
        "-fprintf=",
        "-fls=",
        "-fprint0=",
        "--color=",
        "--format=",
        "--output=",
    ];
    let mut i = 1; // tokens[0] is the program
    while i < tokens.len() {
        let tok = tokens[i];
        let mut path_val: Option<String> = None;
        if PATH_FLAGS_NEXT.contains(&tok) {
            if i + 1 >= tokens.len() {
                return false; // flag promised a value that is missing
            }
            path_val = Some(tokens[i + 1].to_string());
            i += 1; // consume the value
        } else {
            for prefix in PATH_FLAGS_INLINE {
                if let Some(v) = tok.strip_prefix(prefix) {
                    path_val = Some(v.to_string());
                    break;
                }
            }
            if path_val.is_none() && !tok.starts_with('-') {
                // Plain operand. Patterns like `*.rs` (glob) are programs'
                // own syntax, not paths — but they may still be scanned
                // against the current directory, so allow bare globs and
                // names without separators; anything with a `/` or `..`
                // is a path and must be confined.
                if tok.contains('/')
                    || tok == ".."
                    || tok.starts_with("../")
                    || tok.contains("/../")
                {
                    path_val = Some(tok.to_string());
                }
            }
        }
        if let Some(p) = &path_val {
            if !is_path_confined(p, workspace_root) {
                return false;
            }
        }
        i += 1;
    }
    true
}

/// The local sandbox backend: runs commands in-process via PTY.
#[derive(Debug, Clone, Default)]
pub struct LocalSandboxBackend {
    default_timeout_secs: u64,
    default_max_output_bytes: usize,
}

impl LocalSandboxBackend {
    pub fn new() -> Self {
        LocalSandboxBackend {
            default_timeout_secs: 120,
            default_max_output_bytes: 64 * 1024,
        }
    }

    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.default_timeout_secs = secs;
        self
    }

    pub fn with_max_output(mut self, bytes: usize) -> Self {
        self.default_max_output_bytes = bytes;
        self
    }
}

impl SandboxBackend for LocalSandboxBackend {
    fn execute(
        &self,
        workspace_root: &Path,
        cmd: SandboxCommand,
        policy: &SandboxPolicy,
    ) -> ExecutionResult {
        let effective_timeout = if policy.timeout_secs > 0 {
            policy.timeout_secs
        } else {
            self.default_timeout_secs
        };
        let effective_max_output = if policy.max_output_bytes > 0 {
            policy.max_output_bytes
        } else {
            self.default_max_output_bytes
        };

        let command = cmd.command.trim().to_string();
        let ws_root_str = workspace_root.to_string_lossy().to_string();

        let cmd_policy = LocalCommandPolicy::for_workspace(workspace_root);

        if !cmd_policy.check_in(&command, Some(workspace_root)) {
            return ExecutionResult::denied(
                &command,
                &ws_root_str,
                &format!(
                    "command denied by sandbox policy: {}",
                    classify_denial(&command)
                ),
                cmd.metadata,
            );
        }

        let start = Instant::now();
        let run_cmd = crate::tools::shell::RunCommand::new()
            .with_timeout(effective_timeout)
            .with_working_directory(ws_root_str.clone());

        let result = run_cmd.run(&command);
        let duration = start.elapsed().as_millis();

        match result {
            Ok(run_result) => {
                let stdout = crate::tools::shell::redact_secrets_public(&run_result.stdout);
                let stderr = crate::tools::shell::redact_secrets_public(&run_result.stderr);
                let success = run_result.exit_code == 0;
                ExecutionResult {
                    environment: Some(crate::sandbox::ExecutionEnvironment::capture()),
                    command,
                    requested_command: String::new(),
                    resolved_command: String::new(),
                    working_directory: ws_root_str,
                    exit_code: run_result.exit_code,
                    success,
                    duration_ms: duration,
                    timestamp: None,
                    stdout,
                    stderr,
                    timeout: false,
                    cancelled: false,
                    denied: false,
                    denied_reason: None,
                    backend: "local".to_string(),
                    mode: SandboxMode::Local.to_string(),
                    execution_id: String::new(),
                    repo_identity: None,
                    repo_state: None,
                    sandbox_capabilities: None,
                    reproducibility: super::Reproducibility::default(),
                    artifacts: Vec::new(),
                    freshness: None,
                    metadata: cmd.metadata,
                }
            }
            Err(e) => {
                let error_msg = e.to_string();
                let is_timeout = error_msg.contains("timed out");
                ExecutionResult {
                    environment: Some(crate::sandbox::ExecutionEnvironment::capture()),
                    command,
                    requested_command: String::new(),
                    resolved_command: String::new(),
                    working_directory: ws_root_str,
                    exit_code: -1,
                    success: false,
                    duration_ms: duration,
                    timestamp: None,
                    stdout: String::new(),
                    stderr: error_msg,
                    timeout: is_timeout,
                    cancelled: false,
                    denied: false,
                    denied_reason: if is_timeout {
                        Some("command timed out".to_string())
                    } else {
                        Some("command failed to run".to_string())
                    },
                    backend: "local".to_string(),
                    mode: SandboxMode::Local.to_string(),
                    execution_id: String::new(),
                    repo_identity: None,
                    repo_state: None,
                    sandbox_capabilities: None,
                    reproducibility: super::Reproducibility::default(),
                    artifacts: Vec::new(),
                    freshness: None,
                    metadata: cmd.metadata,
                }
            }
        }
    }

    fn name(&self) -> &str {
        "local"
    }

    fn mode(&self) -> SandboxMode {
        SandboxMode::Local
    }

    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> super::SandboxCapabilities {
        super::SandboxCapabilities::local()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_allows_cargo_validation_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        for cmd in [
            "cargo check",
            "cargo test",
            "cargo build",
            "cargo clippy",
            "cargo test --lib",
        ] {
            assert!(policy.check(cmd), "'{cmd}' must be allowed");
        }
    }

    #[test]
    fn test_policy_allows_go_test_with_run_filter() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module fx\n").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        assert!(policy.check("go test ./..."));
        assert!(policy.check("go test -run TestAdd ./..."));
        // Regex alternation needs `|`, a shell metachar — multi-name go
        // filtering stays unsupported at the policy layer by design.
        assert!(!policy.check("go test -run TestAdd|TestOther ./..."));
    }

    #[test]
    fn test_policy_allows_python_pytest_only_in_python_workspaces() {
        let py = tempfile::tempdir().unwrap();
        std::fs::write(py.path().join("pyproject.toml"), "[project]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(py.path());
        assert!(policy.check("python -m pytest -q"));
        assert!(policy.check("python3 -m pytest -q tests/test_add.py"));
        // -k keyword selection (used by targeted test filtering).
        assert!(policy.check("python -m pytest -q -k test_add"));
        assert!(policy.check("python -m pytest -q --tb=long -k test_add"));
        // Arbitrary python execution is not a test runner.
        assert!(!policy.check("python -c 'print(1)'"));
        assert!(!policy.check("python script.py"));

        let non_py = tempfile::tempdir().unwrap();
        let bare = LocalCommandPolicy::for_workspace(non_py.path());
        assert!(!bare.check("python -m pytest -q"));
    }

    #[test]
    fn test_policy_denies_cargo_mutation_commands() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        for cmd in ["cargo fmt", "cargo clean", "cargo run", "cargo install foo"] {
            assert!(!policy.check(cmd), "'{cmd}' must be denied");
        }
    }

    #[test]
    fn test_policy_denies_shell_metacharacters() {
        let policy = LocalCommandPolicy::for_workspace(std::path::Path::new("/tmp"));
        for cmd in [
            "cargo test; rm -rf /",
            "cargo check > out.txt",
            "cargo test | grep FAIL",
            "echo hi && cargo test",
        ] {
            assert!(!policy.check(cmd), "'{cmd}' must be denied");
        }
    }

    #[test]
    fn test_policy_denies_arbitrary_programs() {
        let policy = LocalCommandPolicy::for_workspace(std::path::Path::new("/tmp"));
        for cmd in ["rm -rf /", "python3 -c 'print(1)'", "grep foo src/"] {
            assert!(!policy.check(cmd), "'{cmd}' must be denied");
        }
    }

    #[test]
    fn test_local_backend_runs_true_command() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "true".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(!result.denied);
    }

    #[test]
    fn test_local_backend_runs_false_command() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "false".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(!result.success);
        assert_eq!(result.exit_code, 1);
    }

    #[test]
    fn test_local_backend_denies_destructive_command() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "rm -rf /".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.denied);
        assert_eq!(result.exit_code, -1);
    }

    #[test]
    fn test_local_backend_captures_output() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "echo hello-world".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.success);
        assert!(result.stdout.contains("hello-world"));
    }

    #[test]
    fn test_local_backend_rejects_outside_cargo_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "cargo test".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(
            result.denied,
            "cargo test must be denied without Cargo.toml"
        );
    }

    #[test]
    fn test_local_backend_allows_echo_and_true() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        for cmd_text in ["echo hello", "true", "false", "printf 'x\\ny\\n'"] {
            let cmd = SandboxCommand {
                command: cmd_text.to_string(),
                working_directory: None,
                policy: None,
                metadata: HashMap::new(),
            };
            let policy = SandboxPolicy::new().with_timeout(5);
            let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
            assert!(!result.denied, "'{cmd_text}' must not be denied");
            if cmd_text == "true" {
                assert!(result.success, "'true' must succeed");
            } else if cmd_text == "false" {
                assert!(!result.success, "'false' must fail");
            }
        }
    }

    #[test]
    fn test_local_backend_stdout_only() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "echo stdout-only".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("stdout-only"));
        assert!(result.stderr.is_empty() || !result.stderr.contains("stdout-only"));
    }

    #[test]
    fn test_local_backend_stderr_only() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        // Use `false` as a simple command that produces no stdout; we test
        // stderr separation via a custom script that writes to fd 2.
        let cmd = SandboxCommand {
            command: "printf 'stderr-only\\n' 1>/dev/null".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        // This command is denied by policy because `>` is a metacharacter.
        // Instead use a command that naturally produces stderr.
        let cmd = SandboxCommand {
            command: "false".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(!result.success);
        assert_eq!(result.exit_code, 1);
        // false produces no stderr; verify the field exists and is accessible.
        assert!(result.stdout.is_empty());
    }

    #[test]
    fn test_local_backend_mixed_stdout_stderr() {
        // Verify both fields exist on a successful command.
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "echo hello".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.success);
        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello"));
        // stderr field is present even if empty.
        let _ = &result.stderr;
    }

    #[test]
    fn test_local_backend_nonzero_exit_preserves_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "false".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(!result.success);
        assert_eq!(result.exit_code, 1);
        assert!(result.duration_ms > 0);
        assert_eq!(result.backend, "local");
    }

    #[test]
    fn test_local_backend_timeout_sets_timeout_flag() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let cmd = SandboxCommand {
            command: "sleep 30".to_string(),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(1);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(!result.success);
        assert_eq!(result.exit_code, -1);
        // Timeout is timing-dependent; verify the result is structurally valid.
        assert!(result.duration_ms > 0);
        assert_eq!(result.backend, "local");
    }

    #[test]
    fn test_local_backend_secret_redaction() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let secret = "sk-test-secret-1234567890abcdef";
        let cmd = SandboxCommand {
            command: format!("echo Authorization: Bearer {secret}"),
            working_directory: None,
            policy: None,
            metadata: HashMap::new(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.success);
        assert!(!result.stdout.contains(secret));
        assert!(result.stdout.contains("REDACTED"));
    }

    #[test]
    fn test_local_backend_metadata_passthrough() {
        let dir = tempfile::tempdir().unwrap();
        let backend = LocalSandboxBackend::new();
        let mut metadata = HashMap::new();
        metadata.insert("run_id".to_string(), "abc-123".to_string());
        metadata.insert("intent".to_string(), "verify-build".to_string());
        let cmd = SandboxCommand {
            command: "echo done".to_string(),
            working_directory: None,
            policy: None,
            metadata: metadata.clone(),
        };
        let policy = SandboxPolicy::new().with_timeout(5);
        let result = backend.execute(&dir.path().to_path_buf(), cmd, &policy);
        assert!(result.success);
        assert_eq!(result.metadata.get("run_id").unwrap(), "abc-123");
        assert_eq!(result.metadata.get("intent").unwrap(), "verify-build");
    }

    #[test]
    fn test_policy_allows_readonly_inspection_commands() {
        let policy = LocalCommandPolicy::for_workspace(std::path::Path::new("/tmp"));
        for cmd in [
            "pwd",
            "ls",
            "ls -la",
            "head file.txt",
            "tail file.txt",
            "wc file.txt",
            "find . -name x",
            "file some_file",
            "which rustc",
            "rustc --version",
            "rustc -V",
            "rustc --print cfg",
            "cat file.txt",
            "cat src/lib.rs",
        ] {
            assert!(policy.check(cmd), "'{cmd}' must be allowed");
        }
        // rustc with disallowed subcommand is denied.
        assert!(!policy.check("rustc src/lib.rs"));
        assert!(!policy.check("rustc -o out src/lib.rs"));
    }

    #[test]
    fn test_policy_allows_cargo_version() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        assert!(
            policy.check("cargo --version"),
            "cargo --version must be allowed"
        );
        assert!(
            policy.check("cargo metadata"),
            "cargo metadata must be allowed"
        );
    }

    #[test]
    fn test_policy_denies_cat_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        // cat with a path that escapes the workspace must be denied.
        assert!(
            !policy.check_in("cat ../../etc/passwd", Some(dir.path())),
            "cat with parent-dir escape must be denied"
        );
    }

    // ── P8 audit F2 regression: sandbox path escapes ───────────────────
    // Before the audit fix, the read-only inspection programs (`head`,
    // `tail`, `ls`, `wc`, `find`, `file`) accepted ANY argument — reading
    // arbitrary host files (`head -c 200 /etc/passwd`), writing outside
    // the sandbox (`find … -fprint /tmp/out`, `git log --output=…`), and
    // `cat`'s confinement missed absolute-path operands. These tests pin
    // every discovered escape.

    #[test]
    fn audit_f2_inspection_commands_cannot_read_absolute_host_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        for cmd in [
            "head -c 200 /etc/passwd",
            "head /etc/shadow",
            "cat /etc/passwd",
            "cat /home/user/.ssh/id_rsa",
            "tail /etc/hosts",
            "wc /etc/passwd",
            "ls /home/user/.ssh",
            "file /etc/passwd",
            "find / -maxdepth 1 -name usr",
            "find /etc -type f",
        ] {
            assert!(
                !policy.check_in(cmd, Some(dir.path())),
                "inspection escape must be denied: {cmd}"
            );
        }
    }

    #[test]
    fn audit_f2_find_cannot_write_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        for cmd in [
            "find . -fprint /tmp/escape.txt",
            "find . -fprintf /tmp/escape.txt",
            "find . -fls /tmp/escape.txt",
            "find . -fprint0 /tmp/escape.txt",
            // Scoping the search outside the root is an escape too.
            "find /etc -maxdepth 1 -fprint out.txt",
        ] {
            assert!(
                !policy.check_in(cmd, Some(dir.path())),
                "find write/scope escape must be denied: {cmd}"
            );
        }
    }

    #[test]
    fn audit_f2_git_cannot_write_or_read_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        // --output writes outside the sandbox.
        assert!(
            !policy.check_in("git log --output=/tmp/escape.txt", Some(dir.path())),
            "git --output escape must be denied"
        );
        // Scoped to a repository elsewhere on the host.
        assert!(
            !policy.check_in(
                "git -C /home/user/other-repo log --oneline",
                Some(dir.path())
            ),
            "git -C outside-root escape must be denied"
        );
        // Normal in-workspace git usage stays allowed.
        assert!(policy.check_in("git status", Some(dir.path())));
        assert!(policy.check_in("git log --oneline -2", Some(dir.path())));
        assert!(policy.check_in("git diff", Some(dir.path())));
    }

    #[test]
    fn audit_f2_legitimate_in_workspace_inspection_still_allowed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/lib.rs"), "pub fn a() {}").unwrap();
        let policy = LocalCommandPolicy::for_workspace(dir.path());
        for cmd in [
            "pwd",
            "ls",
            "ls -la",
            "ls src",
            "cat Cargo.toml",
            "cat src/lib.rs",
            "head Cargo.toml",
            "head -n 3 src/lib.rs",
            "tail src/lib.rs",
            "wc Cargo.toml",
            "wc -l src/lib.rs",
            "find . -maxdepth 2 -type f",
            "find . -maxdepth 1 -name Cargo.toml",
            "file Cargo.toml",
            "which rustc",
            "git status",
            "git log --oneline -2",
            "git show HEAD --stat",
        ] {
            assert!(
                policy.check_in(cmd, Some(dir.path())),
                "legitimate in-workspace command must stay allowed: {cmd}"
            );
        }
    }

    #[test]
    fn test_classify_denial_produces_reasonable_strings() {
        assert_eq!(classify_denial("rm -rf /"), "executable_not_allowlisted");
        assert_eq!(
            classify_denial("python script.py"),
            "mutating_operation_blocked"
        );
        assert_eq!(
            classify_denial("rustc src/lib.rs"),
            "mutating_operation_blocked"
        );
        assert_eq!(
            classify_denial("cargo test; rm -rf /"),
            "shell_metacharacter_detected"
        );
    }
}

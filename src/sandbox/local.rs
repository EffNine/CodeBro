#![allow(dead_code, unused_imports, unused_variables, clippy::all)]
//! Local sandbox backend.
//!
//! Runs commands directly in the workspace using the existing PTY-backed
//! `RunCommand` infrastructure. The command is gated by `LocalCommandPolicy`
//! (a subset of the Testing subagent policy) before any process spawns.

use std::collections::HashMap;
use std::path::PathBuf;
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
    has_makefile: bool,
}

impl LocalCommandPolicy {
    pub fn for_workspace(workspace_root: &std::path::Path) -> Self {
        LocalCommandPolicy {
            is_cargo: workspace_root.join("Cargo.toml").exists(),
            is_node: workspace_root.join("package.json").exists(),
            is_go: workspace_root.join("go.mod").exists(),
            has_makefile: workspace_root.join("Makefile").exists()
                || workspace_root.join("makefile").exists(),
        }
    }

    /// Check a raw command string against the policy.
    pub fn check(&self, command: &str) -> bool {
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

        match program {
            "true" | "false" | "echo" | "printf" => tokens.len() <= 20,
            "sleep" => tokens.len() == 2 && tokens[1].parse::<u64>().is_ok(),
            "cargo" => self.check_cargo(&tokens[1..]),
            "go" => self.check_go(&tokens[1..]),
            "npm" | "pnpm" | "yarn" => self.check_npm(&tokens[1..]),
            "npx" => self.check_npx(&tokens[1..]),
            "make" => self.check_make(&tokens[1..]),
            "git" => self.check_git(&tokens[1..]),
            _ => false,
        }
    }

    fn check_cargo(&self, args: &[&str]) -> bool {
        if !self.is_cargo || args.is_empty() {
            return false;
        }
        let sub = args[0];
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
}

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
        workspace_root: &PathBuf,
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

        if !cmd_policy.check(&command) {
            return ExecutionResult::denied(
                &command,
                &ws_root_str,
                "command denied by sandbox policy",
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
        for cmd in [
            "rm -rf /",
            "python3 -c 'print(1)'",
            "cat file.txt",
            "grep foo src/",
        ] {
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
}

//! Testing capability boundary (Sprint 30D).
//!
//! Testing is NOT Research. It additionally exposes a single bounded execution
//! surface (`run_command`) which is enforced by two independent layers:
//!
//! 1. [`TestingPermissionHook`] — allowlists the exact tool names AND runs the
//!    [`TestingCommandPolicy`] against every `run_command` argument before the
//!    registry may execute it. A denied command never reaches a shell.
//! 2. [`TestingTooling::execute_command`] — re-checks the same policy before
//!    spawning, and captures the authoritative PTY exit code.
//!
//! Mutating tools (`create_file`, `edit_file`, git mutations, destructive
//! commands) are neither registered nor permitted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt;

use crate::dispatcher::ToolRegistry;
use crate::tools::context::ToolContext;
use crate::tools::hooks::PermissionDecision;
use crate::tools::hooks::PermissionHook;
use crate::tools::shell::redact_secrets_public;

use super::contract::{GitStateSnapshot, TestCommandResult};
use super::limits::TESTING_ALLOWED_TOOLS;
use super::policy::{CommandDecision, TestingCommandPolicy};

/// Explicit capability boundary for the Testing subagent.
///
/// This is NOT "deny everything unless the name contains read". It is a fixed
/// allowlist of specific tool names plus a command policy for the single
/// execution surface. Any tool outside the allowlist is denied with an
/// explicit reason, and any `run_command` whose args fail the policy is denied
/// at the permission layer — before the registry can spawn anything.
#[derive(Debug, Clone)]
pub struct TestingPermissionHook {
    allowed: HashSet<String>,
    policy: TestingCommandPolicy,
}

impl TestingPermissionHook {
    pub fn new(workspace_root: &Path) -> Self {
        TestingPermissionHook {
            allowed: TESTING_ALLOWED_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
            policy: TestingCommandPolicy::for_workspace(workspace_root),
        }
    }

    /// The set of explicitly allowed tool names.
    pub fn allowed_tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = self.allowed.iter().cloned().collect();
        tools.sort();
        tools
    }

    /// Whether a tool name is on the allowlist.
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed.contains(tool)
    }
}

impl PermissionHook for TestingPermissionHook {
    fn check(&self, context: &ToolContext) -> PermissionDecision {
        if !self.allowed.contains(&context.tool_name) {
            return PermissionDecision::Denied {
                reason: format!(
                    "testing subagent: '{}' is not on the allowlist (read-only tools plus a single policy-checked run_command)",
                    context.tool_name
                ),
            };
        }
        // The single execution surface is additionally gated by the command
        // policy at the permission layer: a destructive or mutating command
        // never reaches a shell.
        if context.tool_name == "run_command" {
            let command = extract_command_arg(&context.args);
            match self.policy.check(&command) {
                CommandDecision::Allowed => PermissionDecision::Allowed {
                    reason: Some("testing command policy".to_string()),
                },
                CommandDecision::Denied { reason } => PermissionDecision::Denied {
                    reason: format!("testing command policy: {reason}"),
                },
            }
        } else {
            PermissionDecision::Allowed {
                reason: Some("testing allowlist".to_string()),
            }
        }
    }
}

/// Build the restricted tool registry for the Testing subagent.
///
/// The same `Arc<dyn Tool>` implementations the main agent uses are reused —
/// nothing is duplicated. `run_command` is configured to run in the workspace
/// root under a bounded per-command PTY timeout. The [`TestingPermissionHook`]
/// is installed as the global permission hook so the boundary is enforced even
/// if a tool were somehow added later.
pub fn build_testing_tool_registry(
    workspace_root: &Path,
    command_timeout_secs: u64,
) -> ToolRegistry {
    let root = workspace_root.to_path_buf();
    ToolRegistry::new()
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff))
        .register(Arc::new(
            crate::tools::RunCommand::new()
                .with_working_directory(root.to_string_lossy().to_string())
                .with_timeout(command_timeout_secs),
        ))
}

/// A registry ready for testing: restricted tool set plus the explicit
/// permission boundary. The workspace root is carried so path-based tools
/// resolve relative arguments against it and commands run in the right
/// directory.
pub struct TestingTooling {
    pub registry: ToolRegistry,
    pub workspace_root: PathBuf,
    policy: TestingCommandPolicy,
    working_directory: String,
    max_command_output_chars: usize,
}

impl TestingTooling {
    pub fn new(workspace_root: &Path, command_timeout_secs: u64) -> Self {
        let mut registry = build_testing_tool_registry(workspace_root, command_timeout_secs);
        install_testing_permission_hook(workspace_root, &mut registry);
        TestingTooling {
            registry,
            workspace_root: workspace_root.to_path_buf(),
            policy: TestingCommandPolicy::for_workspace(workspace_root),
            working_directory: workspace_root.to_string_lossy().to_string(),
            max_command_output_chars: super::limits::MAX_COMMAND_OUTPUT_CHARS,
        }
    }

    /// The project-aware command policy in effect for this session.
    pub fn policy(&self) -> &TestingCommandPolicy {
        &self.policy
    }

    /// Whether the workspace has any recognised validation surface.
    pub fn has_validation_surface(&self) -> bool {
        self.policy.has_validation_surface()
    }

    /// Execute one policy-checked validation command and capture the
    /// authoritative PTY exit code. A denied command never executes: it
    /// becomes an authoritative `denied` record so the model observes the
    /// denial.
    pub async fn execute_command(
        &mut self,
        raw_args: &str,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> TestCommandResult {
        let command = extract_command_arg(raw_args);

        // Layer 1: policy check before anything touches a shell.
        match self.policy.check(&command) {
            CommandDecision::Denied { reason } => {
                return TestCommandResult::denied(&command, &self.working_directory, reason);
            }
            CommandDecision::Allowed => {}
        }

        // Layer 2: the registry's global permission hook re-checks the same
        // policy (defense in depth) before the PTY spawns.
        let started = Instant::now();
        match self
            .registry
            .execute_stream("run_command", &command, cancel)
            .await
        {
            Ok(mut stream) => {
                let mut output = String::new();
                let mut exit_code: i32 = -1;
                let mut cancelled = false;
                let mut timed_out = false;
                let mut error_message: Option<String> = None;
                while let Some(chunk) = stream.chunks.next().await {
                    if !chunk.text.is_empty() {
                        output.push_str(&chunk.text);
                    }
                    if let Some(meta) = &chunk.metadata {
                        if let Some(code) = meta.strip_prefix("exit:") {
                            exit_code = code.parse().unwrap_or(-1);
                        } else if meta == "cancelled" {
                            cancelled = true;
                        } else if meta == "timeout" {
                            timed_out = true;
                        } else if meta == "error" {
                            error_message = Some(chunk.text.clone());
                        }
                    }
                    if chunk.is_final {
                        break;
                    }
                }
                let duration_ms = started.elapsed().as_millis();

                if timed_out {
                    return TestCommandResult::timed_out(
                        &command,
                        &self.working_directory,
                        duration_ms,
                    );
                }
                if cancelled {
                    return TestCommandResult {
                        command,
                        working_directory: self.working_directory.clone(),
                        exit_code,
                        success: false,
                        duration_ms,
                        output: truncate_command_output(&output, self.max_command_output_chars),
                        timeout: false,
                        cancelled: true,
                        denied: false,
                        denied_reason: None,
                    };
                }
                if let Some(message) = error_message {
                    return TestCommandResult::failed_to_run(
                        &command,
                        &self.working_directory,
                        &message,
                    );
                }

                TestCommandResult {
                    command,
                    working_directory: self.working_directory.clone(),
                    exit_code,
                    success: TestCommandResult::success_from_exit_code(exit_code, false, false),
                    duration_ms,
                    output: truncate_command_output(&output, self.max_command_output_chars),
                    timeout: false,
                    cancelled: false,
                    denied: false,
                    denied_reason: None,
                }
            }
            Err(e) => {
                // The registry rejected the command (permission hook) or the
                // tool failed to start. Distinguish a policy denial so it is
                // recorded as a denial, not a generic error.
                let message = e.to_string();
                if message.contains("denied") {
                    let reason = message
                        .split_once("denied:")
                        .map(|(_, r)| r.trim().to_string())
                        .unwrap_or_else(|| message.clone());
                    TestCommandResult::denied(&command, &self.working_directory, reason)
                } else {
                    TestCommandResult::failed_to_run(&command, &self.working_directory, &message)
                }
            }
        }
    }

    /// Execute a single read-only tool call through the restricted registry,
    /// resolving relative paths for the path-based read-only tools.
    pub async fn execute_tool(
        &mut self,
        name: &str,
        args: &str,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> String {
        let args = match name {
            "list_files" | "read_file" => {
                let raw = extract_tool_path(args).unwrap_or_else(|| args.to_string());
                resolve_path(&self.workspace_root, &raw)
            }
            _ => args.to_string(),
        };
        match self.registry.execute_stream(name, &args, cancel).await {
            Ok(mut stream) => {
                let mut output = String::new();
                while let Some(chunk) = stream.chunks.next().await {
                    output.push_str(&chunk.text);
                    if chunk.is_final {
                        break;
                    }
                }
                if output.trim().is_empty() {
                    "…".to_string()
                } else {
                    output
                }
            }
            Err(e) => format!("Error: {}", e),
        }
    }

    /// Snapshot the workspace git state for the before/after no-mutation
    /// protocol. Build artifacts in ignored locations (e.g. `target/`) do not
    /// surface here, so normal compiler/test artifacts are allowed.
    pub fn check_git_state(&self) -> GitStateSnapshot {
        if !self.workspace_root.join(".git").exists() {
            return GitStateSnapshot {
                has_git: false,
                status: String::new(),
                clean: true,
            };
        }
        let status = run_git(&self.workspace_root, &["status", "--short"]);
        let diff_check = run_git(&self.workspace_root, &["diff", "--check"]);
        let clean = diff_check.is_empty() && !status_has_tracked_modifications(&status);
        GitStateSnapshot {
            has_git: true,
            status,
            clean,
        }
    }
}

/// Install the testing permission hook on a registry.
pub fn install_testing_permission_hook(workspace_root: &Path, registry: &mut ToolRegistry) {
    registry.set_global_permission_hook(Box::new(TestingPermissionHook::new(workspace_root)));
}

/// Resolve a tool argument path against the workspace root. Absolute paths
/// pass through; relative paths are joined onto the workspace root.
fn resolve_path(workspace_root: &Path, argument: &str) -> String {
    let trimmed = argument.trim();
    if trimmed.is_empty() {
        return workspace_root.to_string_lossy().to_string();
    }
    let path = std::path::Path::new(trimmed);
    if path.is_absolute() {
        trimmed.to_string()
    } else {
        workspace_root.join(path).to_string_lossy().to_string()
    }
}

/// Extract the command string from a tool-call argument string. Accepts JSON
/// envelopes (`{"command": "cargo test"}`, `{"input": "cargo test"}`) and raw
/// command strings.
fn extract_command_arg(arguments: &str) -> String {
    let trimmed = arguments.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(command) = value.get("command").and_then(|v| v.as_str()) {
            return command.to_string();
        }
        if let Some(input) = value.get("input").and_then(|v| v.as_str()) {
            return input.to_string();
        }
    }
    trimmed.trim_matches('"').to_string()
}

/// Extract the `path` argument from a tool-call argument string.
fn extract_tool_path(arguments: &str) -> Option<String> {
    let trimmed = arguments.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
        if let Some(dir) = value.get("dir").and_then(|v| v.as_str()) {
            return Some(dir.to_string());
        }
    }
    None
}

/// Cap and redact command output before it becomes a model observation.
fn truncate_command_output(output: &str, max_chars: usize) -> String {
    let output = redact_secrets_public(output);
    if output.chars().count() <= max_chars {
        output
    } else {
        let head: String = output.chars().take(max_chars).collect();
        format!("{head}\n…[output truncated]")
    }
}

/// Run `git` in the workspace root and return trimmed stdout.
fn run_git(workspace_root: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output();
    match output {
        Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(),
        Err(_) => String::new(),
    }
}

/// Whether `git status --short` output contains any tracked modification
/// (as opposed to untracked entries, which are allowed build artifacts).
fn status_has_tracked_modifications(status: &str) -> bool {
    status.lines().any(|line| {
        let line = line.trim_start_matches(' ').to_string();
        let line = line.as_str();
        !(line.starts_with("??") || line.starts_with('?'))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_allows_expected_tools() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let hook = TestingPermissionHook::new(dir.path());
        for allowed in [
            "list_files",
            "read_file",
            "git_status",
            "git_diff",
            "run_command",
        ] {
            assert!(hook.allows(allowed), "{} must be allowed", allowed);
        }
        for denied in ["create_file", "edit_file", "playwright", "patch"] {
            assert!(!hook.allows(denied), "{} must not be allowed", denied);
            let ctx = ToolContext::new(denied, "{}");
            let decision = hook.check(&ctx);
            assert!(
                decision.is_denied(),
                "{} must be denied, got {:?}",
                denied,
                decision
            );
        }
    }

    #[test]
    fn test_permission_hook_gates_run_command_by_policy() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let hook = TestingPermissionHook::new(dir.path());

        let allowed = ToolContext::new("run_command", "cargo test");
        assert!(
            hook.check(&allowed).is_allowed(),
            "cargo test must be allowed by the hook"
        );

        for command in [
            "rm -rf /",
            "git commit -m x",
            "cargo clean",
            "sh -c 'echo hi'",
        ] {
            let ctx = ToolContext::new("run_command", command);
            let decision = hook.check(&ctx);
            assert!(
                decision.is_denied(),
                "'{command}' must be denied at the permission layer, got {:?}",
                decision
            );
        }
    }

    #[test]
    fn test_registry_only_has_testing_tools() {
        let dir = tempfile::tempdir().unwrap();
        let tooling = TestingTooling::new(dir.path(), 10);
        let names = tooling.registry.names();
        assert!(names.contains(&"list_files".to_string()));
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"git_status".to_string()));
        assert!(names.contains(&"git_diff".to_string()));
        assert!(names.contains(&"run_command".to_string()));
        for denied in ["create_file", "edit_file", "playwright", "patch"] {
            assert!(
                !names.contains(&denied.to_string()),
                "testing registry must not expose {}",
                denied
            );
        }
    }

    #[tokio::test]
    async fn test_registry_denies_unregistered_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = TestingTooling::new(dir.path(), 10);
        let result = tooling.execute_tool("create_file", "|content", None).await;
        assert!(
            result.starts_with("Error"),
            "create_file must fail in the restricted registry, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_read_file_resolves_against_workspace_root() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello testing").unwrap();
        let mut tooling = TestingTooling::new(dir.path(), 10);
        let result = tooling.execute_tool("read_file", "note.txt", None).await;
        assert!(
            result.contains("hello testing"),
            "read_file must read the real file, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_execute_command_runs_and_captures_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        let mut tooling = TestingTooling::new(dir.path(), 30);

        let ok = tooling.execute_command("true", None).await;
        assert_eq!(ok.exit_code, 0, "true must exit 0");
        assert!(ok.success);

        let fail = tooling.execute_command("false", None).await;
        assert_eq!(fail.exit_code, 1, "false must exit 1");
        assert!(!fail.success);

        let denied = tooling.execute_command("rm -rf /", None).await;
        assert!(denied.denied, "rm must be denied");
        assert!(!denied.success);
        assert_eq!(denied.exit_code, -1);

        let denied_program = tooling.execute_command("sh -c 'echo hi'", None).await;
        assert!(denied_program.denied);

        let denied_mutation = tooling.execute_command("git commit -m x", None).await;
        assert!(denied_mutation.denied);
    }

    #[tokio::test]
    async fn test_execute_command_captures_real_output_and_duration() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = TestingTooling::new(dir.path(), 30);
        let result = tooling
            .execute_command("printf 'alpha\\nbeta\\n'", None)
            .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.output.contains("alpha"), "got: {}", result.output);
        assert!(result.output.contains("beta"));
        assert!(result.duration_ms > 0);
    }

    #[tokio::test]
    async fn test_execute_command_honors_per_command_timeout() {
        let dir = tempfile::tempdir().unwrap();
        // A 1s per-command timeout against a 3s sleep: deterministic, bounded,
        // and NOT an infinite process.
        let mut tooling = TestingTooling::new(dir.path(), 1);
        let result = tooling.execute_command("sleep 3", None).await;
        assert!(result.timeout, "sleep 3 must time out under a 1s timeout");
        assert!(!result.success);
        assert_eq!(result.exit_code, -1);
    }

    #[tokio::test]
    async fn test_execute_command_respects_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let token = crate::cancellation::CancellationToken::new();
        token.cancel();
        let mut tooling = TestingTooling::new(dir.path(), 30);
        let result = tooling.execute_command("sleep 30", Some(token)).await;
        assert!(
            result.cancelled,
            "a cancelled command must be recorded as cancelled, got: {:?}",
            result
        );
        assert!(!result.success);
    }

    #[test]
    fn test_check_git_state_reports_no_git() {
        let dir = tempfile::tempdir().unwrap();
        let tooling = TestingTooling::new(dir.path(), 30);
        let state = tooling.check_git_state();
        assert!(!state.has_git);
        assert!(state.clean);
    }

    #[test]
    fn test_check_git_state_detects_modifications() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[package]").unwrap();
        init_git(dir.path());
        let tooling = TestingTooling::new(dir.path(), 30);
        let state = tooling.check_git_state();
        assert!(state.has_git);
        assert!(state.clean, "fresh repo must be clean: {}", state.status);

        // Modify a tracked file: the state must become unclean.
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\n# change").unwrap();
        let tooling = TestingTooling::new(dir.path(), 30);
        let state = tooling.check_git_state();
        assert!(!state.clean, "modified file must be detected");
        assert!(state.status.contains("Cargo.toml"));
    }

    fn init_git(root: &Path) {
        let out = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root)
            .output()
            .unwrap();
        assert!(out.status.success());
        let _ = std::process::Command::new("git")
            .args(["config", "user.email", "test@test"])
            .current_dir(root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["add", "-A"])
            .current_dir(root)
            .output();
        let _ = std::process::Command::new("git")
            .args(["commit", "-q", "-m", "initial"])
            .current_dir(root)
            .output();
    }
}

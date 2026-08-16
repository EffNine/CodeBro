//! Sandbox execution MCP tools.
//!
//! Exposes sandbox command execution to external AI agents through the MCP
//! protocol. Commands are policy-checked before execution and results are
//! returned as structured JSON evidence.
//!
//! Tools:
//! - `sandbox_exec` — generic command execution
//! - `sandbox_test` — run the project's tests with structured verification
//! - `sandbox_build` — build/check the project with structured verification
//! - `sandbox_status` — check backend availability

use std::collections::HashMap;
use std::path::PathBuf;

use rmcp::{
    handler::server::wrapper::Parameters,
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::sandbox::{
    ExecutionResult, SandboxCommand, SandboxMode, SandboxPolicy, SandboxRuntime, VerificationResult,
};

/// The CodeBro sandbox MCP server extension.
///
/// This is wired into the main `CodeBroMcpServer` as additional tools.
/// The sandbox runtime is created once per server process.
#[derive(Clone)]
pub struct SandboxServer {
    runtime: SandboxRuntime,
}

impl SandboxServer {
    pub fn new(runtime: SandboxRuntime) -> Self {
        SandboxServer { runtime }
    }

    /// Return the runtime for observability.
    pub fn runtime(&self) -> &SandboxRuntime {
        &self.runtime
    }
}

/// Argument schema for `sandbox_exec`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SandboxExecArgs {
    /// The shell command to execute (e.g. `"cargo test --lib"`).
    pub command: String,
    /// Working directory relative to the workspace root (optional; defaults
    /// to the workspace root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Execution timeout in seconds (optional; defaults to 120).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<usize>,
    /// Arbitrary metadata to echo back in the result (optional).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Argument schema for `sandbox_test`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SandboxTestArgs {
    /// Optional override command (default: project-aware test command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Working directory relative to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Execution timeout in seconds (optional; defaults to 120).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<usize>,
    /// Expected exit code (optional; default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_exit_code: Option<i32>,
    /// Expected success flag (optional; default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_success: Option<bool>,
    /// Arbitrary metadata to echo back in the result (optional).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Argument schema for `sandbox_build`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SandboxBuildArgs {
    /// Optional override command (default: project-aware build/check command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    /// Working directory relative to the workspace root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<String>,
    /// Execution timeout in seconds (optional; defaults to 120).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<usize>,
    /// Expected exit code (optional; default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_exit_code: Option<i32>,
    /// Expected success flag (optional; default true).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_success: Option<bool>,
    /// Arbitrary metadata to echo back in the result (optional).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub metadata: HashMap<String, String>,
}

/// Argument schema for `sandbox_status`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct SandboxStatusArgs {}

#[tool_router]
impl SandboxServer {
    /// Execute a command in an isolated sandbox environment.
    ///
    /// The command is policy-checked (read-only build/test/lint commands only)
    /// before execution. Returns structured evidence with provenance: exit code,
    /// stdout, stderr, duration, timeout, denial status, repo state binding,
    /// and capability descriptor.
    #[tool(
        description = "Execute a command in an isolated sandbox. Returns structured evidence: exit_code, stdout, stderr, duration_ms, success, timeout, denied. Only read-only build/test/lint commands are permitted."
    )]
    async fn sandbox_exec(
        &self,
        Parameters(args): Parameters<SandboxExecArgs>,
    ) -> Result<CallToolResult, McpError> {
        let workspace_root = PathBuf::from(".");
        let cmd = SandboxCommand {
            command: args.command,
            working_directory: args.working_directory,
            policy: None,
            metadata: args.metadata,
        };
        let policy = SandboxPolicy::new().with_timeout(args.timeout.unwrap_or(120) as u64);
        let result = self.runtime.execute(&workspace_root, cmd, &policy);
        let payload = serde_json::to_string_pretty(&result)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        Ok(CallToolResult::success(vec![ContentBlock::text(payload)]))
    }

    /// Run the project's tests and return structured verification evidence.
    ///
    /// Auto-detects the project type (Cargo.toml → `cargo test`, package.json
    /// → `npm test`, go.mod → `go test`) and runs the appropriate test
    /// command. Returns an `ExecutionResult` plus a `VerificationResult`
    /// that records whether the test suite passed.
    #[tool(
        description = "Run the project's tests and return structured verification evidence: execution result plus pass/fail verification with exit code, stdout, stderr, duration, and expectation violations."
    )]
    async fn sandbox_test(
        &self,
        Parameters(args): Parameters<SandboxTestArgs>,
    ) -> Result<CallToolResult, McpError> {
        let workspace_root = detect_workspace_root(&args.working_directory);
        let command = resolve_test_command(&workspace_root, args.command.as_deref());
        let cmd = SandboxCommand {
            command: command.clone(),
            working_directory: args.working_directory,
            policy: None,
            metadata: args.metadata,
        };
        let policy = SandboxPolicy::new().with_timeout(args.timeout.unwrap_or(120) as u64);
        let execution = self.runtime.execute(&workspace_root, cmd, &policy);
        let verification = VerificationResult::from_execution_with_expectations(
            execution,
            args.expected_exit_code,
            args.expected_success,
        );
        let payload = json!({
            "execution": verification.execution,
            "verification": {
                "verified": verification.verified,
                "summary": verification.summary,
                "violations": verification.violations,
            },
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    /// Build or check the project and return structured verification evidence.
    ///
    /// Auto-detects the project type (Cargo.toml → `cargo check`, package.json
    /// → `npm run build` or `npm run check`, go.mod → `go build`) and runs
    /// the appropriate build command. Returns an `ExecutionResult` plus a
    /// `VerificationResult`.
    #[tool(
        description = "Build or check the project and return structured verification evidence: execution result plus pass/fail verification with exit code, stdout, stderr, duration, and expectation violations."
    )]
    async fn sandbox_build(
        &self,
        Parameters(args): Parameters<SandboxBuildArgs>,
    ) -> Result<CallToolResult, McpError> {
        let workspace_root = detect_workspace_root(&args.working_directory);
        let command = resolve_build_command(&workspace_root, args.command.as_deref());
        let cmd = SandboxCommand {
            command: command.clone(),
            working_directory: args.working_directory,
            policy: None,
            metadata: args.metadata,
        };
        let policy = SandboxPolicy::new().with_timeout(args.timeout.unwrap_or(120) as u64);
        let execution = self.runtime.execute(&workspace_root, cmd, &policy);
        let verification = VerificationResult::from_execution_with_expectations(
            execution,
            args.expected_exit_code,
            args.expected_success,
        );
        let payload = json!({
            "execution": verification.execution,
            "verification": {
                "verified": verification.verified,
                "summary": verification.summary,
                "violations": verification.violations,
            },
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    /// Return sandbox runtime status: backend name, mode, availability, and
    /// explicit capability descriptor so the agent can inspect guarantees
    /// before executing.
    #[tool(
        description = "Return sandbox runtime status: backend (local/opensandbox), mode, availability, and formal capability descriptor. Call this before sandbox_exec to understand execution guarantees."
    )]
    async fn sandbox_status(
        &self,
        Parameters(_args): Parameters<SandboxStatusArgs>,
    ) -> Result<CallToolResult, McpError> {
        let status = self.runtime.status();
        let payload = json!({
            "backend": status.backend,
            "mode": status.mode,
            "available": status.available,
            "capabilities": status.capabilities,
            "opensandbox_configured": status.opensandbox_configured,
        });
        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }
}

/// Detect the workspace root from an optional working directory.
fn detect_workspace_root(working_dir: &Option<String>) -> PathBuf {
    working_dir
        .as_deref()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Resolve a test command for the given workspace.
///
/// Project-aware defaults:
/// - `Cargo.toml` → `cargo test`
/// - `go.mod` → `go test ./...`
/// - `package.json` → `npm test`
/// - Explicit command overrides all defaults.
fn resolve_test_command(workspace: &PathBuf, explicit: Option<&str>) -> String {
    if let Some(cmd) = explicit {
        return cmd.to_string();
    }
    if workspace.join("Cargo.toml").exists() {
        return "cargo test".to_string();
    }
    if workspace.join("go.mod").exists() {
        return "go test ./...".to_string();
    }
    if workspace.join("package.json").exists() {
        return "npm test".to_string();
    }
    "echo 'no project manifest detected; use sandbox_exec with an explicit command'".to_string()
}

/// Resolve a build/check command for the given workspace.
///
/// Project-aware defaults:
/// - `Cargo.toml` → `cargo check`
/// - `go.mod` → `go build ./...`
/// - `package.json` → `npm run build` (falls back to `npm run check`)
/// - Explicit command overrides all defaults.
fn resolve_build_command(workspace: &PathBuf, explicit: Option<&str>) -> String {
    if let Some(cmd) = explicit {
        return cmd.to_string();
    }
    if workspace.join("Cargo.toml").exists() {
        return "cargo check".to_string();
    }
    if workspace.join("go.mod").exists() {
        return "go build ./...".to_string();
    }
    if workspace.join("package.json").exists() {
        return "npm run build".to_string();
    }
    "echo 'no project manifest detected; use sandbox_exec with an explicit command'".to_string()
}

#[tool_handler]
impl rmcp::ServerHandler for SandboxServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(
                "Sandbox execution tools are available when the agent needs to run \
                 build, test, lint, or verification commands in an isolated context. \
                 Use sandbox_exec for arbitrary commands (structured evidence returned), \
                 sandbox_test for project test suites, sandbox_build for project builds, \
                 and sandbox_status to check backend availability."
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::ServerHandler;

    #[test]
    fn test_sandbox_tools_registered() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        for tool_name in [
            "sandbox_exec",
            "sandbox_test",
            "sandbox_build",
            "sandbox_status",
        ] {
            assert!(
                server.get_tool(tool_name).is_some(),
                "tool {tool_name} missing from sandbox handler"
            );
        }
    }

    #[test]
    fn test_sandbox_exec_runs_true() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxExecArgs {
            command: "true".to_string(),
            working_directory: None,
            timeout: Some(5),
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_exec(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["success"], true);
        assert_eq!(v["exit_code"], 0);
        assert_eq!(v["backend"], "local");
    }

    #[test]
    fn test_sandbox_exec_denies_destructive_command() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxExecArgs {
            command: "rm -rf /".to_string(),
            working_directory: None,
            timeout: Some(5),
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_exec(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["denied"], true);
        assert_eq!(v["exit_code"], -1);
    }

    #[test]
    fn test_sandbox_exec_captures_output() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxExecArgs {
            command: "echo test-output-123".to_string(),
            working_directory: None,
            timeout: Some(5),
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_exec(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["success"], true);
        assert!(v["stdout"].as_str().unwrap().contains("test-output-123"));
    }

    #[test]
    fn test_sandbox_status_returns_available() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_status(Parameters(SandboxStatusArgs {})).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["available"], true);
        assert_eq!(v["backend"], "local");
    }

    #[test]
    fn test_sandbox_test_resolves_cargo_default() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        // No explicit command: should resolve to "cargo test" on a cargo workspace.
        let args = SandboxTestArgs {
            command: None,
            working_directory: None,
            timeout: Some(5),
            expected_exit_code: None,
            expected_success: None,
            metadata: HashMap::new(),
        };
        // Run in the current workspace (has Cargo.toml).
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_test(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        // The execution command should be "cargo test" since this repo has Cargo.toml.
        assert_eq!(v["execution"]["command"], "cargo test");
        // Structure must include both execution and verification.
        assert!(v.get("execution").is_some());
        assert!(v.get("verification").is_some());
        assert!(v["verification"]["verified"].is_boolean());
    }

    #[test]
    fn test_sandbox_build_resolves_cargo_default() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxBuildArgs {
            command: None,
            working_directory: None,
            timeout: Some(5),
            expected_exit_code: None,
            expected_success: None,
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_build(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["execution"]["command"], "cargo check");
        assert!(v.get("execution").is_some());
        assert!(v.get("verification").is_some());
    }

    #[test]
    fn test_sandbox_test_with_explicit_command() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxTestArgs {
            command: Some("echo test-explicit".to_string()),
            working_directory: None,
            timeout: Some(5),
            expected_exit_code: Some(0),
            expected_success: Some(true),
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_test(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["execution"]["command"], "echo test-explicit");
        assert_eq!(v["verification"]["verified"], true);
    }

    #[test]
    fn test_sandbox_test_failure_verification() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxTestArgs {
            command: Some("false".to_string()),
            working_directory: None,
            timeout: Some(5),
            expected_exit_code: Some(0),
            expected_success: Some(true),
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_test(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["execution"]["exit_code"], 1);
        assert_eq!(v["verification"]["verified"], false);
        assert!(!v["verification"]["violations"].is_null());
        assert!(v["verification"]["violations"].as_array().unwrap().len() > 0);
    }

    #[test]
    fn test_sandbox_build_resolves_go_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example\n").unwrap();
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxBuildArgs {
            command: None,
            working_directory: Some(dir.path().to_string_lossy().to_string()),
            timeout: Some(5),
            expected_exit_code: None,
            expected_success: None,
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_build(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["execution"]["command"], "go build ./...");
    }

    #[test]
    fn test_sandbox_test_resolves_go_default() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("go.mod"), "module example\n").unwrap();
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxTestArgs {
            command: None,
            working_directory: Some(dir.path().to_string_lossy().to_string()),
            timeout: Some(5),
            expected_exit_code: None,
            expected_success: None,
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_test(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["execution"]["command"], "go test ./...");
    }

    fn text_of(result: CallToolResult) -> String {
        result
            .content
            .into_iter()
            .filter_map(|b| match b {
                ContentBlock::Text(t) => Some(t.text),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test_sandbox_status_exposes_capabilities() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_status(Parameters(SandboxStatusArgs {})).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert_eq!(v["backend"], "local");
        assert_eq!(v["available"], true);
        assert!(v.get("capabilities").is_some());
        let caps = v["capabilities"].as_object().unwrap();
        assert_eq!(caps["isolation"], "none");
        assert_eq!(caps["filesystem_scope"], "policy_bounded");
        assert_eq!(caps["network_access"], "host");
    }

    #[test]
    fn test_sandbox_exec_includes_provenance_fields() {
        let rt = SandboxRuntime::new(SandboxMode::Local);
        let server = SandboxServer::new(rt);
        let args = SandboxExecArgs {
            command: "echo provenance-test".to_string(),
            working_directory: None,
            timeout: Some(5),
            metadata: HashMap::new(),
        };
        let result = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(async {
                server.sandbox_exec(Parameters(args)).await
            });
        assert!(result.is_ok());
        let text = text_of(result.unwrap());
        let v: serde_json::Value = serde_json::from_str(&text).expect("valid json");
        assert!(v["execution_id"].is_string());
        assert!(!v["execution_id"].as_str().unwrap().is_empty());
        assert!(v["timestamp"].is_string());
        assert!(v["resolved_command"].is_string());
        assert!(v["reproducibility"].is_string());
    }
}

//! LEGACY — capability boundary and restricted registry of the Sprint 30F
//! autonomous coding subagent.
//!
//! This module belongs to legacy architecture and is not part of the CodeBro
//! MCP Runtime. It is preserved only so the legacy regression suite
//! (`crate::legacy::coding_tests`) keeps passing. The production mutation
//! seam ([`crate::coding::change_engine::ChangeEngine`) was carved out of the
//! same file and lives in the live `coding` module.

#![allow(dead_code, unused_imports, unused_variables, clippy::all)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use crate::legacy::dispatcher::ToolRegistry;
use crate::tools::context::ToolContext;
use crate::legacy::tools::hooks::PermissionDecision;
use crate::legacy::tools::hooks::PermissionHook;
use crate::tools::shell::redact_secrets_public;

use super::coding_contract::VerificationRecord;
use super::coding_contract::VerificationSource;
use super::coding_limits::CODING_ALLOWED_TOOLS;

/// Explicit capability boundary for the Coding subagent.
///
/// This is a fixed allowlist of the six tool names. Any tool outside the
/// allowlist is denied with an explicit reason, even if it is registered
/// (defense in depth — the restricted registry is itself limited to the
/// allowlist).
#[derive(Debug, Clone)]
pub struct CodingPermissionHook {
    allowed: HashSet<String>,
}

impl CodingPermissionHook {
    pub fn new() -> Self {
        CodingPermissionHook {
            allowed: CODING_ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect(),
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

impl Default for CodingPermissionHook {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionHook for CodingPermissionHook {
    fn check(&self, context: &ToolContext) -> PermissionDecision {
        if self.allowed.contains(&context.tool_name) {
            PermissionDecision::Allowed {
                reason: Some("coding allowlist".to_string()),
            }
        } else {
            PermissionDecision::Denied {
                reason: format!(
                    "coding subagent: '{}' is not on the allowlist (read-only tools plus propose_change and verify — raw file writes and run_command are never allowed)",
                    context.tool_name
                ),
            }
        }
    }
}

/// A placeholder for the two runtime-intercepted surfaces.
///
/// `propose_change` and `verify` are registered ONLY so their tool definitions
/// are advertised to structured-calling providers. The CodingSubagent loop
/// intercepts them by name and routes them through the [`ChangeEngine`] and
/// the Testing command policy; if one were ever invoked through the registry
/// directly, it fails loudly instead of doing anything.
#[derive(Debug, Clone, Copy)]
pub struct RuntimeInterceptedTool {
    name: &'static str,
    description: &'static str,
}

impl RuntimeInterceptedTool {
    fn new(name: &'static str) -> Self {
        let description = match name {
            "propose_change" => {
                "propose_change — propose AND apply one targeted change to one file. Args (JSON): {\"path\": \"relative/file.rs\", \"old\": \"exact text currently in the file (must match uniquely and must NOT be empty for existing files)\", \"new\": \"replacement text\"}. To CREATE a new file pass old=\"\" and the full content as new. Returns the diff preview after applying."
            }
            "verify" => {
                "verify — run ONE validation command permitted by the Testing command policy and observe the authoritative exit code. Args (JSON): {\"command\": \"cargo test\"}. exit 0 is success; any non-zero exit code is failure regardless of the output text."
            }
            _ => "runtime-intercepted coding surface",
        };
        RuntimeInterceptedTool { name, description }
    }
}

impl crate::tools::Tool for RuntimeInterceptedTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        self.description
    }

    fn execute(&self, args: &str) -> anyhow::Result<String> {
        Err(anyhow::anyhow!(
            "{} is runtime-intercepted by the coding subagent (args: {})",
            self.name,
            args
        ))
    }
}

/// Build the restricted tool registry for the Coding subagent.
///
/// Only the six allowlisted tools are registered. The same `Arc<dyn Tool>`
/// implementations used by the main agent are reused for inspection; the two
/// mutating/verifying surfaces are registered as intercepted placeholders so
/// their definitions reach the provider. The [`CodingPermissionHook`] is
/// installed as the global permission hook for defense in depth.
pub fn build_coding_tool_registry(workspace_root: &Path) -> ToolRegistry {
    let _ = workspace_root;
    ToolRegistry::new()
        .register(Arc::new(crate::legacy::tools::filesystem::ListFiles))
        .register(Arc::new(crate::legacy::tools::filesystem::ReadFile))
        .register(Arc::new(crate::legacy::tools::git::GitStatus))
        .register(Arc::new(crate::legacy::tools::git::GitDiff))
        .register(Arc::new(RuntimeInterceptedTool::new("propose_change")))
        .register(Arc::new(RuntimeInterceptedTool::new("verify")))
}

/// Register the coding permission hook on a registry.
pub fn install_coding_permission_hook(registry: &mut ToolRegistry) {
    registry.set_global_permission_hook(Box::new(CodingPermissionHook::new()));
}

/// A registry ready for coding: restricted tool set plus the explicit
/// permission boundary. Carries the mutation engine (workspace-bound, plan
/// aware) and the policy-checked verification tooling.
pub struct CodingTooling {
    pub registry: ToolRegistry,
    pub workspace_root: PathBuf,
    pub engine: crate::coding::change_engine::ChangeEngine,
    testing: crate::legacy::testing::TestingTooling,
}

impl CodingTooling {
    pub fn new(
        workspace_root: &Path,
        planned_files: &[PathBuf],
        strict: bool,
        command_timeout_secs: u64,
    ) -> Self {
        let mut registry = build_coding_tool_registry(workspace_root);
        install_coding_permission_hook(&mut registry);
        CodingTooling {
            registry,
            workspace_root: workspace_root.to_path_buf(),
            engine: crate::coding::change_engine::ChangeEngine::new(
                workspace_root,
                planned_files,
                strict,
            ),
            testing: crate::legacy::testing::TestingTooling::new(workspace_root, command_timeout_secs),
        }
    }

    /// The policy-checked verification surface (identical authority to the
    /// Testing subagent's command execution).
    pub fn testing(&self) -> &crate::legacy::testing::TestingTooling {
        &self.testing
    }

    /// Execute one policy-checked verification command and record the
    /// authoritative exit code. A denied command never executes: it becomes
    /// an authoritative `denied` record.
    pub async fn execute_verify(
        &mut self,
        raw_args: &str,
        source: VerificationSource,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> VerificationRecord {
        let command = extract_command_arg(raw_args);
        let record = self.testing.execute_command(&command, cancel).await;
        VerificationRecord {
            command: record.command,
            working_directory: record.working_directory,
            exit_code: record.exit_code,
            success: record.success,
            duration_ms: record.duration_ms,
            output: record.output,
            timeout: record.timeout,
            cancelled: record.cancelled,
            denied: record.denied,
            denied_reason: record.denied_reason,
            source,
        }
    }

    /// Execute a read-only tool call through the restricted registry,
    /// resolving relative paths for the path-based inspection tools.
    pub async fn execute_tool(
        &mut self,
        name: &str,
        args: &str,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> String {
        let args = match name {
            "list_files" | "read_file" => {
                let raw = extract_tool_path(args).unwrap_or_else(|| args.to_string());
                resolve_arg_path(&self.workspace_root, &raw)
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

    /// Snapshot the workspace git state (baseline/after observability).
    pub fn check_git_state(&self) -> crate::legacy::testing::GitStateSnapshot {
        self.testing.check_git_state()
    }
}

/// Resolve a tool argument path against the workspace root for INSPECTION
/// tools (absolute passes through, relative joins the root). Inspection is
/// read-only, so a missing file simply yields an error from the tool.
fn resolve_arg_path(workspace_root: &Path, argument: &str) -> String {
    let trimmed = argument.trim();
    if trimmed.is_empty() {
        return workspace_root.to_string_lossy().to_string();
    }
    let path = Path::new(trimmed);
    if path.is_absolute() {
        trimmed.to_string()
    } else {
        workspace_root.join(path).to_string_lossy().to_string()
    }
}

/// Parse a `propose_change` argument string. Accepts the JSON form
/// (`{"path": ..., "old": ..., "new": ...}`) and the pipe form
/// (`path|old|new`).
pub fn parse_proposal_args(arguments: &str) -> Option<(String, String, String)> {
    let trimmed = arguments.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        let path = value.get("path").and_then(|v| v.as_str())?;
        let old = value.get("old").and_then(|v| v.as_str()).unwrap_or("");
        let new = value.get("new").and_then(|v| v.as_str()).unwrap_or("");
        return Some((path.to_string(), old.to_string(), new.to_string()));
    }
    let mut parts = trimmed.splitn(3, '|');
    let path = parts.next()?.trim().trim_matches('"');
    let old = parts.next()?;
    let new = parts.next()?;
    if path.is_empty() {
        return None;
    }
    Some((
        path.to_string(),
        old.trim().to_string(),
        new.trim().to_string(),
    ))
}

/// Extract the command string from a `verify` argument string. Accepts JSON
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

/// Cap and redact command/tool output before it becomes a model observation.
pub fn truncate_and_redact(output: &str, max_chars: usize) -> String {
    let output = redact_secrets_public(output);
    if output.chars().count() <= max_chars {
        output
    } else {
        let head: String = output.chars().take(max_chars).collect();
        format!("{head}\n…[output truncated]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_allows_coding_tools_only() {
        let hook = CodingPermissionHook::new();
        for allowed in [
            "list_files",
            "read_file",
            "git_status",
            "git_diff",
            "propose_change",
            "verify",
        ] {
            assert!(hook.allows(allowed), "{allowed} must be allowed");
        }
        for denied in [
            "create_file",
            "edit_file",
            "run_command",
            "git_commit",
            "git_checkout",
            "playwright",
            "patch",
        ] {
            assert!(!hook.allows(denied), "{denied} must not be allowed");
            let ctx = ToolContext::new(denied, "{}");
            let decision = hook.check(&ctx);
            assert!(
                decision.is_denied(),
                "{denied} must be denied, got {:?}",
                decision
            );
        }
    }

    #[test]
    fn test_registry_is_exactly_the_coding_surface() {
        let dir = tempfile::tempdir().unwrap();
        let tooling = CodingTooling::new(dir.path(), &[], false, 10);
        let names = tooling.registry.names();
        for allowed in [
            "list_files",
            "read_file",
            "git_status",
            "git_diff",
            "propose_change",
            "verify",
        ] {
            assert!(
                names.contains(&allowed.to_string()),
                "coding registry must expose {allowed}: {:?}",
                names
            );
        }
        for denied in ["create_file", "edit_file", "run_command", "git_commit"] {
            assert!(
                !names.contains(&denied.to_string()),
                "coding registry must not expose {denied}"
            );
        }
        assert_eq!(names.len(), 6, "only the six coding tools: {:?}", names);
    }

    #[tokio::test]
    async fn test_registry_denies_raw_mutation_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = CodingTooling::new(dir.path(), &[], false, 10);
        for tool in ["create_file", "edit_file", "run_command", "git_commit"] {
            let result = tooling.execute_tool(tool, "|content", None).await;
            assert!(
                result.starts_with("Error"),
                "{tool} must fail in the coding registry, got: {result}"
            );
        }
    }

    #[tokio::test]
    async fn test_intercepted_tools_fail_if_ever_called_directly() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = CodingTooling::new(dir.path(), &[], false, 10);
        for tool in ["propose_change", "verify"] {
            let result = tooling.execute_tool(tool, "{}", None).await;
            assert!(
                result.contains("runtime-intercepted"),
                "{tool} must fail loudly when called through the registry, got: {result}"
            );
        }
    }

    #[test]
    fn test_parse_proposal_args_json_and_pipe() {
        let json =
            parse_proposal_args(r#"{"path": "src/lib.rs", "old": "fn add", "new": "fn sub"}"#);
        assert_eq!(
            json,
            Some((
                "src/lib.rs".to_string(),
                "fn add".to_string(),
                "fn sub".to_string()
            ))
        );
        let piped = parse_proposal_args(r#""src/lib.rs"|fn add|fn sub"#);
        assert_eq!(
            piped,
            Some((
                "src/lib.rs".to_string(),
                "fn add".to_string(),
                "fn sub".to_string()
            ))
        );
        assert_eq!(parse_proposal_args(""), None);
    }
}

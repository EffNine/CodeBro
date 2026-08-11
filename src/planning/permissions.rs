//! Planning capability boundary (Sprint 30E).
//!
//! The Planning subagent is strictly READ-ONLY. This module defines an
//! explicit, allowlisted permission policy and a restricted tool registry
//! built from the SAME tool implementations the main agent uses — only the
//! available capability set changes. Mutating tools (create_file, edit_file,
//! run_command, git mutations) are neither registered nor permitted.
//!
//! Planning NEVER executes commands. Testing already owns command execution;
//! Planning reasons from Research/Testing evidence and performs targeted
//! reads only when more evidence is required.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use crate::dispatcher::ToolRegistry;
use crate::tools::context::ToolContext;
use crate::tools::hooks::PermissionDecision;
use crate::tools::hooks::PermissionHook;

use super::limits::PLANNING_ALLOWED_TOOLS;

/// Explicit read-only capability boundary for the Planning subagent.
///
/// This is NOT "deny everything unless the name contains read". It is a
/// fixed allowlist of specific tool names. Any tool outside the allowlist is
/// denied with an explicit reason, even if it is registered in the registry
/// (defense in depth — the restricted registry is itself limited to the
/// allowlist).
#[derive(Debug, Clone)]
pub struct PlanningPermissionHook {
    allowed: HashSet<String>,
}

impl PlanningPermissionHook {
    pub fn new() -> Self {
        PlanningPermissionHook {
            allowed: PLANNING_ALLOWED_TOOLS
                .iter()
                .map(|s| s.to_string())
                .collect(),
        }
    }

    /// The set of explicitly allowed tool names.
    pub fn allowed_tools(&self) -> Vec<String> {
        let mut tools: Vec<String> = self.allowed.iter().cloned().collect();
        tools.sort();
        tools
    }

    /// Whether a tool name is on the read-only allowlist.
    pub fn allows(&self, tool: &str) -> bool {
        self.allowed.contains(tool)
    }
}

impl Default for PlanningPermissionHook {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionHook for PlanningPermissionHook {
    fn check(&self, context: &ToolContext) -> PermissionDecision {
        if self.allowed.contains(&context.tool_name) {
            PermissionDecision::Allowed {
                reason: Some("planning read-only allowlist".to_string()),
            }
        } else {
            PermissionDecision::Denied {
                reason: format!(
                    "planning subagent: '{}' is not on the read-only allowlist",
                    context.tool_name
                ),
            }
        }
    }
}

/// Build the restricted tool registry for the Planning subagent.
///
/// Only allowlisted, non-mutating tools are registered. The same `Arc<dyn
/// Tool>` implementations used by the main agent and the Research subagent
/// are reused — nothing is duplicated. The [`PlanningPermissionHook`] is
/// installed as the global permission hook so the boundary is enforced even
/// if a tool were somehow added later.
pub fn build_planning_tool_registry(workspace_root: &Path) -> ToolRegistry {
    let _ = workspace_root;
    ToolRegistry::new()
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff))
}

/// Register the planning permission hook on a registry.
pub fn install_planning_permission_hook(registry: &mut ToolRegistry) {
    registry.set_global_permission_hook(Box::new(PlanningPermissionHook::new()));
}

/// A registry ready for planning: restricted read-only tool set plus the
/// explicit permission boundary. The workspace root is carried so path-based
/// tools (list_files / read_file) resolve relative arguments against it.
pub struct PlanningTooling {
    pub registry: ToolRegistry,
    pub workspace_root: PathBuf,
}

impl PlanningTooling {
    pub fn new(workspace_root: &Path) -> Self {
        let mut registry = build_planning_tool_registry(workspace_root);
        install_planning_permission_hook(&mut registry);
        PlanningTooling {
            registry,
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// Resolve a tool argument path against the workspace root. Absolute
    /// paths pass through; relative paths are joined onto the workspace root
    /// so planning always inspects the intended repository.
    fn resolve_path(&self, argument: &str) -> String {
        let trimmed = argument.trim();
        if trimmed.is_empty() {
            return self.workspace_root.to_string_lossy().to_string();
        }
        let path = std::path::Path::new(trimmed);
        if path.is_absolute() {
            trimmed.to_string()
        } else {
            self.workspace_root.join(path).to_string_lossy().to_string()
        }
    }

    /// Execute a single tool call through the restricted registry, resolving
    /// relative paths for the path-based read-only tools. Mutating or
    /// executing tools are not registered; an attempt returns an error.
    pub async fn execute(
        &mut self,
        name: &str,
        args: &str,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> String {
        match name {
            "list_files" | "read_file" => {
                let args = resolve_tool_path(args).unwrap_or_else(|| args.to_string());
                let args = self.resolve_path(&args);
                self.execute_registry(name, &args, cancel).await
            }
            _ => self.execute_registry(name, args, cancel).await,
        }
    }

    async fn execute_registry(
        &mut self,
        name: &str,
        args: &str,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> String {
        match self.registry.execute_stream(name, args, cancel).await {
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
}

/// Extract the `path` argument from a tool-call argument string. Accepts both
/// JSON-wrapped (`{"path": "src"}`) and raw (`src`) forms.
fn resolve_tool_path(arguments: &str) -> Option<String> {
    let trimmed = arguments.trim();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let Some(path) = value.get("path").and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
        if let Some(dir) = value.get("dir").and_then(|v| v.as_str()) {
            return Some(dir.to_string());
        }
    }
    Some(trimmed.trim_matches('"').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_allows_read_only_tools() {
        let hook = PlanningPermissionHook::new();
        assert!(hook.allows("list_files"));
        assert!(hook.allows("read_file"));
        assert!(hook.allows("git_status"));
        assert!(hook.allows("git_diff"));
    }

    #[test]
    fn test_allowlist_denies_mutating_and_executing_tools() {
        let hook = PlanningPermissionHook::new();
        for denied in [
            "create_file",
            "edit_file",
            "run_command",
            "git_commit",
            "git_checkout",
        ] {
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
    fn test_planning_registry_only_contains_allowed_tools() {
        let dir = tempfile::tempdir().unwrap();
        let tooling = PlanningTooling::new(dir.path());
        let names = tooling.registry.names();
        assert!(names.contains(&"list_files".to_string()));
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"git_status".to_string()));
        assert!(names.contains(&"git_diff".to_string()));
        for denied in [
            "create_file",
            "edit_file",
            "run_command",
            "playwright",
            "patch",
        ] {
            assert!(
                !names.contains(&denied.to_string()),
                "planning registry must not expose {}",
                denied
            );
        }
        assert_eq!(names.len(), 4, "only the four read-only tools: {:?}", names);
    }

    #[tokio::test]
    async fn test_planning_registry_denies_unregistered_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = PlanningTooling::new(dir.path());
        for tool in ["create_file", "edit_file", "run_command", "git_commit"] {
            let result = tooling.execute(tool, "|content", None).await;
            assert!(
                result.starts_with("Error"),
                "{} must fail in the restricted registry, got: {}",
                tool,
                result
            );
        }
    }

    #[tokio::test]
    async fn test_planning_registry_executes_read_file_with_path_resolution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello planning").unwrap();
        let mut tooling = PlanningTooling::new(dir.path());
        // Relative path resolved against the workspace root.
        let result = tooling.execute("read_file", "note.txt", None).await;
        assert!(
            result.contains("hello planning"),
            "read_file must read the real file, got: {}",
            result
        );
        let abs = dir.path().join("note.txt").to_string_lossy().to_string();
        let result = tooling.execute("read_file", &abs, None).await;
        assert!(result.contains("hello planning"));
    }

    #[tokio::test]
    async fn test_permission_hook_denies_mutating_tool_even_if_registered() {
        // Defense in depth: even if a mutating tool were somehow registered,
        // the explicit permission boundary must deny it before execution.
        let dir = tempfile::tempdir().unwrap();
        let mut registry =
            build_planning_tool_registry(dir.path()).register(Arc::new(crate::tools::CreateFile));
        install_planning_permission_hook(&mut registry);

        let result = registry
            .execute_stream("create_file", "/tmp/should-not-exist-planning|boom", None)
            .await;
        assert!(
            result.is_err(),
            "permission hook must deny create_file even when registered"
        );
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("read-only allowlist"),
            "denial must cite the explicit allowlist boundary, got: {}",
            message
        );
        assert!(
            !std::path::Path::new("/tmp/should-not-exist-planning").exists(),
            "denied tool must not execute"
        );
    }
}

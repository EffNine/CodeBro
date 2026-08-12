//! Review capability boundary (Sprint 30G).
//!
//! Review is STRICTLY READ-ONLY. It may never call create_file, edit_file,
//! run_command, propose_change, verify, or any git mutation tool. The
//! allowlist is exactly four inspection tools. Defense in depth: even if a
//! mutating tool were hypothetically registered, the permission hook denies it.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use crate::dispatcher::ToolRegistry;
use crate::tools::context::ToolContext;
use crate::tools::hooks::PermissionDecision;
use crate::tools::hooks::PermissionHook;

use super::limits::REVIEW_ALLOWED_TOOLS;

/// Explicit read-only capability boundary for the Review subagent.
#[derive(Debug, Clone)]
pub struct ReviewPermissionHook {
    allowed: HashSet<String>,
}

impl ReviewPermissionHook {
    pub fn new() -> Self {
        ReviewPermissionHook {
            allowed: REVIEW_ALLOWED_TOOLS.iter().map(|s| s.to_string()).collect(),
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

impl Default for ReviewPermissionHook {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionHook for ReviewPermissionHook {
    fn check(&self, context: &ToolContext) -> PermissionDecision {
        if self.allowed.contains(&context.tool_name) {
            PermissionDecision::Allowed {
                reason: Some("review read-only allowlist".to_string()),
            }
        } else {
            PermissionDecision::Denied {
                reason: format!(
                    "review subagent: '{}' is not on the read-only allowlist (only list_files, read_file, git_status, git_diff are permitted)",
                    context.tool_name
                ),
            }
        }
    }
}

/// Build the restricted tool registry for the Review subagent.
pub fn build_review_tool_registry(_workspace_root: &Path) -> ToolRegistry {
    let _ = _workspace_root;
    ToolRegistry::new()
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff))
}

/// Register the review permission hook on a registry.
pub fn install_review_permission_hook(registry: &mut ToolRegistry) {
    registry.set_global_permission_hook(Box::new(ReviewPermissionHook::new()));
}

/// A registry ready for review: restricted tool set plus the explicit
/// permission boundary.
pub struct ReviewTooling {
    pub registry: ToolRegistry,
    pub workspace_root: PathBuf,
}

impl ReviewTooling {
    pub fn new(workspace_root: &Path) -> Self {
        let mut registry = build_review_tool_registry(workspace_root);
        install_review_permission_hook(&mut registry);
        ReviewTooling {
            registry,
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// Resolve a tool argument path against the workspace root. Absolute paths
    /// pass through; relative paths are joined onto the workspace root.
    fn resolve_path(&self, argument: &str) -> String {
        let trimmed = argument.trim();
        if trimmed.is_empty() {
            return self.workspace_root.to_string_lossy().to_string();
        }
        let path = Path::new(trimmed);
        if path.is_absolute() {
            trimmed.to_string()
        } else {
            self.workspace_root.join(path).to_string_lossy().to_string()
        }
    }

    /// Execute a single tool call through the restricted registry.
    pub async fn execute(
        &mut self,
        name: &str,
        args: &str,
        cancel: Option<crate::cancellation::CancellationToken>,
    ) -> String {
        let args = match name {
            "list_files" | "read_file" => {
                let raw = extract_tool_path(args).unwrap_or_else(|| args.to_string());
                self.resolve_path(&raw)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allowlist_allows_read_only_tools() {
        let hook = ReviewPermissionHook::new();
        assert!(hook.allows("list_files"));
        assert!(hook.allows("read_file"));
        assert!(hook.allows("git_status"));
        assert!(hook.allows("git_diff"));
    }

    #[test]
    fn test_allowlist_denies_mutating_tools() {
        let hook = ReviewPermissionHook::new();
        for denied in [
            "create_file",
            "edit_file",
            "run_command",
            "propose_change",
            "verify",
            "git_commit",
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
    fn test_registry_only_has_read_only_tools() {
        let dir = tempfile::tempdir().unwrap();
        let tooling = ReviewTooling::new(dir.path());
        let names = tooling.registry.names();
        assert!(names.contains(&"list_files".to_string()));
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"git_status".to_string()));
        assert!(names.contains(&"git_diff".to_string()));
        for denied in [
            "create_file",
            "edit_file",
            "run_command",
            "propose_change",
            "verify",
        ] {
            assert!(
                !names.contains(&denied.to_string()),
                "registry must not expose {denied}"
            );
        }
    }

    #[tokio::test]
    async fn test_registry_denies_unregistered_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = ReviewTooling::new(dir.path());
        let result = tooling.execute("create_file", "/tmp/x|content", None).await;
        assert!(
            result.starts_with("Error"),
            "create_file must fail, got: {result}"
        );
    }

    #[tokio::test]
    async fn test_permission_hook_denies_hypothetical_mutation() {
        // Defense in depth: even if a mutating tool were somehow registered,
        // the explicit permission boundary must deny it before execution.
        let dir = tempfile::tempdir().unwrap();
        let mut registry =
            build_review_tool_registry(dir.path()).register(Arc::new(crate::tools::CreateFile));
        install_review_permission_hook(&mut registry);

        let result = registry
            .execute_stream("create_file", "/tmp/should-not-exist.txt|boom", None)
            .await;
        assert!(
            result.is_err(),
            "permission hook must deny create_file even when registered"
        );
        let message = result.unwrap_err().to_string();
        assert!(
            message.contains("read-only allowlist"),
            "denial must cite the explicit allowlist boundary, got: {message}"
        );
        assert!(
            !Path::new("/tmp/should-not-exist.txt").exists(),
            "denied tool must not execute"
        );
    }

    #[tokio::test]
    async fn test_review_reads_real_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello review").unwrap();
        let mut tooling = ReviewTooling::new(dir.path());
        let result = tooling.execute("read_file", "note.txt", None).await;
        assert!(
            result.contains("hello review"),
            "read_file must read the real file, got: {result}"
        );
    }
}

//! Research capability boundary (Sprint 30C).
//!
//! The Research subagent is read-only. This module defines an explicit,
//! allowlisted permission policy and a restricted tool registry built from the
//! SAME tool implementations the main agent uses — only the available
//! capability set changes. Mutating tools (create_file, edit_file,
//! run_command, git mutations) are neither registered nor permitted.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use crate::dispatcher::ToolRegistry;
use crate::tools::context::ToolContext;
use crate::tools::hooks::PermissionDecision;
use crate::tools::hooks::PermissionHook;

use super::limits::RESEARCH_ALLOWED_TOOLS;

/// Explicit read-only capability boundary for the Research subagent.
///
/// This is NOT "deny everything unless the name contains read". It is a
/// fixed allowlist of specific tool names. Any tool outside the allowlist is
/// denied with an explicit reason, even if it is registered in the registry
/// (defense in depth — the restricted registry is itself limited to the
/// allowlist).
#[derive(Debug, Clone)]
pub struct ResearchPermissionHook {
    allowed: HashSet<String>,
}

impl ResearchPermissionHook {
    pub fn new() -> Self {
        ResearchPermissionHook {
            allowed: RESEARCH_ALLOWED_TOOLS
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

impl Default for ResearchPermissionHook {
    fn default() -> Self {
        Self::new()
    }
}

impl PermissionHook for ResearchPermissionHook {
    fn check(&self, context: &ToolContext) -> PermissionDecision {
        if self.allowed.contains(&context.tool_name) {
            PermissionDecision::Allowed {
                reason: Some("research read-only allowlist".to_string()),
            }
        } else {
            PermissionDecision::Denied {
                reason: format!(
                    "research subagent: '{}' is not on the read-only allowlist",
                    context.tool_name
                ),
            }
        }
    }
}

/// Build the restricted tool registry for the Research subagent.
///
/// Only allowlisted, non-mutating tools are registered. The same `Arc<dyn
/// Tool>` implementations used by the main agent are reused — nothing is
/// duplicated. The `ResearchPermissionHook` is installed as the global
/// permission hook so the boundary is enforced even if a tool were somehow
/// added later.
pub fn build_research_tool_registry(workspace_root: &Path) -> ToolRegistry {
    let root = workspace_root.to_path_buf();
    let registry = ToolRegistry::new()
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff));

    registry
}

/// Register the research permission hook on a registry.
pub fn install_research_permission_hook(registry: &mut ToolRegistry) {
    registry.set_global_permission_hook(Box::new(ResearchPermissionHook::new()));
}

/// A registry ready for research: restricted tool set plus the explicit
/// permission boundary. The workspace root is carried so path-based tools
/// (list_files / read_file) resolve relative arguments against it.
pub struct ResearchTooling {
    pub registry: ToolRegistry,
    pub workspace_root: PathBuf,
}

impl ResearchTooling {
    pub fn new(workspace_root: &Path) -> Self {
        let mut registry = build_research_tool_registry(workspace_root);
        install_research_permission_hook(&mut registry);
        ResearchTooling {
            registry,
            workspace_root: workspace_root.to_path_buf(),
        }
    }

    /// Resolve a tool argument path against the workspace root. Absolute
    /// paths pass through; relative paths are joined onto the workspace root
    /// so research always inspects the intended repository.
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
    /// relative paths for the path-based read-only tools.
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

/// Extract the `path` argument from a tool-call argument string. Accepts both
/// JSON-wrapped (`{"path": "src"}`) and raw (`src`) forms.
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
        let hook = ResearchPermissionHook::new();
        assert!(hook.allows("list_files"));
        assert!(hook.allows("read_file"));
        assert!(hook.allows("git_status"));
        assert!(hook.allows("git_diff"));
    }

    #[test]
    fn test_allowlist_denies_mutating_tools() {
        let hook = ResearchPermissionHook::new();
        for denied in ["create_file", "edit_file", "run_command"] {
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
    fn test_research_registry_only_has_read_only_tools() {
        let dir = tempfile::tempdir().unwrap();
        let tooling = ResearchTooling::new(dir.path());
        let names = tooling.registry.names();
        assert!(names.contains(&"list_files".to_string()));
        assert!(names.contains(&"read_file".to_string()));
        assert!(names.contains(&"git_status".to_string()));
        for denied in ["create_file", "edit_file", "run_command", "playwright"] {
            assert!(
                !names.contains(&denied.to_string()),
                "research registry must not expose {}",
                denied
            );
        }
    }

    #[tokio::test]
    async fn test_research_registry_denies_unregistered_tools() {
        let dir = tempfile::tempdir().unwrap();
        let mut tooling = ResearchTooling::new(dir.path());
        let result = tooling.execute("create_file", "|content", None).await;
        assert!(
            result.starts_with("Error"),
            "create_file must fail in the restricted registry, got: {}",
            result
        );
    }

    #[tokio::test]
    async fn test_research_registry_executes_read_file_with_path_resolution() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello research").unwrap();
        let mut tooling = ResearchTooling::new(dir.path());
        // Relative path resolved against the workspace root.
        let result = tooling.execute("read_file", "note.txt", None).await;
        assert!(
            result.contains("hello research"),
            "read_file must read the real file, got: {}",
            result
        );
        // Absolute path also works.
        let abs = dir.path().join("note.txt").to_string_lossy().to_string();
        let result = tooling.execute("read_file", &abs, None).await;
        assert!(result.contains("hello research"));
    }

    #[tokio::test]
    async fn test_permission_hook_denies_mutating_tool_even_if_registered() {
        // Defense in depth: even if a mutating tool were somehow registered,
        // the explicit permission boundary must deny it before execution.
        let dir = tempfile::tempdir().unwrap();
        let mut registry =
            build_research_tool_registry(dir.path()).register(Arc::new(crate::tools::CreateFile));
        install_research_permission_hook(&mut registry);

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
            "denial must cite the explicit allowlist boundary, got: {}",
            message
        );
        assert!(
            !std::path::Path::new("/tmp/should-not-exist.txt").exists(),
            "denied tool must not execute"
        );
    }
}

//! Coding capability boundary and change engine (Sprint 30F).
//!
//! Coding is the FIRST mutating subagent, so the boundary is the strictest:
//!
//! - The tool allowlist is fixed: read-only inspection (list_files, read_file,
//!   git_status, git_diff) plus the two runtime-intercepted surfaces
//!   (`propose_change` and `verify`). Raw filesystem mutation tools
//!   (`create_file`, `edit_file`), arbitrary execution (`run_command`) and git
//!   mutations are neither registered nor permitted.
//! - ALL mutation goes through [`ChangeEngine`], the single mutation seam.
//!   Existing-file changes ride [`ChangePlan`](crate::tools::ChangePlan) /
//!   [`PatchEngine`](crate::tools::PatchEngine); file creation — which
//!   PatchEngine cannot reconstruct from a non-existent on-disk base — goes
//!   through the engine's documented controlled creation seam
//!   ([`ChangeEngine::create_file`]). The engine enforces:
//!   - the workspace-root path boundary (traversal is denied),
//!   - plan adherence (out-of-plan changes are flagged — and denied outright
//!     in strict mode),
//!   - no blind overwrite (an existing file requires a non-empty `old` text),
//!   - unambiguous matches (an `old` text that occurs more than once is
//!     denied — the model must supply more context),
//!   - stale-state protection (a prepared change refuses to apply if the file
//!     changed between preparation and application).
//! - Verification is runtime-intercepted: `verify` never reaches a shell
//!   directly; it is executed through the
//!   [`TestingTooling`](crate::testing::TestingTooling) command policy so the
//!   authoritative exit code is captured and mutation commands are impossible.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;

use crate::dispatcher::ToolRegistry;
use crate::tools::context::ToolContext;
use crate::tools::hooks::PermissionDecision;
use crate::tools::hooks::PermissionHook;
use crate::tools::shell::redact_secrets_public;

use super::contract::VerificationRecord;
use super::contract::VerificationSource;
use super::limits::CODING_ALLOWED_TOOLS;

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
        .register(Arc::new(crate::tools::ListFiles))
        .register(Arc::new(crate::tools::ReadFile))
        .register(Arc::new(crate::tools::GitStatus))
        .register(Arc::new(crate::tools::GitDiff))
        .register(Arc::new(RuntimeInterceptedTool::new("propose_change")))
        .register(Arc::new(RuntimeInterceptedTool::new("verify")))
}

/// Register the coding permission hook on a registry.
pub fn install_coding_permission_hook(registry: &mut ToolRegistry) {
    registry.set_global_permission_hook(Box::new(CodingPermissionHook::new()));
}

/// A change prepared against the current file content, ready to apply.
///
/// Preparation is strictly READ-ONLY: nothing is written until [`ChangeEngine::apply`]
/// runs, and only then if the file still matches the prepared snapshot
/// (stale-state protection).
#[derive(Debug, Clone)]
pub struct PreparedChange {
    /// The target file, resolved to an absolute path.
    pub path: PathBuf,
    /// Whether the file did not exist at preparation time.
    pub created: bool,
    /// Whether the target is outside the plan's affected files.
    pub unplanned: bool,
    /// Readable diff preview (also the tool result the model observes).
    pub preview: String,
    /// The complete file content captured at preparation time ("" for
    /// created files) — the rollback source and the stale-check snapshot.
    pub backup: String,
    /// The exact text that must be uniquely present in the file.
    pub old: String,
    /// The exact replacement text.
    pub new: String,
    /// The resulting full file content.
    pub full_new: String,
}

/// The workspace-bound mutation engine behind `propose_change`.
///
/// Existing-file writes route through a
/// [`ChangePlan`](crate::tools::ChangePlan) built by
/// [`crate::tools::PatchEngine`]; created files go through the engine's
/// controlled creation seam ([`ChangeEngine::create_file`]). The engine never
/// calls `fs::write` on source files outside these two seams — it is the ONLY
/// mutation path Coding uses.
pub struct ChangeEngine {
    workspace_root: PathBuf,
    planned_files: Vec<PathBuf>,
    strict: bool,
}

impl ChangeEngine {
    pub fn new(workspace_root: &Path, planned_files: &[PathBuf], strict: bool) -> Self {
        let root = workspace_root.to_path_buf();
        let planned_files = planned_files
            .iter()
            .filter_map(|p| resolve_path(&root, &p.display().to_string()).ok())
            .collect();
        ChangeEngine {
            workspace_root: root,
            planned_files,
            strict,
        }
    }

    /// The strict-plan flag: when true, out-of-plan changes are denied.
    pub fn strict(&self) -> bool {
        self.strict
    }

    /// Resolve a tool argument path and enforce the workspace-root boundary.
    /// Absolute paths outside the root (and any `..` traversal) are denied.
    pub fn resolve(&self, argument: &str) -> crate::error::Result<PathBuf> {
        resolve_path(&self.workspace_root, argument).map_err(|e| {
            crate::error::CodeBroError::Permission(format!("change engine path boundary: {e}"))
        })
    }

    /// Prepare a change against the CURRENT file content (read-only).
    ///
    /// Enforcement happens here, before any mutation:
    /// - path boundary,
    /// - plan adherence (strict mode denies out-of-plan targets),
    /// - existing files require a unique, non-empty `old` match (no blind
    ///   overwrite, no ambiguous edits),
    /// - created files require `old` to be empty.
    pub fn prepare(
        &self,
        path: &str,
        old: &str,
        new: &str,
    ) -> crate::error::Result<PreparedChange> {
        let abs = self.resolve(path)?;
        let unplanned = !self.planned_files.contains(&abs) && !self.planned_files.is_empty();
        if self.strict && unplanned {
            return Err(crate::error::CodeBroError::Permission(format!(
                "plan adherence: '{}' is not among the plan's affected files and strict plan adherence is enabled",
                abs.display()
            )));
        }

        if abs.exists() {
            let content = std::fs::read_to_string(&abs).map_err(|e| {
                crate::error::CodeBroError::Patch(format!(
                    "Cannot read {} for change proposal: {e}",
                    abs.display()
                ))
            })?;
            if old.is_empty() {
                return Err(crate::error::CodeBroError::Permission(format!(
                    "blind overwrite denied: '{}' already exists — provide the exact `old` text to replace, not an empty match",
                    abs.display()
                )));
            }
            let occurrences = content.matches(old).count();
            if occurrences == 0 {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "stale content: the provided `old` text does not occur in '{}'",
                    abs.display()
                )));
            }
            if occurrences > 1 {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "ambiguous change denied: the provided `old` text occurs {occurrences} times in '{}' — supply more surrounding context for a unique match",
                    abs.display()
                )));
            }
            let full_new = content.replacen(old, new, 1);
            let plan = crate::tools::ChangePlan::propose_between(&abs, &content, &full_new)?;
            Ok(PreparedChange {
                path: abs,
                created: false,
                unplanned,
                preview: plan.preview().to_string(),
                backup: content,
                old: old.to_string(),
                new: new.to_string(),
                full_new,
            })
        } else {
            if !old.trim().is_empty() {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "cannot create '{}': the file does not exist — to create it pass old=\"\" with the full content as new",
                    abs.display()
                )));
            }
            let plan = crate::tools::ChangePlan::propose_between(&abs, "", new)?;
            Ok(PreparedChange {
                path: abs,
                created: true,
                unplanned,
                preview: plan.preview().to_string(),
                backup: String::new(),
                old: String::new(),
                new: new.to_string(),
                full_new: new.to_string(),
            })
        }
    }

    /// Apply a prepared change — but ONLY if the file still matches the
    /// preparation-time snapshot. A file changed by anyone else between
    /// preparation and application is never clobbered.
    ///
    /// This is the prepare/apply seam: preparation stays read-only and
    /// reversible. Existing-file changes are a single
    /// [`ChangePlan::apply`](crate::tools::ChangePlan::apply) routed through
    /// [`PatchEngine`](crate::tools::PatchEngine); created files use the
    /// engine's controlled creation seam
    /// ([`ChangeEngine::create_file`]) because PatchEngine reconstructs the
    /// new content from a file's on-disk base, which cannot exist for a file
    /// being created.
    pub fn apply(&self, prepared: &PreparedChange) -> crate::error::Result<String> {
        let current = if prepared.created {
            if prepared.path.exists() {
                return Err(crate::error::CodeBroError::Patch(format!(
                    "stale state: '{}' was created by someone else since the proposal — refusing to overwrite it",
                    prepared.path.display()
                )));
            }
            String::new()
        } else {
            std::fs::read_to_string(&prepared.path).map_err(|e| {
                crate::error::CodeBroError::Patch(format!(
                    "Cannot read {} for change apply: {e}",
                    prepared.path.display()
                ))
            })?
        };
        if current != prepared.backup {
            return Err(crate::error::CodeBroError::Patch(format!(
                "stale state: '{}' changed since the proposal — refusing to apply over unknown content",
                prepared.path.display()
            )));
        }
        if prepared.created {
            return self.create_file(prepared);
        }
        let mut plan = crate::tools::ChangePlan::propose_between(
            &prepared.path,
            &prepared.backup,
            &prepared.full_new,
        )?;
        plan.apply()
    }

    /// The CONTROLLED creation path of the engine — the sole filesystem write
    /// that does not ride a [`ChangePlan`](crate::tools::ChangePlan).
    ///
    /// File creation cannot go through [`PatchEngine`](crate::tools::PatchEngine):
    /// [`PatchEngine::apply`](crate::tools::PatchEngine::apply) reconstructs
    /// the new content from the file's on-disk base, which does not exist for
    /// a file being created. Creation therefore stays INSIDE the engine as a
    /// single, explicitly documented write, still protected by the
    /// prepare/apply staleness check that ran in
    /// [`ChangeEngine::apply`] (a file created by someone else between prepare
    /// and apply is never clobbered). Nothing else in Coding writes files.
    fn create_file(&self, prepared: &PreparedChange) -> crate::error::Result<String> {
        if let Some(parent) = prepared.path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    crate::error::CodeBroError::Patch(format!(
                        "Cannot create parent dirs for {}: {e}",
                        prepared.path.display()
                    ))
                })?;
            }
        }
        std::fs::write(&prepared.path, &prepared.full_new)
            .map_err(|e| crate::error::CodeBroError::Patch(format!("Failed to write file: {e}")))?;
        Ok(format!("Patch applied to {}", prepared.path.display()))
    }
}

/// A registry ready for coding: restricted tool set plus the explicit
/// permission boundary. Carries the mutation engine (workspace-bound, plan
/// aware) and the policy-checked verification tooling.
pub struct CodingTooling {
    pub registry: ToolRegistry,
    pub workspace_root: PathBuf,
    pub engine: ChangeEngine,
    testing: crate::testing::TestingTooling,
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
            engine: ChangeEngine::new(workspace_root, planned_files, strict),
            testing: crate::testing::TestingTooling::new(workspace_root, command_timeout_secs),
        }
    }

    /// The policy-checked verification surface (identical authority to the
    /// Testing subagent's command execution).
    pub fn testing(&self) -> &crate::testing::TestingTooling {
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
    pub fn check_git_state(&self) -> crate::testing::GitStateSnapshot {
        self.testing.check_git_state()
    }
}

/// Resolve a change path against the workspace root, denying any escape.
fn resolve_path(workspace_root: &Path, argument: &str) -> crate::error::Result<PathBuf> {
    let trimmed = argument.trim();
    if trimmed.is_empty() {
        return Err(crate::error::CodeBroError::Permission(
            "empty path".to_string(),
        ));
    }
    let raw = std::path::Path::new(trimmed);
    let candidate = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        workspace_root.join(raw)
    };
    // Deny `..` traversal outright (a real component, not a normalized one).
    for component in candidate.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(crate::error::CodeBroError::Permission(format!(
                "path traversal denied: '{}'",
                trimmed
            )));
        }
    }
    // Deny paths that escape the workspace root after normalization.
    if !candidate.starts_with(workspace_root) {
        return Err(crate::error::CodeBroError::Permission(format!(
            "outside workspace root: '{}'",
            trimmed
        )));
    }
    Ok(candidate)
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

    fn write(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

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

    #[test]
    fn test_engine_rejects_path_traversal_and_outside_paths() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        for bad in [
            "../escape.txt".to_string(),
            "sub/../../escape.txt".to_string(),
            dir.path()
                .parent()
                .unwrap()
                .join("outside.txt")
                .to_string_lossy()
                .to_string(),
        ] {
            assert!(
                engine.resolve(&bad).is_err(),
                "'{bad}' must be denied by the path boundary"
            );
        }
    }

    #[test]
    fn test_engine_resolves_relative_paths_into_root() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let abs = engine.resolve("src/lib.rs").unwrap();
        assert_eq!(abs, dir.path().join("src/lib.rs"));
    }

    #[test]
    fn test_engine_modify_prepare_is_read_only_and_applies() {
        let dir = tempfile::tempdir().unwrap();
        write(
            &dir.path().join("main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        );
        let engine = ChangeEngine::new(dir.path(), &[PathBuf::from("main.rs")], false);

        let prepared = engine
            .prepare("main.rs", "println!(\"hi\")", "println!(\"hello\")")
            .unwrap();
        assert!(!prepared.created);
        assert!(!prepared.unplanned);
        assert!(prepared
            .preview
            .contains("+fn main() { println!(\"hello\"); }"));
        // Preparation must NOT touch the file.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "fn main() { println!(\"hi\"); }\n"
        );
        engine.apply(&prepared).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "fn main() { println!(\"hello\"); }\n"
        );
    }

    #[test]
    fn test_engine_denies_blind_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "keep this content");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("main.rs", "", "replacement")
            .expect_err("empty old text on an existing file must be denied");
        assert!(err.to_string().contains("blind overwrite"), "got: {err}");
    }

    #[test]
    fn test_engine_denies_ambiguous_match() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "let x = 1;\nlet y = 1;\n");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("main.rs", "= 1;", "= 2;")
            .expect_err("a non-unique old text must be denied");
        assert!(err.to_string().contains("ambiguous"), "got: {err}");
    }

    #[test]
    fn test_engine_rejects_stale_old_text_at_prepare() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "alpha\nbeta\n");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("main.rs", "gamma", "delta")
            .expect_err("an old text absent from the file must be denied");
        assert!(err.to_string().contains("stale"), "got: {err}");
    }

    #[test]
    fn test_engine_apply_refuses_stale_state_between_prepare_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("main.rs"), "original line\n");
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let prepared = engine
            .prepare("main.rs", "original line", "changed line")
            .unwrap();
        // Someone else modifies the file between preparation and application.
        write(&dir.path().join("main.rs"), "someone else's content\n");
        let err = engine
            .apply(&prepared)
            .expect_err("apply must refuse stale content");
        assert!(err.to_string().contains("stale state"), "got: {err}");
        // The foreign content is preserved.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("main.rs")).unwrap(),
            "someone else's content\n"
        );
    }

    #[test]
    fn test_engine_create_new_file_and_backup() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let prepared = engine
            .prepare("src/new.rs", "", "pub fn fresh() {}\n")
            .unwrap();
        assert!(prepared.created);
        assert!(prepared.backup.is_empty());
        // The observable diff is produced at PREPARE time (read-only): the
        // file is still absent, yet the preview is the full addition.
        assert!(
            prepared.preview.contains("+pub fn fresh() {}"),
            "created-file preview must be the observable diff: {}",
            prepared.preview
        );
        engine.apply(&prepared).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/new.rs")).unwrap(),
            "pub fn fresh() {}\n"
        );
    }

    #[test]
    fn test_engine_apply_refuses_stale_create_between_prepare_and_apply() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let prepared = engine
            .prepare("src/new.rs", "", "pub fn fresh() {}\n")
            .unwrap();
        assert!(prepared.created);
        // Someone else creates the file between preparation and application.
        write(&dir.path().join("src/new.rs"), "someone else's file\n");
        let err = engine
            .apply(&prepared)
            .expect_err("apply must refuse to clobber a file created since the proposal");
        assert!(err.to_string().contains("stale state"), "got: {err}");
        // The foreign file is preserved untouched — the session never wrote.
        assert_eq!(
            std::fs::read_to_string(dir.path().join("src/new.rs")).unwrap(),
            "someone else's file\n"
        );
    }

    #[test]
    fn test_engine_denies_create_outside_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        // Traversal escape is denied.
        let traversal = engine
            .prepare("../outside.rs", "", "content")
            .expect_err("a traversal create must be denied");
        assert!(
            traversal.to_string().contains("path boundary"),
            "got: {traversal}"
        );
        // An absolute path outside the root is denied.
        let outside = dir.path().parent().unwrap().join("outside-create.rs");
        let outside_err = engine
            .prepare(&outside.to_string_lossy(), "", "content")
            .expect_err("a create outside the workspace root must be denied");
        assert!(
            outside_err.to_string().contains("path boundary"),
            "got: {outside_err}"
        );
        assert!(!outside.exists(), "no file may be written outside the root");
    }

    #[test]
    fn test_engine_create_requires_empty_old() {
        let dir = tempfile::tempdir().unwrap();
        let engine = ChangeEngine::new(dir.path(), &[], false);
        let err = engine
            .prepare("src/new.rs", "text", "content")
            .expect_err("creating a file with non-empty old must be denied");
        assert!(err.to_string().contains("does not exist"), "got: {err}");
    }

    #[test]
    fn test_engine_marks_unplanned_changes_but_applies_by_default() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("planned.rs"), "planned\n");
        write(&dir.path().join("extra.rs"), "extra\n");
        let engine = ChangeEngine::new(dir.path(), &[PathBuf::from("planned.rs")], false);

        let planned = engine.prepare("planned.rs", "planned", "planned!").unwrap();
        assert!(!planned.unplanned);

        let extra = engine.prepare("extra.rs", "extra", "extra!").unwrap();
        assert!(
            extra.unplanned,
            "a file outside the plan must be flagged as unplanned"
        );
        engine.apply(&extra).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.path().join("extra.rs")).unwrap(),
            "extra!\n"
        );
    }

    #[test]
    fn test_engine_strict_mode_denies_unplanned_changes() {
        let dir = tempfile::tempdir().unwrap();
        write(&dir.path().join("planned.rs"), "planned\n");
        write(&dir.path().join("extra.rs"), "extra\n");
        let engine = ChangeEngine::new(dir.path(), &[PathBuf::from("planned.rs")], true);
        let err = engine
            .prepare("extra.rs", "extra", "extra!")
            .expect_err("strict mode must deny out-of-plan changes");
        assert!(err.to_string().contains("plan adherence"), "got: {err}");
        assert!(engine.prepare("planned.rs", "planned", "planned!").is_ok());
    }
}

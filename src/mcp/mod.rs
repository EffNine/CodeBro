//! CodeBro Engineering Runtime — MCP server interface.
//!
//! Exposes the engineering runtime (project identity, verified facts,
//! engineering memory, guarded change application) over the Model Context
//! Protocol so that battle-tested agents — Claude Code, OpenCode, Codex,
//! Cursor, Goose — can act as the frontend while CodeBro owns project
//! truth, persistent engineering context and guarded mutations.
//!
//! Run with `codebro serve` (stdio transport). See `docs/design/MCP_SERVER.md`
//! for the roadmap.

use std::path::PathBuf;
use std::sync::Arc;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The CodeBro MCP server: a stateless router over the engineering runtime.
///
/// Every tool call constructs a fresh view of the runtime from the workspace
/// root, so the server holds no mutable state and can be shared freely.
#[derive(Clone)]
pub struct CodeBroMcpServer {
    workspace_root: PathBuf,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl CodeBroMcpServer {
    /// Create a server bound to a workspace root.
    pub fn new(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            tool_router: Self::tool_router(),
        }
    }

    /// Load the project identity for the workspace, tolerating absence.
    fn identity_snapshot(&self) -> (bool, Option<crate::project_identity::ProjectIdentity>) {
        let mut identity = crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        match identity.load() {
            Ok(_) => (true, Some(identity.snapshot())),
            Err(e) => {
                tracing::debug!("no project identity for {}: {e}", self.workspace_root.display());
                (false, None)
            }
        }
    }

    /// Build the fact store for the workspace. Facts are frozen models;
    /// a persisted `.codebro/facts.json` is restored if present.
    fn fact_store(&self) -> crate::fact_store::FactStore {
        let path = self.workspace_root.join(".codebro/facts.json");
        match std::fs::read(&path) {
            Ok(bytes) => match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
                Ok(model) => crate::fact_store::FactStore::from_model(&model),
                Err(e) => {
                    tracing::warn!("ignoring unparseable {}: {e}", path.display());
                    crate::fact_store::FactStore::empty()
                }
            },
            Err(_) => crate::fact_store::FactStore::empty(),
        }
    }

    // ── Tool 1: workspace context ─────────────────────────────────────

    /// Return the workspace context: project identity, workspace root and
    /// the state of the engineering runtime for this project. Call this
    /// first to understand what project the agent is operating in.
    #[tool(description = "Return the workspace context: project identity, workspace root, and engineering runtime state. Call this first to orient the agent in the project.")]
    async fn workspace_context(&self) -> Result<CallToolResult, McpError> {
        let (identity_loaded, identity) = self.identity_snapshot();
        let store = self.fact_store();
        let counts = store.collection().counts();

        let payload = json!({
            "workspace_root": self.workspace_root.display().to_string(),
            "identity_loaded": identity_loaded,
            "project_identity": identity,
            "fact_counts": {
                "workspaces": counts.workspaces,
                "modules": counts.modules,
                "packages": counts.packages,
                "symbols": counts.symbols,
                "tests": counts.tests,
                "build_targets": counts.build_targets,
                "dependencies": counts.dependencies,
                "relationships": counts.relationships,
                "references": counts.references,
                "diagnostics": counts.diagnostics,
                "architecture_rules": counts.architecture_rules,
                "total": counts.total,
            },
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 2: engineering facts ─────────────────────────────────────

    /// Query verified engineering facts about the project (modules,
    /// packages, symbols, tests, dependencies, architecture rules).
    #[tool(description = "Query verified engineering facts about the project: modules, packages, symbols, tests, dependencies, architecture rules. Optionally filter by fact kind.")]
    async fn engineering_facts(
        &self,
        Parameters(args): Parameters<FactsArgs>,
    ) -> Result<CallToolResult, McpError> {
        use crate::engineering_facts::FactKind;

        let store = self.fact_store();
        let counts = store.collection().counts();

        let kind = match args.kind.as_deref() {
            None => None,
            Some(raw) => {
                let parsed = match raw.to_ascii_lowercase().as_str() {
                    "workspace" => FactKind::Workspace,
                    "module" => FactKind::Module,
                    "package" => FactKind::Package,
                    "symbol" => FactKind::Symbol,
                    "test" => FactKind::Test,
                    "build_target" | "buildtarget" => FactKind::BuildTarget,
                    "dependency" => FactKind::Dependency,
                    "relationship" => FactKind::Relationship,
                    "reference" => FactKind::Reference,
                    "diagnostic" => FactKind::Diagnostic,
                    "architecture_rule" | "architecturerule" => FactKind::ArchitectureRule,
                    other => {
                        return Err(McpError::invalid_params(
                            format!("unknown fact kind '{other}'"),
                            None,
                        ))
                    }
                };
                Some(parsed)
            }
        };

        let ids = kind
            .map(|k| store.query().by_kind(k).to_vec())
            .unwrap_or_default();

        let payload = json!({
            "counts": {
                "workspaces": counts.workspaces,
                "modules": counts.modules,
                "packages": counts.packages,
                "symbols": counts.symbols,
                "tests": counts.tests,
                "build_targets": counts.build_targets,
                "dependencies": counts.dependencies,
                "relationships": counts.relationships,
                "references": counts.references,
                "diagnostics": counts.diagnostics,
                "architecture_rules": counts.architecture_rules,
                "total": counts.total,
            },
            "kind": args.kind,
            "matching_ids": ids.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 3: engineering memory ────────────────────────────────────

    /// Resolve engineering memory (decisions, constraints, prior context)
    /// relevant to a task query.
    #[tool(description = "Resolve relevant engineering memory for a task: recorded decisions, constraints, and prior implementation context with confidence scores. Pass task keywords to retrieve the most relevant entries.")]
    async fn engineering_memory(
        &self,
        Parameters(args): Parameters<MemoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let identity = crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        let mut memory =
            crate::engineering_memory::EngineeringMemoryRuntime::new(&self.workspace_root, identity);
        let _ = memory.load(); // absent store is not an error for a read query

        let context = memory.resolve_for_task(&args.task_keywords, &args.active_file_tags);
        let payload = json!({
            "entries": context.entries,
            "budget_remaining": context.budget_remaining,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 4: guarded change application ────────────────────────────

    /// Apply a guarded change to a workspace file through the change
    /// engine: path-boundary enforcement, plan awareness, stale-content
    /// protection and audit. No blind overwrites.
    #[tool(description = "Apply a guarded change to a single workspace file through the change engine. Provide the exact old text to replace (or empty old to create a new file). Enforces workspace boundary and refuses stale or ambiguous edits.")]
    async fn apply_change(
        &self,
        Parameters(args): Parameters<ChangeArgs>,
    ) -> Result<CallToolResult, McpError> {
        // Plan-less, non-strict engine: boundary + staleness enforcement only.
        let engine = crate::coding::permissions::ChangeEngine::new(&self.workspace_root, &[], false);

        let prepared = engine
            .prepare(&args.path, &args.old, &args.new)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let result = engine
            .apply(&prepared)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![ContentBlock::text(
            result,
        )]))
    }
}

/// Argument schema for `engineering_facts`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct FactsArgs {
    /// Optional fact kind filter: workspace, module, package, symbol,
    /// test, build_target, dependency, relationship, reference,
    /// diagnostic, architecture_rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
}

/// Argument schema for `engineering_memory`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct MemoryArgs {
    /// Task keywords used to resolve relevant memory entries.
    #[serde(default)]
    pub task_keywords: Vec<String>,
    /// Active-file tags to bias resolution toward current context.
    #[serde(default)]
    pub active_file_tags: Vec<String>,
}

/// Argument schema for `apply_change`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct ChangeArgs {
    /// Path to the file, relative to the workspace root.
    pub path: String,
    /// Exact existing text to replace; empty to create a new file.
    pub old: String,
    /// Replacement text.
    pub new: String,
}

#[tool_handler]
impl rmcp::ServerHandler for CodeBroMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "CodeBro Engineering Runtime: persistent engineering context, project \
             intelligence and guarded code operations for AI coding agents.",
        )
    }
}

/// Run the MCP server over stdio until the client disconnects.
pub async fn serve(workspace_root: PathBuf) -> anyhow::Result<()> {
    let server = CodeBroMcpServer::new(workspace_root);
    let service = server
        .serve(stdio())
        .await
        .inspect_err(|e| tracing::error!("MCP server failed to start: {e}"))?;
    service.waiting().await?;
    Ok(())
}

/// Convenience: `Arc`-wrapped server instance, kept for future shared-state
/// extensions (e.g. a live workspace session).
#[allow(dead_code)]
type SharedServer = Arc<CodeBroMcpServer>;

//! CodeBro Engineering Context — MCP server interface.
//!
//! Exposes the engineering context layer (project identity, verified facts,
//! engineering memory, optional guarded changes) over the Model Context
//! Protocol so that battle-tested agents — Claude Code, OpenCode, Codex,
//! Cursor, Goose — can act as the frontend while CodeBro owns project
//! truth and persistent engineering context.
//!
//! Run with `codebro serve` (stdio transport). See `docs/design/MCP_SERVER.md`
//! for the roadmap.

use std::path::PathBuf;
use std::sync::Arc;

pub mod facts;

use rmcp::{
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    transport::stdio,
    ErrorData as McpError, ServiceExt,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The CodeBro MCP server: a stateless router over the engineering context
/// layer.
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
        let mut identity =
            crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        match identity.load() {
            Ok(_) => (true, Some(identity.snapshot())),
            Err(e) => {
                tracing::debug!(
                    "no project identity for {}: {e}",
                    self.workspace_root.display()
                );
                (false, None)
            }
        }
    }

    /// Build the fact store for the workspace. Facts are frozen models;
    /// a persisted `.codebro/facts.json` is restored if present.
    fn fact_store(&self) -> crate::fact_store::FactStore {
        let path = self.workspace_root.join(".codebro/facts.json");
        match std::fs::read(&path) {
            Ok(bytes) => {
                match serde_json::from_slice::<crate::engineering_facts::FactsModel>(&bytes) {
                    Ok(model) => crate::fact_store::FactStore::from_model(&model),
                    Err(e) => {
                        tracing::warn!("ignoring unparseable {}: {e}", path.display());
                        crate::fact_store::FactStore::empty()
                    }
                }
            }
            Err(_) => crate::fact_store::FactStore::empty(),
        }
    }

    // ── Tool 1: workspace context ─────────────────────────────────────

    /// Return the workspace context: project identity, workspace root and
    /// the state of the engineering runtime for this project. Call this
    /// first to understand what project the agent is operating in.
    #[tool(
        description = "Return the workspace context: project identity, workspace root, and engineering runtime state. Call this first to orient the agent in the project."
    )]
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

    // ── Tool 2: engineering facts (semantic retrieval) ───────────────

    /// Search verified engineering facts semantically: symbols, modules,
    /// tests, packages, build targets, dependencies. Returns compact fact
    /// records with names, paths, locations and provenance — not raw ids.
    #[tool(
        description = "Search verified engineering facts about the project: symbols, modules, tests, packages, build targets, dependencies. Provide a query (symbol/module name or path fragment); optionally filter by kind and path. Returns compact fact records with locations and provenance."
    )]
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

        let facts = crate::mcp::facts::search(
            &store,
            &crate::mcp::facts::FactSearch {
                query: &args.query,
                kind,
                path: args.path.as_deref(),
                limit: args.limit.unwrap_or(crate::mcp::facts::DEFAULT_LIMIT),
            },
        );

        let payload = json!({
            "store": {
                "modules": counts.modules,
                "symbols": counts.symbols,
                "tests": counts.tests,
                "packages": counts.packages,
                "dependencies": counts.dependencies,
                "build_targets": counts.build_targets,
                "total": counts.total,
            },
            "query": args.query,
            "kind": args.kind,
            "path": args.path,
            "returned": facts.len(),
            "facts": facts,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }

    // ── Tool 3: engineering memory ────────────────────────────────────

    /// Resolve engineering memory (decisions, constraints, prior context)
    /// relevant to a task query.
    #[tool(
        description = "Resolve relevant engineering memory for a task: recorded decisions, constraints, and prior implementation context with confidence scores, source and tags. Pass task keywords to retrieve the most relevant entries."
    )]
    async fn engineering_memory(
        &self,
        Parameters(args): Parameters<MemoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let identity = crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        let mut memory = crate::engineering_memory::EngineeringMemoryRuntime::new(
            &self.workspace_root,
            identity,
        );
        let _ = memory.load(); // absent store is not an error for a read query

        let context = memory.resolve_for_task(&args.task_keywords, &args.active_file_tags);

        // Enrich resolved entries with provenance (source + tags) from the
        // persisted snapshot so agents can judge trustworthiness. This is a
        // read-side projection only — the memory system is untouched.
        let snapshot = memory.snapshot();
        let entries: Vec<serde_json::Value> = context
            .entries
            .iter()
            .map(|entry| {
                let src = snapshot.iter().find(|s| s.key == entry.key);
                json!({
                    "key": entry.key,
                    "value": entry.value,
                    "confidence": entry.confidence,
                    "tier": entry.tier,
                    "source": src.and_then(|s| s.metadata.source.clone()),
                    "tags": src.map(|s| s.metadata.tags.clone()).unwrap_or_default(),
                })
            })
            .collect();

        let payload = json!({
            "entries": entries,
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
    #[tool(
        description = "Apply a guarded change to a single workspace file through the change engine. Provide the exact old text to replace (or empty old to create a new file). Enforces workspace boundary and refuses stale or ambiguous edits."
    )]
    async fn apply_change(
        &self,
        Parameters(args): Parameters<ChangeArgs>,
    ) -> Result<CallToolResult, McpError> {
        // Plan-less, non-strict engine: boundary + staleness enforcement only.
        let engine =
            crate::coding::permissions::ChangeEngine::new(&self.workspace_root, &[], false);

        let prepared = engine
            .prepare(&args.path, &args.old, &args.new)
            .map_err(|e| McpError::invalid_params(e.to_string(), None))?;
        let result = engine
            .apply(&prepared)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![ContentBlock::text(result)]))
    }

    // ── Tool 5: record engineering memory (guarded write) ─────────────

    /// Record or update an engineering memory entry. Values are
    /// secret-redacted before storage; the entry is persisted to
    /// `.codebro/engineering_memory.json`.
    #[tool(
        description = "Record or update an engineering memory entry (decision, constraint, context). Values are secret-redacted before storage. Pass the same key to update an existing entry. Persisted to .codebro/engineering_memory.json."
    )]
    async fn record_memory(
        &self,
        Parameters(args): Parameters<RecordMemoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let key = args.key.trim();
        if key.is_empty() {
            return Err(McpError::invalid_params("key must not be empty", None));
        }
        if args.value.trim().is_empty() {
            return Err(McpError::invalid_params("value must not be empty", None));
        }
        const MAX_VALUE_LEN: usize = 64 * 1024;
        if args.value.len() > MAX_VALUE_LEN {
            return Err(McpError::invalid_params(
                format!("value exceeds {MAX_VALUE_LEN} bytes"),
                None,
            ));
        }

        // Hardening: redact secrets before anything touches storage.
        let value = crate::tools::shell::redact_secrets_public(&args.value);

        let mut tags = args.tags.clone();
        tags.sort();
        tags.dedup();

        let mut metadata = crate::engineering_memory::types::EngineeringMemoryMetadata::new()
            .with_confidence(args.confidence.clamp(0.0, 1.0))
            .with_importance(args.importance.clamp(0.0, 1.0));
        for tag in &tags {
            metadata = metadata.with_tag(tag);
        }
        if let Some(source) = args.source.as_deref() {
            metadata = metadata.with_source(source);
        }

        let identity = crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        let mut memory = crate::engineering_memory::EngineeringMemoryRuntime::new(
            &self.workspace_root,
            identity,
        );
        let _ = memory.load();

        // Deterministic id from the key: upsert semantics.
        let id = format!("mem::{key}");
        let exists = memory.snapshot().iter().any(|e| e.id == id);

        if exists {
            memory
                .update(&id, value)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        } else {
            let entry = crate::engineering_memory::types::EngineeringMemoryEntry::new(
                id.clone(),
                key,
                value,
            )
            .with_metadata(metadata);
            memory
                .record(entry)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        }
        memory
            .persist()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let action = if exists { "updated" } else { "recorded" };
        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "memory {action}: {key}"
        ))]))
    }

    // ── Tool 6: delete engineering memory (guarded write) ─────────────

    /// Delete an engineering memory entry by its exact key.
    #[tool(
        description = "Delete an engineering memory entry by its exact key. Persisted to .codebro/engineering_memory.json."
    )]
    async fn delete_memory(
        &self,
        Parameters(args): Parameters<DeleteMemoryArgs>,
    ) -> Result<CallToolResult, McpError> {
        let key = args.key.trim();
        if key.is_empty() {
            return Err(McpError::invalid_params("key must not be empty", None));
        }
        let id = format!("mem::{key}");

        let identity = crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        let mut memory = crate::engineering_memory::EngineeringMemoryRuntime::new(
            &self.workspace_root,
            identity,
        );
        let _ = memory.load();

        let exists = memory.snapshot().iter().any(|e| e.id == id);
        if !exists {
            return Err(McpError::invalid_params(
                format!("no entry for key '{key}'"),
                None,
            ));
        }
        memory
            .delete(&id)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;
        memory
            .persist()
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![ContentBlock::text(format!(
            "memory deleted: {key}"
        ))]))
    }

    // ── Tool 7: memory statistics ─────────────────────────────────────

    /// Return read-only statistics about the engineering memory store:
    /// entry count, configured token budget, tag distribution, average
    /// confidence, and oldest/newest entry timestamps.
    #[tool(
        description = "Return read-only statistics about the engineering memory store: number of entries, total token budget, tag distribution, average confidence, and oldest/newest entry timestamps. Call this to judge whether engineering memory holds meaningful state before relying on it."
    )]
    async fn memory_stats(&self) -> Result<CallToolResult, McpError> {
        let identity = crate::project_identity::ProjectIdentityRuntime::new(&self.workspace_root);
        let mut memory = crate::engineering_memory::EngineeringMemoryRuntime::new(
            &self.workspace_root,
            identity,
        );
        let _ = memory.load(); // absent store is not an error for a read query

        let total_budget = crate::engineering_memory::resolver::DEFAULT_TOKEN_BUDGET;
        let entries = memory.snapshot();

        // Tag distribution (deterministic: sorted).
        let mut tag_counts: std::collections::BTreeMap<String, usize> =
            std::collections::BTreeMap::new();
        let mut confidence_sum = 0.0f64;
        let mut oldest: Option<u64> = None;
        let mut newest: Option<u64> = None;
        let mut with_source = 0usize;
        for e in &entries {
            confidence_sum += e.metadata.confidence;
            for tag in &e.metadata.tags {
                *tag_counts.entry(tag.clone()).or_insert(0) += 1;
            }
            if e.metadata.source.is_some() {
                with_source += 1;
            }
            oldest = Some(oldest.map_or(e.created_at, |o: u64| o.min(e.created_at)));
            newest = Some(newest.map_or(e.created_at, |n: u64| n.max(e.created_at)));
        }
        let avg_confidence = if entries.is_empty() {
            0.0
        } else {
            confidence_sum / entries.len() as f64
        };

        let payload = json!({
            "entry_count": entries.len(),
            "total_budget": total_budget,
            "entries_with_source": with_source,
            "avg_confidence": (avg_confidence * 100.0).round() / 100.0,
            "oldest_created_at": oldest,
            "newest_created_at": newest,
            "tags": tag_counts,
        });

        Ok(CallToolResult::success(vec![ContentBlock::text(
            serde_json::to_string_pretty(&payload)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?,
        )]))
    }
}

/// Argument schema for `engineering_facts`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct FactsArgs {
    /// Required query: a symbol/module/test name, a name fragment, or a
    /// path fragment (matched case-insensitively).
    pub query: String,
    /// Optional fact kind filter: workspace, module, package, symbol,
    /// test, build_target, dependency, relationship, reference,
    /// diagnostic, architecture_rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Optional path substring filter (e.g. "coding/permissions").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Maximum results returned; defaults to 10, capped at 50.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<usize>,
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

/// Argument schema for `record_memory`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct RecordMemoryArgs {
    /// Stable key for this memory entry (e.g. "architecture:change-engine").
    pub key: String,
    /// Full memory value: the decision, constraint or context.
    pub value: String,
    /// Associative tags for filtering; sorted and de-duplicated server-side.
    #[serde(default)]
    pub tags: Vec<String>,
    /// Confidence score in [0.0, 1.0]; clamped server-side. Default 0.5.
    #[serde(default = "default_half")]
    pub confidence: f64,
    /// Importance score in [0.0, 1.0]; clamped server-side. Default 0.5.
    #[serde(default = "default_half")]
    pub importance: f64,
    /// Optional provenance source (e.g. "sprint-31-review").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// Argument schema for `delete_memory`.
#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema)]
pub struct DeleteMemoryArgs {
    /// Exact key of the entry to delete.
    pub key: String,
}

fn default_half() -> f64 {
    0.5
}

#[tool_handler]
impl rmcp::ServerHandler for CodeBroMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "You are connected to CodeBro, the engineering context & memory layer for THIS \
             workspace. It maintains a verified fact store (symbols, modules, packages, tests, \
             build targets, dependencies), project identity, and persistent engineering memory \
             recorded by agents.\n\
             \n\
             WHEN TO USE CODEBRO:\n\
             - Session start or unfamiliar project -> call codebro_workspace_context once to \
               orient yourself (project identity + fact counts).\n\
             - Any question about project-wide scope: \"how many symbols/tests/modules\", \"what \
               functions/structs exist\", \"where is X defined\", \"which module owns Y\" -> call \
               codebro_engineering_facts with a query (e.g. query=\"ChangeEngine\", \
               kind=\"symbol\", path=\"coding\"). It returns actual fact records with locations, \
               not raw ids. Prefer it over grepping when you need verified, project-wide answers.\n\
             - \"How was X implemented before\", \"what decisions constrain this area\" -> call \
               codebro_engineering_memory with task keywords.\n\
             - Before trusting memory, call codebro_memory_stats to check whether the store holds \
               meaningful state (entry count, confidence, recency).\n\
             - After learning a durable decision or constraint -> record it with \
               codebro_record_memory so future sessions are not amnesic (key like \
               'architecture:area', tags, confidence).\n\
             - Remove stale/wrong entries with codebro_delete_memory by exact key.\n\
             \n\
             WRITE PATH (OPTIONAL):\n\
             - codebro_apply_change is an optional guarded mutation API for controlled/autonomous \
               workflows. Use your native editing tools for normal coding edits. If you do use \
               apply_change, it enforces the workspace boundary and refuses stale/ambiguous \
               edits; create files with old=\"\".\n\
             \n\
             HARD RULES:\n\
             - Never invent symbol names, ids, counts or file locations. If codebro returns \
               empty results, state that facts/memory are empty rather than guessing.\n\
             - Treat engineering_memory content as agent-recorded context (with confidence \
               scores), not as verified engineering truth; engineering_facts are the verified \
               store.",
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

#[cfg(test)]
mod tests {
    use super::*;
    use rmcp::ServerHandler;

    /// Regression test for the P0.1 tool-description defect: an agent
    /// (or a human) once wrote the full user task into a `#[tool(description)]
    /// attribute. Tool descriptions must be short, static, and free of any
    /// prompt/task phrasing so they are useful to LLMs selecting tools.
    #[test]
    fn tool_descriptions_are_static_concise_and_prompt_free() {
        let server = CodeBroMcpServer::new(PathBuf::from("/tmp/unused-root"));

        let expected = [
            "workspace_context",
            "engineering_facts",
            "engineering_memory",
            "apply_change",
            "record_memory",
            "delete_memory",
            "memory_stats",
        ];

        for name in expected {
            let tool = server
                .get_tool(name)
                .unwrap_or_else(|| panic!("tool {name} missing from tool handler"));
            let desc = tool
                .description
                .as_deref()
                .unwrap_or_else(|| panic!("tool {name} has no description"));

            // 1. Static: must be a borrowed (compile-time) string, never a
            //    dynamically constructed string (e.g. built from a prompt).
            assert!(
                matches!(tool.description, Some(std::borrow::Cow::Borrowed(_))),
                "tool {name}: description must be a static &'static str, not dynamic"
            );

            // 2. Concise: LLMs select tools from descriptions; keep them tight.
            assert!(
                desc.len() <= 300,
                "tool {name}: description too long ({desc} chars): {desc}"
            );

            // 3. Prompt-free: must not contain task/prompt phrasing.
            let lower = desc.to_lowercase();
            for banned in [
                "report what you did",
                "step by step",
                "then record",
                "study the existing",
                "add a new",
                "follow the exact same",
                "make the change",
            ] {
                assert!(
                    !lower.contains(banned),
                    "tool {name}: description contains prompt-like text: {banned}"
                );
            }
        }
    }

    /// Every registered tool must be callable via the router (the macro
    /// generates a route per `#[tool]` method; this catches tools that are
    /// declared but not routed).
    #[test]
    fn all_tools_have_router_entries() {
        let server = CodeBroMcpServer::new(PathBuf::from("/tmp/unused-root"));
        for expected in [
            "workspace_context",
            "engineering_facts",
            "engineering_memory",
            "apply_change",
            "record_memory",
            "delete_memory",
            "memory_stats",
        ] {
            assert!(
                server.get_tool(expected).is_some(),
                "tool {expected} missing from tool handler"
            );
        }
    }

    /// P0.3: `memory_stats` must report meaningful state — entry count,
    /// budget, confidence, tags — and degrade gracefully when empty.
    #[tokio::test]
    async fn memory_stats_reports_meaningful_state() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = CodeBroMcpServer::new(dir.path().to_path_buf());

        // Empty store: entry_count 0, budget present, no tags.
        let empty = call_tool_text(&server, "memory_stats", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&empty).expect("valid json");
        assert_eq!(v["entry_count"], 0);
        assert_eq!(v["total_budget"], 500);
        assert_eq!(v["tags"], serde_json::json!({}));

        // Record one entry, then stats must reflect it.
        call_tool_text(
            &server,
            "record_memory",
            json!({
                "key": "architecture:test",
                "value": "Test decision",
                "tags": ["architecture", "test"],
                "confidence": 0.8,
            }),
        )
        .await;
        let after = call_tool_text(&server, "memory_stats", json!({})).await;
        let v: serde_json::Value = serde_json::from_str(&after).expect("valid json");
        assert_eq!(v["entry_count"], 1);
        assert_eq!(v["avg_confidence"], 0.8);
        assert_eq!(v["tags"]["architecture"], 1);
        assert_eq!(v["tags"]["test"], 1);
        assert!(v["oldest_created_at"].is_u64());
        assert!(v["newest_created_at"].is_u64());
    }

    /// P0.2: `engineering_facts` returns actual fact records (with name,
    /// path, provenance) — not raw ids — and honours query/kind/path/limit.
    #[tokio::test]
    async fn engineering_facts_returns_records_not_ids() {
        let dir = tempfile::tempdir().expect("tempdir");
        let server = CodeBroMcpServer::new(dir.path().to_path_buf());

        let out = call_tool_text(
            &server,
            "engineering_facts",
            json!({"query": "zzz-no-such", "limit": 5}),
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&out).expect("valid json");
        // Empty store: no facts, but the shape must be present.
        assert_eq!(v["returned"], 0);
        assert!(v["facts"].is_array());
        assert!(v["store"].is_object());

        // Invalid kind must be rejected, not silently ignored.
        let err = call_tool_err(
            &server,
            "engineering_facts",
            json!({"query": "x", "kind": "bogus"}),
        )
        .await;
        assert!(err.to_string().contains("unknown fact kind"));
    }

    /// Helper: call a tool method directly (the tool methods are private
    /// but visible to the module's own tests) and return its text content.
    async fn call_tool_text(
        server: &CodeBroMcpServer,
        name: &str,
        args: serde_json::Value,
    ) -> String {
        let result = call_tool(server, name, args).await;
        result.expect("tool call succeeds")
    }

    async fn call_tool_err(
        server: &CodeBroMcpServer,
        name: &str,
        args: serde_json::Value,
    ) -> String {
        let result = call_tool(server, name, args).await;
        result.expect_err("tool call must fail")
    }

    /// Drive the tool methods directly with their `Parameters` wrappers.
    async fn call_tool(
        server: &CodeBroMcpServer,
        name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        let result = match name {
            "memory_stats" => {
                let r = server.memory_stats().await.map_err(|e| e.to_string())?;
                text_of(r)
            }
            "record_memory" => {
                let p: RecordMemoryArgs =
                    serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .record_memory(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            "engineering_facts" => {
                let p: FactsArgs = serde_json::from_value(args).map_err(|e| e.to_string())?;
                let r = server
                    .engineering_facts(Parameters(p))
                    .await
                    .map_err(|e| e.to_string())?;
                text_of(r)
            }
            other => return Err(format!("test helper: unsupported tool {other}")),
        };
        Ok(result)
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
}
